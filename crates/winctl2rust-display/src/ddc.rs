use std::collections::BTreeMap;
use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::{DdcSetVcpRequest, DisplayBackend};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VcpCodeDefinition {
    pub code: u8,
    pub name: &'static str,
    pub read_only: bool,
    pub write_only: bool,
    pub discrete_values: &'static [(u16, &'static str)],
}

static VCP_DICTIONARY: &[VcpCodeDefinition] = &[
    VcpCodeDefinition {
        code: 0x02,
        name: "New Control Value",
        read_only: false,
        write_only: false,
        discrete_values: &[(1, "No new control"), (2, "New control present")],
    },
    VcpCodeDefinition {
        code: 0x04,
        name: "Restore Factory Defaults",
        read_only: false,
        write_only: true,
        discrete_values: &[(1, "Restore")],
    },
    VcpCodeDefinition {
        code: 0x05,
        name: "Restore Factory Brightness/Contrast",
        read_only: false,
        write_only: true,
        discrete_values: &[(1, "Restore")],
    },
    VcpCodeDefinition {
        code: 0x06,
        name: "Restore Factory Geometry",
        read_only: false,
        write_only: true,
        discrete_values: &[(1, "Restore")],
    },
    VcpCodeDefinition {
        code: 0x08,
        name: "Restore Factory Color",
        read_only: false,
        write_only: true,
        discrete_values: &[(1, "Restore")],
    },
    VcpCodeDefinition {
        code: 0x0B,
        name: "Color Temperature Increment",
        read_only: true,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x0C,
        name: "Color Temperature Request",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x10,
        name: "Luminance (Brightness)",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x12,
        name: "Contrast",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x14,
        name: "Select Color Preset",
        read_only: false,
        write_only: false,
        discrete_values: &[
            (0x01, "sRGB"),
            (0x02, "Display Native"),
            (0x03, "4000K"),
            (0x04, "5000K"),
            (0x05, "6500K"),
            (0x06, "7500K"),
            (0x07, "8200K"),
            (0x08, "9300K"),
            (0x09, "10000K"),
            (0x0A, "11500K"),
            (0x0B, "User 1"),
            (0x0C, "User 2"),
            (0x0D, "User 3"),
        ],
    },
    VcpCodeDefinition {
        code: 0x16,
        name: "Video Gain (Drive): Red",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x18,
        name: "Video Gain (Drive): Green",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x1A,
        name: "Video Gain (Drive): Blue",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x52,
        name: "Active Control",
        read_only: true,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x60,
        name: "Input Source",
        read_only: false,
        write_only: false,
        discrete_values: &[
            (0x01, "VGA-1"),
            (0x02, "VGA-2"),
            (0x03, "DVI-1"),
            (0x04, "DVI-2"),
            (0x0F, "DisplayPort-1"),
            (0x10, "DisplayPort-2"),
            (0x11, "HDMI-1"),
            (0x12, "HDMI-2"),
            (0x1B, "USB-C"),
        ],
    },
    VcpCodeDefinition {
        code: 0x62,
        name: "Audio Speaker Volume",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x6C,
        name: "Video Black Level: Red",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x6E,
        name: "Video Black Level: Green",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x70,
        name: "Video Black Level: Blue",
        read_only: false,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0x8D,
        name: "Audio Mute",
        read_only: false,
        write_only: false,
        discrete_values: &[(0x01, "Mute"), (0x02, "Unmute")],
    },
    VcpCodeDefinition {
        code: 0xAC,
        name: "Horizontal Frequency",
        read_only: true,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0xAE,
        name: "Vertical Frequency",
        read_only: true,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0xB6,
        name: "Display Technology Type",
        read_only: true,
        write_only: false,
        discrete_values: &[
            (0x01, "CRT (Shadow Mask)"),
            (0x02, "CRT (Aperture Grille)"),
            (0x03, "LCD (Active Matrix)"),
            (0x04, "LCos"),
            (0x05, "Plasma"),
            (0x06, "OLED"),
            (0x07, "EL"),
            (0x08, "MEM"),
        ],
    },
    VcpCodeDefinition {
        code: 0xC6,
        name: "Application Enable Key",
        read_only: true,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0xC8,
        name: "Display Controller Type",
        read_only: true,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0xC9,
        name: "Display Firmware Level",
        read_only: true,
        write_only: false,
        discrete_values: &[],
    },
    VcpCodeDefinition {
        code: 0xCA,
        name: "OSD",
        read_only: false,
        write_only: false,
        discrete_values: &[(0x01, "OSD Disabled"), (0x02, "OSD Enabled")],
    },
    VcpCodeDefinition {
        code: 0xCC,
        name: "OSD Language",
        read_only: false,
        write_only: false,
        discrete_values: &[(0x02, "English"), (0x03, "French"), (0x04, "German"), (0x0A, "Spanish")],
    },
    VcpCodeDefinition {
        code: 0xD6,
        name: "Power Mode",
        read_only: false,
        write_only: false,
        discrete_values: &[
            (0x01, "On"),
            (0x02, "Standby"),
            (0x03, "Suspend"),
            (0x04, "Off (DPM)"),
            (0x05, "Hard Off"),
        ],
    },
    VcpCodeDefinition {
        code: 0xDF,
        name: "VCP Version",
        read_only: true,
        write_only: false,
        discrete_values: &[],
    },
];

