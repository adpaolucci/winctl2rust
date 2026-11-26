use std::collections::BTreeMap;
use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::CmdBackend;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunAsMode {
    #[default]
    CurrentUser,
    Administrator,
}

impl RunAsMode {
    pub fn parse_str(raw: &str) -> WinctlResult<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "current" | "currentuser" | "current_user" | "normal" => Ok(Self::CurrentUser),
            "admin" | "administrator" | "elevated" | "runas" => Ok(Self::Administrator),
            other => Err(WinctlError::ParseError(format!("unsupported run_as_mode '{other}'"))),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriorityClass {
    Idle,
    BelowNormal,
    #[default]
    Normal,
    AboveNormal,
    High,
    Realtime,
}

impl PriorityClass {
    pub fn parse_str(raw: &str) -> WinctlResult<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "normal" => Ok(Self::Normal),
            "idle" | "low" => Ok(Self::Idle),
            "belownormal" | "below_normal" => Ok(Self::BelowNormal),
            "abovenormal" | "above_normal" => Ok(Self::AboveNormal),
            "high" => Ok(Self::High),
            "realtime" => Ok(Self::Realtime),
            other => Err(WinctlError::ParseError(format!("unsupported priority_class '{other}'"))),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowState {
    #[default]
    Normal,
    Hidden,
    Minimized,
    Maximized,
}

impl WindowState {
    pub fn parse_str(raw: &str) -> WinctlResult<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "normal" | "show" | "shownormal" => Ok(Self::Normal),
            "hide" | "hidden" => Ok(Self::Hidden),
            "min" | "minimize" | "minimized" => Ok(Self::Minimized),
            "max" | "maximize" | "maximized" => Ok(Self::Maximized),
            other => Err(WinctlError::ParseError(format!("unsupported window_state '{other}'"))),
        }
    }

    pub fn sw_code(self) -> i32 {
        match self {
            Self::Normal => 1,
            Self::Hidden => 0,
            Self::Minimized => 6,
            Self::Maximized => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchProfile {
    pub name: String,
    pub exe_path: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub run_as_mode: RunAsMode,
    pub priority_class: PriorityClass,
    pub affinity_mask: Option<usize>,
    pub window_state: WindowState,
    pub env_overrides: BTreeMap<String, String>,
    pub inherit_env: bool,
    pub compatibility_layer: Vec<String>,
    pub new_console: bool,
    pub no_window: bool,
    pub detached: bool,
    pub wait_for_exit: bool,
}

impl Default for LaunchProfile {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            exe_path: String::new(),
            args: Vec::new(),
            working_dir: None,
            run_as_mode: RunAsMode::CurrentUser,
            priority_class: PriorityClass::Normal,
            affinity_mask: None,
            window_state: WindowState::Normal,
            env_overrides: BTreeMap::new(),
            inherit_env: true,
            compatibility_layer: Vec::new(),
            new_console: false,
            no_window: false,
            detached: false,
            wait_for_exit: false,
        }
    }
}

impl LaunchProfile {
    pub fn validate(&self) -> WinctlResult<()> {
        if self.exe_path.trim().is_empty() {
            return Err(WinctlError::InvalidInput(
                "LaunchProfile.exe_path must not be empty".to_string(),
            ));
        }
        let mut r4_extra = self.args.clone();
        if let Some(ref wd) = self.working_dir {
            r4_extra.push(wd.clone());
        }
        for (k, v) in &self.env_overrides {
            r4_extra.push(k.clone());
            r4_extra.push(v.clone());
        }
        validate_r4_compliance(&self.exe_path, &r4_extra)?;
        if let Some(mask) = self.affinity_mask
            && mask == 0
        {
            return Err(WinctlError::InvalidInput(
                "LaunchProfile.affinity_mask must be non-zero when specified".to_string(),
            ));
        }
        if self.new_console && self.detached {
            return Err(WinctlError::InvalidInput(
                "LaunchProfile cannot set both new_console=true and detached=true".to_string(),
            ));
        }
        for (k, v) in &self.env_overrides {
            if k.is_empty() || k.contains('=') || k.contains('\0') || v.contains('\0') {
                return Err(WinctlError::InvalidInput(format!(
                    "invalid environment override key/value for '{k}'"
                )));
            }
        }
        Ok(())
    }

    pub fn build_command_line(&self) -> String {
        let mut out = quote_windows_arg(&self.exe_path);
        for arg in &self.args {
            out.push(' ');
            out.push_str(&quote_windows_arg(arg));
        }
        out
    }

