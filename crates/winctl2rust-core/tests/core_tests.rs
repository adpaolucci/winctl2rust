use std::collections::HashSet;
use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpServerRegistry, McpToolError, PolicyMode};
use guap_table::{
    CellValue, ColumnDef, ColumnType, InMemoryTableProvider, TableData, TableProvider, TableQuery, TableSchema,
};
use serde_json::json;
use winctl2rust_core::{
    DefaultPrivilegeProbe, ExecutionOptions, PrivilegeProbe, ToolBundle, ToolCategory, WinctlError,
    builtin_suite_catalog, dry_run_preview, init_logging, validate_r4_compliance,
};

struct DummyAction;

impl ActionCommand for DummyAction {
    fn command_id(&self) -> &'static str {
        "dummy_action"
    }

    fn description(&self) -> &'static str {
        "Dummy action for registry tests"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::ReadOnly
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn execute(&self, _args: &serde_json::Value, dry_run: bool) -> Result<serde_json::Value, McpToolError> {
        Ok(json!({ "ok": true, "dry_run": dry_run }))
    }
}

#[test]
fn catalog_has_22_unique_non_hooking_tools() {
    let catalog = builtin_suite_catalog();
    assert_eq!(catalog.len(), 22);

    let mut ids = HashSet::new();
    let mut counts = [0usize; 4];
    for tool in catalog {
        assert!(ids.insert(tool.tool_id), "duplicate tool_id {}", tool.tool_id);
        let lower_name = tool.nirsoft_name.to_ascii_lowercase();
        assert!(!lower_name.contains("runasdate"));
        assert!(!lower_name.contains("cred"));
        match tool.category {
            ToolCategory::Cmd => counts[0] += 1,
            ToolCategory::Audio => counts[1] += 1,
            ToolCategory::Display => counts[2] += 1,
            ToolCategory::Desktop => counts[3] += 1,
        }
    }
    assert_eq!(counts, [4, 4, 4, 10]);
}

#[test]
fn execution_options_validate_action_enforces_gates() {
    let ro = ExecutionOptions::read_only();
    assert!(ro.validate_action("read_op", ActionRiskLevel::ReadOnly).is_ok());
    assert!(matches!(
        ro.validate_action("mut_op", ActionRiskLevel::Mutating),
        Err(WinctlError::PolicyDenied(_))
    ));

    let dry = ExecutionOptions::dry_run_only();
    assert!(dry.validate_action("mut_op", ActionRiskLevel::Mutating).is_ok());
    assert!(
        dry.validate_action("priv_op", ActionRiskLevel::PrivilegedMutating)
            .is_ok()
    );

    let unconfirmed = ExecutionOptions {
        dry_run: false,
        allow_mutating: true,
        confirm_destructive: false,
    };
    assert!(matches!(
        unconfirmed.validate_action("mut_op", ActionRiskLevel::Mutating),
        Err(WinctlError::PolicyDenied(_))
    ));

    let allowed = ExecutionOptions::allow_all_mutating();
    assert!(allowed.can_mutate());
    assert!(
        allowed
            .validate_action("priv_op", ActionRiskLevel::PrivilegedMutating)
            .is_ok()
    );

    let dry_with_flags = ExecutionOptions {
        dry_run: true,
        allow_mutating: true,
        confirm_destructive: true,
    };
    assert!(!dry_with_flags.can_mutate());
    assert!(!unconfirmed.can_mutate());
    assert!(
        !(ExecutionOptions {
            dry_run: false,
            allow_mutating: false,
            confirm_destructive: true,
        })
        .can_mutate()
    );
}

#[test]
fn r4_compliance_rejects_prohibited_identifiers() {
    assert!(validate_r4_compliance("notepad.exe", &["--help".to_string()]).is_ok());

    for bad in [
        "RunAsDate.exe",
        "cred2rust",
        "CreateRemoteThread",
        "NtCreateThreadEx",
        "QueueUserAPC",
        "SetWindowsHookExW",
        "SniffPass",
    ] {
        let err = validate_r4_compliance(bad, &[]).unwrap_err();
        assert!(
            matches!(err, WinctlError::PolicyDenied(_)),
            "expected PolicyDenied for {bad}"
        );
    }

    let arg_err = validate_r4_compliance("launcher.exe", &["--inject=CreateRemoteThread".to_string()]).unwrap_err();
    assert!(matches!(arg_err, WinctlError::PolicyDenied(_)));
}

#[test]
fn error_mapping_to_mcp_tool_error_and_dry_run_preview() {
    let invalid: McpToolError = WinctlError::InvalidInput("bad arg".to_string()).into();
    assert!(matches!(invalid, McpToolError::InvalidParams(_)));

    let parse: McpToolError = WinctlError::ParseError("syntax".to_string()).into();
    assert!(matches!(parse, McpToolError::InvalidParams(_)));

    let policy: McpToolError = WinctlError::PolicyDenied("blocked".to_string()).into();
    assert!(matches!(policy, McpToolError::PolicyDenied(_)));

    let perm: McpToolError = WinctlError::PermissionDenied("denied".to_string()).into();
    assert!(matches!(perm, McpToolError::ExecutionFailed(_)));

    let preview = dry_run_preview("cmd_run", "Launch notepad", json!({ "exe": "notepad.exe" }));
    assert_eq!(preview["dry_run"], json!(true));
    assert_eq!(preview["applied"], json!(false));
    assert_eq!(preview["command_id"], json!("cmd_run"));
}

#[test]
fn tool_bundle_merge_and_register_into_mcp() {
    let schema = TableSchema::new(
        "dummy_table",
        "Dummy Table",
        vec![ColumnDef::new("name", "Name", ColumnType::Text)],
    );
    let provider: Arc<dyn TableProvider> = Arc::new(InMemoryTableProvider::new(
        "dummy_table",
        "Dummy table provider",
        schema.clone(),
        vec![guap_table::Row::new(1, vec![CellValue::Text("ok".to_string())])],
    ));
    let action: Arc<dyn ActionCommand> = Arc::new(DummyAction);

    let mut bundle_a = ToolBundle::new();
    bundle_a.providers.push(provider);

    let mut bundle_b = ToolBundle::new();
    bundle_b.actions.push(action);
    bundle_b.tools.push(builtin_suite_catalog()[0].clone());

    bundle_a.merge(bundle_b);
    assert!(bundle_a.find_provider("dummy_table").is_some());
    assert!(bundle_a.find_action("dummy_action").is_some());
    assert_eq!(bundle_a.tools.len(), 1);

    let data = bundle_a
        .find_provider("dummy_table")
        .unwrap()
        .query(&TableQuery::default())
        .unwrap();
    assert_eq!(data.rows.len(), 1);

    let mut registry = McpServerRegistry::new(PolicyMode::ReadOnlyOnly);
    bundle_a.register_into(&mut registry);
    let tools_json = registry.list_tools();
    let arr = tools_json.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    let res = registry.dispatch("dummy_action", &json!({ "dry_run": true })).unwrap();
    assert_eq!(res["ok"], json!(true));
    assert_eq!(res["dry_run"], json!(true));
}

#[test]
fn privilege_probe_and_logging_init_are_safe() {
    init_logging("warn");
    init_logging("info");

    let status = DefaultPrivilegeProbe.probe();
    assert!(!status.platform.is_empty());
    assert_eq!(status.windows_supported, cfg!(windows));

    let empty_data = TableData {
        schema: TableSchema::new("t", "T", vec![]),
        rows: vec![],
        total_rows: 0,
    };
    assert_eq!(empty_data.total_rows, 0);
}