pub fn vcp_dictionary() -> &'static [VcpCodeDefinition] {
    VCP_DICTIONARY
}

pub fn vcp_code_info(code: u8) -> Option<&'static VcpCodeDefinition> {
    VCP_DICTIONARY.iter().find(|d| d.code == code)
}

pub fn parse_vcp_code(raw: &str) -> WinctlResult<u8> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WinctlError::InvalidInput("VCP code must not be empty".to_string()));
    }
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        u8::from_str_radix(hex, 16).map_err(|_| WinctlError::InvalidInput(format!("invalid hex VCP code '{raw}'")))
    } else if let Ok(dec) = trimmed.parse::<u8>() {
        Ok(dec)
    } else if trimmed.len() <= 2 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        u8::from_str_radix(trimmed, 16).map_err(|_| WinctlError::InvalidInput(format!("invalid hex VCP code '{raw}'")))
    } else {
        Err(WinctlError::InvalidInput(format!(
            "invalid VCP code '{raw}' (expected 0x10 or 16)"
        )))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedDdcCapabilities {
    pub prot: Option<String>,
    pub display_type: Option<String>,
    pub model: Option<String>,
    pub mccs_ver: Option<String>,
    pub cmds: Vec<u8>,
    pub vcp: BTreeMap<u8, Vec<u16>>,
}

#[derive(Debug)]
enum CapNode {
    Atom(String),
    Group(String, Vec<CapNode>),
}

pub fn parse_capabilities_string(raw: &str) -> WinctlResult<ParsedDdcCapabilities> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WinctlError::ParseError(
            "DDC/CI capabilities string must not be empty".to_string(),
        ));
    }

    let nodes = parse_cap_tokens(trimmed)?;
    let mut caps = ParsedDdcCapabilities::default();
    walk_cap_nodes(&nodes, &mut caps)?;
    Ok(caps)
}

fn parse_cap_tokens(input: &str) -> WinctlResult<Vec<CapNode>> {
    let mut stack: Vec<(Option<String>, Vec<CapNode>)> = vec![(None, Vec::new())];
    let mut word = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '(' => {
                let tag = std::mem::take(&mut word).trim().to_string();
                stack.push((if tag.is_empty() { None } else { Some(tag) }, Vec::new()));
            }
            ')' => {
                if !word.trim().is_empty() {
                    let atom = std::mem::take(&mut word).trim().to_string();
                    if let Some((_, children)) = stack.last_mut() {
                        children.push(CapNode::Atom(atom));
                    }
                } else {
                    word.clear();
                }
                if stack.len() <= 1 {
                    return Err(WinctlError::ParseError(
                        "unmatched ')' in DDC/CI capabilities string".to_string(),
                    ));
                }
                let (tag_opt, children) = stack.pop().unwrap();
                let node = CapNode::Group(tag_opt.unwrap_or_default(), children);
                stack.last_mut().unwrap().1.push(node);
            }
            c if c.is_whitespace() => {
                if !word.trim().is_empty() {
                    let atom = std::mem::take(&mut word).trim().to_string();
                    stack.last_mut().unwrap().1.push(CapNode::Atom(atom));
                } else {
                    word.clear();
                }
            }
            other => {
                word.push(other);
            }
        }
        i += 1;
    }

    if !word.trim().is_empty() {
        stack.last_mut().unwrap().1.push(CapNode::Atom(word.trim().to_string()));
    }
    if stack.len() != 1 {
        return Err(WinctlError::ParseError(
            "unclosed '(' in DDC/CI capabilities string".to_string(),
        ));
    }
    Ok(stack.pop().unwrap().1)
}

