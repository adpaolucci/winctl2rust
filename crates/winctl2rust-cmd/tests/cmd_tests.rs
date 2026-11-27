use std::fs;
use std::sync::{Arc, Mutex};

use chrono::{Local, TimeZone};
use guap_mcp::{ActionCommand, JsonRpcError, McpServerRegistry, PolicyMode};
use guap_table::TableQuery;
use serde_json::json;
use tempfile::tempdir;
use winctl2rust_cmd::atnow::{AtNowAction, AtNowScheduleResult, AtNowTaskInfo, AtNowTaskRequest, ScheduleTimeSpec};
use winctl2rust_cmd::backend::{CmdBackend, UnsupportedCmdBackend};
use winctl2rust_cmd::from_proc::{FromProcessRequest, FromProcessResult, ProcessSummary};
use winctl2rust_cmd::run::{
    LaunchProfile, LaunchResult, PriorityClass, RunAsMode, RunLaunchAction, WindowState, parse_profile_ini,
    parse_profile_json, quote_windows_arg, tokenize_windows_args,
};
use winctl2rust_cmd::script::{
    IniOp, ScriptCommand, ScriptStepResult, ScriptVars, WinSelector, apply_ini_operation, expand_vars, parse_script,
    tokenize_line,
};
use winctl2rust_cmd::{bundle, providers};
use winctl2rust_core::{WinctlError, WinctlResult};

#[derive(Default)]
struct IntegrationBackend {
    calls: Mutex<Vec<String>>,
}

impl CmdBackend for IntegrationBackend {
    fn execute_script(&self, cmd: &ScriptCommand) -> WinctlResult<ScriptStepResult> {
        self.calls.lock().unwrap().push(format!("script:{}", cmd.verb_name()));
        Ok(ScriptStepResult {
            verb: cmd.verb_name().to_string(),
            applied: true,
            detail: "ok".to_string(),
        })
    }

    fn launch_process(&self, profile: &LaunchProfile) -> WinctlResult<LaunchResult> {
        self.calls.lock().unwrap().push(format!("run:{}", profile.exe_path));
        Ok(LaunchResult {
            pid: Some(1234),
            command_line: profile.build_command_line(),
            run_as: format!("{:?}", profile.run_as_mode),
            exit_code: None,
        })
    }

    fn list_candidate_processes(&self) -> WinctlResult<Vec<ProcessSummary>> {
        Ok(vec![ProcessSummary {
            pid: 1024,
            ppid: 640,
            process_name: "explorer.exe".to_string(),
            threads: 40,
            elevation_hint: "Standard/Desktop".to_string(),
        }])
    }

    fn spawn_from_parent(&self, req: &FromProcessRequest) -> WinctlResult<FromProcessResult> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("from_proc:{}:{}", req.parent_pid, req.child_exe));
        Ok(FromProcessResult {
            parent_pid: req.parent_pid,
            child_pid: 2048,
            child_tid: 2049,
            command_line: req.build_command_line(),
        })
    }

    fn list_scheduled_tasks(&self, folder: &str) -> WinctlResult<Vec<AtNowTaskInfo>> {
        Ok(vec![AtNowTaskInfo {
            task_name: "Job1".to_string(),
            folder: folder.to_string(),
            state: "Ready".to_string(),
            enabled: true,
            next_run: None,
            last_run: None,
            last_result: 0,
            command: "notepad.exe".to_string(),
        }])
    }

    fn schedule_task(&self, req: &AtNowTaskRequest) -> WinctlResult<AtNowScheduleResult> {
        self.calls.lock().unwrap().push(format!("atnow:{}", req.task_name));
        Ok(AtNowScheduleResult {
            task_name: req.task_name.clone(),
            folder: req.folder_path.clone(),
            start_boundary: req.run_at.resolve_start_boundary(Local::now())?,
            state: "Ready".to_string(),
        })
    }
}

