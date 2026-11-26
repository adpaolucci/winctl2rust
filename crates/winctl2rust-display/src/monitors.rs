use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::Deserialize;
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::{
    DisplayBackend, MonitorApplyOp, MonitorConfigEntry, MonitorConfigProfile, MonitorOrientation, MonitorTopologyInfo,
    topology_to_config_entries,
};

fn monitor_section_sort_key(sec: &str, insertion_idx: usize) -> (u8, usize, usize) {
    let lower = sec.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("monitor")
        && let Ok(num) = rest.trim().parse::<usize>()
    {
        return (0, num, insertion_idx);
    }
    (1, 0, insertion_idx)
}

pub fn parse_monitor_config_ini(text: &str) -> WinctlResult<MonitorConfigProfile> {
    let mut sections: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    let mut current_idx: Option<usize> = None;

    for (line_idx, raw_line) in text.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let sec_name = trimmed[1..trimmed.len() - 1].trim().to_string();
            if sec_name.is_empty() {
                return Err(WinctlError::ParseError(format!(
                    "empty INI section header at line {}",
                    line_idx + 1
                )));
            }
            if let Some(existing) = sections.iter().position(|(s, _)| s.eq_ignore_ascii_case(&sec_name)) {
                current_idx = Some(existing);
            } else {
                sections.push((sec_name, BTreeMap::new()));
                current_idx = Some(sections.len() - 1);
            }
            continue;
        }
        let Some(sec_idx) = current_idx else {
            return Err(WinctlError::ParseError(format!(
                "key=value before any [MonitorN] section at line {}",
                line_idx + 1
            )));
        };
        let (k, v) = trimmed
            .split_once('=')
            .ok_or_else(|| WinctlError::ParseError(format!("expected Key=Value at line {}", line_idx + 1)))?;
        sections[sec_idx]
            .1
            .insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }

    let mut indexed: Vec<(usize, String, BTreeMap<String, String>)> = sections
        .into_iter()
        .enumerate()
        .filter(|(_, (sec, _))| sec.to_ascii_lowercase().starts_with("monitor"))
        .map(|(idx, (sec, map))| (idx, sec, map))
        .collect();
    indexed.sort_by_key(|(idx, sec, _)| monitor_section_sort_key(sec, *idx));

    let mut monitors = Vec::new();
    for (_, sec, map) in indexed {
        let name = map
            .get("name")
            .cloned()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| WinctlError::ParseError(format!("[{sec}] missing Name")))?;
        let monitor_id = map.get("monitorid").cloned().unwrap_or_else(|| name.clone());
        let friendly_name = map.get("friendlyname").cloned().unwrap_or_else(|| name.clone());
        let active = parse_ini_bool(map.get("active").map(String::as_str).unwrap_or("1"))?;
        let primary = parse_ini_bool(map.get("primary").map(String::as_str).unwrap_or("0"))?;
        let width = parse_ini_u32(map.get("width").map(String::as_str).unwrap_or("1920"), "Width")?;
        let height = parse_ini_u32(map.get("height").map(String::as_str).unwrap_or("1080"), "Height")?;
        let bpp = parse_ini_u32(map.get("bitsperpel").map(String::as_str).unwrap_or("32"), "BitsPerPel")?;
        let display_frequency = parse_ini_u32(
            map.get("displayfrequency").map(String::as_str).unwrap_or("60"),
            "DisplayFrequency",
        )?;
        let position_x = parse_ini_i32(map.get("positionx").map(String::as_str).unwrap_or("0"), "PositionX")?;
        let position_y = parse_ini_i32(map.get("positiony").map(String::as_str).unwrap_or("0"), "PositionY")?;
        let orientation_deg = parse_ini_u32(map.get("orientation").map(String::as_str).unwrap_or("0"), "Orientation")?;
        let orientation = MonitorOrientation::from_degrees(orientation_deg)?;

        monitors.push(MonitorConfigEntry {
            monitor_id,
            name,
            friendly_name,
            active,
            primary,
            width,
            height,
            bpp,
            display_frequency,
            position_x,
            position_y,
            orientation,
        });
    }

    if monitors.is_empty() {
        return Err(WinctlError::ParseError(
            "INI monitor profile contained no [MonitorN] sections".to_string(),
        ));
    }
    Ok(MonitorConfigProfile { monitors })
}

