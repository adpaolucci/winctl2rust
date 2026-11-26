use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::CmdBackend;
use crate::run::WindowState;

pub type ScriptVars = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardOp {
    Set,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MuteMode {
    Unmute,
    Mute,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorState {
    On,
    Low,
    Off,
}

impl MonitorState {
    pub fn lparam_code(self) -> isize {
        match self {
            Self::On => -1,
            Self::Low => 1,
            Self::Off => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IniOp {
    SetVal,
    DelVal,
    DelSec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegValueKind {
    Sz,
    ExpandSz,
    Dword,
    Qword,
    Binary,
    MultiSz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceAction {
    Start,
    Stop,
    Pause,
    Continue,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinSelector {
    Title,
    Class,
    Process,
    Handle,
    Active,
    AllTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinAction {
    Show,
    Hide,
    Min,
    Max,
    Normal,
    Close,
    Activate,
    SetText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerAction {
    LockWorkstation,
    Logoff { force: bool },
    Reboot { force: bool },
    Shutdown { force: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScriptCommand {
    Speak {
        text: String,
        async_mode: bool,
    },
    Clipboard {
        op: ClipboardOp,
        text: Option<String>,
    },
    MuteSysVolume {
        mode: MuteMode,
    },
    SetVolume {
        device: u32,
        left: u16,
        right: u16,
    },
    Monitor {
        state: MonitorState,
    },
    ScreenSaver,
    Shortcut {
        target: String,
        folder: String,
        title: String,
        args: Option<String>,
        working_dir: Option<String>,
        icon_file: Option<String>,
        icon_idx: Option<i32>,
        show_cmd: WindowState,
        hotkey: Option<u16>,
        description: Option<String>,
    },
    IniFile {
        op: IniOp,
        file: PathBuf,
        section: String,
        key: Option<String>,
        value: Option<String>,
    },
    RegSetVal {
        reg_type: RegValueKind,
        key: String,
        value_name: String,
        data: String,
    },
    Service {
        action: ServiceAction,
        name: String,
    },
    Win {
        action: WinAction,
        find_by: WinSelector,
        target: String,
        extra: Option<String>,
    },
    Beep {
        freq_hz: u32,
        duration_ms: u32,
    },
    Wait {
        ms: u64,
    },
    Exec {
        show_mode: WindowState,
        exe: String,
        args: Vec<String>,
    },
    ShellExec {
        verb: String,
        target: String,
        params: Option<String>,
        dir: Option<String>,
    },
    Power {
        action: PowerAction,
    },
}

impl ScriptCommand {
    pub fn verb_name(&self) -> &'static str {
        match self {
            Self::Speak { .. } => "speak",
            Self::Clipboard { .. } => "clipboard",
            Self::MuteSysVolume { .. } => "mutesysvolume",
            Self::SetVolume { .. } => "setvolume",
            Self::Monitor { .. } => "monitor",
            Self::ScreenSaver => "screensaver",
            Self::Shortcut { .. } => "shortcut",
            Self::IniFile { .. } => "inifile",
            Self::RegSetVal { .. } => "regsetval",
            Self::Service { .. } => "service",
            Self::Win { .. } => "win",
            Self::Beep { .. } => "beep",
            Self::Wait { .. } => "wait",
            Self::Exec { .. } => "exec",
            Self::ShellExec { .. } => "shellexec",
            Self::Power { .. } => "power",
        }
    }

    pub fn target_summary(&self) -> String {
        match self {
            Self::Speak { text, .. } => text.clone(),
            Self::Clipboard { op, text } => match op {
                ClipboardOp::Clear => "clear".to_string(),
                ClipboardOp::Set => text.clone().unwrap_or_default(),
            },
            Self::MuteSysVolume { mode } => format!("{mode:?}"),
            Self::SetVolume { device, left, right } => format!("dev={device} left={left} right={right}"),
            Self::Monitor { state } => format!("{state:?}"),
            Self::ScreenSaver => "HWND_BROADCAST".to_string(),
            Self::Shortcut { folder, title, .. } => format!("{folder}\\{title}.lnk"),
            Self::IniFile { file, section, .. } => format!("{} [{section}]", file.display()),
            Self::RegSetVal { key, value_name, .. } => format!("{key}\\{value_name}"),
            Self::Service { name, .. } => name.clone(),
            Self::Win { target, .. } => target.clone(),
            Self::Beep { freq_hz, duration_ms } => format!("{freq_hz}Hz {duration_ms}ms"),
            Self::Wait { ms } => format!("{ms}ms"),
            Self::Exec { exe, .. } => exe.clone(),
            Self::ShellExec { target, .. } => target.clone(),
            Self::Power { action } => format!("{action:?}"),
        }
    }

    pub fn risk_label(&self) -> &'static str {
        match self {
            Self::Power { .. } | Self::RegSetVal { .. } | Self::Service { .. } => "High",
            Self::Wait { .. } | Self::Beep { .. } => "Low",
            _ => "Mutating",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedScriptCommand {
    pub line_no: usize,
    pub raw_line: String,
    pub command: ScriptCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptStepResult {
    pub verb: String,
    pub applied: bool,
    pub detail: String,
}

pub fn expand_vars(input: &str, vars: &ScriptVars) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        if chars[i] == '~' && i + 1 < chars.len() && chars[i + 1] == '$' {
            if let Some(end_rel) = chars[i + 2..].iter().position(|&c| c == '$') {
                let key: String = chars[i + 2..i + 2 + end_rel].iter().collect();
                if let Some(val) = resolve_builtin_or_custom_var(&key, vars) {
                    out.push_str(&val);
                } else {
                    out.push_str("~$");
                    out.push_str(&key);
                    out.push('$');
                }
                i += 2 + end_rel + 1;
                continue;
            }
        } else if chars[i] == '$'
            && let Some(end_rel) = chars[i + 1..].iter().position(|&c| c == '$')
        {
            let key: String = chars[i + 1..i + 1 + end_rel].iter().collect();
            if !key.is_empty() {
                if let Some(val) = resolve_builtin_or_custom_var(&key, vars) {
                    out.push_str(&val);
                } else {
                    out.push('$');
                    out.push_str(&key);
                    out.push('$');
                }
                i += 1 + end_rel + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

fn resolve_builtin_or_custom_var(key: &str, vars: &ScriptVars) -> Option<String> {
    if let Some(v) = vars.get(key) {
        return Some(v.clone());
    }
    match key.to_ascii_lowercase().as_str() {
        "sys.temp" => Some(std::env::temp_dir().display().to_string()),
        "sys.currentdir" => std::env::current_dir().ok().map(|p| p.display().to_string()),
        "sys.windows" => std::env::var("WINDIR").ok(),
        "sys.system" => std::env::var("WINDIR").ok().map(|w| format!("{w}\\System32")),
        _ => None,
    }
}

pub fn tokenize_line(line: &str) -> WinctlResult<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut token_started = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '\\' => {
                    if let Some(&next) = chars.peek() {
                        if next == '"' || next == '\\' {
                            current.push(next);
                            chars.next();
                        } else {
                            current.push('\\');
                        }
                    } else {
                        current.push('\\');
                    }
                }
                '"' => {
                    in_quotes = false;
                }
                other => current.push(other),
            }
        } else {
            match c {
                '"' => {
                    in_quotes = true;
                    token_started = true;
                }
                c if c.is_whitespace() => {
                    if token_started {
                        tokens.push(std::mem::take(&mut current));
                        token_started = false;
                    }
                }
                other => {
                    current.push(other);
                    token_started = true;
                }
            }
        }
    }

    if in_quotes {
        return Err(WinctlError::ParseError(
            "unterminated double quote in script line".to_string(),
        ));
    }
    if token_started {
        tokens.push(current);
    }

    Ok(tokens)
}

pub fn parse_script(script: &str, vars: &ScriptVars) -> WinctlResult<Vec<ParsedScriptCommand>> {
    let mut commands = Vec::new();
    for (idx, raw) in script.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let expanded = expand_vars(trimmed, vars);
        let tokens = tokenize_line(&expanded)?;
        if tokens.is_empty() {
            continue;
        }
        validate_r4_compliance(&tokens[0], &tokens[1..])?;
        let command = parse_command_tokens(&tokens).map_err(|e| match e {
            WinctlError::ParseError(msg) => WinctlError::ParseError(format!("line {line_no}: {msg}")),
            WinctlError::InvalidInput(msg) => WinctlError::InvalidInput(format!("line {line_no}: {msg}")),
            other => other,
        })?;
        commands.push(ParsedScriptCommand {
            line_no,
            raw_line: trimmed.to_string(),
            command,
        });
    }
    if commands.is_empty() {
        return Err(WinctlError::InvalidInput(
            "command script contains no executable statements".to_string(),
        ));
    }
    Ok(commands)
}

fn parse_command_tokens(tokens: &[String]) -> WinctlResult<ScriptCommand> {
    let verb = tokens[0].to_ascii_lowercase();
    let args = &tokens[1..];

    match verb.as_str() {
        "speak" => {
            if args.is_empty() {
                return Err(WinctlError::ParseError("speak requires <text>".to_string()));
            }
            if args[0].eq_ignore_ascii_case("text") {
                if args.len() < 2 {
                    return Err(WinctlError::ParseError("speak text requires <text>".to_string()));
                }
                let async_mode = args.get(2).is_some_and(|s| s.eq_ignore_ascii_case("async"));
                Ok(ScriptCommand::Speak {
                    text: args[1].clone(),
                    async_mode,
                })
            } else {
                let async_mode = args.last().is_some_and(|s| s.eq_ignore_ascii_case("async")) && args.len() > 1;
                let text = if async_mode {
                    args[..args.len() - 1].join(" ")
                } else {
                    args.join(" ")
                };
                Ok(ScriptCommand::Speak { text, async_mode })
            }
        }
        "clipboard" => {
            let sub = args
                .first()
                .ok_or_else(|| WinctlError::ParseError("clipboard requires subcommand 'set' or 'clear'".to_string()))?
                .to_ascii_lowercase();
            match sub.as_str() {
                "set" => {
                    let text = args
                        .get(1)
                        .ok_or_else(|| WinctlError::ParseError("clipboard set requires <text>".to_string()))?;
                    Ok(ScriptCommand::Clipboard {
                        op: ClipboardOp::Set,
                        text: Some(text.clone()),
                    })
                }
                "clear" => Ok(ScriptCommand::Clipboard {
                    op: ClipboardOp::Clear,
                    text: None,
                }),
                other => Err(WinctlError::ParseError(format!(
                    "unsupported clipboard subcommand '{other}'"
                ))),
            }
        }
        "mutesysvolume" => {
            let raw = args
                .first()
                .ok_or_else(|| WinctlError::ParseError("mutesysvolume requires 0|1|2".to_string()))?;
            let mode = match raw.as_str() {
                "0" => MuteMode::Unmute,
                "1" => MuteMode::Mute,
                "2" => MuteMode::Toggle,
                other => {
                    return Err(WinctlError::ParseError(format!(
                        "mutesysvolume expects 0, 1, or 2, got '{other}'"
                    )));
                }
            };
            Ok(ScriptCommand::MuteSysVolume { mode })
        }
        "setsysvolume" => {
            let raw = args
                .first()
                .ok_or_else(|| WinctlError::ParseError("setsysvolume requires <0..65535>".to_string()))?;
            let level: u16 = raw
                .parse()
                .map_err(|_| WinctlError::ParseError(format!("invalid volume '{raw}'")))?;
            Ok(ScriptCommand::SetVolume {
                device: 0,
                left: level,
                right: level,
            })
        }
        "setvolume" => {
            if args.len() < 3 {
                return Err(WinctlError::ParseError(
                    "setvolume requires <device> <left> <right>".to_string(),
                ));
            }
            let device: u32 = args[0]
                .parse()
                .map_err(|_| WinctlError::ParseError(format!("invalid device index '{}'", args[0])))?;
            let left: u16 = args[1]
                .parse()
                .map_err(|_| WinctlError::ParseError(format!("invalid left volume '{}'", args[1])))?;
            let right: u16 = args[2]
                .parse()
                .map_err(|_| WinctlError::ParseError(format!("invalid right volume '{}'", args[2])))?;
            Ok(ScriptCommand::SetVolume { device, left, right })
        }
        "monitor" => {
            let raw = args
                .first()
                .ok_or_else(|| WinctlError::ParseError("monitor requires on|low|off".to_string()))?
                .to_ascii_lowercase();
            let state = match raw.as_str() {
                "on" | "async_on" => MonitorState::On,
                "low" | "async_low" => MonitorState::Low,
                "off" | "async_off" => MonitorState::Off,
                other => {
                    return Err(WinctlError::ParseError(format!("unsupported monitor state '{other}'")));
                }
            };
            Ok(ScriptCommand::Monitor { state })
        }
        "screensaver" => Ok(ScriptCommand::ScreenSaver),
        "shortcut" => {
            if args.len() < 3 {
                return Err(WinctlError::ParseError(
                    "shortcut requires <target> <folder> <title>".to_string(),
                ));
            }
            let icon_idx = args
                .get(6)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| {
                    s.parse::<i32>()
                        .map_err(|_| WinctlError::ParseError(format!("invalid shortcut icon_idx '{s}'")))
                })
                .transpose()?;
            let show_cmd = args
                .get(7)
                .map(|s| WindowState::parse_str(s))
                .transpose()?
                .unwrap_or(WindowState::Normal);
            let hotkey = args
                .get(8)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| {
                    s.parse::<u16>()
                        .map_err(|_| WinctlError::ParseError(format!("invalid shortcut hotkey '{s}'")))
                })
                .transpose()?;
            Ok(ScriptCommand::Shortcut {
                target: args[0].clone(),
                folder: args[1].clone(),
                title: args[2].clone(),
                args: args.get(3).filter(|s| !s.is_empty()).cloned(),
                working_dir: args.get(4).filter(|s| !s.is_empty()).cloned(),
                icon_file: args.get(5).filter(|s| !s.is_empty()).cloned(),
                icon_idx,
                show_cmd,
                hotkey,
                description: args.get(9).filter(|s| !s.is_empty()).cloned(),
            })
        }
        "inifile" => {
            if args.len() < 3 {
                return Err(WinctlError::ParseError(
                    "inifile requires <setval|delval|delsec> <file> <section> ...".to_string(),
                ));
            }
            let op = match args[0].to_ascii_lowercase().as_str() {
                "setval" => IniOp::SetVal,
                "delval" => IniOp::DelVal,
                "delsec" => IniOp::DelSec,
                other => {
                    return Err(WinctlError::ParseError(format!(
                        "unsupported inifile operation '{other}'"
                    )));
                }
            };
            if matches!(op, IniOp::SetVal) && args.len() < 5 {
                return Err(WinctlError::ParseError(
                    "inifile setval requires <file> <section> <key> <value>".to_string(),
                ));
            }
            if matches!(op, IniOp::DelVal) && args.len() < 4 {
                return Err(WinctlError::ParseError(
                    "inifile delval requires <file> <section> <key>".to_string(),
                ));
            }
            Ok(ScriptCommand::IniFile {
                op,
                file: PathBuf::from(&args[1]),
                section: args[2].clone(),
                key: args.get(3).cloned(),
                value: args.get(4).cloned(),
            })
        }
        "regsetval" => {
            if args.len() < 4 {
                return Err(WinctlError::ParseError(
                    "regsetval requires <type> <key> <value_name> <data>".to_string(),
                ));
            }
            let reg_type = match args[0].to_ascii_lowercase().as_str() {
                "sz" => RegValueKind::Sz,
                "expand_sz" => RegValueKind::ExpandSz,
                "dword" => RegValueKind::Dword,
                "qword" => RegValueKind::Qword,
                "binary" => RegValueKind::Binary,
                "multi_sz" => RegValueKind::MultiSz,
                other => {
                    return Err(WinctlError::ParseError(format!(
                        "unsupported registry value type '{other}'"
                    )));
                }
            };
            validate_registry_key_hive(&args[1])?;
            Ok(ScriptCommand::RegSetVal {
                reg_type,
                key: args[1].clone(),
                value_name: args[2].clone(),
                data: args[3].clone(),
            })
        }
        "service" => {
            if args.len() < 2 {
                return Err(WinctlError::ParseError(
                    "service requires <start|stop|pause|continue|restart> <service_name>".to_string(),
                ));
            }
            let action = match args[0].to_ascii_lowercase().as_str() {
                "start" => ServiceAction::Start,
                "stop" => ServiceAction::Stop,
                "pause" => ServiceAction::Pause,
                "continue" => ServiceAction::Continue,
                "restart" => ServiceAction::Restart,
                other => {
                    return Err(WinctlError::ParseError(format!("unsupported service action '{other}'")));
                }
            };
            Ok(ScriptCommand::Service {
                action,
                name: args[1].clone(),
            })
        }
        "win" => {
            if args.len() < 2 {
                return Err(WinctlError::ParseError(
                    "win requires <action> <find_by> [target] [extra]".to_string(),
                ));
            }
            let action = match args[0].to_ascii_lowercase().as_str() {
                "show" => WinAction::Show,
                "hide" => WinAction::Hide,
                "min" => WinAction::Min,
                "max" => WinAction::Max,
                "normal" => WinAction::Normal,
                "close" => WinAction::Close,
                "activate" => WinAction::Activate,
                "settext" => WinAction::SetText,
                other => return Err(WinctlError::ParseError(format!("unsupported win action '{other}'"))),
            };
            let find_by = match args[1].to_ascii_lowercase().as_str() {
                "title" | "ititle" => WinSelector::Title,
                "class" => WinSelector::Class,
                "process" => WinSelector::Process,
                "handle" => WinSelector::Handle,
                "active" => WinSelector::Active,
                "alltop" => WinSelector::AllTop,
                other => return Err(WinctlError::ParseError(format!("unsupported win selector '{other}'"))),
            };
            if !matches!(find_by, WinSelector::Active | WinSelector::AllTop) && args.len() < 3 {
                return Err(WinctlError::ParseError(
                    "win requires <action> <find_by> <target> [extra]".to_string(),
                ));
            }
            let target = args.get(2).cloned().unwrap_or_default();
            if find_by == WinSelector::Handle {
                let parsed = parse_win_handle_usize(&target)?;
                if parsed == 0 {
                    return Err(WinctlError::InvalidInput(
                        "HWND(0) is not a valid window handle".to_string(),
                    ));
                }
            }
            Ok(ScriptCommand::Win {
                action,
                find_by,
                target,
                extra: args.get(3).cloned(),
            })
        }
        "beep" => {
            if args.len() < 2 {
                return Err(WinctlError::ParseError(
                    "beep requires <freq_hz> <duration_ms>".to_string(),
                ));
            }
            let freq_hz: u32 = args[0]
                .parse()
                .map_err(|_| WinctlError::ParseError(format!("invalid beep frequency '{}'", args[0])))?;
            if !(37..=32767).contains(&freq_hz) {
                return Err(WinctlError::InvalidInput(format!(
                    "beep frequency {freq_hz} Hz out of Win32 Beep range (37..=32767)"
                )));
            }
            let duration_ms: u32 = args[1]
                .parse()
                .map_err(|_| WinctlError::ParseError(format!("invalid beep duration '{}'", args[1])))?;
            Ok(ScriptCommand::Beep { freq_hz, duration_ms })
        }
        "wait" => {
            let raw = args
                .first()
                .ok_or_else(|| WinctlError::ParseError("wait requires <milliseconds>".to_string()))?;
            let ms: u64 = raw
                .parse()
                .map_err(|_| WinctlError::ParseError(format!("invalid wait ms '{raw}'")))?;
            Ok(ScriptCommand::Wait { ms })
        }
        "exec" | "execmd" => {
            if args.len() < 2 {
                return Err(WinctlError::ParseError(
                    "exec requires <show|hide|min|max> <exe> [args...]".to_string(),
                ));
            }
            let show_mode = WindowState::parse_str(&args[0])?;
            let exe = args[1].clone();
            let extra = args[2..].to_vec();
            Ok(ScriptCommand::Exec {
                show_mode,
                exe,
                args: extra,
            })
        }
        "shellexec" => {
            if args.len() < 2 {
                return Err(WinctlError::ParseError(
                    "shellexec requires <verb> <target> [params] [dir]".to_string(),
                ));
            }
            Ok(ScriptCommand::ShellExec {
                verb: args[0].clone(),
                target: args[1].clone(),
                params: args.get(2).filter(|s| !s.is_empty()).cloned(),
                dir: args.get(3).filter(|s| !s.is_empty()).cloned(),
            })
        }
        "lockws" => Ok(ScriptCommand::Power {
            action: PowerAction::LockWorkstation,
        }),
        "exitwin" => {
            let sub = args
                .first()
                .ok_or_else(|| WinctlError::ParseError("exitwin requires logoff|reboot|shutdown|poweroff".to_string()))?
                .to_ascii_lowercase();
            let force = args.iter().skip(1).any(|a| a.eq_ignore_ascii_case("force"));
            let action = match sub.as_str() {
                "logoff" => PowerAction::Logoff { force },
                "reboot" => PowerAction::Reboot { force },
                "shutdown" | "poweroff" => PowerAction::Shutdown { force },
                other => return Err(WinctlError::ParseError(format!("unsupported exitwin mode '{other}'"))),
            };
            Ok(ScriptCommand::Power { action })
        }
        other => Err(WinctlError::ParseError(format!("unsupported script verb '{other}'"))),
    }
}

fn validate_registry_key_hive(full: &str) -> WinctlResult<()> {
    let (hive, sub) = full
        .split_once('\\')
        .ok_or_else(|| WinctlError::InvalidInput(format!("registry path '{full}' missing hive prefix")))?;
    if sub.trim().is_empty() {
        return Err(WinctlError::InvalidInput(format!(
            "registry path '{full}' missing subkey after hive prefix"
        )));
    }
    match hive.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" | "HKLM" | "HKEY_LOCAL_MACHINE" | "HKCR" | "HKEY_CLASSES_ROOT" | "HKU"
        | "HKEY_USERS" => Ok(()),
        other => Err(WinctlError::InvalidInput(format!(
            "unsupported registry hive '{other}'"
        ))),
    }
}

fn parse_win_handle_usize(raw: &str) -> WinctlResult<usize> {
    let trimmed = raw.trim();
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16)
    } else {
        trimmed.parse::<usize>()
    }
    .map_err(|_| WinctlError::InvalidInput(format!("invalid HWND value '{raw}'")))
}

enum IniLine {
    Raw(String),
    Entry(String, String),
}

pub fn apply_ini_operation(
    op: IniOp,
    file: &Path,
    section: &str,
    key: Option<&str>,
    value: Option<&str>,
) -> WinctlResult<()> {
    let existing = if file.exists() {
        fs::read_to_string(file)?
    } else {
        String::new()
    };

    let mut preamble: Vec<String> = Vec::new();
    let mut sections: Vec<(String, Vec<IniLine>)> = Vec::new();
    let mut current_sec: Option<String> = None;

    for raw in existing.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
            let sec_name = trimmed[1..trimmed.len() - 1].trim().to_string();
            current_sec = Some(sec_name.clone());
            if !sections.iter().any(|(s, _)| s.eq_ignore_ascii_case(&sec_name)) {
                sections.push((sec_name, Vec::new()));
            }
        } else if let Some(ref sec_name) = current_sec {
            if let Some((_, lines)) = sections.iter_mut().find(|(s, _)| s.eq_ignore_ascii_case(sec_name)) {
                if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
                    lines.push(IniLine::Raw(raw.to_string()));
                } else if let Some((k, v)) = trimmed.split_once('=') {
                    lines.push(IniLine::Entry(k.trim().to_string(), v.trim().to_string()));
                } else {
                    lines.push(IniLine::Raw(raw.to_string()));
                }
            }
        } else {
            preamble.push(raw.to_string());
        }
    }

    match op {
        IniOp::SetVal => {
            let k = key.ok_or_else(|| WinctlError::InvalidInput("inifile setval missing key".to_string()))?;
            let v = value.unwrap_or("");
            if let Some((_, lines)) = sections.iter_mut().find(|(s, _)| s.eq_ignore_ascii_case(section)) {
                if let Some(IniLine::Entry(_, ev)) = lines
                    .iter_mut()
                    .find(|line| matches!(line, IniLine::Entry(ek, _) if ek.eq_ignore_ascii_case(k)))
                {
                    *ev = v.to_string();
                } else {
                    let insert_at = lines
                        .iter()
                        .rposition(|line| !matches!(line, IniLine::Raw(s) if s.trim().is_empty()))
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    lines.insert(insert_at, IniLine::Entry(k.to_string(), v.to_string()));
                }
            } else {
                sections.push((section.to_string(), vec![IniLine::Entry(k.to_string(), v.to_string())]));
            }
        }
        IniOp::DelVal => {
            let k = key.ok_or_else(|| WinctlError::InvalidInput("inifile delval missing key".to_string()))?;
            if let Some((_, lines)) = sections.iter_mut().find(|(s, _)| s.eq_ignore_ascii_case(section)) {
                lines.retain(|line| !matches!(line, IniLine::Entry(ek, _) if ek.eq_ignore_ascii_case(k)));
            }
        }
        IniOp::DelSec => {
            sections.retain(|(s, _)| !s.eq_ignore_ascii_case(section));
        }
    }

    let mut rendered = String::new();
    for line in &preamble {
        rendered.push_str(line);
        rendered.push('\n');
    }
    for (idx, (sec_name, lines)) in sections.iter().enumerate() {
        if idx > 0 {
            let prev_ended_with_blank = sections[idx - 1]
                .1
                .last()
                .is_some_and(|line| matches!(line, IniLine::Raw(s) if s.trim().is_empty()));
            if !prev_ended_with_blank {
                rendered.push('\n');
            }
        }
        rendered.push_str(&format!("[{sec_name}]\n"));
        for line in lines {
            match line {
                IniLine::Raw(raw) => {
                    rendered.push_str(raw);
                    rendered.push('\n');
                }
                IniLine::Entry(k, v) => {
                    rendered.push_str(&format!("{k}={v}\n"));
                }
            }
        }
    }

    if let Some(parent) = file.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, rendered)?;
    Ok(())
}

pub fn preview_table_schema() -> TableSchema {
    TableSchema::new(
        "cmd_script_preview",
        "Command Script DSL Preview",
        vec![
            ColumnDef::new("line_no", "Line", ColumnType::Uint).with_width_hint(70),
            ColumnDef::new("verb", "Verb", ColumnType::Text).with_width_hint(130),
            ColumnDef::new("target", "Target", ColumnType::Text).with_width_hint(240),
            ColumnDef::new("arguments", "Arguments", ColumnType::Text).with_width_hint(260),
            ColumnDef::new("risk", "Risk", ColumnType::Text).with_width_hint(90),
            ColumnDef::new("dry_run_preview", "Dry-Run Preview", ColumnType::Text).with_width_hint(260),
            ColumnDef::new("status", "Status", ColumnType::Text).with_width_hint(100),
        ],
    )
}

pub fn script_to_table_rows(script: &str, vars: &ScriptVars) -> WinctlResult<Vec<Row>> {
    let parsed = parse_script(script, vars)?;
    let rows = parsed
        .into_iter()
        .enumerate()
        .map(|(idx, step)| {
            let preview_json = serde_json::to_string(&step.command).unwrap_or_default();
            Row::new(
                idx as u64 + 1,
                vec![
                    CellValue::Uint(step.line_no as u64),
                    CellValue::Text(step.command.verb_name().to_string()),
                    CellValue::Text(step.command.target_summary()),
                    CellValue::Text(step.raw_line.clone()),
                    CellValue::Text(step.command.risk_label().to_string()),
                    CellValue::Text(preview_json),
                    CellValue::Text("Parsed (Ready)".to_string()),
                ],
            )
            .with_detail_json(serde_json::to_value(&step).unwrap_or(Value::Null))
        })
        .collect();
    Ok(rows)
}

pub struct ScriptPreviewProvider {
    sample_script: String,
}

impl Default for ScriptPreviewProvider {
    fn default() -> Self {
        Self {
            sample_script: concat!(
                "# Built-in non-hooking command script DSL preview examples\n",
                "speak \"Winctl2Rust command script DSL ready\"\n",
                "beep 880 120\n",
                "clipboard set \"winctl2rust\"\n",
                "mutesysvolume 2\n",
                "monitor off\n",
            )
            .to_string(),
        }
    }
}

impl ScriptPreviewProvider {
    pub fn with_script(script: impl Into<String>) -> Self {
        Self {
            sample_script: script.into(),
        }
    }
}

impl TableProvider for ScriptPreviewProvider {
    fn tool_id(&self) -> &'static str {
        "cmd_script_preview"
    }

    fn description(&self) -> &'static str {
        "Parse and preview command script DSL statements (non-hooking pure-Rust parser)"
    }

    fn schema(&self) -> TableSchema {
        preview_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let rows = script_to_table_rows(&self.sample_script, &ScriptVars::new())
            .map_err(|e| TableError::Provider(e.to_string()))?;
        apply_query(&self.schema(), &rows, req)
    }
}

