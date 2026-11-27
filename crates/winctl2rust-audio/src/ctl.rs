use std::sync::Arc;
use std::thread;
use std::time::Duration;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::{AudioBackend, DefaultRole, MuteAction, resolve_audio_target};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveFileEncoding {
    Utf8,
    Utf16,
    Ascii,
}

impl SaveFileEncoding {
    pub fn parse_str(raw: &str) -> WinctlResult<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "utf8" | "utf-8" | "1" => Ok(Self::Utf8),
            "utf16" | "utf-16" | "2" => Ok(Self::Utf16),
            "ascii" | "ansi" | "0" => Ok(Self::Ascii),
            other => Err(WinctlError::ParseError(format!(
                "unsupported /SaveFileEncoding '{other}' (expected utf8, utf16, or ascii)"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum AudioCtlVerb {
    SetVolume {
        target: String,
        percent: f32,
    },
    SetVolumeChannels {
        target: String,
        channels: Vec<f32>,
    },
    ChangeVolume {
        target: String,
        delta_percent: f32,
    },
    SetVolumeDecibel {
        target: String,
        db: f32,
    },
    ChangeVolumeDecibel {
        target: String,
        delta_db: f32,
    },
    Mute {
        target: String,
    },
    Unmute {
        target: String,
    },
    Switch {
        target: String,
    },
    SetDefault {
        target: String,
        roles: Vec<DefaultRole>,
    },
    SetAppDefault {
        target: String,
        roles: Vec<DefaultRole>,
        process_or_pid: String,
    },
    Enable {
        target: String,
    },
    Disable {
        target: String,
    },
    GetPercent {
        target: String,
    },
    GetMute {
        target: String,
    },
    GetDecibel {
        target: String,
    },
}

impl AudioCtlVerb {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SetVolume { .. } => "/SetVolume",
            Self::SetVolumeChannels { .. } => "/SetVolumeChannels",
            Self::ChangeVolume { .. } => "/ChangeVolume",
            Self::SetVolumeDecibel { .. } => "/SetVolumeDecibel",
            Self::ChangeVolumeDecibel { .. } => "/ChangeVolumeDecibel",
            Self::Mute { .. } => "/Mute",
            Self::Unmute { .. } => "/Unmute",
            Self::Switch { .. } => "/Switch",
            Self::SetDefault { .. } => "/SetDefault",
            Self::SetAppDefault { .. } => "/SetAppDefault",
            Self::Enable { .. } => "/Enable",
            Self::Disable { .. } => "/Disable",
            Self::GetPercent { .. } => "/GetPercent",
            Self::GetMute { .. } => "/GetMute",
            Self::GetDecibel { .. } => "/GetDecibel",
        }
    }

    pub fn target(&self) -> &str {
        match self {
            Self::SetVolume { target, .. }
            | Self::SetVolumeChannels { target, .. }
            | Self::ChangeVolume { target, .. }
            | Self::SetVolumeDecibel { target, .. }
            | Self::ChangeVolumeDecibel { target, .. }
            | Self::Mute { target }
            | Self::Unmute { target }
            | Self::Switch { target }
            | Self::SetDefault { target, .. }
            | Self::SetAppDefault { target, .. }
            | Self::Enable { target }
            | Self::Disable { target }
            | Self::GetPercent { target }
            | Self::GetMute { target }
            | Self::GetDecibel { target } => target,
        }
    }

    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::GetPercent { .. } | Self::GetMute { .. } | Self::GetDecibel { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedAudioCtlCommand {
    pub verb: AudioCtlVerb,
    pub wait_ms: Option<u64>,
    pub save_file_encoding: Option<SaveFileEncoding>,
    pub raw_tokens: Vec<String>,
}

pub fn tokenize_ctl_args(input: &str) -> WinctlResult<Vec<String>> {
    Ok(tokenize_ctl_args_with_quotes(input)?
        .into_iter()
        .map(|(tok, _)| tok)
        .collect())
}

fn tokenize_ctl_args_with_quotes(input: &str) -> WinctlResult<Vec<(String, bool)>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut started = false;
    let mut was_quoted = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '\\' => {
                    if let Some(&next) = chars.peek()
                        && (next == '"' || next == '\\')
                    {
                        current.push(next);
                        chars.next();
                    } else {
                        current.push('\\');
                    }
                }
                '"' => in_quotes = false,
                other => current.push(other),
            }
        } else {
            match c {
                '"' => {
                    in_quotes = true;
                    started = true;
                    was_quoted = true;
                }
                ch if ch.is_whitespace() => {
                    if started {
                        tokens.push((std::mem::take(&mut current), was_quoted));
                        started = false;
                        was_quoted = false;
                    }
                }
                other => {
                    current.push(other);
                    started = true;
                }
            }
        }
    }