    pub fn build_utf16_env_block(&self) -> Vec<u16> {
        let mut merged: BTreeMap<String, (String, String)> = BTreeMap::new();
        if self.inherit_env {
            for (k, v) in std::env::vars() {
                merged.insert(k.to_uppercase(), (k, v));
            }
        }
        for (k, v) in &self.env_overrides {
            merged.insert(k.to_uppercase(), (k.clone(), v.clone()));
        }

        const COMPAT_KEY: &str = "__COMPAT_LAYER";
        if !self.compatibility_layer.is_empty() {
            let mut tokens: Vec<String> = Vec::new();
            if let Some((_, existing)) = merged.get(COMPAT_KEY) {
                for tok in existing.split_whitespace() {
                    if !tokens.iter().any(|t| t.eq_ignore_ascii_case(tok)) {
                        tokens.push(tok.to_string());
                    }
                }
            }
            for raw in &self.compatibility_layer {
                for tok in raw.split_whitespace() {
                    if !tokens.iter().any(|t| t.eq_ignore_ascii_case(tok)) {
                        tokens.push(tok.to_string());
                    }
                }
            }
            if !tokens.is_empty() {
                let key_name = merged
                    .get(COMPAT_KEY)
                    .map(|(k, _)| k.clone())
                    .unwrap_or_else(|| COMPAT_KEY.to_string());
                merged.insert(COMPAT_KEY.to_string(), (key_name, tokens.join(" ")));
            }
        }

        let mut entries: Vec<(String, String)> = merged.into_values().collect();
        entries.sort_by(|(ka, _), (kb, _)| ka.to_uppercase().encode_utf16().cmp(kb.to_uppercase().encode_utf16()));

        let mut block = Vec::new();
        for (k, v) in entries {
            let entry = format!("{k}={v}");
            block.extend(entry.encode_utf16());
            block.push(0);
        }
        if block.is_empty() {
            block.push(0);
        }
        block.push(0);
        block
    }
}

pub fn tokenize_windows_args(line: &str) -> WinctlResult<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut token_started = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if !in_quotes && (c == ' ' || c == '\t') {
            if token_started {
                tokens.push(std::mem::take(&mut current));
                token_started = false;
            }
            continue;
        }
        token_started = true;
        match c {
            '\\' => {
                let mut backslashes = 1usize;
                while matches!(chars.peek(), Some('\\')) {
                    chars.next();
                    backslashes += 1;
                }
                if matches!(chars.peek(), Some('"')) {
                    for _ in 0..(backslashes / 2) {
                        current.push('\\');
                    }
                    if backslashes % 2 == 1 {
                        chars.next();
                        current.push('"');
                    }
                } else {
                    for _ in 0..backslashes {
                        current.push('\\');
                    }
                }
            }
            '"' => {
                if in_quotes && matches!(chars.peek(), Some('"')) {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = !in_quotes;
                }
            }
            other => current.push(other),
        }
    }

    if in_quotes {
        return Err(WinctlError::ParseError(
            "unterminated double quote in launch profile args".to_string(),
        ));
    }
    if token_started {
        tokens.push(current);
    }
    Ok(tokens)
}

pub fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
        return arg.to_string();
    }
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                for _ in 0..(backslashes * 2 + 1) {
                    quoted.push('\\');
                }
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                for _ in 0..backslashes {
                    quoted.push('\\');
                }
                backslashes = 0;
                quoted.push(other);
            }
        }
    }
    for _ in 0..(backslashes * 2) {
        quoted.push('\\');
    }
    quoted.push('"');
    quoted
}

pub fn parse_profile_json(raw: &str) -> WinctlResult<LaunchProfile> {
    let profile: LaunchProfile =
        serde_json::from_str(raw).map_err(|e| WinctlError::ParseError(format!("invalid launch profile JSON: {e}")))?;
    profile.validate()?;
    Ok(profile)
}