pub fn serialize_monitor_config_ini(profile: &MonitorConfigProfile) -> String {
    let mut out = String::new();
    for (idx, m) in profile.monitors.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&format!("[Monitor{idx}]\n"));
        out.push_str(&format!("MonitorID={}\n", m.monitor_id));
        out.push_str(&format!("Name={}\n", m.name));
        out.push_str(&format!("FriendlyName={}\n", m.friendly_name));
        out.push_str(&format!("Active={}\n", i32::from(m.active)));
        out.push_str(&format!("Primary={}\n", i32::from(m.primary)));
        out.push_str(&format!("Width={}\n", m.width));
        out.push_str(&format!("Height={}\n", m.height));
        out.push_str(&format!("BitsPerPel={}\n", m.bpp));
        out.push_str(&format!("DisplayFrequency={}\n", m.display_frequency));
        out.push_str(&format!("PositionX={}\n", m.position_x));
        out.push_str(&format!("PositionY={}\n", m.position_y));
        out.push_str(&format!("Orientation={}\n", m.orientation.degrees()));
    }
    out
}

pub fn parse_monitor_config_auto(text: &str) -> WinctlResult<MonitorConfigProfile> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        serde_json::from_str::<MonitorConfigProfile>(trimmed)
            .map_err(|e| WinctlError::ParseError(format!("invalid JSON monitor profile: {e}")))
    } else {
        parse_monitor_config_ini(trimmed)
    }
}

fn parse_ini_bool(raw: &str) -> WinctlResult<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        other => Err(WinctlError::ParseError(format!("invalid INI boolean '{other}'"))),
    }
}

fn parse_ini_u32(raw: &str, field: &str) -> WinctlResult<u32> {
    raw.trim()
        .parse::<u32>()
        .map_err(|_| WinctlError::ParseError(format!("invalid integer '{raw}' for {field}")))
}

fn parse_ini_i32(raw: &str, field: &str) -> WinctlResult<i32> {
    raw.trim()
        .parse::<i32>()
        .map_err(|_| WinctlError::ParseError(format!("invalid signed integer '{raw}' for {field}")))
}

pub fn monitors_table_schema() -> TableSchema {
    TableSchema::new(
        "display_monitors",
        "MultiMonitorTool Topology",
        vec![
            ColumnDef::new("monitor_id", "Monitor ID", ColumnType::Text).with_width_hint(220),
            ColumnDef::new("device_name", "Device Name", ColumnType::Text).with_width_hint(140),
            ColumnDef::new("friendly_name", "Friendly Name", ColumnType::Text).with_width_hint(200),
            ColumnDef::new("primary", "Primary", ColumnType::Bool).with_width_hint(70),
            ColumnDef::new("active", "Active", ColumnType::Bool).with_width_hint(70),
            ColumnDef::new("resolution", "Resolution", ColumnType::Text).with_width_hint(120),
            ColumnDef::new("position_xy", "Position", ColumnType::Text).with_width_hint(110),
            ColumnDef::new("orientation", "Orientation", ColumnType::Text).with_width_hint(100),
            ColumnDef::new("bpp", "BPP", ColumnType::Uint).with_width_hint(60),
            ColumnDef::new("refresh_hz", "Refresh Hz", ColumnType::Uint).with_width_hint(90),
            ColumnDef::new("scale_pct", "Scale %", ColumnType::Uint).with_width_hint(80),
        ],
    )
}