    if in_quotes {
        return Err(WinctlError::ParseError(
            "unterminated double quote in audio ctl command line".to_string(),
        ));
    }
    if started {
        tokens.push((current, was_quoted));
    }
    if tokens.is_empty() {
        return Err(WinctlError::InvalidInput(
            "audio ctl command line must not be empty".to_string(),
        ));
    }
    Ok(tokens)
}

fn strip_verb_prefix(tok: &str) -> Option<String> {
    if let Some(rest) = tok.strip_prefix('/') {
        Some(rest.to_ascii_lowercase())
    } else {
        tok.strip_prefix("--").map(|rest| rest.to_ascii_lowercase())
    }
}

pub fn parse_ctl_command(command_line: &str) -> WinctlResult<ParsedAudioCtlCommand> {
    let pairs = tokenize_ctl_args_with_quotes(command_line)?;
    let raw_tokens: Vec<String> = pairs.iter().map(|(t, _)| t.clone()).collect();
    validate_r4_compliance(&raw_tokens[0], &raw_tokens[1..])?;
    parse_ctl_token_pairs(&pairs)
}

pub fn parse_ctl_tokens(tokens: &[String]) -> WinctlResult<ParsedAudioCtlCommand> {
    let pairs: Vec<(String, bool)> = tokens.iter().map(|t| (t.clone(), false)).collect();
    parse_ctl_token_pairs(&pairs)
}

