use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::CmdBackend;
use crate::run::quote_windows_arg;

pub const PROCESS_CREATE_PROCESS_ACCESS: u32 = 0x0080;
pub const EXTENDED_STARTUPINFO_PRESENT_FLAG: u32 = 0x0008_0000;
pub const PROC_THREAD_ATTRIBUTE_PARENT_PROCESS_VALUE: usize = 0x0002_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessSummary {
    pub pid: u32,
    pub ppid: u32,
    pub process_name: String,
    pub threads: u32,
    pub elevation_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FromProcessRequest {
    pub parent_pid: u32,
    pub child_exe: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub inherit_handles: bool,
    #[serde(default)]
    pub new_console: bool,
}

impl FromProcessRequest {
    pub fn validate(&self) -> WinctlResult<()> {
        if self.parent_pid == 0 {
            return Err(WinctlError::InvalidInput(
                "parent_pid must be greater than 0 (cannot use System Idle Process PID 0)".to_string(),
            ));
        }
        if self.child_exe.trim().is_empty() {
            return Err(WinctlError::InvalidInput("child_exe must not be empty".to_string()));
        }
        let mut r4_extra = self.args.clone();
        if let Some(ref wd) = self.working_dir {
            r4_extra.push(wd.clone());
        }
        validate_r4_compliance(&self.child_exe, &r4_extra)?;
        Ok(())
    }

    pub fn build_command_line(&self) -> String {
        let mut out = quote_windows_arg(&self.child_exe);
        for arg in &self.args {
            out.push(' ');
            out.push_str(&quote_windows_arg(arg));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FromProcessResult {
    pub parent_pid: u32,
    pub child_pid: u32,
    pub child_tid: u32,
    pub command_line: String,
}

pub struct FromProcessTableProvider {
    backend: Arc<dyn CmdBackend>,
}

impl FromProcessTableProvider {
    pub fn new(backend: Arc<dyn CmdBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for FromProcessTableProvider {
    fn tool_id(&self) -> &'static str {
        "cmd_from_proc_candidates"
    }

    fn description(&self) -> &'static str {
        "Enumerate candidate parent processes for parent-attributed launch via ToolHelp32"
    }

    fn schema(&self) -> TableSchema {
        TableSchema::new(
            "cmd_from_proc_candidates",
            "Candidate Parent Processes",
            vec![
                ColumnDef::new("pid", "PID", ColumnType::Uint).with_width_hint(80),
                ColumnDef::new("ppid", "Parent PID", ColumnType::Uint).with_width_hint(90),
                ColumnDef::new("process_name", "Process Name", ColumnType::Text).with_width_hint(200),
                ColumnDef::new("threads", "Threads", ColumnType::Uint).with_width_hint(80),
                ColumnDef::new("elevation_hint", "Elevation Hint", ColumnType::Text).with_width_hint(220),
                ColumnDef::new("command_preview", "Example Action", ColumnType::Text).with_width_hint(260),
            ],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let procs = self
            .backend
            .list_candidate_processes()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = procs
            .into_iter()
            .enumerate()
            .map(|(idx, p)| {
                let preview = format!("winctl2rust cmd from-proc --parent-pid {} -- <exe>", p.pid);
                Row::new(
                    idx as u64 + 1,
                    vec![
                        CellValue::Uint(u64::from(p.pid)),
                        CellValue::Uint(u64::from(p.ppid)),
                        CellValue::Text(p.process_name.clone()),
                        CellValue::Uint(u64::from(p.threads)),
                        CellValue::Text(p.elevation_hint.clone()),
                        CellValue::Text(preview),
                    ],
                )
                .with_detail_json(serde_json::to_value(&p).unwrap_or(Value::Null))
            })
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

pub struct FromProcessAction {
    backend: Arc<dyn CmdBackend>,
}

impl FromProcessAction {
    pub fn new(backend: Arc<dyn CmdBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for FromProcessAction {
    fn command_id(&self) -> &'static str {
        "cmd_from_proc"
    }

    fn description(&self) -> &'static str {
        "Launch a child process with parent-process attribution via PROC_THREAD_ATTRIBUTE_PARENT_PROCESS (non-injecting)"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::PrivilegedMutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "parent_pid": { "type": "integer", "minimum": 1 },
                "child_exe": { "type": "string" },
                "args": { "type": "array", "items": { "type": "string" } },
                "working_dir": { "type": "string" },
                "inherit_handles": { "type": "boolean", "default": false },
                "new_console": { "type": "boolean", "default": false }
            },
            "required": ["parent_pid", "child_exe"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let req: FromProcessRequest = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid cmd_from_proc request: {e}")))?;
        req.validate().map_err(McpToolError::from)?;

        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview PROC_THREAD_ATTRIBUTE_PARENT_PROCESS spawn without opening process or spawning",
                json!({
                    "parent_pid": req.parent_pid,
                    "child_exe": req.child_exe,
                    "command_line": req.build_command_line(),
                    "working_dir": req.working_dir,
                    "access_mask": format!("{PROCESS_CREATE_PROCESS_ACCESS:#06x} (PROCESS_CREATE_PROCESS)"),
                    "creation_flags": format!("{EXTENDED_STARTUPINFO_PRESENT_FLAG:#010x} (EXTENDED_STARTUPINFO_PRESENT)"),
                    "attribute": format!("{PROC_THREAD_ATTRIBUTE_PARENT_PROCESS_VALUE:#010x} (PROC_THREAD_ATTRIBUTE_PARENT_PROCESS)"),
                }),
            ));
        }

        let res = self.backend.spawn_from_parent(&req).map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "result": res,
        }))
    }
}
