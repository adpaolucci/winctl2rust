#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;

use clap::Parser;
use serde_json::{Value, json};
use tempfile::tempdir;
use winctl2rust_audio::{AudioBackend, MockAudioBackend};
use winctl2rust_cli::{Cli, Commands, run_cli_with_bundle};
use winctl2rust_cmd::{CmdBackend, MockCmdBackend};
use winctl2rust_core::ToolBundle;
use winctl2rust_desktop::fixtures::{
    SyntheticLnkSpec, build_synthetic_lnk, build_synthetic_name_table_format0, build_synthetic_os2_v4,
    build_synthetic_sfnt, encode_utf16_be,
};
use winctl2rust_desktop::lnk::HOTKEYF_CONTROL;
use winctl2rust_desktop::{DesktopBackend, MockDesktopBackend};
use winctl2rust_display::fixtures::synthetic_edid_256_bytes;
use winctl2rust_display::{DisplayBackend, MockDisplayBackend};

struct MockHandles {
    cmd: Arc<MockCmdBackend>,
    audio: Arc<MockAudioBackend>,
    display: Arc<MockDisplayBackend>,
    desktop: Arc<MockDesktopBackend>,
    _dir: tempfile::TempDir,
    db_path: std::path::PathBuf,
}

fn make_mock_bundle() -> (ToolBundle, MockHandles) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("clip.db");

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

    (
        bundle,
        MockHandles {
            cmd,
            audio,
            display,
            desktop,
            _dir: dir,
            db_path,
        },
    )
}

#[test]
fn cli_parses_all_subcommand_groups_and_global_flags() {
    let c1 = Cli::try_parse_from([
        "winctl2rust",
        "--dry-run",
        "--format",
        "json",
        "--filter",
        "active == true",
        "--sort",
        "name:asc",
        "--limit",
        "5",
        "display",
        "monitors",
    ])
    .unwrap();
    assert!(c1.dry_run);
    assert_eq!(c1.limit, Some(5));
    assert!(matches!(c1.command, Commands::Display { .. }));

    let c2 = Cli::try_parse_from(["winctl2rust", "audio", "ctl", "/SetVolume", "Speakers", "50"]).unwrap();
    assert!(matches!(c2.command, Commands::Audio { .. }));

    let c3 = Cli::try_parse_from([
        "winctl2rust",
        "display",
        "monitors",
        "--operation",
        "move",
        "--target",
        "\\\\.\\DISPLAY2",
        "--x",
        "-1920",
        "--y",
        "0",
    ])
    .unwrap();
    assert!(matches!(c3.command, Commands::Display { .. }));

    let c4 = Cli::try_parse_from(["winctl2rust", "gui", "--iced"]).unwrap();
    assert!(matches!(c4.command, Commands::Gui(g) if g.iced));
}

