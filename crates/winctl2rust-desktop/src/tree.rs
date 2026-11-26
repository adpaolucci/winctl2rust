use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::Deserialize;
use serde_json::{Value, json};
use winctl2rust_core::{dry_run_preview, validate_r4_compliance};

use crate::backend::{DesktopBackend, TreeControlOp, TreeWindowNode, format_hwnd, parse_hwnd};

pub fn tree_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_tree",
        "Window & Control Hierarchy",
        vec![
            ColumnDef::new("hwnd", "HWND", ColumnType::Hex).with_width_hint(110),
            ColumnDef::new("parent_hwnd", "Parent HWND", ColumnType::Hex).with_width_hint(110),
            ColumnDef::new("depth", "Depth", ColumnType::Uint).with_width_hint(65),
            ColumnDef::new("control_id", "Control ID", ColumnType::Uint).with_width_hint(90),
            ColumnDef::new("class_name", "Class Name", ColumnType::Text).with_width_hint(160),
            ColumnDef::new("text", "Text", ColumnType::Text).with_width_hint(220),
            ColumnDef::new("bounds", "Bounds", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("visible", "Visible", ColumnType::Bool).with_width_hint(70),
            ColumnDef::new("enabled", "Enabled", ColumnType::Bool).with_width_hint(70),
            ColumnDef::new("pid", "PID", ColumnType::Uint).with_width_hint(70),
        ],
    )
}

pub fn tree_node_to_row(id: u64, n: &TreeWindowNode) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Text(format_hwnd(n.hwnd)),
            CellValue::Text(format_hwnd(n.parent_hwnd)),
            CellValue::Uint(u64::from(n.depth)),
            CellValue::Uint(u64::from(n.control_id)),
            CellValue::Text(n.class_name.clone()),
            CellValue::Text(n.text.clone()),
            CellValue::Text(format!("{},{},{},{}", n.left, n.top, n.right, n.bottom)),
            CellValue::Bool(n.visible),
            CellValue::Bool(n.enabled),
            CellValue::Uint(u64::from(n.pid)),
        ],
    )
    .with_detail_json(serde_json::to_value(n).unwrap_or(Value::Null))
}

pub struct TreeTableProvider {
    backend: Arc<dyn DesktopBackend>,
}

impl TreeTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for TreeTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_tree"
    }

    fn description(&self) -> &'static str {
        "Walk recursive HWND parent/child window trees and inspect control IDs, classes, and text"
    }

    fn schema(&self) -> TableSchema {
        tree_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let nodes = self
            .backend
            .list_window_tree()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = nodes
            .iter()
            .enumerate()
            .map(|(idx, node)| tree_node_to_row(idx as u64 + 1, node))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TreeControlInput {
    operation: String,
    hwnd: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    x: Option<i32>,
    #[serde(default)]
    y: Option<i32>,
    #[serde(default)]
    width: Option<i32>,
    #[serde(default)]
    height: Option<i32>,
}

pub struct TreeControlAction {
    backend: Arc<dyn DesktopBackend>,
}

impl TreeControlAction {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for TreeControlAction {
    fn command_id(&self) -> &'static str {
        "desktop_tree_control"
    }

    fn description(&self) -> &'static str {
        "Update child/control window text, size, position, or visibility"
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
                    "enum": ["set_text", "resize", "move", "show", "hide"]
                },
                "hwnd": { "type": "string", "description": "Target child or window HWND" },
                "text": { "type": "string", "description": "New window/control text for set_text" },
                "x": { "type": "integer" },
                "y": { "type": "integer" },
                "width": { "type": "integer", "minimum": 1 },
                "height": { "type": "integer", "minimum": 1 }
            },
            "required": ["operation", "hwnd"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let input: TreeControlInput = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid desktop_tree_control request: {e}")))?;

        let mut r4_args = vec![input.hwnd.clone()];
        if let Some(ref t) = input.text {
            r4_args.push(t.clone());
        }
        validate_r4_compliance(&input.operation, &r4_args).map_err(McpToolError::from)?;

        let hwnd = parse_hwnd(&input.hwnd).map_err(McpToolError::from)?;
        let op_norm = input.operation.trim().to_ascii_lowercase();
        let parsed_op = match op_norm.as_str() {
            "set_text" => {
                let text = input
                    .text
                    .ok_or_else(|| McpToolError::InvalidParams("set_text requires 'text'".to_string()))?;
                TreeControlOp::SetText { text }
            }
            "resize" => {
                let width = input
                    .width
                    .filter(|&w| w > 0)
                    .ok_or_else(|| McpToolError::InvalidParams("resize requires positive 'width'".to_string()))?;
                let height = input
                    .height
                    .filter(|&h| h > 0)
                    .ok_or_else(|| McpToolError::InvalidParams("resize requires positive 'height'".to_string()))?;
                TreeControlOp::Resize { width, height }
            }
            "move" => {
                let x = input
                    .x
                    .ok_or_else(|| McpToolError::InvalidParams("move requires 'x'".to_string()))?;
                let y = input
                    .y
                    .ok_or_else(|| McpToolError::InvalidParams("move requires 'y'".to_string()))?;
                TreeControlOp::Move { x, y }
            }
            "show" => TreeControlOp::Show,
            "hide" => TreeControlOp::Hide,
            other => {
                return Err(McpToolError::InvalidParams(format!(
                    "unsupported desktop_tree_control operation '{other}'"
                )));
            }
        };

        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview control mutation without calling SetWindowTextW/MoveWindow/ShowWindow",
                json!({
                    "hwnd": format_hwnd(hwnd),
                    "operation": op_norm,
                    "parsed_op": parsed_op,
                }),
            ));
        }

        let res = self
            .backend
            .control_tree_window(hwnd, &parsed_op)
            .map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "result": res,
        }))
    }
}
