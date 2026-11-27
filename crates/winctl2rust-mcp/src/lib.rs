#![forbid(unsafe_code)]

use guap_mcp::{McpServerRegistry, PolicyMode};
use serde_json::Value;
use winctl2rust_core::ToolBundle;

pub const SERVER_NAME: &str = "winctl2rust-mcp";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MCP_PROTOCOL_VERSION: &str = guap_mcp::MCP_PROTOCOL_VERSION;

pub fn collect_default_bundle() -> ToolBundle {
    let mut bundle = ToolBundle::new();
    bundle.merge(winctl2rust_cmd::default_bundle());
    bundle.merge(winctl2rust_audio::default_bundle());
    bundle.merge(winctl2rust_display::default_bundle());
    bundle.merge(winctl2rust_desktop::default_bundle());
    bundle
}

pub fn build_registry_from_bundle(policy: PolicyMode, bundle: &ToolBundle) -> McpServerRegistry {
    let mut registry = McpServerRegistry::new(policy).with_server_info(SERVER_NAME, SERVER_VERSION);
    bundle.register_into(&mut registry);
    registry
}

pub fn build_registry(policy: PolicyMode) -> McpServerRegistry {
    let bundle = collect_default_bundle();
    build_registry_from_bundle(policy, &bundle)
}

pub fn get_tools_manifest(policy: PolicyMode) -> Value {
    build_registry(policy).list_tools()
}

