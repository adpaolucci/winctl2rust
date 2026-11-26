use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::Deserialize;
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, dry_run_preview, validate_r4_compliance};

use crate::backend::{AudioBackend, AudioEndpointItem, DefaultRole, MuteAction};

pub fn audio_view_schema() -> TableSchema {
    TableSchema::new(
        "audio_view",
        "CoreAudio Endpoints & Sessions",
        vec![
            ColumnDef::new("item_id", "Item ID", ColumnType::Text).with_width_hint(260),
            ColumnDef::new("name", "Name", ColumnType::Text).with_width_hint(220),
            ColumnDef::new("kind", "Kind", ColumnType::Text).with_width_hint(90),
            ColumnDef::new("direction", "Direction", ColumnType::Text).with_width_hint(90),
            ColumnDef::new("device_name", "Device Name", ColumnType::Text).with_width_hint(200),
            ColumnDef::new("default_roles", "Default Roles", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("muted", "Muted", ColumnType::Bool).with_width_hint(70),
            ColumnDef::new("volume_percent", "Volume %", ColumnType::Float).with_width_hint(90),
            ColumnDef::new("volume_db", "Volume dB", ColumnType::Float).with_width_hint(90),
            ColumnDef::new("channels", "Channels", ColumnType::Uint).with_width_hint(80),
            ColumnDef::new("channel_volumes", "Channel Volumes", ColumnType::Text).with_width_hint(160),
            ColumnDef::new("peak_level", "Peak", ColumnType::Float).with_width_hint(80),
            ColumnDef::new("pid", "PID", ColumnType::Uint).with_width_hint(80),
            ColumnDef::new("state", "State", ColumnType::Text).with_width_hint(90),
            ColumnDef::new("command_line_id", "Command-Line ID", ColumnType::Text).with_width_hint(260),
        ],
    )
}

pub fn item_to_row(id: u64, item: &AudioEndpointItem) -> Row {
    let roles = item
        .default_roles
        .iter()
        .map(|r| r.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let ch_vols = item
        .channel_volumes
        .iter()
        .map(|v| format!("{v:.1}%"))
        .collect::<Vec<_>>()
        .join(", ");
    Row::new(
        id,
        vec![
            CellValue::Text(item.item_id.clone()),
            CellValue::Text(item.name.clone()),
            CellValue::Text(item.kind.as_str().to_string()),
            CellValue::Text(item.direction.as_str().to_string()),
            CellValue::Text(item.device_name.clone()),
            CellValue::Text(roles),
            CellValue::Bool(item.muted),
            CellValue::Float(f64::from(item.volume_percent)),
            CellValue::Float(f64::from(item.volume_db)),
            CellValue::Uint(u64::from(item.channels)),
            CellValue::Text(ch_vols),
            CellValue::Float(f64::from(item.peak_level)),
            item.pid
                .map(|p| CellValue::Uint(u64::from(p)))
                .unwrap_or(CellValue::Null),
            CellValue::Text(item.state.as_str().to_string()),
            CellValue::Text(item.command_line_id.clone()),
        ],
    )
    .with_detail_json(serde_json::to_value(item).unwrap_or(Value::Null))
}

pub struct AudioViewTableProvider {
    backend: Arc<dyn AudioBackend>,
}

impl AudioViewTableProvider {
    pub fn new(backend: Arc<dyn AudioBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for AudioViewTableProvider {
    fn tool_id(&self) -> &'static str {
        "audio_view"
    }

    fn description(&self) -> &'static str {
        "Enumerate CoreAudio endpoints, topology subunits, and per-application audio sessions"
    }

    fn schema(&self) -> TableSchema {
        audio_view_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let items = self
            .backend
            .list_audio_items()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = items
            .iter()
            .enumerate()
            .map(|(idx, item)| item_to_row(idx as u64 + 1, item))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AudioViewControlRequest {
    action: String,
    target: String,
    #[serde(default)]
    volume_percent: Option<f32>,
    #[serde(default)]
    channel_volumes: Option<Vec<f32>>,
    #[serde(default)]
    volume_db: Option<f32>,
    #[serde(default)]
    mute_mode: Option<String>,
    #[serde(default)]
    roles: Option<Vec<String>>,
    #[serde(default)]
    app_target: Option<String>,
}

pub struct AudioViewControlAction {
    backend: Arc<dyn AudioBackend>,
}

impl AudioViewControlAction {
    pub fn new(backend: Arc<dyn AudioBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for AudioViewControlAction {
    fn command_id(&self) -> &'static str {
        "audio_view_control"
    }

    fn description(&self) -> &'static str {
        "Control CoreAudio endpoint/session volume, mute, default roles, and device enable/disable state"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "set_volume",
                        "set_channel_volumes",
                        "set_volume_db",
                        "set_mute",
                        "set_default",
                        "enable",
                        "disable",
                        "set_app_default"
                    ]
                },
                "target": { "type": "string", "description": "Endpoint ID, command-line ID, friendly name, process name, PID, or DefaultRenderDevice" },
                "volume_percent": { "type": "number", "minimum": 0.0, "maximum": 100.0 },
                "channel_volumes": { "type": "array", "items": { "type": "number", "minimum": 0.0, "maximum": 100.0 } },
                "volume_db": { "type": "number" },
                "mute_mode": { "type": "string", "enum": ["mute", "unmute", "toggle"] },
                "roles": { "type": "array", "items": { "type": "string" } },
                "app_target": { "type": "string" }
            },
            "required": ["action", "target"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let req: AudioViewControlRequest = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid audio_view_control request: {e}")))?;
        let target = req.target.trim();
        if target.is_empty() {
            return Err(McpToolError::InvalidParams("target must not be empty".to_string()));
        }
        let mut r4_args = vec![target.to_string()];
        if let Some(ref app) = req.app_target {
            r4_args.push(app.clone());
        }
        validate_r4_compliance(&req.action, &r4_args).map_err(McpToolError::from)?;

        let action_norm = req.action.trim().to_ascii_lowercase();
        match action_norm.as_str() {
            "set_volume" => {
                let pct = req.volume_percent.ok_or_else(|| {
                    McpToolError::InvalidParams("set_volume requires 'volume_percent' (0..=100)".to_string())
                })?;
                if !pct.is_finite() || !(0.0..=100.0).contains(&pct) {
                    return Err(McpToolError::from(WinctlError::InvalidInput(format!(
                        "volume_percent {pct} out of range 0.0..=100.0"
                    ))));
                }
                if dry_run {
                    return Ok(dry_run_preview(
                        self.command_id(),
                        "Preview CoreAudio volume update without modifying endpoint/session",
                        json!({ "action": "set_volume", "target": req.target, "volume_percent": pct }),
                    ));
                }
                let snap = self
                    .backend
                    .set_volume_scalar(&req.target, pct / 100.0)
                    .map_err(McpToolError::from)?;
                Ok(json!({ "dry_run": false, "applied": true, "snapshot": snap }))
            }
            "set_channel_volumes" => {
                let chs = req.channel_volumes.as_ref().ok_or_else(|| {
                    McpToolError::InvalidParams("set_channel_volumes requires 'channel_volumes' array".to_string())
                })?;
                if chs.is_empty() || chs.iter().any(|v| !v.is_finite() || !(0.0..=100.0).contains(v)) {
                    return Err(McpToolError::from(WinctlError::InvalidInput(
                        "channel_volumes must be a non-empty array of values in 0.0..=100.0".to_string(),
                    )));
                }
                if dry_run {
                    return Ok(dry_run_preview(
                        self.command_id(),
                        "Preview per-channel CoreAudio volume update",
                        json!({ "action": "set_channel_volumes", "target": req.target, "channel_volumes": chs }),
                    ));
                }
                let scalars: Vec<f32> = chs.iter().map(|v| v / 100.0).collect();
                let snap = self
                    .backend
                    .set_channel_volumes(&req.target, &scalars)
                    .map_err(McpToolError::from)?;
                Ok(json!({ "dry_run": false, "applied": true, "snapshot": snap }))
            }
            "set_volume_db" => {
                let db = req
                    .volume_db
                    .ok_or_else(|| McpToolError::InvalidParams("set_volume_db requires 'volume_db'".to_string()))?;
                if !db.is_finite() {
                    return Err(McpToolError::from(WinctlError::InvalidInput(
                        "volume_db must be finite".to_string(),
                    )));
                }
                if dry_run {
                    return Ok(dry_run_preview(
                        self.command_id(),
                        "Preview CoreAudio decibel volume update",
                        json!({ "action": "set_volume_db", "target": req.target, "volume_db": db }),
                    ));
                }
                let snap = self
                    .backend
                    .set_volume_db(&req.target, db)
                    .map_err(McpToolError::from)?;
                Ok(json!({ "dry_run": false, "applied": true, "snapshot": snap }))
            }
            "set_mute" => {
                let mode_str = req.mute_mode.as_deref().unwrap_or("toggle");
                let mode = MuteAction::parse_str(mode_str).map_err(McpToolError::from)?;
                if dry_run {
                    return Ok(dry_run_preview(
                        self.command_id(),
                        "Preview CoreAudio mute state update",
                        json!({ "action": "set_mute", "target": req.target, "mute_mode": mode }),
                    ));
                }
                let snap = self.backend.set_mute(&req.target, mode).map_err(McpToolError::from)?;
                Ok(json!({ "dry_run": false, "applied": true, "snapshot": snap }))
            }
            "set_default" => {
                let roles = parse_roles_opt(req.roles.as_deref())?;
                if dry_run {
                    return Ok(dry_run_preview(
                        self.command_id(),
                        "Preview IPolicyConfig::SetDefaultEndpoint switch",
                        json!({ "action": "set_default", "target": req.target, "roles": roles }),
                    ));
                }
                self.backend
                    .set_default_endpoint(&req.target, &roles)
                    .map_err(McpToolError::from)?;
                Ok(json!({ "dry_run": false, "applied": true, "target": req.target, "roles": roles }))
            }
            "enable" | "disable" => {
                let enabled = action_norm == "enable";
                if dry_run {
                    return Ok(dry_run_preview(
                        self.command_id(),
                        "Preview IPolicyConfig::SetEndpointVisibility",
                        json!({ "action": action_norm, "target": req.target, "enabled": enabled }),
                    ));
                }
                self.backend
                    .set_endpoint_enabled(&req.target, enabled)
                    .map_err(McpToolError::from)?;
                Ok(json!({ "dry_run": false, "applied": true, "target": req.target, "enabled": enabled }))
            }
            "set_app_default" => {
                let app = req
                    .app_target
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| McpToolError::InvalidParams("set_app_default requires 'app_target'".to_string()))?;
                let roles = parse_roles_opt(req.roles.as_deref())?;
                if dry_run {
                    return Ok(dry_run_preview(
                        self.command_id(),
                        "Preview per-application default audio endpoint routing",
                        json!({ "action": "set_app_default", "target": req.target, "app_target": app, "roles": roles }),
                    ));
                }
                self.backend
                    .set_app_default_device(app, &req.target, &roles)
                    .map_err(McpToolError::from)?;
                Ok(
                    json!({ "dry_run": false, "applied": true, "target": req.target, "app_target": app, "roles": roles }),
                )
            }
            other => Err(McpToolError::InvalidParams(format!(
                "unsupported audio_view_control action '{other}'"
            ))),
        }
    }
}

fn parse_roles_opt(roles: Option<&[String]>) -> Result<Vec<DefaultRole>, McpToolError> {
    let Some(list) = roles else {
        return Ok(vec![
            DefaultRole::Console,
            DefaultRole::Multimedia,
            DefaultRole::Communications,
        ]);
    };
    if list.is_empty() {
        return Ok(vec![
            DefaultRole::Console,
            DefaultRole::Multimedia,
            DefaultRole::Communications,
        ]);
    }
    let mut parsed = Vec::new();
    for raw in list {
        for r in DefaultRole::parse_role_list(raw).map_err(McpToolError::from)? {
            if !parsed.contains(&r) {
                parsed.push(r);
            }
        }
    }
    Ok(parsed)
}
