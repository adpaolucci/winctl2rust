use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDateTime, NaiveTime, TimeDelta, TimeZone};
use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::CmdBackend;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ScheduleTimeSpec {
    Now,
    OffsetSeconds(i64),
    TimeOfDay { hour: u32, minute: u32 },
    IsoDateTime(String),
}

impl ScheduleTimeSpec {
    pub fn parse_str(raw: &str) -> WinctlResult<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(WinctlError::ParseError(
                "schedule time specification must not be empty".to_string(),
            ));
        }
        if trimmed.eq_ignore_ascii_case("now") {
            return Ok(Self::Now);
        }
        if let Some(rest) = trimmed.strip_prefix('+') {
            if rest.len() < 2 {
                return Err(WinctlError::ParseError(format!(
                    "invalid relative schedule offset '{trimmed}'"
                )));
            }
            let (num_part, unit) = rest.split_at(rest.len() - 1);
            let value: i64 = num_part
                .parse()
                .map_err(|_| WinctlError::ParseError(format!("invalid offset number in '{trimmed}'")))?;
            if value < 0 {
                return Err(WinctlError::ParseError(
                    "relative schedule offset must be non-negative".to_string(),
                ));
            }
            let seconds = match unit.to_ascii_lowercase().as_str() {
                "s" => Some(value),
                "m" => value.checked_mul(60),
                "h" => value.checked_mul(3600),
                _ => {
                    return Err(WinctlError::ParseError(format!(
                        "unsupported relative unit in '{trimmed}'; expected s, m, or h"
                    )));
                }
            }
            .ok_or_else(|| {
                WinctlError::ParseError(format!("relative schedule offset '{trimmed}' overflows seconds"))
            })?;
            if TimeDelta::try_seconds(seconds).is_none() {
                return Err(WinctlError::ParseError(format!(
                    "relative schedule offset '{trimmed}' is out of TimeDelta range"
                )));
            }
            return Ok(Self::OffsetSeconds(seconds));
        }

        if let Ok(t) = NaiveTime::parse_from_str(trimmed, "%H:%M") {
            use chrono::Timelike;
            return Ok(Self::TimeOfDay {
                hour: t.hour(),
                minute: t.minute(),
            });
        }

        if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
            let local_naive = dt.with_timezone(&Local).naive_local();
            return Ok(Self::IsoDateTime(local_naive.format("%Y-%m-%dT%H:%M:%S").to_string()));
        }

        if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S") {
            return Ok(Self::IsoDateTime(ndt.format("%Y-%m-%dT%H:%M:%S").to_string()));
        }
        if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
            return Ok(Self::IsoDateTime(ndt.format("%Y-%m-%dT%H:%M:%S").to_string()));
        }

        Err(WinctlError::ParseError(format!(
            "unrecognized AtNow schedule spec '{trimmed}' (expected 'now', '+Ns/+Nm/+Nh', 'HH:MM', or 'YYYY-MM-DDTHH:MM:SS')"
        )))
    }

    pub fn validate(&self) -> WinctlResult<()> {
        match self {
            Self::Now => Ok(()),
            Self::OffsetSeconds(secs) => {
                if *secs < 0 {
                    return Err(WinctlError::ParseError(
                        "relative schedule offset must be non-negative".to_string(),
                    ));
                }
                let delta = TimeDelta::try_seconds(*secs).ok_or_else(|| {
                    WinctlError::ParseError(format!("relative schedule offset '{secs}s' is out of TimeDelta range"))
                })?;
                if Local::now().checked_add_signed(delta).is_none() {
                    return Err(WinctlError::ParseError(format!(
                        "relative schedule offset '{secs}s' overflows calendar timestamp"
                    )));
                }
                Ok(())
            }
            Self::TimeOfDay { hour, minute } => {
                if *hour >= 24 || *minute >= 60 {
                    return Err(WinctlError::ParseError(format!(
                        "invalid time_of_day {hour:02}:{minute:02} (expected hour 0..23 and minute 0..59)"
                    )));
                }
                Ok(())
            }
            Self::IsoDateTime(s) => {
                if DateTime::parse_from_rfc3339(s).is_ok()
                    || NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").is_ok()
                {
                    Ok(())
                } else {
                    Err(WinctlError::ParseError(format!(
                        "invalid IsoDateTime '{s}' (expected YYYY-MM-DDTHH:MM:SS or RFC3339)"
                    )))
                }
            }
        }
    }

    pub fn resolve_start_boundary(&self, reference: DateTime<Local>) -> WinctlResult<String> {
        match self {
            Self::Now => {
                let dt = reference
                    .checked_add_signed(TimeDelta::seconds(2))
                    .ok_or_else(|| WinctlError::ParseError("schedule timestamp overflow".to_string()))?;
                Ok(dt.format("%Y-%m-%dT%H:%M:%S").to_string())
            }
            Self::OffsetSeconds(secs) => {
                if *secs < 0 {
                    return Err(WinctlError::ParseError(
                        "relative schedule offset must be non-negative".to_string(),
                    ));
                }
                let delta = TimeDelta::try_seconds(*secs).ok_or_else(|| {
                    WinctlError::ParseError(format!("relative schedule offset '{secs}s' is out of TimeDelta range"))
                })?;
                let dt = reference.checked_add_signed(delta).ok_or_else(|| {
                    WinctlError::ParseError(format!("relative schedule offset '{secs}s' overflows timestamp"))
                })?;
                Ok(dt.format("%Y-%m-%dT%H:%M:%S").to_string())
            }
            Self::TimeOfDay { hour, minute } => {
                let target_time = NaiveTime::from_hms_opt(*hour, *minute, 0).ok_or_else(|| {
                    WinctlError::ParseError(format!(
                        "invalid time_of_day {hour:02}:{minute:02} (expected hour 0..23 and minute 0..59)"
                    ))
                })?;
                let today = reference.date_naive().and_time(target_time);
                let target_local = match Local.from_local_datetime(&today) {
                    chrono::LocalResult::Single(dt) => dt,
                    chrono::LocalResult::Ambiguous(earliest, _) => earliest,
                    chrono::LocalResult::None => {
                        return Err(WinctlError::ParseError(format!(
                            "nonexistent local time {hour:02}:{minute:02} on {}",
                            reference.date_naive()
                        )));
                    }
                };
                let resolved = if target_local <= reference {
                    target_local
                        .checked_add_signed(TimeDelta::days(1))
                        .ok_or_else(|| WinctlError::ParseError("schedule timestamp overflow".to_string()))?
                } else {
                    target_local
                };
                Ok(resolved.format("%Y-%m-%dT%H:%M:%S").to_string())
            }
            Self::IsoDateTime(s) => {
                if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                    let local_naive = dt.with_timezone(&Local).naive_local();
                    Ok(local_naive.format("%Y-%m-%dT%H:%M:%S").to_string())
                } else if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
                    Ok(ndt.format("%Y-%m-%dT%H:%M:%S").to_string())
                } else {
                    Err(WinctlError::ParseError(format!(
                        "invalid IsoDateTime '{s}' (expected YYYY-MM-DDTHH:MM:SS or RFC3339)"
                    )))
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtNowTaskRequest {
    pub task_name: String,
    pub command: String,
    #[serde(default)]
    pub arguments: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    pub run_at: ScheduleTimeSpec,
    #[serde(default)]
    pub run_as_system: bool,
    #[serde(default = "default_true")]
    pub interactive: bool,
    #[serde(default)]
    pub highest_privileges: bool,
    #[serde(default = "default_root_folder")]
    pub folder_path: String,
}

fn default_true() -> bool {
    true
}

fn default_root_folder() -> String {
    "\\".to_string()
}

impl AtNowTaskRequest {
    pub fn validate(&self) -> WinctlResult<()> {
        if self.task_name.trim().is_empty() {
            return Err(WinctlError::InvalidInput(
                "AtNow task_name must not be empty".to_string(),
            ));
        }
        if self.command.trim().is_empty() {
            return Err(WinctlError::InvalidInput("AtNow command must not be empty".to_string()));
        }
        if !self.folder_path.starts_with('\\') {
            return Err(WinctlError::InvalidInput(
                "AtNow folder_path must start with '\\'".to_string(),
            ));
        }
        self.run_at.validate()?;
        let mut extra: Vec<String> = Vec::new();
        extra.push(self.task_name.clone());
        if let Some(ref args) = self.arguments {
            extra.push(args.clone());
        }
        if let Some(ref wd) = self.working_dir {
            extra.push(wd.clone());
        }
        validate_r4_compliance(&self.command, &extra)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtNowTaskInfo {
    pub task_name: String,
    pub folder: String,
    pub state: String,
    pub enabled: bool,
    pub next_run: Option<String>,
    pub last_run: Option<String>,
    pub last_result: i32,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtNowScheduleResult {
    pub task_name: String,
    pub folder: String,
    pub start_boundary: String,
    pub state: String,
}

pub struct AtNowTasksTableProvider {
    backend: Arc<dyn CmdBackend>,
}

impl AtNowTasksTableProvider {
    pub fn new(backend: Arc<dyn CmdBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for AtNowTasksTableProvider {
    fn tool_id(&self) -> &'static str {
        "cmd_atnow_tasks"
    }

    fn description(&self) -> &'static str {
        "List Windows Task Scheduler v2 tasks in the root folder"
    }

    fn schema(&self) -> TableSchema {
        TableSchema::new(
            "cmd_atnow_tasks",
            "AtNow Scheduled Tasks",
            vec![
                ColumnDef::new("task_name", "Task Name", ColumnType::Text).with_width_hint(180),
                ColumnDef::new("folder", "Folder", ColumnType::Text).with_width_hint(90),
                ColumnDef::new("state", "State", ColumnType::Text).with_width_hint(100),
                ColumnDef::new("enabled", "Enabled", ColumnType::Bool).with_width_hint(80),
                ColumnDef::new("next_run", "Next Run", ColumnType::Text).with_width_hint(180),
                ColumnDef::new("last_run", "Last Run", ColumnType::Text).with_width_hint(180),
                ColumnDef::new("last_result", "Last Result", ColumnType::Int).with_width_hint(100),
                ColumnDef::new("command", "Command", ColumnType::Text).with_width_hint(260),
            ],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let tasks = self
            .backend
            .list_scheduled_tasks("\\")
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = tasks
            .into_iter()
            .enumerate()
            .map(|(idx, t)| {
                Row::new(
                    idx as u64 + 1,
                    vec![
                        CellValue::Text(t.task_name.clone()),
                        CellValue::Text(t.folder.clone()),
                        CellValue::Text(t.state.clone()),
                        CellValue::Bool(t.enabled),
                        t.next_run.clone().map(CellValue::Text).unwrap_or(CellValue::Null),
                        t.last_run.clone().map(CellValue::Text).unwrap_or(CellValue::Null),
                        CellValue::Int(i64::from(t.last_result)),
                        CellValue::Text(t.command.clone()),
                    ],
                )
                .with_detail_json(serde_json::to_value(&t).unwrap_or(Value::Null))
            })
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

pub struct AtNowAction {
    backend: Arc<dyn CmdBackend>,
}

impl AtNowAction {
    pub fn new(backend: Arc<dyn CmdBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for AtNowAction {
    fn command_id(&self) -> &'static str {
        "cmd_atnow"
    }

    fn description(&self) -> &'static str {
        "Schedule or dry-run preview a one-shot delayed task via Windows Task Scheduler v2 ITaskService"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_name": { "type": "string" },
                "command": { "type": "string" },
                "arguments": { "type": "string" },
                "working_dir": { "type": "string" },
                "when": { "type": "string", "description": "'now', '+Ns/+Nm/+Nh', 'HH:MM', or 'YYYY-MM-DDTHH:MM:SS'" },
                "run_as_system": { "type": "boolean", "default": false },
                "interactive": { "type": "boolean", "default": true },
                "highest_privileges": { "type": "boolean", "default": false },
                "folder_path": { "type": "string", "default": "\\" }
            },
            "required": ["task_name", "command", "when"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let req = parse_atnow_args(args)?;
        req.validate().map_err(McpToolError::from)?;

        let start_boundary = req
            .run_at
            .resolve_start_boundary(Local::now())
            .map_err(McpToolError::from)?;
        if dry_run {
            let logon_type = if req.run_as_system {
                "TASK_LOGON_SERVICE_ACCOUNT"
            } else if req.interactive {
                "TASK_LOGON_INTERACTIVE_TOKEN"
            } else {
                "TASK_LOGON_S4U"
            };
            let run_level = if req.highest_privileges {
                "TASK_RUNLEVEL_HIGHEST"
            } else {
                "TASK_RUNLEVEL_LUA"
            };
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview Task Scheduler v2 ITaskService registration without creating scheduled task",
                json!({
                    "task_name": req.task_name,
                    "folder_path": req.folder_path,
                    "command": req.command,
                    "arguments": req.arguments,
                    "working_dir": req.working_dir,
                    "start_boundary": start_boundary,
                    "trigger_type": "TASK_TRIGGER_TIME",
                    "action_type": "TASK_ACTION_EXEC",
                    "logon_type": logon_type,
                    "run_level": run_level,
                }),
            ));
        }

        let res = self.backend.schedule_task(&req).map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "result": res,
        }))
    }
}

fn parse_atnow_args(args: &Value) -> Result<AtNowTaskRequest, McpToolError> {
    let task_name = args
        .get("task_name")
        .and_then(Value::as_str)
        .ok_or_else(|| McpToolError::InvalidParams("missing string 'task_name'".to_string()))?
        .to_string();
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| McpToolError::InvalidParams("missing string 'command'".to_string()))?
        .to_string();
    let run_at = if let Some(when_str) = args.get("when").and_then(Value::as_str) {
        ScheduleTimeSpec::parse_str(when_str).map_err(McpToolError::from)?
    } else if let Some(spec_val) = args.get("run_at") {
        serde_json::from_value(spec_val.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid 'run_at': {e}")))?
    } else {
        return Err(McpToolError::InvalidParams(
            "missing string 'when' or structured 'run_at'".to_string(),
        ));
    };

    Ok(AtNowTaskRequest {
        task_name,
        command,
        arguments: args.get("arguments").and_then(Value::as_str).map(ToString::to_string),
        working_dir: args.get("working_dir").and_then(Value::as_str).map(ToString::to_string),
        run_at,
        run_as_system: args.get("run_as_system").and_then(Value::as_bool).unwrap_or(false),
        interactive: args.get("interactive").and_then(Value::as_bool).unwrap_or(true),
        highest_privileges: args.get("highest_privileges").and_then(Value::as_bool).unwrap_or(false),
        folder_path: args
            .get("folder_path")
            .and_then(Value::as_str)
            .unwrap_or("\\")
            .to_string(),
    })
}