#[test]
fn cli_policy_gates_dry_run_and_mock_mutations_work() {
    let (bundle, handles) = make_mock_bundle();
    let dir = tempdir().unwrap();
    let audit_path = dir.path().join("audit.jsonl");
    let out_json = dir.path().join("script_out.json");

    // 1. Mutating without --allow-mutating --confirm-destructive -> PolicyDenied error
    let denied = Cli::try_parse_from([
        "winctl2rust",
        "--custody",
        audit_path.to_str().unwrap(),
        "cmd",
        "script",
        "--script",
        "beep 440 100",
    ])
    .unwrap();
    let err = run_cli_with_bundle(&denied, &bundle).unwrap_err();
    assert!(err.to_string().contains("policy denied"), "unexpected error: {err}");
    assert!(handles.cmd.executed_scripts.lock().unwrap().is_empty());
    assert!(audit_path.exists());

    // 2. --allow-mutating WITHOUT --confirm-destructive -> still denied
    let half_allowed =
        Cli::try_parse_from(["winctl2rust", "--allow-mutating", "audio", "ctl", "/Mute", "vlc.exe"]).unwrap();
    let err2 = run_cli_with_bundle(&half_allowed, &bundle).unwrap_err();
    assert!(err2.to_string().contains("confirm_destructive"));

    // 3. --dry-run succeeds and writes preview JSON without mutating
    let dry = Cli::try_parse_from([
        "winctl2rust",
        "--dry-run",
        "--output",
        out_json.to_str().unwrap(),
        "cmd",
        "script",
        "--script",
        "beep 440 100",
    ])
    .unwrap();
    run_cli_with_bundle(&dry, &bundle).unwrap();
    let dry_val: Value = serde_json::from_str(&fs::read_to_string(&out_json).unwrap()).unwrap();
    assert_eq!(dry_val["dry_run"], json!(true));
    assert_eq!(dry_val["applied"], json!(false));
    assert!(handles.cmd.executed_scripts.lock().unwrap().is_empty());

    // 4. Read-only audio ctl (/GetPercent) succeeds without --allow-mutating
    let ro_ctl_out = dir.path().join("ctl_get.json");
    let ro_ctl = Cli::try_parse_from([
        "winctl2rust",
        "--output",
        ro_ctl_out.to_str().unwrap(),
        "audio",
        "ctl",
        "/GetPercent",
        "DefaultRenderDevice",
    ])
    .unwrap();
    run_cli_with_bundle(&ro_ctl, &bundle).unwrap();
    let ctl_val: Value = serde_json::from_str(&fs::read_to_string(&ro_ctl_out).unwrap()).unwrap();
    assert_eq!(ctl_val["applied"], json!(false));
    assert_eq!(ctl_val["result"]["exit_code"], json!(650));

    // 5. Real mock mutation succeeds with --allow-mutating --confirm-destructive
    let mut_cmd = Cli::try_parse_from([
        "winctl2rust",
        "--allow-mutating",
        "--confirm-destructive",
        "--custody",
        audit_path.to_str().unwrap(),
        "--output",
        out_json.to_str().unwrap(),
        "cmd",
        "script",
        "--script",
        "beep 440 100",
    ])
    .unwrap();
    run_cli_with_bundle(&mut_cmd, &bundle).unwrap();
    assert_eq!(handles.cmd.executed_scripts.lock().unwrap().len(), 1);

    // 6. Desktop clip_hist + clipboard + audio + display mutations against mock backends
    let clip_out = dir.path().join("clip.json");
    let clip_cmd = Cli::try_parse_from([
        "winctl2rust",
        "--allow-mutating",
        "--confirm-destructive",
        "--output",
        clip_out.to_str().unwrap(),
        "desktop",
        "clip-hist",
        "--operation",
        "capture_current",
        "--db-path",
        handles.db_path.to_str().unwrap(),
    ])
    .unwrap();
    run_cli_with_bundle(&clip_cmd, &bundle).unwrap();
    let clip_val: Value = serde_json::from_str(&fs::read_to_string(&clip_out).unwrap()).unwrap();
    assert_eq!(clip_val["applied"], json!(true));
    assert_eq!(clip_val["captured_count"], json!(2));

    let set_clip = Cli::try_parse_from([
        "winctl2rust",
        "--allow-mutating",
        "--confirm-destructive",
        "desktop",
        "clipboard",
        "--operation",
        "set_text",
        "--text",
        "GUAP-CLI-Test",
    ])
    .unwrap();
    run_cli_with_bundle(&set_clip, &bundle).unwrap();
    assert!(!handles.desktop.action_log.lock().unwrap().is_empty());

    let set_audio = Cli::try_parse_from([
        "winctl2rust",
        "--allow-mutating",
        "--confirm-destructive",
        "audio",
        "view",
        "--action",
        "set_volume",
        "--target",
        "Speakers",
        "--volume-percent",
        "45",
    ])
    .unwrap();
    run_cli_with_bundle(&set_audio, &bundle).unwrap();
    assert!(!handles.audio.action_log.lock().unwrap().is_empty());

    let set_ddc = Cli::try_parse_from([
        "winctl2rust",
        "--allow-mutating",
        "--confirm-destructive",
        "display",
        "ddc",
        "--monitor",
        "GUAP-U27Q",
        "--vcp-code",
        "0x10",
        "--value",
        "70",
    ])
    .unwrap();
    run_cli_with_bundle(&set_ddc, &bundle).unwrap();
    assert!(!handles.display.action_log.lock().unwrap().is_empty());
}