pub struct ScriptParseAction;

impl ActionCommand for ScriptParseAction {
    fn command_id(&self) -> &'static str {
        "cmd_script_parse"
    }

    fn description(&self) -> &'static str {
        "Read-only parser and R4 validator for command script DSL inputs"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "script": { "type": "string", "description": "One or more command script DSL lines" },
                "vars": { "type": "object", "additionalProperties": { "type": "string" } }
            },
            "required": ["script"]
        })
    }

    fn execute(&self, args: &Value, _dry_run: bool) -> Result<Value, McpToolError> {
        let script = args
            .get("script")
            .and_then(Value::as_str)
            .ok_or_else(|| McpToolError::InvalidParams("missing string parameter 'script'".to_string()))?;
        let vars = extract_vars(args)?;
        let parsed = parse_script(script, &vars).map_err(McpToolError::from)?;
        Ok(json!({
            "valid": true,
            "step_count": parsed.len(),
            "steps": parsed,
        }))
    }
}

pub struct ScriptAction {
    backend: Arc<dyn CmdBackend>,
}

impl ScriptAction {
    pub fn new(backend: Arc<dyn CmdBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for ScriptAction {
    fn command_id(&self) -> &'static str {
        "cmd_script"
    }

    fn description(&self) -> &'static str {
        "Parse, dry-run preview, or execute command script DSL lines without DLL injection or API hooking"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "script": { "type": "string", "description": "Command script DSL (newline-separated commands)" },
                "vars": { "type": "object", "additionalProperties": { "type": "string" } }
            },
            "required": ["script"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let script = args
            .get("script")
            .and_then(Value::as_str)
            .ok_or_else(|| McpToolError::InvalidParams("missing string parameter 'script'".to_string()))?;
        let vars = extract_vars(args)?;
        let parsed = parse_script(script, &vars).map_err(McpToolError::from)?;

        if dry_run {
            let plan = parsed
                .iter()
                .map(|p| {
                    json!({
                        "line_no": p.line_no,
                        "verb": p.command.verb_name(),
                        "target": p.command.target_summary(),
                        "command": p.command,
                    })
                })
                .collect::<Vec<_>>();
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview command script DSL execution without modifying OS state",
                Value::Array(plan),
            ));
        }

        let mut results = Vec::with_capacity(parsed.len());
        for step in &parsed {
            let res = self.backend.execute_script(&step.command).map_err(McpToolError::from)?;
            results.push(res);
        }

        Ok(json!({
            "dry_run": false,
            "applied": true,
            "steps_executed": results.len(),
            "results": results,
        }))
    }
}

fn extract_vars(args: &Value) -> Result<ScriptVars, McpToolError> {
    let mut vars = ScriptVars::new();
    if let Some(obj) = args.get("vars").and_then(Value::as_object) {
        for (k, v) in obj {
            let s = v
                .as_str()
                .ok_or_else(|| McpToolError::InvalidParams(format!("vars['{k}'] must be a string")))?;
            vars.insert(k.clone(), s.to_string());
        }
    }
    Ok(vars)
}
