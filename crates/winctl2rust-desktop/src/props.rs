use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::Deserialize;
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, dry_run_preview, validate_r4_compliance};

use crate::backend::{DesktopBackend, WindowPropInfo, WindowPropOp, format_hwnd, parse_hwnd};

pub fn props_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_props",
        "Window Properties",
        vec![
            ColumnDef::new("hwnd", "HWND", ColumnType::Hex).with_width_hint(110),
            ColumnDef::new("title", "Title", ColumnType::Text).with_width_hint(220),
            ColumnDef::new("class_name", "Class Name", ColumnType::Text).with_width_hint(160),
            ColumnDef::new("pid", "PID", ColumnType::Uint).with_width_hint(70),
            ColumnDef::new("tid", "TID", ColumnType::Uint).with_width_hint(70),
            ColumnDef::new("process_name", "Process", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("visible", "Visible", ColumnType::Bool).with_width_hint(70),
            ColumnDef::new("enabled", "Enabled", ColumnType::Bool).with_width_hint(70),
            ColumnDef::new("topmost", "Topmost", ColumnType::Bool).with_width_hint(75),
            ColumnDef::new("style", "Style", ColumnType::Hex).with_width_hint(105),
            ColumnDef::new("ex_style", "ExStyle", ColumnType::Hex).with_width_hint(105),
            ColumnDef::new("rect", "Rect", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("parent_hwnd", "Parent HWND", ColumnType::Hex).with_width_hint(110),
        ],
    )
}

pub fn prop_to_row(id: u64, w: &WindowPropInfo) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Text(format_hwnd(w.hwnd)),
            CellValue::Text(w.title.clone()),
            CellValue::Text(w.class_name.clone()),
            CellValue::Uint(u64::from(w.pid)),
            CellValue::Uint(u64::from(w.tid)),
            CellValue::Text(w.process_name.clone()),
            CellValue::Bool(w.visible),
            CellValue::Bool(w.enabled),
            CellValue::Bool(w.topmost),
            CellValue::Text(format!("0x{:08X}", w.style)),
            CellValue::Text(format!("0x{:08X}", w.ex_style)),
            CellValue::Text(format!("{},{},{},{}", w.left, w.top, w.right, w.bottom)),
            CellValue::Text(format_hwnd(w.parent_hwnd)),
        ],
    )
    .with_detail_json(serde_json::to_value(w).unwrap_or(Value::Null))
}

pub struct PropsTableProvider {
    backend: Arc<dyn DesktopBackend>,
}

impl PropsTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for PropsTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_props"
    }

    fn description(&self) -> &'static str {
        "Inspect top-level and child window properties, styles, bounds, and owning process"
    }

    fn schema(&self) -> TableSchema {
        props_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let items = self
            .backend
            .list_window_props()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = items
            .iter()
            .enumerate()
            .map(|(idx, item)| prop_to_row(idx as u64 + 1, item))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PropsControlInput {
    operation: String,
    hwnd: String,
    #[serde(default)]
    opacity: Option<u16>,
}

pub struct PropsControlAction {
    backend: Arc<dyn DesktopBackend>,
}

impl PropsControlAction {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for PropsControlAction {
    fn command_id(&self) -> &'static str {
        "desktop_props_control"
    }

    fn description(&self) -> &'static str {
        "Control window visibility, enabled state, topmost z-order, opacity, or close via Win32 APIs"
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
                    "enum": ["show", "hide", "enable", "disable", "topmost", "untopmost", "close", "set_opacity"]
                },
                "hwnd": {
                    "type": "string",
                    "description": "Target window handle (hex '0x...' or decimal)"
                },
                "opacity": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 255,
                    "description": "Alpha opacity (0..=255) required when operation='set_opacity'"
                }
            },
            "required": ["operation", "hwnd"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let input: PropsControlInput = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid desktop_props_control request: {e}")))?;

        let r4_args = vec![input.hwnd.clone()];
        validate_r4_compliance(&input.operation, &r4_args).map_err(McpToolError::from)?;

        let hwnd = parse_hwnd(&input.hwnd).map_err(McpToolError::from)?;
        let op_norm = input.operation.trim().to_ascii_lowercase();
        let parsed_op = match op_norm.as_str() {
            "show" => WindowPropOp::Show,
            "hide" => WindowPropOp::Hide,
            "enable" => WindowPropOp::Enable,
            "disable" => WindowPropOp::Disable,
            "topmost" => WindowPropOp::Topmost,
            "untopmost" => WindowPropOp::Untopmost,
            "close" => WindowPropOp::Close,
            "set_opacity" => {
                let raw = input.opacity.ok_or_else(|| {
                    McpToolError::InvalidParams("set_opacity requires 'opacity' (0..=255)".to_string())
                })?;
                let opacity = u8::try_from(raw).map_err(|_| {
                    McpToolError::from(WinctlError::InvalidInput(format!("opacity {raw} out of range 0..=255")))
                })?;
                WindowPropOp::SetOpacity { opacity }
            }
            other => {
                return Err(McpToolError::InvalidParams(format!(
                    "unsupported desktop_props_control operation '{other}'"
                )));
            }
        };

        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview window property change without modifying live HWND state",
                json!({
                    "hwnd": format_hwnd(hwnd),
                    "operation": op_norm,
                    "parsed_op": parsed_op,
                }),
            ));
        }

        let res = self
            .backend
            .control_window_prop(hwnd, &parsed_op)
            .map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "result": res,
        }))
    }
}
