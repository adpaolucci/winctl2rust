#![deny(unsafe_code)]

pub mod atnow;
pub mod backend;
pub mod from_proc;
pub mod run;
pub mod script;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod win32_backend;

use std::sync::Arc;

use guap_mcp::ActionCommand;
use guap_table::TableProvider;
use winctl2rust_core::{ToolBundle, ToolCategory, builtin_suite_catalog};

#[cfg(any(test, feature = "test-support"))]
pub use backend::MockCmdBackend;
pub use backend::{CmdBackend, UnsupportedCmdBackend};
#[cfg(windows)]
pub use win32_backend::Win32CmdBackend;

pub fn default_backend() -> Arc<dyn CmdBackend> {
    #[cfg(windows)]
    {
        Arc::new(Win32CmdBackend::new())
    }
    #[cfg(not(windows))]
    {
        Arc::new(UnsupportedCmdBackend)
    }
}

pub fn providers(backend: Arc<dyn CmdBackend>) -> Vec<Arc<dyn TableProvider>> {
    vec![
        Arc::new(script::ScriptPreviewProvider::default()),
        Arc::new(run::RunProfilesTableProvider::default()),
        Arc::new(from_proc::FromProcessTableProvider::new(Arc::clone(&backend))),
        Arc::new(atnow::AtNowTasksTableProvider::new(backend)),
    ]
}

pub fn actions(backend: Arc<dyn CmdBackend>) -> Vec<Arc<dyn ActionCommand>> {
    vec![
        Arc::new(script::ScriptParseAction),
        Arc::new(script::ScriptAction::new(Arc::clone(&backend))),
        Arc::new(run::RunValidateAction),
        Arc::new(run::RunLaunchAction::new(Arc::clone(&backend))),
        Arc::new(from_proc::FromProcessAction::new(Arc::clone(&backend))),
        Arc::new(atnow::AtNowAction::new(backend)),
    ]
}

pub fn bundle(backend: Arc<dyn CmdBackend>) -> ToolBundle {
    let tools = builtin_suite_catalog()
        .iter()
        .filter(|t| t.category == ToolCategory::Cmd)
        .cloned()
        .collect();
    ToolBundle {
        providers: providers(Arc::clone(&backend)),
        actions: actions(backend),
        tools,
    }
}

pub fn default_providers() -> Vec<Arc<dyn TableProvider>> {
    providers(default_backend())
}

pub fn default_actions() -> Vec<Arc<dyn ActionCommand>> {
    actions(default_backend())
}

pub fn default_bundle() -> ToolBundle {
    bundle(default_backend())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use guap_mcp::{JsonRpcError, McpServerRegistry, PolicyMode};
    use guap_table::TableQuery;
    use serde_json::json;
    use winctl2rust_core::WinctlError;

    use crate::backend::{CmdBackend, MockCmdBackend};
    use crate::from_proc::FromProcessRequest;
    use crate::{bundle, providers};

    #[test]
    fn mock_backend_permission_denied_propagates() {
        let mock = Arc::new(MockCmdBackend::with_sample_data());
        mock.inject_permission_denied_on_from_proc
            .store(true, Ordering::Relaxed);
        let req = FromProcessRequest {
            parent_pid: 4,
            child_exe: "notepad.exe".to_string(),
            args: vec![],
            working_dir: None,
            inherit_handles: false,
            new_console: false,
        };
        assert!(matches!(
            mock.spawn_from_parent(&req),
            Err(WinctlError::PermissionDenied(_))
        ));
    }

    #[test]
    fn mock_backend_mcp_policy_gates_and_dry_run_work_end_to_end() {
        let mock = Arc::new(MockCmdBackend::with_sample_data());
        let cmd_bundle = bundle(Arc::clone(&mock) as Arc<dyn CmdBackend>);
        assert_eq!(cmd_bundle.tools.len(), 4);
        assert_eq!(cmd_bundle.providers.len(), 4);
        assert_eq!(cmd_bundle.actions.len(), 6);

        let mut registry = McpServerRegistry::new(PolicyMode::ReadOnlyOnly);
        cmd_bundle.register_into(&mut registry);

        let list = registry.list_tools();
        assert_eq!(list.as_array().unwrap().len(), 10);

        for table_id in [
            "cmd_script_preview",
            "cmd_run_profiles",
            "cmd_from_proc_candidates",
            "cmd_atnow_tasks",
        ] {
            let res = registry.dispatch(table_id, &json!({})).unwrap();
            assert!(res["rows"].is_array(), "missing rows for {table_id}");
        }

        let parsed = registry
            .dispatch("cmd_script_parse", &json!({ "script": "beep 440 100" }))
            .unwrap();
        assert_eq!(parsed["valid"], json!(true));

        let validated = registry
            .dispatch("cmd_run_validate", &json!({ "exe_path": "notepad.exe" }))
            .unwrap();
        assert_eq!(validated["valid"], json!(true));

        let mutating_calls = [
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
        ];

        for (name, args) in &mutating_calls {
            let err = registry.dispatch(name, args).unwrap_err();
            assert_eq!(err.code, JsonRpcError::POLICY_DENIED);
        }

        registry.set_policy(PolicyMode::AllowMutating);
        for (name, args) in &mutating_calls {
            let err = registry.dispatch(name, args).unwrap_err();
            assert_eq!(err.code, JsonRpcError::POLICY_DENIED);
        }

        for (name, args) in &mutating_calls {
            let mut payload = args.clone();
            payload["confirm_destructive"] = json!(true);
            let res = registry.dispatch(name, &payload).unwrap();
            assert_eq!(res["dry_run"], json!(true));
            assert_eq!(res["applied"], json!(false));
        }
        assert!(mock.executed_scripts.lock().unwrap().is_empty());

        let res_script = registry
            .dispatch(
                "cmd_script",
                &json!({ "script": "beep 440 100", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_script["applied"], json!(true));
        assert_eq!(mock.executed_scripts.lock().unwrap().len(), 1);

        let res_run = registry
            .dispatch(
                "cmd_run",
                &json!({ "exe_path": "notepad.exe", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_run["applied"], json!(true));
        assert_eq!(mock.launched_profiles.lock().unwrap().len(), 1);

        let res_proc = registry
            .dispatch(
                "cmd_from_proc",
                &json!({ "parent_pid": 1024, "child_exe": "notepad.exe", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_proc["applied"], json!(true));
        assert_eq!(mock.spawned_from_proc.lock().unwrap().len(), 1);

        let res_at = registry
            .dispatch(
                "cmd_atnow",
                &json!({ "task_name": "T1", "command": "notepad.exe", "when": "+5m", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_at["applied"], json!(true));
        assert_eq!(mock.scheduled_requests.lock().unwrap().len(), 1);

        let provs = providers(mock);
        let data = provs[0].query(&TableQuery::default()).unwrap();
        assert!(!data.rows.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn win32_backend_lists_live_processes_and_default_bundle() {
        let b = crate::default_bundle();
        assert_eq!(b.tools.len(), 4);
        assert_eq!(b.providers.len(), 4);
        assert_eq!(b.actions.len(), 6);

        let backend = crate::Win32CmdBackend::new();
        let procs = backend.list_candidate_processes().unwrap();
        assert!(!procs.is_empty());
        let self_pid = std::process::id();
        assert!(
            procs.iter().any(|p| p.pid == self_pid),
            "expected current test process PID {self_pid} in ToolHelp32 snapshot"
        );
    }
}