fn parse_ctl_token_pairs(tokens: &[(String, bool)]) -> WinctlResult<ParsedAudioCtlCommand> {
    if tokens.is_empty() {
        return Err(WinctlError::InvalidInput(
            "audio ctl command must not be empty".to_string(),
        ));
    }

    let mut wait_ms: Option<u64> = None;
    let mut save_file_encoding: Option<SaveFileEncoding> = None;
    let mut primary_verb: Option<String> = None;
    let mut primary_args: Vec<String> = Vec::new();

    let mut i = 0usize;
    while i < tokens.len() {
        let (tok, was_quoted) = &tokens[i];
        let opt_verb = if *was_quoted { None } else { strip_verb_prefix(tok) };
        if let Some(opt) = opt_verb {
            match opt.as_str() {
                "savefileencoding" => {
                    let val = tokens.get(i + 1).map(|(t, _)| t).ok_or_else(|| {
                        WinctlError::ParseError("/SaveFileEncoding requires <utf8|utf16|ascii>".to_string())
                    })?;
                    save_file_encoding = Some(SaveFileEncoding::parse_str(val)?);
                    i += 2;
                    continue;
                }
                "wait" => {
                    let val = tokens
                        .get(i + 1)
                        .map(|(t, _)| t)
                        .ok_or_else(|| WinctlError::ParseError("/Wait requires <milliseconds>".to_string()))?;
                    let ms = val
                        .parse::<u64>()
                        .map_err(|_| WinctlError::ParseError(format!("invalid /Wait duration '{val}'")))?;
                    if ms > 30_000 {
                        return Err(WinctlError::InvalidInput(format!(
                            "/Wait duration {ms}ms exceeds maximum safe bound (30000ms)"
                        )));
                    }
                    wait_ms = Some(ms);
                    i += 2;
                    continue;
                }
                other => {
                    if primary_verb.is_some() {
                        return Err(WinctlError::ParseError(format!(
                            "unexpected second audio ctl verb '/{other}'"
                        )));
                    }
                    primary_verb = Some(other.to_string());
                    i += 1;
                    while i < tokens.len() && (tokens[i].1 || strip_verb_prefix(&tokens[i].0).is_none()) {
                        primary_args.push(tokens[i].0.clone());
                        i += 1;
                    }
                    continue;
                }
            }
        }
        if primary_verb.is_some() {
            primary_args.push(tok.clone());
            i += 1;
        } else {
            return Err(WinctlError::ParseError(format!(
                "expected audio ctl /Verb before argument '{tok}'"
            )));
        }
    }

    let verb_name = primary_verb
        .ok_or_else(|| WinctlError::ParseError("audio ctl command line missing primary /Verb".to_string()))?;

    let verb = match verb_name.as_str() {
        "setvolume" => {
            if primary_args.len() != 2 {
                return Err(WinctlError::ParseError(
                    "/SetVolume requires <Target> <0..=100>".to_string(),
                ));
            }
            let target = require_non_empty_target(&primary_args[0], "/SetVolume")?;
            let pct = parse_percent(&primary_args[1])?;
            AudioCtlVerb::SetVolume { target, percent: pct }
        }
        "setvolumechannels" => {
            if primary_args.len() < 2 {
                return Err(WinctlError::ParseError(
                    "/SetVolumeChannels requires <Target> <ch0> [ch1 ...]".to_string(),
                ));
            }
            let target = require_non_empty_target(&primary_args[0], "/SetVolumeChannels")?;
            let mut chs = Vec::with_capacity(primary_args.len() - 1);
            for raw in &primary_args[1..] {
                chs.push(parse_percent(raw)?);
            }
            AudioCtlVerb::SetVolumeChannels { target, channels: chs }
        }
        "changevolume" => {
            if primary_args.len() != 2 {
                return Err(WinctlError::ParseError(
                    "/ChangeVolume requires <Target> <delta_percent>".to_string(),
                ));
            }
            let target = require_non_empty_target(&primary_args[0], "/ChangeVolume")?;
            let delta = parse_signed_f32(&primary_args[1], "/ChangeVolume delta")?;
            if !(-100.0..=100.0).contains(&delta) {
                return Err(WinctlError::InvalidInput(format!(
                    "/ChangeVolume delta {delta} out of range -100..=100"
                )));
            }
            AudioCtlVerb::ChangeVolume {
                target,
                delta_percent: delta,
            }
        }
        "setvolumedecibel" => {
            if primary_args.len() != 2 {
                return Err(WinctlError::ParseError(
                    "/SetVolumeDecibel requires <Target> <dB>".to_string(),
                ));
            }
            let target = require_non_empty_target(&primary_args[0], "/SetVolumeDecibel")?;
            let db = parse_signed_f32(&primary_args[1], "/SetVolumeDecibel dB")?;
            AudioCtlVerb::SetVolumeDecibel { target, db }
        }
        "changevolumedecibel" => {
            if primary_args.len() != 2 {
                return Err(WinctlError::ParseError(
                    "/ChangeVolumeDecibel requires <Target> <delta_dB>".to_string(),
                ));
            }
            let target = require_non_empty_target(&primary_args[0], "/ChangeVolumeDecibel")?;
            let delta_db = parse_signed_f32(&primary_args[1], "/ChangeVolumeDecibel delta_dB")?;
            AudioCtlVerb::ChangeVolumeDecibel { target, delta_db }
        }
        "mute" => {
            let target = require_single_target(&primary_args, "/Mute")?;
            AudioCtlVerb::Mute { target }
        }
        "unmute" => {
            let target = require_single_target(&primary_args, "/Unmute")?;
            AudioCtlVerb::Unmute { target }
        }
        "switch" => {
            let target = require_single_target(&primary_args, "/Switch")?;
            AudioCtlVerb::Switch { target }
        }
        "setdefault" => {
            if primary_args.len() != 2 {
                return Err(WinctlError::ParseError(
                    "/SetDefault requires <DeviceTarget> <0|1|2|all>".to_string(),
                ));
            }
            let target = require_non_empty_target(&primary_args[0], "/SetDefault")?;
            let roles = DefaultRole::parse_role_list(&primary_args[1])?;
            AudioCtlVerb::SetDefault { target, roles }
        }
        "setappdefault" => {
            if primary_args.len() != 3 {
                return Err(WinctlError::ParseError(
                    "/SetAppDefault requires <DeviceTarget> <0|1|2|all> <ProcessOrPID>".to_string(),
                ));
            }
            let target = require_non_empty_target(&primary_args[0], "/SetAppDefault")?;
            let roles = DefaultRole::parse_role_list(&primary_args[1])?;
            if primary_args[2].trim().is_empty() {
                return Err(WinctlError::ParseError(
                    "/SetAppDefault requires a non-empty <ProcessOrPID>".to_string(),
                ));
            }
            AudioCtlVerb::SetAppDefault {
                target,
                roles,
                process_or_pid: primary_args[2].clone(),
            }
        }
        "enable" => {
            let target = require_single_target(&primary_args, "/Enable")?;
            AudioCtlVerb::Enable { target }
        }
        "disable" => {
            let target = require_single_target(&primary_args, "/Disable")?;
            AudioCtlVerb::Disable { target }
        }
        "getpercent" => {
            let target = require_single_target(&primary_args, "/GetPercent")?;
            AudioCtlVerb::GetPercent { target }
        }
        "getmute" => {
            let target = require_single_target(&primary_args, "/GetMute")?;
            AudioCtlVerb::GetMute { target }
        }
        "getdecibel" => {
            let target = require_single_target(&primary_args, "/GetDecibel")?;
            AudioCtlVerb::GetDecibel { target }
        }
        other => {
            return Err(WinctlError::ParseError(format!(
                "unsupported audio ctl verb '/{other}'"
            )));
        }
    };

    Ok(ParsedAudioCtlCommand {
        verb,
        wait_ms,
        save_file_encoding,
        raw_tokens: tokens.iter().map(|(t, _)| t.clone()).collect(),
    })
}