pub fn run_stdio_server(policy: PolicyMode) -> anyhow::Result<()> {
    build_registry(policy).serve_stdio()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::io::Cursor;
    use std::sync::Arc;

    use guap_mcp::{ActionRiskLevel, JsonRpcError, PolicyMode};
    use serde_json::{Value, json};
    use tempfile::tempdir;
    use winctl2rust_audio::{AudioBackend, MockAudioBackend};
    use winctl2rust_cmd::{CmdBackend, MockCmdBackend};
    use winctl2rust_core::{ToolBundle, builtin_suite_catalog};
    use winctl2rust_desktop::{DesktopBackend, MockDesktopBackend};
    use winctl2rust_display::{DisplayBackend, MockDisplayBackend};

    use super::*;

    struct MockSuiteContext {
        cmd: Arc<MockCmdBackend>,
        audio: Arc<MockAudioBackend>,
        display: Arc<MockDisplayBackend>,
        desktop: Arc<MockDesktopBackend>,
        _temp_dir: tempfile::TempDir,
        db_path: std::path::PathBuf,
        bmp_path: std::path::PathBuf,
    }

    fn build_mock_bundle() -> (ToolBundle, MockSuiteContext) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("clip_hist.db");
        let bmp_path = temp_dir.path().join("webcam.bmp");

        let cmd = Arc::new(MockCmdBackend::with_sample_data());
        let audio = Arc::new(MockAudioBackend::with_sample_data());
        let display = Arc::new(MockDisplayBackend::with_sample_data());
        let desktop = Arc::new(MockDesktopBackend::with_sample_data().with_clip_hist_db_path(db_path.clone()));

        let mut bundle = ToolBundle::new();
        bundle.merge(winctl2rust_cmd::bundle(Arc::clone(&cmd) as Arc<dyn CmdBackend>));
        bundle.merge(winctl2rust_audio::bundle(Arc::clone(&audio) as Arc<dyn AudioBackend>));
        bundle.merge(winctl2rust_display::bundle(
            Arc::clone(&display) as Arc<dyn DisplayBackend>
        ));
        bundle.merge(winctl2rust_desktop::bundle(
            Arc::clone(&desktop) as Arc<dyn DesktopBackend>
        ));

        let ctx = MockSuiteContext {
            cmd,
            audio,
            display,
            desktop,
            _temp_dir: temp_dir,
            db_path,
            bmp_path,
        };
        (bundle, ctx)
    }

    #[test]
    fn default_and_mock_bundles_expose_all_22_tools_and_40_mcp_endpoints() {
        let default_bundle = collect_default_bundle();
        assert_eq!(default_bundle.tools.len(), 22);
        assert_eq!(default_bundle.providers.len(), 22);
        assert_eq!(default_bundle.actions.len(), 18);

        let (mock_bundle, _ctx) = build_mock_bundle();
        assert_eq!(mock_bundle.tools.len(), 22);
        assert_eq!(mock_bundle.providers.len(), 22);
        assert_eq!(mock_bundle.actions.len(), 18);

        let mut expected_ids = HashSet::new();
        for tool in builtin_suite_catalog() {
            for tid in tool.table_ids {
                assert!(mock_bundle.find_provider(tid).is_some(), "missing provider '{tid}'");
                expected_ids.insert(*tid);
            }
            for aid in tool.action_ids {
                assert!(mock_bundle.find_action(aid).is_some(), "missing action '{aid}'");
                expected_ids.insert(*aid);
            }
        }
        assert_eq!(expected_ids.len(), 40);

        let manifest = get_tools_manifest(PolicyMode::ReadOnlyOnly);
        let tools = manifest.as_array().unwrap();
        assert_eq!(tools.len(), 40);

        let mut ro_count = 0usize;
        let mut mut_count = 0usize;
        for t in tools {
            let name = t["name"].as_str().unwrap();
            assert!(expected_ids.contains(name), "unexpected tool '{name}'");
            let ro = t["annotations"]["readOnlyHint"].as_bool().unwrap();
            let dest = t["annotations"]["destructiveHint"].as_bool().unwrap();
            assert_eq!(ro, !dest);
            if ro {
                ro_count += 1;
            } else {
                mut_count += 1;
                let props = t["inputSchema"]["properties"].as_object().unwrap();
                assert!(props.contains_key("dry_run"), "missing dry_run on '{name}'");
                assert!(
                    props.contains_key("confirm_destructive"),
                    "missing confirm_destructive on '{name}'"
                );
            }
        }
        // 22 TableProviders + 2 ReadOnly ActionCommands (cmd_script_parse, cmd_run_validate) = 24 readOnlyHint: true
        assert_eq!(ro_count, 24);
        // 16 Mutating / PrivilegedMutating ActionCommands = 16 destructiveHint: true
        assert_eq!(mut_count, 16);
    }

    #[test]
    fn jsonrpc_2024_11_05_roundtrip_and_strict_policy_gates_work() {
        let (bundle, ctx) = build_mock_bundle();
        let registry = build_registry_from_bundle(PolicyMode::ReadOnlyOnly, &bundle);

        let mutating_payloads: Vec<(&str, Value)> = vec![
            ("cmd_script", json!({ "script": "beep 440 100", "dry_run": true })),
            ("cmd_run", json!({ "exe_path": "notepad.exe", "dry_run": true })),
            (
                "cmd_from_proc",
                json!({ "parent_pid": 1024, "child_exe": "notepad.exe", "dry_run": true }),
            ),
            (
                "cmd_atnow",
                json!({ "task_name": "T1", "command": "notepad.exe", "when": "+5m", "dry_run": true }),
            ),
            (
                "audio_view_control",
                json!({ "action": "set_volume", "target": "Speakers", "volume_percent": 50.0, "dry_run": true }),
            ),
            ("audio_ctl", json!({ "command_line": "/Mute vlc.exe", "dry_run": true })),
            (
                "audio_app_config_update",
                json!({ "subkey": "186430e1_0", "volume_percent": 40.0, "dry_run": true }),
            ),
            ("audio_wheel", json!({ "mode": "validate_rules", "dry_run": true })),
            (
                "display_monitors_apply",
                json!({ "operation": "set_primary", "target": "\\\\.\\DISPLAY1", "dry_run": true }),
            ),
            (
                "display_ddc_set",
                json!({ "monitor": "GUAP-U27Q", "vcp_code": "0x10", "value": 75, "dry_run": true }),
            ),
            (
                "desktop_props_control",
                json!({ "operation": "show", "hwnd": "0x00010420", "dry_run": true }),
            ),
            (
                "desktop_windows_control",
                json!({ "operation": "minimize", "hwnd": "0x00010420", "dry_run": true }),
            ),
            (
                "desktop_tree_control",
                json!({ "operation": "set_text", "hwnd": "0x00010422", "text": "Hello", "dry_run": true }),
            ),
            (
                "desktop_clipboard_control",
                json!({ "operation": "set_text", "text": "GUAP", "dry_run": true }),
            ),
            (
                "desktop_clip_hist_control",
                json!({ "operation": "capture_current", "db_path": ctx.db_path.display().to_string(), "dry_run": true }),
            ),
            (
                "desktop_webcam_capture",
                json!({ "device_index": 0, "output_path": ctx.bmp_path.display().to_string(), "format": "bmp", "dry_run": true }),
            ),
        ];
        assert_eq!(mutating_payloads.len(), 16);

        // Verify all 22 tables succeed in ReadOnlyOnly mode
        for provider in &bundle.providers {
            let res = registry.dispatch(provider.tool_id(), &json!({})).unwrap();
            assert!(res["rows"].is_array(), "expected rows array for {}", provider.tool_id());
        }

        // Verify both ReadOnly actions succeed in ReadOnlyOnly mode
        let parsed = registry
            .dispatch("cmd_script_parse", &json!({ "script": "beep 440 100" }))
            .unwrap();
        assert_eq!(parsed["valid"], json!(true));
        let validated = registry
            .dispatch("cmd_run_validate", &json!({ "exe_path": "notepad.exe" }))
            .unwrap();
        assert_eq!(validated["valid"], json!(true));

        // Verify all 16 mutating actions return -32003 under ReadOnlyOnly
        for (name, args) in &mutating_payloads {
            let err = registry.dispatch(name, args).unwrap_err();
            assert_eq!(
                err.code,
                JsonRpcError::POLICY_DENIED,
                "expected -32003 for '{name}' under ReadOnlyOnly"
            );
        }

        // Verify stdio JSON-RPC 2024-11-05 round-trip via serve_reader_writer
        let mut requests = Vec::new();
        requests.push(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "test", "version": "1.0" } }
            })
            .to_string(),
        );
        requests.push(
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            })
            .to_string(),
        );
        requests.push(json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }).to_string());
        requests.push(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }).to_string());
        requests.push(
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": { "name": "display_edid", "arguments": {} }
            })
            .to_string(),
        );
        requests.push(
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": { "name": "cmd_script", "arguments": { "script": "beep 440 100" } }
            })
            .to_string(),
        );

        let input = requests.join("\n") + "\n";
        let mut output_buf = Vec::new();
        registry
            .serve_reader_writer(Cursor::new(input.as_bytes()), &mut output_buf)
            .unwrap();

        let output_str = String::from_utf8(output_buf).unwrap();
        let lines: Vec<Value> = output_str
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .collect();
        // notifications/initialized yields no response -> 5 responses total
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0]["id"], json!(1));
        assert_eq!(lines[0]["result"]["protocolVersion"], json!(MCP_PROTOCOL_VERSION));
        assert_eq!(lines[0]["result"]["serverInfo"]["name"], json!(SERVER_NAME));
        assert_eq!(lines[1]["id"], json!(2));
        assert_eq!(lines[1]["result"], json!({}));
        assert_eq!(lines[2]["id"], json!(3));
        assert_eq!(lines[2]["result"]["tools"].as_array().unwrap().len(), 40);
        assert_eq!(lines[3]["id"], json!(4));
        assert_eq!(lines[3]["result"]["isError"], json!(false));
        assert_eq!(lines[4]["id"], json!(5));
        assert_eq!(lines[4]["error"]["code"], json!(JsonRpcError::POLICY_DENIED));

        // Now test AllowMutating without confirm_destructive -> -32003, with confirm_destructive + dry_run -> preview
        let mut_registry = build_registry_from_bundle(PolicyMode::AllowMutating, &bundle);
        for (name, args) in &mutating_payloads {
            let err = mut_registry.dispatch(name, args).unwrap_err();
            assert_eq!(
                err.code,
                JsonRpcError::POLICY_DENIED,
                "expected -32003 for '{name}' without confirm_destructive"
            );

            let mut confirmed = args.clone();
            confirmed["confirm_destructive"] = json!(true);
            let res = mut_registry.dispatch(name, &confirmed).unwrap();
            if *name != "audio_wheel" {
                assert_eq!(res["dry_run"], json!(true), "dry_run mismatch for {name}");
            }
            assert_eq!(res["applied"], json!(false), "applied mismatch for {name}");
        }

        // Nothing should have been mutated in dry_run
        assert!(ctx.cmd.executed_scripts.lock().unwrap().is_empty());
        assert!(ctx.audio.action_log.lock().unwrap().is_empty());
        assert!(ctx.display.action_log.lock().unwrap().is_empty());
        assert!(ctx.desktop.action_log.lock().unwrap().is_empty());
        assert!(!ctx.bmp_path.exists());

        // Real mock mutations succeed when confirm_destructive=true and dry_run=false
        let mut_res = mut_registry
            .dispatch(
                "cmd_script",
                &json!({ "script": "beep 440 100", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(mut_res["applied"], json!(true));
        assert_eq!(ctx.cmd.executed_scripts.lock().unwrap().len(), 1);

        let audio_res = mut_registry
            .dispatch(
                "audio_view_control",
                &json!({ "action": "set_volume", "target": "Speakers", "volume_percent": 42.0, "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(audio_res["applied"], json!(true));
        assert!(!ctx.audio.action_log.lock().unwrap().is_empty());

        let disp_res = mut_registry
            .dispatch(
                "display_ddc_set",
                &json!({ "monitor": "GUAP-U27Q", "vcp_code": "0x10", "value": 80, "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(disp_res["applied"], json!(true));
        assert!(!ctx.display.action_log.lock().unwrap().is_empty());

        let cam_res = mut_registry
            .dispatch(
                "desktop_webcam_capture",
                &json!({ "device_index": 0, "output_path": ctx.bmp_path.display().to_string(), "format": "bmp", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(cam_res["applied"], json!(true));
        assert!(ctx.bmp_path.exists());

        // Verify risk classification consistency
        for a in &bundle.actions {
            let is_ro = matches!(a.command_id(), "cmd_script_parse" | "cmd_run_validate");
            if is_ro {
                assert_eq!(a.risk_level(), ActionRiskLevel::ReadOnly);
            } else if a.command_id() == "cmd_from_proc" {
                assert_eq!(a.risk_level(), ActionRiskLevel::PrivilegedMutating);
            } else {
                assert_eq!(a.risk_level(), ActionRiskLevel::Mutating);
            }
        }
    }
}