pub fn parse_profile_ini(raw: &str) -> WinctlResult<LaunchProfile> {
    let mut profile = LaunchProfile::default();
    let mut in_env_section = false;

    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = trimmed[1..trimmed.len() - 1].trim();
            in_env_section = section.eq_ignore_ascii_case("Environment") || section.eq_ignore_ascii_case("Env");
            continue;
        }
        let Some((k, v)) = trimmed.split_once('=') else {
            return Err(WinctlError::ParseError(format!(
                "INI line {} missing '=': {trimmed}",
                idx + 1
            )));
        };
        let key = k.trim();
        let val = v.trim();
        if in_env_section {
            profile.env_overrides.insert(key.to_string(), val.to_string());
            continue;
        }
        match key.to_ascii_lowercase().as_str() {
            "name" | "profilename" => profile.name = val.to_string(),
            "exepath" | "exe_path" | "command" => profile.exe_path = val.to_string(),
            "args" | "arguments" => {
                profile.args = if val.is_empty() {
                    Vec::new()
                } else {
                    tokenize_windows_args(val)?
                };
            }
            "workingdir" | "working_dir" => {
                profile.working_dir = if val.is_empty() { None } else { Some(val.to_string()) };
            }
            "runas" | "runasmode" | "run_as_mode" => profile.run_as_mode = RunAsMode::parse_str(val)?,
            "priority" | "priorityclass" | "priority_class" => {
                profile.priority_class = PriorityClass::parse_str(val)?;
            }
            "affinity" | "affinitymask" | "affinity_mask" => {
                if val.is_empty() {
                    profile.affinity_mask = None;
                } else {
                    let parsed = if let Some(hex) = val.strip_prefix("0x").or_else(|| val.strip_prefix("0X")) {
                        usize::from_str_radix(hex, 16)
                    } else {
                        val.parse::<usize>()
                    }
                    .map_err(|_| WinctlError::ParseError(format!("invalid affinity mask '{val}'")))?;
                    profile.affinity_mask = Some(parsed);
                }
            }
            "windowstate" | "window_state" => profile.window_state = WindowState::parse_str(val)?,
            "inheritenv" | "inherit_env" => profile.inherit_env = parse_bool_str(val)?,
            "compat" | "compatibilitylayer" | "compatibility_layer" => {
                profile.compatibility_layer = val
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "newconsole" | "new_console" => profile.new_console = parse_bool_str(val)?,
            "nowindow" | "no_window" => profile.no_window = parse_bool_str(val)?,
            "detached" => profile.detached = parse_bool_str(val)?,
            "waitforexit" | "wait_for_exit" => profile.wait_for_exit = parse_bool_str(val)?,
            _ => {}
        }
    }

    profile.validate()?;
    Ok(profile)
}

fn parse_bool_str(raw: &str) -> WinctlResult<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(WinctlError::ParseError(format!("invalid boolean '{other}'"))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchResult {
    pub pid: Option<u32>,
    pub command_line: String,
    pub run_as: String,
    pub exit_code: Option<u32>,
}

pub struct RunProfilesTableProvider {
    profiles: Vec<LaunchProfile>,
}

impl Default for RunProfilesTableProvider {
    fn default() -> Self {
        Self {
            profiles: vec![
                LaunchProfile {
                    name: "notepad-normal".to_string(),
                    exe_path: "notepad.exe".to_string(),
                    args: vec![],
                    priority_class: PriorityClass::Normal,
                    window_state: WindowState::Normal,
                    ..LaunchProfile::default()
                },
                LaunchProfile {
                    name: "cmd-high-compat".to_string(),
                    exe_path: "C:\\Windows\\System32\\cmd.exe".to_string(),
                    args: vec!["/c".to_string(), "echo".to_string(), "ready".to_string()],
                    priority_class: PriorityClass::AboveNormal,
                    affinity_mask: Some(0b11),
                    compatibility_layer: vec!["HIGHDPIAWARE".to_string()],
                    no_window: true,
                    ..LaunchProfile::default()
                },
            ],
        }
    }
}

impl RunProfilesTableProvider {
    pub fn with_profiles(profiles: Vec<LaunchProfile>) -> Self {
        Self { profiles }
    }
}

impl TableProvider for RunProfilesTableProvider {
    fn tool_id(&self) -> &'static str {
        "cmd_run_profiles"
    }

    fn description(&self) -> &'static str {
        "Inspect and validate AdvancedRun launch profiles"
    }

    fn schema(&self) -> TableSchema {
        TableSchema::new(
            "cmd_run_profiles",
            "AdvancedRun Launch Profiles",
            vec![
                ColumnDef::new("profile_name", "Profile", ColumnType::Text).with_width_hint(140),
                ColumnDef::new("exe_path", "Executable", ColumnType::Text).with_width_hint(240),
                ColumnDef::new("args", "Arguments", ColumnType::Text).with_width_hint(180),
                ColumnDef::new("working_dir", "Working Dir", ColumnType::Text).with_width_hint(180),
                ColumnDef::new("run_as", "Run As", ColumnType::Text).with_width_hint(110),
                ColumnDef::new("priority", "Priority", ColumnType::Text).with_width_hint(110),
                ColumnDef::new("affinity_hex", "Affinity", ColumnType::Text).with_width_hint(100),
                ColumnDef::new("window_state", "Window", ColumnType::Text).with_width_hint(100),
                ColumnDef::new("compat_layer", "Compat Layer", ColumnType::Text).with_width_hint(140),
                ColumnDef::new("valid", "Valid", ColumnType::Bool).with_width_hint(70),
                ColumnDef::new("validation_note", "Validation", ColumnType::Text).with_width_hint(180),
            ],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let rows: Vec<Row> = self
            .profiles
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let (valid, note) = match p.validate() {
                    Ok(()) => (true, "Valid".to_string()),
                    Err(e) => (false, e.to_string()),
                };
                Row::new(
                    idx as u64 + 1,
                    vec![
                        CellValue::Text(p.name.clone()),
                        CellValue::Text(p.exe_path.clone()),
                        CellValue::Text(p.args.join(" ")),
                        CellValue::Text(p.working_dir.clone().unwrap_or_default()),
                        CellValue::Text(format!("{:?}", p.run_as_mode)),
                        CellValue::Text(format!("{:?}", p.priority_class)),
                        CellValue::Text(
                            p.affinity_mask
                                .map(|m| format!("{m:#x}"))
                                .unwrap_or_else(|| "SystemDefault".to_string()),
                        ),
                        CellValue::Text(format!("{:?}", p.window_state)),
                        CellValue::Text(p.compatibility_layer.join(" ")),
                        CellValue::Bool(valid),
                        CellValue::Text(note),
                    ],
                )
                .with_detail_json(serde_json::to_value(p).unwrap_or(Value::Null))
            })
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

