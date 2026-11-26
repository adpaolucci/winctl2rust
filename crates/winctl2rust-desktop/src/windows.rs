use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::Deserialize;
use serde_json::{Value, json};
use winctl2rust_core::{dry_run_preview, validate_r4_compliance};

use crate::backend::{DesktopBackend, TopWindowInfo, WindowListerOp, format_hwnd, parse_hwnd};

pub fn windows_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_windows",
        "WinLister Open Windows",
        vec![
            ColumnDef::new("hwnd", "HWND", ColumnType::Hex).with_width_hint(110),
            ColumnDef::new("title", "Title", ColumnType::Text).with_width_hint(240),
            ColumnDef::new("class_name", "Class Name", ColumnType::Text).with_width_hint(160),
            ColumnDef::new("pid", "PID", ColumnType::Uint).with_width_hint(70),
            ColumnDef::new("process_name", "Process", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("state", "State", ColumnType::Text).with_width_hint(90),
            ColumnDef::new("bounds", "Bounds", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("width", "Width", ColumnType::Uint).with_width_hint(75),
            ColumnDef::new("height", "Height", ColumnType::Uint).with_width_hint(75),
            ColumnDef::new("visible", "Visible", ColumnType::Bool).with_width_hint(70),
            ColumnDef::new("minimized", "Minimized", ColumnType::Bool).with_width_hint(80),
            ColumnDef::new("maximized", "Maximized", ColumnType::Bool).with_width_hint(80),
        ],
    )
}

pub fn top_window_to_row(id: u64, w: &TopWindowInfo) -> Row {
    let width = u64::try_from(w.right.saturating_sub(w.left).max(0)).unwrap_or(0);
    let height = u64::try_from(w.bottom.saturating_sub(w.top).max(0)).unwrap_or(0);
    Row::new(
        id,
        vec![
            CellValue::Text(format_hwnd(w.hwnd)),
            CellValue::Text(w.title.clone()),
            CellValue::Text(w.class_name.clone()),
            CellValue::Uint(u64::from(w.pid)),
            CellValue::Text(w.process_name.clone()),
            CellValue::Text(w.state.clone()),
            CellValue::Text(format!("{},{},{},{}", w.left, w.top, w.right, w.bottom)),
            CellValue::Uint(width),
            CellValue::Uint(height),
            CellValue::Bool(w.visible),
            CellValue::Bool(w.minimized),
            CellValue::Bool(w.maximized),
        ],
    )
    .with_detail_json(serde_json::to_value(w).unwrap_or(Value::Null))
}

pub struct WindowsTableProvider {
    backend: Arc<dyn DesktopBackend>,
}

impl WindowsTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for WindowsTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_windows"
    }

    fn description(&self) -> &'static str {
        "List open top-level windows with placement state and bounds (WinLister)"
    }

    fn schema(&self) -> TableSchema {
        windows_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let windows = self
            .backend
            .list_top_windows()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = windows
            .iter()
            .enumerate()
            .map(|(idx, w)| top_window_to_row(idx as u64 + 1, w))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct WindowsControlInput {
    operation: String,
    hwnd: String,
}

pub struct WindowsControlAction {
    backend: Arc<dyn DesktopBackend>,
}

impl WindowsControlAction {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for WindowsControlAction {
    fn command_id(&self) -> &'static str {
        "desktop_windows_control"
    }

    fn description(&self) -> &'static str {
        "Minimize, maximize, restore, focus, or close an open window (WinLister)"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["minimize", "maximize", "restore", "close", "focus"]
                },
                "hwnd": {
                    "type": "string",
                    "description": "Target top-level HWND (hex '0x...' or decimal)"
                }
            },
            "required": ["operation", "hwnd"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let input: WindowsControlInput = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid desktop_windows_control request: {e}")))?;

        let r4_args = vec![input.hwnd.clone()];
        validate_r4_compliance(&input.operation, &r4_args).map_err(McpToolError::from)?;

        let hwnd = parse_hwnd(&input.hwnd).map_err(McpToolError::from)?;
        let op_norm = input.operation.trim().to_ascii_lowercase();
        let parsed_op = match op_norm.as_str() {
            "minimize" => WindowListerOp::Minimize,
            "maximize" => WindowListerOp::Maximize,
            "restore" => WindowListerOp::Restore,
            "close" => WindowListerOp::Close,
            "focus" => WindowListerOp::Focus,
            other => {
                return Err(McpToolError::InvalidParams(format!(
                    "unsupported desktop_windows_control operation '{other}'"
                )));
            }
        };

        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview WinLister window placement/state change without modifying live HWND",
                json!({
                    "hwnd": format_hwnd(hwnd),
                    "operation": op_norm,
                    "parsed_op": parsed_op,
                }),
            ));
        }

        let res = self
            .backend
            .control_top_window(hwnd, &parsed_op)
            .map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "result": res,
        }))
    }
}
