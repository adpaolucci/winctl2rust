#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use guap_gui_win32::{Win32TableHostConfig, run_win32_suite};
use winctl2rust_core::init_logging;
use winctl2rust_mcp::collect_default_bundle;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchArgs {
    pub tool: Option<String>,
    pub light: bool,
}

pub fn parse_launch_args_from<I>(args: I) -> LaunchArgs
where
    I: IntoIterator<Item = String>,
{
    let mut out = LaunchArgs::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--light" => out.light = true,
            "--tool" => {
                if let Some(t) = iter.next() {
                    out.tool = Some(t);
                }
            }
            "--help" | "-h" => {
                println!(
                    "winctl2rust-win32 [--tool <TOOL_ID>] [--light]\n\
                     Launch the native Win32/WinSafe Winctl2Rust Console."
                );
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => {}
        }
    }
    out
}

fn main() -> anyhow::Result<()> {
    init_logging("warn");
    let args = parse_launch_args_from(std::env::args().skip(1));
    let mut bundle = collect_default_bundle();

    if let Some(ref initial_tool) = args.tool
        && let Some(pos) = bundle.providers.iter().position(|p| p.tool_id() == initial_tool)
        && pos > 0
    {
        bundle.providers.swap(0, pos);
    }

    let config = Win32TableHostConfig {
        app_title: "Winctl2Rust — Windows Control & Automation Console".to_string(),
        dark_mode: !args.light,
        show_hex_pane: true,
        default_export_dir: None,
    };

    run_win32_suite(config, bundle.providers, bundle.actions)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use guap_gui_win32::Win32GridState;
    use tempfile::tempdir;
    use winctl2rust_audio::{AudioBackend, MockAudioBackend};
    use winctl2rust_cmd::{CmdBackend, MockCmdBackend};
    use winctl2rust_core::ToolBundle;
    use winctl2rust_desktop::{DesktopBackend, MockDesktopBackend};
    use winctl2rust_display::{DisplayBackend, MockDisplayBackend};

    use super::*;

    fn mock_suite_bundle(db_path: std::path::PathBuf) -> ToolBundle {
        let mut bundle = ToolBundle::new();
        bundle.merge(winctl2rust_cmd::bundle(
            Arc::new(MockCmdBackend::with_sample_data()) as Arc<dyn CmdBackend>
        ));
        bundle.merge(winctl2rust_audio::bundle(
            Arc::new(MockAudioBackend::with_sample_data()) as Arc<dyn AudioBackend>,
        ));
        bundle.merge(winctl2rust_display::bundle(
            Arc::new(MockDisplayBackend::with_sample_data()) as Arc<dyn DisplayBackend>,
        ));
        bundle.merge(winctl2rust_desktop::bundle(Arc::new(
            MockDesktopBackend::with_sample_data().with_clip_hist_db_path(db_path),
        ) as Arc<dyn DesktopBackend>));
        bundle
    }

    #[test]
    fn parses_launch_args_cleanly() {
        let parsed = parse_launch_args_from(["--tool", "display_edid", "--light"].into_iter().map(str::to_string));
        assert_eq!(parsed.tool.as_deref(), Some("display_edid"));
        assert!(parsed.light);
    }

    #[test]
    fn win32_grid_state_loads_all_22_providers_and_18_actions() {
        let dir = tempdir().unwrap();
        let bundle = mock_suite_bundle(dir.path().join("clip.db"));
        assert_eq!(bundle.providers.len(), 22);
        assert_eq!(bundle.actions.len(), 18);

        let expected_ids: Vec<String> = bundle.providers.iter().map(|p| p.tool_id().to_string()).collect();

        let config = Win32TableHostConfig {
            app_title: "Winctl2Rust Test".to_string(),
            dark_mode: true,
            show_hex_pane: true,
            default_export_dir: Some(dir.path().to_path_buf()),
        };

        let mut state = Win32GridState::new(config, bundle.providers, bundle.actions);
        assert_eq!(state.providers.len(), 22);
        assert_eq!(state.actions.len(), 18);

        for tool_id in &expected_ids {
            assert!(state.select_tool_by_id(tool_id), "failed selecting '{tool_id}'");
            assert_eq!(state.active_tool_id(), Some(tool_id.as_str()));
            assert!(
                state.last_error.is_none(),
                "unexpected error for '{tool_id}': {:?}",
                state.last_error
            );
            assert!(!state.columns().is_empty(), "empty columns for '{tool_id}'");
        }

        // Verify EDID row selection & CRLF inspector formatting
        assert!(state.select_tool_by_id("display_edid"));
        assert!(state.row_count() > 0);
        state.selected_row = Some(0);
        let inspector = state.format_inspector_crlf();
        assert!(inspector.contains("\r\n"));
        assert!(inspector.contains("Row ID:"));

        // Ensure run_win32_suite symbol is linked (verifying Common-Controls v6 manifest in test binary)
        type RunSuiteFn = fn(
            Win32TableHostConfig,
            Vec<Arc<dyn guap_table::TableProvider>>,
            Vec<Arc<dyn guap_mcp::ActionCommand>>,
        ) -> anyhow::Result<()>;
        let _fn_ptr: RunSuiteFn = run_win32_suite;
    }
}
