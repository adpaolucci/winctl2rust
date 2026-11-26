#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

use guap_gui_iced::{IcedSuiteConfig, run_iced_suite};
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
                    "winctl2rust-iced [--tool <TOOL_ID>] [--light]\n\
                     Launch the cross-platform Iced 0.14 Winctl2Rust Console."
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

fn main() -> iced::Result {
    init_logging("warn");
    let args = parse_launch_args_from(std::env::args().skip(1));
    let mut bundle = collect_default_bundle();

    if let Some(ref initial_tool) = args.tool
        && let Some(pos) = bundle.providers.iter().position(|p| p.tool_id() == initial_tool)
        && pos > 0
    {
        bundle.providers.swap(0, pos);
    }

    let config = IcedSuiteConfig {
        app_title: "Winctl2Rust — Windows Control & Automation Console (Iced)".to_string(),
        dark_mode: !args.light,
        window_size: (1360.0, 820.0),
    };

    run_iced_suite(config, bundle.providers)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use guap_gui_iced::{DataGridMessage, IcedDataGrid};
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
        let parsed = parse_launch_args_from(["--tool", "audio_view", "--light"].into_iter().map(str::to_string));
        assert_eq!(parsed.tool.as_deref(), Some("audio_view"));
        assert!(parsed.light);
    }

    #[test]
    fn iced_data_grid_switches_across_all_22_providers_cleanly() {
        let dir = tempdir().unwrap();
        let bundle = mock_suite_bundle(dir.path().join("clip.db"));
        assert_eq!(bundle.providers.len(), 22);

        let expected_ids: Vec<String> = bundle.providers.iter().map(|p| p.tool_id().to_string()).collect();
        let mut grid = IcedDataGrid::new(bundle.providers)
            .with_dark_mode(true)
            .with_export_dir(dir.path());

        for tool_id in &expected_ids {
            let _ = grid.update(DataGridMessage::SelectTool(tool_id.clone()));
            assert_eq!(grid.active_tool_id(), Some(tool_id.as_str()));
            assert!(
                grid.last_error().is_none(),
                "unexpected IcedDataGrid error for '{tool_id}': {:?}",
                grid.last_error()
            );
            assert!(!grid.columns().is_empty());
        }

        let _ = grid.update(DataGridMessage::SelectTool("display_edid".to_string()));
        assert!(grid.total_rows() > 0);
        let _ = grid.update(DataGridMessage::SelectRow(0));
        assert!(grid.selected_row().is_some());
        let _ = grid.update(DataGridMessage::ToggleDarkMode);
        assert!(!grid.dark_mode());
        let _ = grid.update(DataGridMessage::ToggleHex);
        assert!(!grid.show_hex_pane());
    }
}