#[test]
fn cli_offline_edid_lnk_font_and_bundle_export_work() {
    let (bundle, _handles) = make_mock_bundle();
    let dir = tempdir().unwrap();

    // 1. Offline EDID binary -> CSV + custody sidecar
    let edid_bin = dir.path().join("edid.bin");
    fs::write(&edid_bin, synthetic_edid_256_bytes()).unwrap();
    let edid_csv = dir.path().join("edid.csv");
    let edid_custody = dir.path().join("edid.csv.custody.json");
    let edid_cli = Cli::try_parse_from([
        "winctl2rust",
        "--format",
        "csv",
        "--output",
        edid_csv.to_str().unwrap(),
        "--custody",
        edid_custody.to_str().unwrap(),
        "display",
        "edid",
        "--file",
        edid_bin.to_str().unwrap(),
    ])
    .unwrap();
    run_cli_with_bundle(&edid_cli, &bundle).unwrap();
    let csv_text = fs::read_to_string(&edid_csv).unwrap();
    assert!(csv_text.contains("GUP"));
    assert!(csv_text.contains("GUAP-U27Q"));
    let custody_json: Value = serde_json::from_str(&fs::read_to_string(&edid_custody).unwrap()).unwrap();
    assert!(
        !custody_json["custody"]["merkle_root_sha256"]
            .as_str()
            .unwrap()
            .is_empty()
    );

    // 2. Offline .lnk shortcut -> JSON
    let lnk_path = dir.path().join("shortcut.lnk");
    let hk = ((u16::from(HOTKEYF_CONTROL)) << 8) | u16::from(b'G');
    let lnk_bytes = build_synthetic_lnk(&SyntheticLnkSpec {
        hot_key_raw: hk,
        unicode: true,
        include_id_list: true,
        include_link_info: true,
        force_no_link_info: false,
        local_base_path: Some("C:\\Tools\\guap.exe"),
        name: Some("GUAP Shortcut"),
        relative_path: Some(".\\guap.exe"),
        working_dir: Some("C:\\Tools"),
        arguments: Some("--run"),
    });
    fs::write(&lnk_path, lnk_bytes).unwrap();
    let lnk_out = dir.path().join("hotkeys.json");
    let lnk_cli = Cli::try_parse_from([
        "winctl2rust",
        "--format",
        "json",
        "--output",
        lnk_out.to_str().unwrap(),
        "desktop",
        "hotkeys",
        "--lnk",
        lnk_path.to_str().unwrap(),
    ])
    .unwrap();
    run_cli_with_bundle(&lnk_cli, &bundle).unwrap();
    let lnk_json: Value = serde_json::from_str(&fs::read_to_string(&lnk_out).unwrap()).unwrap();
    assert_eq!(lnk_json[0]["hotkey"], json!("Ctrl+G"));

    // 3. Offline OpenType font -> JSON
    let font_path = dir.path().join("test.ttf");
    let name_t = build_synthetic_name_table_format0(&[
        (3, 1, 0x0409, 1, encode_utf16_be("CLI Test Sans")),
        (3, 1, 0x0409, 2, encode_utf16_be("Regular")),
    ]);
    let os2 = build_synthetic_os2_v4(400, 5, 0, 0x0040, *b"GUAP");
    let sfnt = build_synthetic_sfnt([0x00, 0x01, 0x00, 0x00], vec![(*b"name", name_t), (*b"OS/2", os2)]);
    fs::write(&font_path, sfnt).unwrap();
    let font_out = dir.path().join("fonts.json");
    let font_cli = Cli::try_parse_from([
        "winctl2rust",
        "--format",
        "json",
        "--output",
        font_out.to_str().unwrap(),
        "desktop",
        "fonts",
        "--file",
        font_path.to_str().unwrap(),
    ])
    .unwrap();
    run_cli_with_bundle(&font_cli, &bundle).unwrap();
    let font_json: Value = serde_json::from_str(&fs::read_to_string(&font_out).unwrap()).unwrap();
    assert_eq!(font_json[0]["family"], json!("CLI Test Sans"));

    // 4. Full 22-table bundle export with Merkle custody manifest
    let bundle_dir = dir.path().join("bundle_export");
    let export_cli = Cli::try_parse_from([
        "winctl2rust",
        "--format",
        "json",
        "--output",
        bundle_dir.to_str().unwrap(),
        "export",
        "--all",
    ])
    .unwrap();
    run_cli_with_bundle(&export_cli, &bundle).unwrap();
    let manifest_path = bundle_dir.join("custody_manifest.json");
    assert!(manifest_path.is_file());
    let manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["artifact_paths"].as_array().unwrap().len(), 22);
}