fn collect_atoms_text(nodes: &[CapNode]) -> String {
    nodes
        .iter()
        .filter_map(|n| match n {
            CapNode::Atom(s) => Some(s.as_str()),
            CapNode::Group(_, _) => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn walk_cap_nodes(nodes: &[CapNode], out: &mut ParsedDdcCapabilities) -> WinctlResult<()> {
    for node in nodes {
        if let CapNode::Group(tag, children) = node {
            let key = tag.to_ascii_lowercase();
            match key.as_str() {
                "" => {
                    walk_cap_nodes(children, out)?;
                }
                "prot" => out.prot = Some(collect_atoms_text(children)),
                "type" => out.display_type = Some(collect_atoms_text(children)),
                "model" => out.model = Some(collect_atoms_text(children)),
                "mccs_ver" => out.mccs_ver = Some(collect_atoms_text(children)),
                "cmds" => {
                    for child in children {
                        if let CapNode::Atom(hex) = child
                            && let Ok(code) = u8::from_str_radix(hex, 16)
                        {
                            out.cmds.push(code);
                        }
                    }
                }
                "vcp" => {
                    let mut last_code: Option<u8> = None;
                    for child in children {
                        match child {
                            CapNode::Atom(hex) => {
                                if let Ok(code) = u8::from_str_radix(hex, 16) {
                                    out.vcp.entry(code).or_default();
                                    last_code = Some(code);
                                }
                            }
                            CapNode::Group(prefix_code, val_nodes) => {
                                let code_opt = if !prefix_code.is_empty() {
                                    u8::from_str_radix(prefix_code, 16).ok()
                                } else {
                                    last_code
                                };
                                if let Some(code) = code_opt {
                                    let entry = out.vcp.entry(code).or_default();
                                    for vn in val_nodes {
                                        if let CapNode::Atom(vh) = vn
                                            && let Ok(v) = u16::from_str_radix(vh, 16)
                                        {
                                            entry.push(v);
                                        }
                                    }
                                    last_code = None;
                                }
                            }
                        }
                    }
                }
                _ => {
                    walk_cap_nodes(children, out)?;
                }
            }
        }
    }
    Ok(())
}

pub fn ddc_vcp_schema() -> TableSchema {
    TableSchema::new(
        "display_ddc_vcp",
        "ControlMyMonitor DDC/CI VCP Features",
        vec![
            ColumnDef::new("monitor_name", "Monitor", ColumnType::Text).with_width_hint(200),
            ColumnDef::new("vcp_code_hex", "VCP Code", ColumnType::Text).with_width_hint(90),
            ColumnDef::new("vcp_name", "Feature Name", ColumnType::Text).with_width_hint(220),
            ColumnDef::new("current_value", "Current", ColumnType::Uint).with_width_hint(90),
            ColumnDef::new("max_value", "Maximum", ColumnType::Uint).with_width_hint(90),
            ColumnDef::new("possible_values", "Possible Values", ColumnType::Text).with_width_hint(220),
            ColumnDef::new("read_only", "Read Only", ColumnType::Bool).with_width_hint(80),
        ],
    )
}

pub struct DdcVcpTableProvider {
    backend: Arc<dyn DisplayBackend>,
}

impl DdcVcpTableProvider {
    pub fn new(backend: Arc<dyn DisplayBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for DdcVcpTableProvider {
    fn tool_id(&self) -> &'static str {
        "display_ddc_vcp"
    }

    fn description(&self) -> &'static str {
        "Query DDC/CI VCP features and MCCS 2.2a monitor capabilities (ControlMyMonitor)"
    }

    fn schema(&self) -> TableSchema {
        ddc_vcp_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let snaps = self
            .backend
            .query_ddc_vcp(None)
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let mut rows = Vec::new();
        let mut row_id = 1u64;
        for snap in snaps {
            for r in &snap.vcp_readings {
                let possible = if r.possible_values.is_empty() {
                    if let Some(def) = vcp_code_info(r.vcp_code) {
                        def.discrete_values
                            .iter()
                            .map(|(v, label)| format!("{v:#04X}={label}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    } else {
                        String::new()
                    }
                } else {
                    let dict = vcp_code_info(r.vcp_code);
                    r.possible_values
                        .iter()
                        .map(|v| {
                            if let Some(label) =
                                dict.and_then(|d| d.discrete_values.iter().find(|(dv, _)| dv == v).map(|(_, l)| *l))
                            {
                                format!("{v:#04X}={label}")
                            } else {
                                format!("{v:#04X}")
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let detail = json!({
                    "monitor_id": snap.monitor_id,
                    "monitor_name": snap.monitor_name,
                    "device_name": snap.device_name,
                    "vcp_code": r.vcp_code,
                    "vcp_name": r.vcp_name,
                    "current_value": r.current_value,
                    "max_value": r.max_value,
                    "is_momentary": r.is_momentary,
                    "read_only": r.read_only,
                    "possible_values": r.possible_values,
                });
                rows.push(
                    Row::new(
                        row_id,
                        vec![
                            CellValue::Text(snap.monitor_name.clone()),
                            CellValue::Text(format!("0x{:02X}", r.vcp_code)),
                            CellValue::Text(r.vcp_name.clone()),
                            CellValue::Uint(u64::from(r.current_value)),
                            CellValue::Uint(u64::from(r.max_value)),
                            CellValue::Text(possible),
                            CellValue::Bool(r.read_only),
                        ],
                    )
                    .with_detail_json(detail),
                );
                row_id += 1;
            }
        }
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct DdcSetVcpInput {
    monitor: String,
    vcp_code: Value,
    value: u32,
}

pub struct DdcSetVcpAction {
    backend: Arc<dyn DisplayBackend>,
}

impl DdcSetVcpAction {
    pub fn new(backend: Arc<dyn DisplayBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for DdcSetVcpAction {
    fn command_id(&self) -> &'static str {
        "display_ddc_set"
    }

    fn description(&self) -> &'static str {
        "Set a DDC/CI VCP feature value on a physical monitor (ControlMyMonitor)"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "monitor": { "type": "string", "description": "Monitor ID, device name, or friendly name" },
                "vcp_code": { "description": "VCP code as hex string (e.g. '0x10') or integer (e.g. 16)" },
                "value": { "type": "integer", "minimum": 0, "maximum": 65535 }
            },
            "required": ["monitor", "vcp_code", "value"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let input: DdcSetVcpInput = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid display_ddc_set request: {e}")))?;
        if input.monitor.trim().is_empty() {
            return Err(McpToolError::InvalidParams("monitor must not be empty".to_string()));
        }
        validate_r4_compliance("display_ddc_set", std::slice::from_ref(&input.monitor)).map_err(McpToolError::from)?;

        let vcp_code = match &input.vcp_code {
            Value::Number(n) => {
                let v = n
                    .as_u64()
                    .ok_or_else(|| McpToolError::InvalidParams("vcp_code must be 0..=255".to_string()))?;
                u8::try_from(v)
                    .map_err(|_| McpToolError::InvalidParams(format!("vcp_code {v} out of range 0..=255")))?
            }
            Value::String(s) => parse_vcp_code(s).map_err(McpToolError::from)?,
            _ => {
                return Err(McpToolError::InvalidParams(
                    "vcp_code must be a hex string (e.g. '0x10') or integer".to_string(),
                ));
            }
        };

        if let Some(def) = vcp_code_info(vcp_code)
            && def.read_only
        {
            return Err(McpToolError::from(WinctlError::InvalidInput(format!(
                "VCP code {vcp_code:#04X} ({}) is read-only according to MCCS 2.2a",
                def.name
            ))));
        }

        if input.value > u32::from(u16::MAX) {
            return Err(McpToolError::from(WinctlError::InvalidInput(format!(
                "DDC/CI VCP value {} exceeds 16-bit maximum (65535)",
                input.value
            ))));
        }

        if let Ok(snaps) = self.backend.query_ddc_vcp(Some(&input.monitor))
            && let Some(entry) = snaps
                .iter()
                .flat_map(|s| s.vcp_readings.iter())
                .find(|r| r.vcp_code == vcp_code)
        {
            if entry.read_only {
                return Err(McpToolError::from(WinctlError::InvalidInput(format!(
                    "VCP code {vcp_code:#04X} ({}) is read-only",
                    entry.vcp_name
                ))));
            }
            if entry.max_value > 0 && input.value > entry.max_value {
                return Err(McpToolError::from(WinctlError::InvalidInput(format!(
                    "DDC/CI VCP {vcp_code:#04X} ({}) value {} exceeds max_value {}",
                    entry.vcp_name, input.value, entry.max_value
                ))));
            }
            let val_u16 = input.value as u16;
            if !entry.possible_values.is_empty() && !entry.possible_values.contains(&val_u16) {
                return Err(McpToolError::from(WinctlError::InvalidInput(format!(
                    "DDC/CI VCP {vcp_code:#04X} ({}) value {:#04X} is not in supported values {:?}",
                    entry.vcp_name, input.value, entry.possible_values
                ))));
            }
        }

        let vcp_name = vcp_code_info(vcp_code).map(|d| d.name).unwrap_or("Custom VCP Feature");

        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview DDC/CI SetVCPFeature without altering physical monitor state",
                json!({
                    "monitor": input.monitor,
                    "vcp_code": format!("0x{vcp_code:02X}"),
                    "vcp_name": vcp_name,
                    "value": input.value,
                }),
            ));
        }

        let req = DdcSetVcpRequest {
            monitor: input.monitor,
            vcp_code,
            value: input.value,
        };
        let res = self.backend.set_ddc_vcp(&req).map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "result": res,
        }))
    }
}