fn require_non_empty_target(raw: &str, verb: &str) -> WinctlResult<String> {
    if raw.trim().is_empty() {
        return Err(WinctlError::ParseError(format!("{verb} requires a non-empty <Target>")));
    }
    Ok(raw.to_string())
}

fn require_single_target(args: &[String], verb: &str) -> WinctlResult<String> {
    if args.len() != 1 || args[0].trim().is_empty() {
        return Err(WinctlError::ParseError(format!("{verb} requires <Target>")));
    }
    Ok(args[0].clone())
}

fn parse_percent(raw: &str) -> WinctlResult<f32> {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_suffix('%').unwrap_or(trimmed);
    let v = trimmed
        .parse::<f32>()
        .map_err(|_| WinctlError::ParseError(format!("invalid volume percentage '{raw}'")))?;
    if !v.is_finite() || !(0.0..=100.0).contains(&v) {
        return Err(WinctlError::InvalidInput(format!(
            "volume percentage {v} out of range 0.0..=100.0"
        )));
    }
    Ok(v)
}

fn parse_signed_f32(raw: &str, label: &str) -> WinctlResult<f32> {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_suffix('%').unwrap_or(trimmed);
    let v = trimmed
        .parse::<f32>()
        .map_err(|_| WinctlError::ParseError(format!("invalid {label} '{raw}'")))?;
    if !v.is_finite() {
        return Err(WinctlError::InvalidInput(format!("{label} must be finite")));
    }
    Ok(v)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioCtlExecutionResult {
    pub command: String,
    pub target: String,
    pub read_only: bool,
    pub previous_state: Option<Value>,
    pub new_state: Option<Value>,
    pub result_value: Value,
    pub exit_code: i32,
    pub applied: bool,
}

pub fn execute_ctl(
    backend: &dyn AudioBackend,
    parsed: &ParsedAudioCtlCommand,
) -> WinctlResult<AudioCtlExecutionResult> {
    let items = backend.list_audio_items()?;
    let item = resolve_audio_target(&items, parsed.verb.target())?.clone();
    let prev_json = json!({
        "item_id": item.item_id,
        "name": item.name,
        "volume_percent": item.volume_percent,
        "volume_db": item.volume_db,
        "muted": item.muted,
        "state": item.state,
        "default_roles": item.default_roles,
    });

    if let Some(ms) = parsed.wait_ms
        && ms > 0
    {
        thread::sleep(Duration::from_millis(ms));
    }

    let (new_state, result_value, exit_code, applied) = match &parsed.verb {
        AudioCtlVerb::SetVolume { percent, .. } => {
            let snap = backend.set_volume_scalar(&item.item_id, *percent / 100.0)?;
            (Some(json!(snap)), json!(snap.volume_percent), 0, true)
        }
        AudioCtlVerb::SetVolumeChannels { channels, .. } => {
            let scalars: Vec<f32> = channels.iter().map(|c| c / 100.0).collect();
            let snap = backend.set_channel_volumes(&item.item_id, &scalars)?;
            (Some(json!(snap)), json!(snap.channel_volumes), 0, true)
        }
        AudioCtlVerb::ChangeVolume { delta_percent, .. } => {
            let next = (item.volume_percent + *delta_percent).clamp(0.0, 100.0);
            let snap = backend.set_volume_scalar(&item.item_id, next / 100.0)?;
            (Some(json!(snap)), json!(snap.volume_percent), 0, true)
        }
        AudioCtlVerb::SetVolumeDecibel { db, .. } => {
            let snap = backend.set_volume_db(&item.item_id, *db)?;
            (Some(json!(snap)), json!(snap.volume_db), 0, true)
        }
        AudioCtlVerb::ChangeVolumeDecibel { delta_db, .. } => {
            let next_db = item.volume_db + *delta_db;
            let snap = backend.set_volume_db(&item.item_id, next_db)?;
            (Some(json!(snap)), json!(snap.volume_db), 0, true)
        }
        AudioCtlVerb::Mute { .. } => {
            let snap = backend.set_mute(&item.item_id, MuteAction::Mute)?;
            (Some(json!(snap)), json!(snap.muted), 0, true)
        }
        AudioCtlVerb::Unmute { .. } => {
            let snap = backend.set_mute(&item.item_id, MuteAction::Unmute)?;
            (Some(json!(snap)), json!(snap.muted), 0, true)
        }
        AudioCtlVerb::Switch { .. } => {
            let snap = backend.set_mute(&item.item_id, MuteAction::Toggle)?;
            (Some(json!(snap)), json!(snap.muted), 0, true)
        }
        AudioCtlVerb::SetDefault { roles, .. } => {
            backend.set_default_endpoint(&item.item_id, roles)?;
            (Some(json!({ "default_roles": roles })), json!(roles), 0, true)
        }
        AudioCtlVerb::SetAppDefault {
            roles, process_or_pid, ..
        } => {
            backend.set_app_default_device(process_or_pid, &item.item_id, roles)?;
            (
                Some(json!({ "app": process_or_pid, "device_id": item.item_id, "roles": roles })),
                json!({ "app": process_or_pid, "roles": roles }),
                0,
                true,
            )
        }
        AudioCtlVerb::Enable { .. } => {
            backend.set_endpoint_enabled(&item.item_id, true)?;
            (Some(json!({ "state": "Active" })), json!(true), 0, true)
        }
        AudioCtlVerb::Disable { .. } => {
            backend.set_endpoint_enabled(&item.item_id, false)?;
            (Some(json!({ "state": "Disabled" })), json!(false), 0, true)
        }
        AudioCtlVerb::GetPercent { .. } => {
            let code = (item.volume_percent * 10.0).round() as i32;
            (None, json!(item.volume_percent), code, false)
        }
        AudioCtlVerb::GetMute { .. } => {
            let code = i32::from(item.muted);
            (None, json!(item.muted), code, false)
        }
        AudioCtlVerb::GetDecibel { .. } => {
            let code = (item.volume_db * 10.0).round() as i32;
            (None, json!(item.volume_db), code, false)
        }
    };

    Ok(AudioCtlExecutionResult {
        command: parsed.verb.name().to_string(),
        target: item.item_id,
        read_only: parsed.verb.is_read_only(),
        previous_state: Some(prev_json),
        new_state,
        result_value,
        exit_code,
        applied,
    })
}

pub struct AudioCtlStatusTableProvider {
    backend: Arc<dyn AudioBackend>,
}

impl AudioCtlStatusTableProvider {
    pub fn new(backend: Arc<dyn AudioBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for AudioCtlStatusTableProvider {
    fn tool_id(&self) -> &'static str {
        "audio_ctl_status"
    }

    fn description(&self) -> &'static str {
        "Inspect CoreAudio CLI verb grammar and live resolvable target selectors"
    }

    fn schema(&self) -> TableSchema {
        TableSchema::new(
            "audio_ctl_status",
            "CoreAudio CLI Target & Verb Status",
            vec![
                ColumnDef::new("command_line_id", "Target Selector", ColumnType::Text).with_width_hint(260),
                ColumnDef::new("name", "Name", ColumnType::Text).with_width_hint(200),
                ColumnDef::new("kind", "Kind", ColumnType::Text).with_width_hint(90),
                ColumnDef::new("volume_percent", "Volume %", ColumnType::Float).with_width_hint(90),
                ColumnDef::new("volume_db", "Volume dB", ColumnType::Float).with_width_hint(90),
                ColumnDef::new("muted", "Muted", ColumnType::Bool).with_width_hint(70),
                ColumnDef::new("example_ctl", "Example Command", ColumnType::Text).with_width_hint(320),
            ],
        )
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
            .into_iter()
            .enumerate()
            .map(|(idx, item)| {
                let example = format!("/SetVolume \"{}\" {:.0}", item.command_line_id, item.volume_percent);
                Row::new(
                    idx as u64 + 1,
                    vec![
                        CellValue::Text(item.command_line_id.clone()),
                        CellValue::Text(item.name.clone()),
                        CellValue::Text(item.kind.as_str().to_string()),
                        CellValue::Float(f64::from(item.volume_percent)),
                        CellValue::Float(f64::from(item.volume_db)),
                        CellValue::Bool(item.muted),
                        CellValue::Text(example),
                    ],
                )
                .with_detail_json(serde_json::to_value(&item).unwrap_or(Value::Null))
            })
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

pub struct AudioCtlAction {
    backend: Arc<dyn AudioBackend>,
}

impl AudioCtlAction {
    pub fn new(backend: Arc<dyn AudioBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for AudioCtlAction {
    fn command_id(&self) -> &'static str {
        "audio_ctl"
    }

    fn description(&self) -> &'static str {
        "Parse and execute CoreAudio CLI control verbs against CoreAudio endpoints and sessions"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command_line": {
                    "type": "string",
                    "description": "CoreAudio CLI expression, e.g. '/SetVolume \"Speakers\" 50' or '/Mute vlc.exe'"
                },
                "verb": { "type": "string" },
                "target": { "type": "string" },
                "args": { "type": "array", "items": { "type": "string" } }
            }
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let parsed = if let Some(cmd) = args.get("command_line").and_then(Value::as_str) {
            parse_ctl_command(cmd).map_err(McpToolError::from)?
        } else if let Some(verb) = args.get("verb").and_then(Value::as_str) {
            let target = args
                .get("target")
                .and_then(Value::as_str)
                .ok_or_else(|| McpToolError::InvalidParams("structured audio_ctl requires 'target'".to_string()))?;
            let mut pairs = Vec::new();
            let mut raw_tokens = Vec::new();
            let verb_tok = if verb.starts_with('/') || verb.starts_with("--") {
                verb.to_string()
            } else {
                format!("/{verb}")
            };
            raw_tokens.push(verb_tok.clone());
            pairs.push((verb_tok, false));
            raw_tokens.push(target.to_string());
            pairs.push((target.to_string(), true));
            if let Some(extra) = args.get("args").and_then(Value::as_array) {
                for v in extra {
                    let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                    raw_tokens.push(s.clone());
                    pairs.push((s, true));
                }
            }
            validate_r4_compliance(&raw_tokens[0], &raw_tokens[1..]).map_err(McpToolError::from)?;
            parse_ctl_token_pairs(&pairs).map_err(McpToolError::from)?
        } else {
            return Err(McpToolError::InvalidParams(
                "audio_ctl requires 'command_line' or structured { verb, target }".to_string(),
            ));
        };

        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview CoreAudio CLI verb execution without modifying audio state",
                json!({
                    "verb": parsed.verb.name(),
                    "target": parsed.verb.target(),
                    "read_only": parsed.verb.is_read_only(),
                    "parsed": parsed,
                }),
            ));
        }

        let res = execute_ctl(self.backend.as_ref(), &parsed).map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": res.applied,
            "result": res,
        }))
    }
}