pub fn monitor_to_row(id: u64, m: &MonitorTopologyInfo) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Text(m.monitor_id.clone()),
            CellValue::Text(m.device_name.clone()),
            CellValue::Text(m.friendly_name.clone()),
            CellValue::Bool(m.primary),
            CellValue::Bool(m.active),
            CellValue::Text(format!("{}x{}", m.width, m.height)),
            CellValue::Text(format!("{},{}", m.position_x, m.position_y)),
            CellValue::Text(m.orientation.to_string()),
            CellValue::Uint(u64::from(m.bpp)),
            CellValue::Uint(u64::from(m.refresh_hz)),
            CellValue::Uint(u64::from(m.scale_pct)),
        ],
    )
    .with_detail_json(serde_json::to_value(m).unwrap_or(Value::Null))
}

pub struct MonitorsTableProvider {
    backend: Arc<dyn DisplayBackend>,
}

impl MonitorsTableProvider {
    pub fn new(backend: Arc<dyn DisplayBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for MonitorsTableProvider {
    fn tool_id(&self) -> &'static str {
        "display_monitors"
    }

    fn description(&self) -> &'static str {
        "Enumerate multi-monitor topology, resolution, DPI scale, orientation, and primary display (MultiMonitorTool)"
    }

    fn schema(&self) -> TableSchema {
        monitors_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let monitors = self
            .backend
            .list_monitors()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = monitors
            .iter()
            .enumerate()
            .map(|(idx, m)| monitor_to_row(idx as u64 + 1, m))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct MonitorsApplyInput {
    operation: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    x: Option<i32>,
    #[serde(default)]
    y: Option<i32>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    refresh_hz: Option<u32>,
    #[serde(default)]
    bpp: Option<u32>,
    #[serde(default)]
    orientation: Option<u32>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    profile_text: Option<String>,
    #[serde(default)]
    profile: Option<MonitorConfigProfile>,
}

pub struct MonitorsApplyAction {
    backend: Arc<dyn DisplayBackend>,
}

impl MonitorsApplyAction {
    pub fn new(backend: Arc<dyn DisplayBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for MonitorsApplyAction {
    fn command_id(&self) -> &'static str {
        "display_monitors_apply"
    }

    fn description(&self) -> &'static str {
        "Apply MultiMonitorTool display topology changes (primary, enable/disable, move, resolution, orientation, save/load profile)"
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
                    "enum": [
                        "set_primary",
                        "enable",
                        "disable",
                        "move",
                        "set_resolution",
                        "set_orientation",
                        "save_config",
                        "load_config"
                    ]
                },
                "target": { "type": "string", "description": "Device name (\\\\.\\DISPLAY1), Monitor ID, or friendly name" },
                "x": { "type": "integer" },
                "y": { "type": "integer" },
                "width": { "type": "integer", "minimum": 1 },
                "height": { "type": "integer", "minimum": 1 },
                "refresh_hz": { "type": "integer", "minimum": 1 },
                "bpp": { "type": "integer", "minimum": 8 },
                "orientation": { "type": "integer", "enum": [0, 90, 180, 270] },
                "path": { "type": "string", "description": "Optional profile file path for save_config/load_config" },
                "format": { "type": "string", "enum": ["json", "ini"] },
                "profile_text": { "type": "string" },
                "profile": { "type": "object" }
            },
            "required": ["operation"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let input: MonitorsApplyInput = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid display_monitors_apply request: {e}")))?;

        let mut r4_args = Vec::new();
        if let Some(ref t) = input.target {
            r4_args.push(t.clone());
        }
        if let Some(ref p) = input.path {
            r4_args.push(p.clone());
        }
        validate_r4_compliance(&input.operation, &r4_args).map_err(McpToolError::from)?;

        let op_norm = input.operation.trim().to_ascii_lowercase();
        let require_target = || -> Result<String, McpToolError> {
            let t = input
                .target
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| McpToolError::InvalidParams(format!("operation '{op_norm}' requires 'target'")))?;
            Ok(t.to_string())
        };

        let parsed_op = match op_norm.as_str() {
            "set_primary" => MonitorApplyOp::SetPrimary {
                target: require_target()?,
            },
            "enable" => MonitorApplyOp::Enable {
                target: require_target()?,
            },
            "disable" => MonitorApplyOp::Disable {
                target: require_target()?,
            },
            "move" => {
                let target = require_target()?;
                let x = input
                    .x
                    .ok_or_else(|| McpToolError::InvalidParams("move requires 'x'".to_string()))?;
                let y = input
                    .y
                    .ok_or_else(|| McpToolError::InvalidParams("move requires 'y'".to_string()))?;
                MonitorApplyOp::Move { target, x, y }
            }
            "set_resolution" => {
                let target = require_target()?;
                let width = input.width.filter(|&w| w > 0).ok_or_else(|| {
                    McpToolError::InvalidParams("set_resolution requires positive 'width'".to_string())
                })?;
                let height = input.height.filter(|&h| h > 0).ok_or_else(|| {
                    McpToolError::InvalidParams("set_resolution requires positive 'height'".to_string())
                })?;
                MonitorApplyOp::SetResolution {
                    target,
                    width,
                    height,
                    refresh_hz: input.refresh_hz,
                    bpp: input.bpp,
                }
            }
            "set_orientation" => {
                let target = require_target()?;
                let deg = input.orientation.ok_or_else(|| {
                    McpToolError::InvalidParams("set_orientation requires 'orientation' (0|90|180|270)".to_string())
                })?;
                let orientation = MonitorOrientation::from_degrees(deg).map_err(McpToolError::from)?;
                MonitorApplyOp::SetOrientation { target, orientation }
            }
            "save_config" => MonitorApplyOp::SaveConfig {
                path: input.path.clone(),
                format: input.format.clone(),
            },
            "load_config" => {
                let profile = if let Some(p) = input.profile {
                    p
                } else if let Some(ref text) = input.profile_text {
                    parse_monitor_config_auto(text).map_err(McpToolError::from)?
                } else if let Some(ref path) = input.path {
                    let content = fs::read_to_string(Path::new(path))
                        .map_err(|e| McpToolError::ExecutionFailed(format!("failed to read '{path}': {e}")))?;
                    parse_monitor_config_auto(&content).map_err(McpToolError::from)?
                } else {
                    return Err(McpToolError::InvalidParams(
                        "load_config requires 'profile', 'profile_text', or 'path'".to_string(),
                    ));
                };
                if profile.monitors.is_empty() || !profile.monitors.iter().any(|m| m.active) {
                    return Err(McpToolError::from(WinctlError::InvalidInput(
                        "monitor profile must contain at least one active monitor".to_string(),
                    )));
                }
                MonitorApplyOp::LoadConfig { profile }
            }
            other => {
                return Err(McpToolError::InvalidParams(format!(
                    "unsupported display_monitors_apply operation '{other}'"
                )));
            }
        };

        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview MultiMonitorTool display configuration without calling ChangeDisplaySettingsExW/SetDisplayConfig",
                json!({
                    "operation": op_norm,
                    "parsed_op": parsed_op,
                }),
            ));
        }

        if let MonitorApplyOp::SaveConfig { ref path, ref format } = parsed_op {
            let monitors = self.backend.list_monitors().map_err(McpToolError::from)?;
            let profile = MonitorConfigProfile {
                monitors: topology_to_config_entries(&monitors),
            };
            let fmt_norm = format.as_deref().unwrap_or("ini").trim().to_ascii_lowercase();
            let serialized = match fmt_norm.as_str() {
                "json" => {
                    serde_json::to_string_pretty(&profile).map_err(|e| McpToolError::ExecutionFailed(e.to_string()))?
                }
                "ini" => serialize_monitor_config_ini(&profile),
                other => {
                    return Err(McpToolError::InvalidParams(format!(
                        "unsupported save_config format '{other}' (expected json or ini)"
                    )));
                }
            };
            if let Some(p) = path {
                fs::write(Path::new(p), &serialized)
                    .map_err(|e| McpToolError::ExecutionFailed(format!("failed to write '{p}': {e}")))?;
            }
            return Ok(json!({
                "dry_run": false,
                "applied": true,
                "operation": "save_config",
                "format": fmt_norm,
                "path": path,
                "profile": profile,
                "serialized": serialized,
            }));
        }

        let res = self
            .backend
            .apply_monitor_action(&parsed_op)
            .map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "result": res,
        }))
    }
}