pub struct RunValidateAction;

impl ActionCommand for RunValidateAction {
    fn command_id(&self) -> &'static str {
        "cmd_run_validate"
    }

    fn description(&self) -> &'static str {
        "Read-only parser and validator for AdvancedRun JSON or INI launch profiles"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "profile": { "type": "object", "description": "LaunchProfile JSON object" },
                "ini": { "type": "string", "description": "AdvancedRun INI/CFG text" }
            }
        })
    }

    fn execute(&self, args: &Value, _dry_run: bool) -> Result<Value, McpToolError> {
        let profile = extract_profile(args)?;
        let env_block = profile.build_utf16_env_block();
        Ok(json!({
            "valid": true,
            "command_line": profile.build_command_line(),
            "utf16_env_units": env_block.len(),
            "profile": profile,
        }))
    }
}

pub struct RunLaunchAction {
    backend: Arc<dyn CmdBackend>,
}

impl RunLaunchAction {
    pub fn new(backend: Arc<dyn CmdBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for RunLaunchAction {
    fn command_id(&self) -> &'static str {
        "cmd_run"
    }

    fn description(&self) -> &'static str {
        "Launch or dry-run preview an executable using an AdvancedRun profile"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "profile": { "type": "object", "description": "LaunchProfile JSON object" },
                "ini": { "type": "string", "description": "AdvancedRun INI/CFG text" }
            }
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let profile = extract_profile(args)?;
        if dry_run {
            let mut plan = json!({
                "command_line": profile.build_command_line(),
                "working_dir": profile.working_dir,
                "run_as_mode": profile.run_as_mode,
                "priority_class": profile.priority_class,
                "affinity_mask": profile.affinity_mask,
                "window_state": profile.window_state,
                "compatibility_layer": profile.compatibility_layer,
                "utf16_env_units": profile.build_utf16_env_block().len(),
            });
            if profile.run_as_mode == RunAsMode::Administrator {
                plan["env_overrides_applied"] = json!(false);
                if !profile.env_overrides.is_empty() || !profile.compatibility_layer.is_empty() {
                    plan["notes"] = json!([
                        "Administrator mode launches via ShellExecuteExW('runas'), which omits custom env_overrides and compatibility_layer"
                    ]);
                }
            } else {
                plan["env_overrides_applied"] = json!(true);
            }
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview AdvancedRun process launch without spawning process",
                plan,
            ));
        }

        let res = self.backend.launch_process(&profile).map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "result": res,
        }))
    }
}

fn extract_profile(args: &Value) -> Result<LaunchProfile, McpToolError> {
    if let Some(ini) = args.get("ini").and_then(Value::as_str) {
        return parse_profile_ini(ini).map_err(McpToolError::from);
    }
    if let Some(profile_val) = args.get("profile") {
        let profile: LaunchProfile = serde_json::from_value(profile_val.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid 'profile' object: {e}")))?;
        profile.validate().map_err(McpToolError::from)?;
        return Ok(profile);
    }
    if args.get("exe_path").is_some() {
        let profile: LaunchProfile = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid launch profile fields: {e}")))?;
        profile.validate().map_err(McpToolError::from)?;
        return Ok(profile);
    }
    Err(McpToolError::InvalidParams(
        "expected 'profile' object, 'ini' string, or 'exe_path' field".to_string(),
    ))
}