#[test]
fn script_dsl_parses_all_verbs_comments_quotes_and_vars() {
    let mut vars = ScriptVars::new();
    vars.insert("greeting".to_string(), "Hello \"GUAP\"".to_string());
    vars.insert("app".to_string(), "notepad.exe".to_string());

    let script = r#"
        # Hash comment
        ; Semicolon comment

        speak "Hello \"$app$\"" async
        clipboard set "copied value"
        clipboard clear
        mutesysvolume 2
        setsysvolume 32768
        setvolume 0 16000 32000
        monitor off
        screensaver
        shortcut "C:\Windows\notepad.exe" "C:\Temp" "My Notepad" "--flag" "C:\Work" "icon.ico" 1 max 112 "Desc"
        inifile setval "C:\Temp\test.ini" "Section" "Key" "Value"
        regsetval dword "HKCU\Software\Winctl2Rust" "Enabled" "1"
        service restart Spooler
        win close title "Untitled - Notepad"
        beep 880 150
        wait 50
        exec hide "$app$" /p
        shellexec open "https://guap.ca/winctl2rust/"
        lockws
        exitwin reboot force
    "#;

    let expanded = expand_vars("echo ~$sys.temp$ $greeting$", &vars);
    assert!(!expanded.contains("~$sys.temp$"));

    let tokens = tokenize_line(r#"speak "a \"quoted\" \\path""#).unwrap();
    assert_eq!(tokens, vec!["speak", r#"a "quoted" \path"#]);

    let parsed = parse_script(script, &vars).unwrap();
    assert_eq!(parsed.len(), 19);
    assert!(matches!(
        parsed[0].command,
        ScriptCommand::Speak { async_mode: true, .. }
    ));
    assert!(matches!(parsed[1].command, ScriptCommand::Clipboard { .. }));
    assert!(matches!(parsed[15].command, ScriptCommand::Exec { ref exe, .. } if exe == "notepad.exe"));
    assert!(matches!(parsed[18].command, ScriptCommand::Power { .. }));
}

#[test]
fn script_rejects_bad_syntax_and_r4_prohibitions() {
    let vars = ScriptVars::new();
    assert!(matches!(
        parse_script("speak \"unclosed", &vars),
        Err(WinctlError::ParseError(_))
    ));
    assert!(matches!(
        parse_script("unknownverb 1 2", &vars),
        Err(WinctlError::ParseError(_))
    ));
    assert!(matches!(
        parse_script("exec show RunAsDate.exe /immediate", &vars),
        Err(WinctlError::PolicyDenied(_))
    ));
    assert!(matches!(
        parse_script("exec show tool.exe CreateRemoteThread", &vars),
        Err(WinctlError::PolicyDenied(_))
    ));
}

#[test]
fn script_ini_file_editor_roundtrips_cleanly() {
    let dir = tempdir().unwrap();
    let ini_path = dir.path().join("sub").join("app.ini");

    apply_ini_operation(IniOp::SetVal, &ini_path, "Main", Some("Volume"), Some("75")).unwrap();
    apply_ini_operation(IniOp::SetVal, &ini_path, "Main", Some("Muted"), Some("0")).unwrap();
    apply_ini_operation(IniOp::SetVal, &ini_path, "Other", Some("Flag"), Some("1")).unwrap();
    apply_ini_operation(IniOp::DelVal, &ini_path, "Main", Some("Muted"), None).unwrap();
    apply_ini_operation(IniOp::DelSec, &ini_path, "Other", None, None).unwrap();

    let text = fs::read_to_string(&ini_path).unwrap();
    assert!(text.contains("[Main]"));
    assert!(text.contains("Volume=75"));
    assert!(!text.contains("Muted="));
    assert!(!text.contains("[Other]"));
}

#[test]
fn launch_profile_parsing_validation_quoting_and_env_block() {
    let ini = r#"
        [LaunchProfile]
        Name=TestProfile
        ExePath=C:\Program Files\Demo\app.exe
        Args=--flag "with space" C:\Dir\
        WorkingDir=C:\Work
        RunAsMode=Administrator
        PriorityClass=High
        AffinityMask=0x0F
        WindowState=Maximized
        InheritEnv=false
        CompatibilityLayer=HIGHDPIAWARE WIN8RTM

        [Environment]
        FOO=bar
        BAZ=qux
    "#;

    let profile = parse_profile_ini(ini).unwrap();
    assert_eq!(profile.name, "TestProfile");
    assert_eq!(profile.run_as_mode, RunAsMode::Administrator);
    assert_eq!(profile.priority_class, PriorityClass::High);
    assert_eq!(profile.affinity_mask, Some(0x0F));
    assert_eq!(profile.window_state, WindowState::Maximized);
    assert!(!profile.inherit_env);

    let cmd_line = profile.build_command_line();
    assert_eq!(
        cmd_line,
        r#""C:\Program Files\Demo\app.exe" --flag "with space" C:\Dir\"#
    );

    let utf16 = profile.build_utf16_env_block();
    assert!(utf16.ends_with(&[0, 0]));
    let decoded = String::from_utf16_lossy(&utf16);
    assert!(decoded.contains("FOO=bar\0"));
    assert!(decoded.contains("__COMPAT_LAYER=HIGHDPIAWARE WIN8RTM\0"));

    let bad_affinity = LaunchProfile {
        exe_path: "notepad.exe".to_string(),
        affinity_mask: Some(0),
        ..LaunchProfile::default()
    };
    assert!(matches!(bad_affinity.validate(), Err(WinctlError::InvalidInput(_))));

    let r4_bad = LaunchProfile {
        exe_path: "RunAsDate.exe".to_string(),
        ..LaunchProfile::default()
    };
    assert!(matches!(r4_bad.validate(), Err(WinctlError::PolicyDenied(_))));

    let json_ok =
        parse_profile_json(r#"{"exe_path":"cmd.exe","args":["/c","echo","hi"],"env_overrides":{"A":"1"}}"#).unwrap();
    assert_eq!(json_ok.exe_path, "cmd.exe");
}

#[test]
fn from_process_validation_rejects_pid_zero_and_r4() {
    let bad_pid = FromProcessRequest {
        parent_pid: 0,
        child_exe: "cmd.exe".to_string(),
        args: vec![],
        working_dir: None,
        inherit_handles: false,
        new_console: false,
    };
    assert!(matches!(bad_pid.validate(), Err(WinctlError::InvalidInput(_))));

    let bad_r4 = FromProcessRequest {
        parent_pid: 100,
        child_exe: "cred2rust.exe".to_string(),
        args: vec![],
        working_dir: None,
        inherit_handles: false,
        new_console: false,
    };
    assert!(matches!(bad_r4.validate(), Err(WinctlError::PolicyDenied(_))));
}

#[test]
fn atnow_schedule_time_spec_parses_all_formats() {
    let ref_dt = Local.with_ymd_and_hms(2026, 11, 23, 10, 0, 0).unwrap();

    let now_spec = ScheduleTimeSpec::parse_str("now").unwrap();
    assert_eq!(now_spec.resolve_start_boundary(ref_dt).unwrap(), "2026-11-23T10:00:02");

    let plus_5m = ScheduleTimeSpec::parse_str("+5m").unwrap();
    assert_eq!(plus_5m.resolve_start_boundary(ref_dt).unwrap(), "2026-11-23T10:05:00");

    let plus_2h = ScheduleTimeSpec::parse_str("+2h").unwrap();
    assert_eq!(plus_2h.resolve_start_boundary(ref_dt).unwrap(), "2026-11-23T12:00:00");

    let past_today = ScheduleTimeSpec::parse_str("09:30").unwrap();
    assert_eq!(
        past_today.resolve_start_boundary(ref_dt).unwrap(),
        "2026-11-24T09:30:00"
    );

    let later_today = ScheduleTimeSpec::parse_str("15:45").unwrap();
    assert_eq!(
        later_today.resolve_start_boundary(ref_dt).unwrap(),
        "2026-11-23T15:45:00"
    );

    let iso = ScheduleTimeSpec::parse_str("2026-12-01T08:15:00").unwrap();
    assert_eq!(iso.resolve_start_boundary(ref_dt).unwrap(), "2026-12-01T08:15:00");

    assert!(ScheduleTimeSpec::parse_str("invalid-time").is_err());

    let req = AtNowTaskRequest {
        task_name: "CheckJob".to_string(),
        command: "cmd.exe".to_string(),
        arguments: Some("/c echo hi".to_string()),
        working_dir: None,
        run_at: plus_5m,
        run_as_system: false,
        interactive: true,
        highest_privileges: false,
        folder_path: "\\".to_string(),
    };
    assert!(req.validate().is_ok());
}

#[test]
fn unsupported_backend_reports_graceful_platform_errors() {
    let b = UnsupportedCmdBackend;
    assert!(matches!(
        b.execute_script(&ScriptCommand::ScreenSaver),
        Err(WinctlError::UnsupportedOnPlatform(_))
    ));
    assert!(matches!(
        b.list_candidate_processes(),
        Err(WinctlError::UnsupportedOnPlatform(_))
    ));
    assert!(matches!(
        b.list_scheduled_tasks("\\"),
        Err(WinctlError::UnsupportedOnPlatform(_))
    ));
}

#[test]
fn integration_mcp_policy_gates_with_custom_backend() {
    let backend = Arc::new(IntegrationBackend::default());
    let cmd_bundle = bundle(Arc::clone(&backend) as Arc<dyn CmdBackend>);

    let mut registry = McpServerRegistry::new(PolicyMode::ReadOnlyOnly);
    cmd_bundle.register_into(&mut registry);

    let err = registry
        .dispatch("cmd_script", &json!({ "script": "beep 440 100", "dry_run": true }))
        .unwrap_err();
    assert_eq!(err.code, JsonRpcError::POLICY_DENIED);

    registry.set_policy(PolicyMode::AllowMutating);
    let preview = registry
        .dispatch(
            "cmd_atnow",
            &json!({
                "task_name": "T1",
                "command": "notepad.exe",
                "when": "+5m",
                "confirm_destructive": true,
                "dry_run": true
            }),
        )
        .unwrap();
    assert_eq!(preview["dry_run"], json!(true));
    assert!(backend.calls.lock().unwrap().is_empty());

    let provs = providers(backend);
    let data = provs[0].query(&TableQuery::default()).unwrap();
    assert!(!data.rows.is_empty());
}

#[test]
fn script_exec_preserves_quoted_argument_boundaries() {
    let vars = ScriptVars::new();
    let parsed = parse_script(
        r#"exec show notepad.exe "C:\My Docs\notes.txt" "/msg=\"hi there\"""#,
        &vars,
    )
    .unwrap();
    assert_eq!(parsed.len(), 1);
    match &parsed[0].command {
        ScriptCommand::Exec { show_mode, exe, args } => {
            assert_eq!(*show_mode, WindowState::Normal);
            assert_eq!(exe, "notepad.exe");
            assert_eq!(args, &vec![r"C:\My Docs\notes.txt", r#"/msg="hi there""#]);
        }
        other => panic!("expected ScriptCommand::Exec, got {other:?}"),
    }

    let parsed_empty = parse_script("execmd min calc.exe", &vars).unwrap();
    match &parsed_empty[0].command {
        ScriptCommand::Exec { args, .. } => assert!(args.is_empty()),
        other => panic!("expected ScriptCommand::Exec, got {other:?}"),
    }
}

#[test]
fn script_parser_validates_beep_regsetval_shortcut_win_and_preserves_ini_comments() {
    let vars = ScriptVars::new();

    // Beep frequency bounds (37..=32767 Hz)
    assert!(parse_script("beep 0 100", &vars).is_err());
    assert!(parse_script("beep 36 100", &vars).is_err());
    assert!(parse_script("beep 32768 100", &vars).is_err());
    assert!(parse_script("beep 37 100", &vars).is_ok());
    assert!(parse_script("beep 32767 100", &vars).is_ok());

    // Registry hive validation
    assert!(parse_script("regsetval sz INVALID_HIVE\\Software\\Test Val Data", &vars).is_err());
    assert!(parse_script("regsetval sz HKCU\\ Val Data", &vars).is_err());
    assert!(parse_script("regsetval sz NoSlash Val Data", &vars).is_err());
    assert!(parse_script("regsetval sz HKCU\\Software\\Test Val Data", &vars).is_ok());

    // Shortcut numeric parse error propagation
    assert!(matches!(
        parse_script("shortcut app.exe C:\\Temp MyLink \"\" \"\" \"\" not_an_int", &vars),
        Err(WinctlError::ParseError(_))
    ));
    assert!(matches!(
        parse_script(
            "shortcut app.exe C:\\Temp MyLink \"\" \"\" \"\" 0 normal bad_hotkey",
            &vars
        ),
        Err(WinctlError::ParseError(_))
    ));
    assert!(
        parse_script(
            "shortcut app.exe C:\\Temp MyLink \"\" \"\" \"\" \"\" normal \"\"",
            &vars
        )
        .is_ok()
    );

    // 2-arg win active/alltop & HWND(0) rejection
    let win_cmds = parse_script("win min active\nwin normal alltop", &vars).unwrap();
    assert!(matches!(
        win_cmds[0].command,
        ScriptCommand::Win {
            find_by: WinSelector::Active,
            ..
        }
    ));
    assert!(matches!(
        win_cmds[1].command,
        ScriptCommand::Win {
            find_by: WinSelector::AllTop,
            ..
        }
    ));
    assert!(matches!(
        parse_script("win close handle 0", &vars),
        Err(WinctlError::InvalidInput(_))
    ));
    assert!(matches!(
        parse_script("win close handle 0x0", &vars),
        Err(WinctlError::InvalidInput(_))
    ));

    // INI editor comment/blank preservation
    let dir = tempdir().unwrap();
    let ini_path = dir.path().join("comments.ini");
    fs::write(
        &ini_path,
        "; Top preamble comment\n\n[Main]\n# Keep this comment\nVolume=50\n\n[Other]\nFlag=1\n",
    )
    .unwrap();
    apply_ini_operation(IniOp::SetVal, &ini_path, "Main", Some("Volume"), Some("80")).unwrap();
    apply_ini_operation(IniOp::SetVal, &ini_path, "Main", Some("NewKey"), Some("Yes")).unwrap();
    let updated = fs::read_to_string(&ini_path).unwrap();
    assert!(updated.contains("; Top preamble comment"));
    assert!(updated.contains("# Keep this comment"));
    assert!(updated.contains("Volume=80"));
    assert!(updated.contains("NewKey=Yes"));
}

#[test]
fn launch_profile_env_block_dedups_sorts_merges_compat_and_preserves_unc_ini_args() {
    // 1. Case-insensitive deduplication & Windows CreateProcessW sort order (windir < __COMPAT_LAYER)
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert("path".to_string(), "C:\\custom".to_string());
    overrides.insert("windir".to_string(), "C:\\Windows".to_string());
    overrides.insert("__COMPAT_LAYER".to_string(), "RunAsAdmin HIGHDPIAWARE".to_string());

    let profile = LaunchProfile {
        exe_path: "notepad.exe".to_string(),
        inherit_env: true,
        env_overrides: overrides,
        compatibility_layer: vec!["HIGHDPIAWARE".to_string(), "WIN8RTM".to_string()],
        ..LaunchProfile::default()
    };

    let utf16 = profile.build_utf16_env_block();
    let decoded = String::from_utf16_lossy(&utf16);
    let entries: Vec<&str> = decoded.split('\0').filter(|s| !s.is_empty()).collect();

    let path_entries: Vec<&&str> = entries
        .iter()
        .filter(|e| e.split_once('=').is_some_and(|(k, _)| k.eq_ignore_ascii_case("PATH")))
        .collect();
    assert_eq!(
        path_entries.len(),
        1,
        "expected single PATH entry, got {path_entries:?}"
    );
    assert_eq!(*path_entries[0], "path=C:\\custom");

    let windir_pos = entries
        .iter()
        .position(|e| e.starts_with("windir="))
        .expect("windir missing");
    let compat_pos = entries
        .iter()
        .position(|e| e.starts_with("__COMPAT_LAYER="))
        .expect("__COMPAT_LAYER missing");
    assert!(
        windir_pos < compat_pos,
        "windir (idx {windir_pos}) must precede __COMPAT_LAYER (idx {compat_pos})"
    );
    assert_eq!(entries[compat_pos], "__COMPAT_LAYER=RunAsAdmin HIGHDPIAWARE WIN8RTM");

    // 2. Quoted UNC paths & Windows CommandLineToArgvW roundtrip in INI args
    let ini = "exe_path=cmd.exe\nargs=\"\\\\server\\share\\file.txt\" \"\\\\server\\share\\my doc.txt\" \"C:\\dir with space\\\\\"\n";
    let parsed = parse_profile_ini(ini).unwrap();
    assert_eq!(
        parsed.args,
        vec![
            r"\\server\share\file.txt",
            r"\\server\share\my doc.txt",
            r"C:\dir with space\",
        ]
    );

    for sample in [
        vec!["".to_string()],
        vec!["hello world".to_string()],
        vec![r#"a"b"#.to_string()],
        vec![r"C:\dir\".to_string()],
        vec![r"C:\dir with space\".to_string()],
        vec![r#"C:\dir\""#.to_string()],
        vec![r#"\\"\ "#.trim().to_string()],
        vec![r"\\server\share\file.txt".to_string()],
    ] {
        let joined = sample
            .iter()
            .map(|a| quote_windows_arg(a))
            .collect::<Vec<_>>()
            .join(" ");
        let roundtrip = tokenize_windows_args(&joined).unwrap();
        assert_eq!(roundtrip, sample, "roundtrip mismatch for {joined}");
    }

    // 3. Administrator dry-run preview notes omitted env overrides
    let admin_profile = LaunchProfile {
        exe_path: "notepad.exe".to_string(),
        run_as_mode: RunAsMode::Administrator,
        env_overrides: [("FOO".to_string(), "BAR".to_string())].into_iter().collect(),
        ..LaunchProfile::default()
    };
    let action = RunLaunchAction::new(Arc::new(IntegrationBackend::default()));
    let preview = action.execute(&json!({ "profile": admin_profile }), true).unwrap();
    assert_eq!(preview["plan"]["env_overrides_applied"], json!(false));
    assert!(preview["plan"]["notes"].is_array());
}

#[test]
fn atnow_rejects_extreme_offsets_and_invalid_structured_run_at() {
    // Overflow relative offsets return Err instead of panicking
    assert!(matches!(
        ScheduleTimeSpec::parse_str("+9999999999999999h"),
        Err(WinctlError::ParseError(_))
    ));
    assert!(matches!(
        ScheduleTimeSpec::OffsetSeconds(i64::MAX).resolve_start_boundary(Local::now()),
        Err(WinctlError::ParseError(_))
    ));

    let action = AtNowAction::new(Arc::new(IntegrationBackend::default()));
    assert!(
        action
            .execute(
                &json!({
                    "task_name": "Overflow",
                    "command": "notepad.exe",
                    "when": "+9999999999999999h"
                }),
                true
            )
            .is_err()
    );

    // Invalid structured IsoDateTime & TimeOfDay fail validation even in dry_run
    assert!(
        action
            .execute(
                &json!({
                    "task_name": "BadIso",
                    "command": "notepad.exe",
                    "run_at": { "kind": "iso_date_time", "value": "not-a-timestamp" }
                }),
                true
            )
            .is_err()
    );
    assert!(
        action
            .execute(
                &json!({
                    "task_name": "BadTod",
                    "command": "notepad.exe",
                    "run_at": { "kind": "time_of_day", "value": { "hour": 99, "minute": 99 } }
                }),
                true
            )
            .is_err()
    );

    // interactive: false wires to TASK_LOGON_S4U
    let s4u_preview = action
        .execute(
            &json!({
                "task_name": "S4UJob",
                "command": "notepad.exe",
                "when": "+5m",
                "interactive": false
            }),
            true,
        )
        .unwrap();
    assert_eq!(s4u_preview["plan"]["logon_type"], json!("TASK_LOGON_S4U"));
}

#[test]
fn r4_compliance_enforced_on_working_dir_env_overrides_and_task_name() {
    let bad_wd = LaunchProfile {
        exe_path: "notepad.exe".to_string(),
        working_dir: Some("C:\\tools\\RunAsDate".to_string()),
        ..LaunchProfile::default()
    };
    assert!(matches!(bad_wd.validate(), Err(WinctlError::PolicyDenied(_))));

    let bad_env = LaunchProfile {
        exe_path: "notepad.exe".to_string(),
        env_overrides: [("PAYLOAD".to_string(), "mimikatz".to_string())].into_iter().collect(),
        ..LaunchProfile::default()
    };
    assert!(matches!(bad_env.validate(), Err(WinctlError::PolicyDenied(_))));

    let bad_from_proc = FromProcessRequest {
        parent_pid: 100,
        child_exe: "notepad.exe".to_string(),
        args: vec![],
        working_dir: Some("C:\\cred2rust".to_string()),
        inherit_handles: false,
        new_console: false,
    };
    assert!(matches!(bad_from_proc.validate(), Err(WinctlError::PolicyDenied(_))));

    let bad_atnow_name = AtNowTaskRequest {
        task_name: "RunAsDateTask".to_string(),
        command: "notepad.exe".to_string(),
        arguments: None,
        working_dir: None,
        run_at: ScheduleTimeSpec::Now,
        run_as_system: false,
        interactive: true,
        highest_privileges: false,
        folder_path: "\\".to_string(),
    };
    assert!(matches!(bad_atnow_name.validate(), Err(WinctlError::PolicyDenied(_))));

    let bad_atnow_wd = AtNowTaskRequest {
        task_name: "NormalTask".to_string(),
        command: "notepad.exe".to_string(),
        arguments: None,
        working_dir: Some("C:\\lsass".to_string()),
        run_at: ScheduleTimeSpec::Now,
        run_as_system: false,
        interactive: true,
        highest_privileges: false,
        folder_path: "\\".to_string(),
    };
    assert!(matches!(bad_atnow_wd.validate(), Err(WinctlError::PolicyDenied(_))));
}