#[test]
fn real_winctl2rust_binary_e2e_help_dry_run_policy_and_mcp_stdio() {
    let bin = env!("CARGO_BIN_EXE_winctl2rust");

    // 1. --help
    let help = Command::new(bin).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help_out = String::from_utf8_lossy(&help.stdout);
    assert!(help_out.contains("winctl2rust"));
    assert!(help_out.contains("catalog"));

    // 2. cmd script --dry-run -> Exit 0
    let dry = Command::new(bin)
        .args(["cmd", "script", "--script", "beep 440 100", "--dry-run"])
        .output()
        .unwrap();
    assert!(
        dry.status.success(),
        "dry-run failed: {}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let dry_json: Value = serde_json::from_slice(&dry.stdout).unwrap();
    assert_eq!(dry_json["dry_run"], json!(true));
    assert_eq!(dry_json["applied"], json!(false));

    // 3. cmd script without --allow-mutating -> non-zero policy refusal
    let denied = Command::new(bin)
        .args(["cmd", "script", "--script", "beep 440 100"])
        .output()
        .unwrap();
    assert!(!denied.status.success());
    let denied_err = String::from_utf8_lossy(&denied.stderr);
    assert!(denied_err.contains("policy denied"), "stderr: {denied_err}");

    // 4. mcp --tools -> 40 tools
    let tools_out = Command::new(bin).args(["mcp", "--tools"]).output().unwrap();
    assert!(tools_out.status.success());
    let tools: Value = serde_json::from_slice(&tools_out.stdout).unwrap();
    assert_eq!(tools.as_array().unwrap().len(), 40);

    // 5. Real stdin/stdout JSON-RPC 2024-11-05 round-trip
    let mut child = Command::new(bin)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let payload = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
        })
        .to_string(),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }).to_string(),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "cmd_script_parse", "arguments": { "script": "beep 440 100" } }
        })
        .to_string(),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "cmd_script", "arguments": { "script": "beep 440 100" } }
        })
        .to_string(),
    ]
    .join("\n")
        + "\n";

    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout_str = String::from_utf8(output.stdout).unwrap();
    let resps: Vec<Value> = stdout_str
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect();
    assert_eq!(resps.len(), 4);
    assert_eq!(resps[0]["result"]["protocolVersion"], json!("2024-11-05"));
    assert_eq!(resps[1]["result"]["tools"].as_array().unwrap().len(), 40);
    assert_eq!(resps[2]["result"]["isError"], json!(false));
    assert_eq!(resps[3]["error"]["code"], json!(-32003));
}
