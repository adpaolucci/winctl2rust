use std::fmt;

use serde::{Deserialize, Serialize};
use winctl2rust_core::{WinctlError, WinctlResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioItemKind {
    Device,
    Subunit,
    Application,
}

impl AudioItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Device => "Device",
            Self::Subunit => "Subunit",
            Self::Application => "Application",
        }
    }
}

impl fmt::Display for AudioItemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioDirection {
    Render,
    Capture,
    Unknown,
}

impl AudioDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Render => "Render",
            Self::Capture => "Capture",
            Self::Unknown => "Unknown",
        }
    }
}

impl fmt::Display for AudioDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DefaultRole {
    Console = 0,
    Multimedia = 1,
    Communications = 2,
}

impl DefaultRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Console => "Console",
            Self::Multimedia => "Multimedia",
            Self::Communications => "Communications",
        }
    }

    pub fn parse_str(raw: &str) -> WinctlResult<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "0" | "console" => Ok(Self::Console),
            "1" | "multimedia" => Ok(Self::Multimedia),
            "2" | "communications" => Ok(Self::Communications),
            other => Err(WinctlError::InvalidInput(format!(
                "invalid audio default role '{other}' (expected 0/Console, 1/Multimedia, or 2/Communications)"
            ))),
        }
    }

    pub fn parse_role_list(raw: &str) -> WinctlResult<Vec<Self>> {
        let trimmed = raw.trim();
        let mut has_all = false;
        let mut out = Vec::new();
        for part in trimmed.split([',', ';', '|', ' ']) {
            let token = part.trim();
            if token.is_empty() {
                continue;
            }
            if token.eq_ignore_ascii_case("all") {
                has_all = true;
                continue;
            }
            let role = Self::parse_str(token)?;
            if !out.contains(&role) {
                out.push(role);
            }
        }
        if has_all {
            return Ok(vec![Self::Console, Self::Multimedia, Self::Communications]);
        }
        if out.is_empty() {
            return Err(WinctlError::InvalidInput(
                "role specification must not be empty".to_string(),
            ));
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioDeviceState {
    Active,
    Inactive,
    Disabled,
    Unplugged,
}

impl AudioDeviceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Inactive => "Inactive",
            Self::Disabled => "Disabled",
            Self::Unplugged => "Unplugged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MuteAction {
    Mute,
    Unmute,
    Toggle,
}

impl MuteAction {
    pub fn parse_str(raw: &str) -> WinctlResult<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "mute" | "1" | "true" => Ok(Self::Mute),
            "unmute" | "0" | "false" => Ok(Self::Unmute),
            "toggle" | "switch" | "2" => Ok(Self::Toggle),
            other => Err(WinctlError::InvalidInput(format!(
                "invalid mute mode '{other}' (expected mute, unmute, or toggle)"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioEndpointItem {
    pub item_id: String,
    pub name: String,
    pub kind: AudioItemKind,
    pub direction: AudioDirection,
    pub device_name: String,
    pub device_id: String,
    pub default_roles: Vec<DefaultRole>,
    pub muted: bool,
    pub volume_percent: f32,
    pub volume_db: f32,
    pub min_db: f32,
    pub max_db: f32,
    pub channels: u32,
    pub channel_volumes: Vec<f32>,
    pub peak_level: f32,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    pub state: AudioDeviceState,
    pub command_line_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStateSnapshot {
    pub item_id: String,
    pub name: String,
    pub volume_percent: f32,
    pub volume_db: f32,
    pub muted: bool,
    pub channel_volumes: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAppPropertyStoreRecord {
    pub subkey: String,
    pub registry_key: String,
    pub default_value: String,
    pub blob_3_volume: Option<Vec<u8>>,
    pub blob_4_channels: Option<Vec<u8>>,
    pub blob_5_mute: Option<Vec<u8>>,
    pub raw_1sps_container: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppPropertyStorePatch {
    pub volume_scalar: Option<f32>,
    pub channel_volumes: Option<Vec<f32>>,
    pub muted: Option<bool>,
    pub delete_subkey: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenEdgeKind {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum WheelCondition {
    Any,
    Alt,
    Ctrl,
    Shift,
    Win,
    LeftButton,
    RightButton,
    MiddleButton,
    OverTaskbar,
    OverDesktop,
    OverWindowTitle,
    ScreenEdge(ScreenEdgeKind),
    WindowClassContains(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WheelRule {
    pub rule_id: String,
    pub enabled: bool,
    pub priority: i32,
    pub conditions: Vec<WheelCondition>,
    pub target_device: String,
    pub step_percent: f32,
    pub min_percent: f32,
    pub max_percent: f32,
    pub consume_wheel: bool,
    pub invert_direction: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerWheelEvent {
    pub wheel_delta: i16,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub win: bool,
    #[serde(default)]
    pub left_button: bool,
    #[serde(default)]
    pub right_button: bool,
    #[serde(default)]
    pub middle_button: bool,
    #[serde(default)]
    pub over_taskbar: bool,
    #[serde(default)]
    pub over_desktop: bool,
    #[serde(default)]
    pub over_window_title: bool,
    #[serde(default)]
    pub screen_edge: Option<ScreenEdgeKind>,
    #[serde(default)]
    pub window_class: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WheelActionResult {
    pub rule_id: String,
    pub target_device: String,
    pub previous_percent: f32,
    pub new_percent: f32,
    pub wheel_delta: i16,
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerOptions {
    pub duration_ms: u64,
    pub max_events: usize,
}

impl Default for ListenerOptions {
    fn default() -> Self {
        Self {
            duration_ms: 1000,
            max_events: 16,
        }
    }
}

pub trait AudioBackend: Send + Sync {
    fn list_audio_items(&self) -> WinctlResult<Vec<AudioEndpointItem>>;
    fn set_volume_scalar(&self, target_id: &str, scalar: f32) -> WinctlResult<AudioStateSnapshot>;
    fn set_channel_volumes(&self, target_id: &str, channels: &[f32]) -> WinctlResult<AudioStateSnapshot>;
    fn set_volume_db(&self, target_id: &str, db: f32) -> WinctlResult<AudioStateSnapshot>;
    fn set_mute(&self, target_id: &str, mode: MuteAction) -> WinctlResult<AudioStateSnapshot>;
    fn set_default_endpoint(&self, device_id: &str, roles: &[DefaultRole]) -> WinctlResult<()>;
    fn set_endpoint_enabled(&self, device_id: &str, enabled: bool) -> WinctlResult<()>;
    fn set_app_default_device(&self, process_or_pid: &str, device_id: &str, roles: &[DefaultRole]) -> WinctlResult<()>;
    fn read_app_audio_configs(&self) -> WinctlResult<Vec<RawAppPropertyStoreRecord>>;
    fn write_app_audio_config(&self, subkey: &str, patch: &AppPropertyStorePatch) -> WinctlResult<()>;
    fn run_wheel_listener(
        &self,
        rules: &[WheelRule],
        opts: &ListenerOptions,
    ) -> WinctlResult<Vec<WheelActionResult>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedAudioBackend;

impl AudioBackend for UnsupportedAudioBackend {
    fn list_audio_items(&self) -> WinctlResult<Vec<AudioEndpointItem>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "CoreAudio enumeration requires Windows".to_string(),
        ))
    }

    fn set_volume_scalar(&self, target_id: &str, _scalar: f32) -> WinctlResult<AudioStateSnapshot> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "CoreAudio set_volume_scalar('{target_id}') requires Windows"
        )))
    }

    fn set_channel_volumes(&self, target_id: &str, _channels: &[f32]) -> WinctlResult<AudioStateSnapshot> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "CoreAudio set_channel_volumes('{target_id}') requires Windows"
        )))
    }

    fn set_volume_db(&self, target_id: &str, _db: f32) -> WinctlResult<AudioStateSnapshot> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "CoreAudio set_volume_db('{target_id}') requires Windows"
        )))
    }

    fn set_mute(&self, target_id: &str, _mode: MuteAction) -> WinctlResult<AudioStateSnapshot> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "CoreAudio set_mute('{target_id}') requires Windows"
        )))
    }

    fn set_default_endpoint(&self, device_id: &str, _roles: &[DefaultRole]) -> WinctlResult<()> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "IPolicyConfig::SetDefaultEndpoint('{device_id}') requires Windows"
        )))
    }

    fn set_endpoint_enabled(&self, device_id: &str, _enabled: bool) -> WinctlResult<()> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "IPolicyConfig::SetEndpointVisibility('{device_id}') requires Windows"
        )))
    }

    fn set_app_default_device(
        &self,
        process_or_pid: &str,
        _device_id: &str,
        _roles: &[DefaultRole],
    ) -> WinctlResult<()> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "App default endpoint routing for '{process_or_pid}' requires Windows"
        )))
    }

    fn read_app_audio_configs(&self) -> WinctlResult<Vec<RawAppPropertyStoreRecord>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "HKCU PolicyConfig\\PropertyStore enumeration requires Windows Registry".to_string(),
        ))
    }

    fn write_app_audio_config(&self, subkey: &str, _patch: &AppPropertyStorePatch) -> WinctlResult<()> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "HKCU PolicyConfig\\PropertyStore update ('{subkey}') requires Windows Registry"
        )))
    }

    fn run_wheel_listener(
        &self,
        _rules: &[WheelRule],
        _opts: &ListenerOptions,
    ) -> WinctlResult<Vec<WheelActionResult>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "WM_INPUT raw mouse listener requires Windows".to_string(),
        ))
    }
}

pub fn resolve_audio_target<'a>(items: &'a [AudioEndpointItem], selector: &str) -> WinctlResult<&'a AudioEndpointItem> {
    let trimmed = selector.trim();
    if trimmed.is_empty() {
        return Err(WinctlError::InvalidInput(
            "audio target selector must not be empty".to_string(),
        ));
    }

    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "defaultrenderdevice" | "default_render_device" | "defaultrender" => {
            return items
                .iter()
                .find(|i| {
                    i.kind == AudioItemKind::Device
                        && i.direction == AudioDirection::Render
                        && i.default_roles.contains(&DefaultRole::Console)
                })
                .ok_or_else(|| WinctlError::HardwareUnavailable("no default render audio endpoint".to_string()));
        }
        "defaultcapturedevice" | "default_capture_device" | "defaultcapture" => {
            return items
                .iter()
                .find(|i| {
                    i.kind == AudioItemKind::Device
                        && i.direction == AudioDirection::Capture
                        && i.default_roles.contains(&DefaultRole::Console)
                })
                .ok_or_else(|| WinctlError::HardwareUnavailable("no default capture audio endpoint".to_string()));
        }
        "defaultmultimediadevice" | "default_multimedia_device" => {
            return items
                .iter()
                .find(|i| {
                    i.kind == AudioItemKind::Device
                        && i.direction == AudioDirection::Render
                        && i.default_roles.contains(&DefaultRole::Multimedia)
                })
                .ok_or_else(|| WinctlError::HardwareUnavailable("no default multimedia audio endpoint".to_string()));
        }
        "defaultcommunicationsdevice" | "default_communications_device" => {
            return items
                .iter()
                .find(|i| {
                    i.kind == AudioItemKind::Device
                        && i.direction == AudioDirection::Render
                        && i.default_roles.contains(&DefaultRole::Communications)
                })
                .ok_or_else(|| {
                    WinctlError::HardwareUnavailable("no default communications audio endpoint".to_string())
                });
        }
        _ => {}
    }

    if let Some(exact) = items
        .iter()
        .find(|i| i.item_id.eq_ignore_ascii_case(trimmed) || i.command_line_id.eq_ignore_ascii_case(trimmed))
    {
        return Ok(exact);
    }

    if let Some(by_name) = items
        .iter()
        .find(|i| i.name.eq_ignore_ascii_case(trimmed) || i.device_name.eq_ignore_ascii_case(trimmed))
    {
        return Ok(by_name);
    }

    if let Ok(pid) = trimmed.parse::<u32>()
        && let Some(by_pid) = items
            .iter()
            .find(|i| i.kind == AudioItemKind::Application && i.pid == Some(pid))
    {
        return Ok(by_pid);
    }

    if let Some(by_proc) = items.iter().find(|i| {
        i.kind == AudioItemKind::Application
            && i.process_name.as_deref().is_some_and(|p| {
                let pl = p.to_ascii_lowercase();
                let stem = pl.strip_suffix(".exe").unwrap_or(&pl);
                pl == lower || stem == lower
            })
    }) {
        return Ok(by_proc);
    }

    if let Some(by_sub) = items.iter().find(|i| {
        i.item_id.to_ascii_lowercase().contains(&lower)
            || i.command_line_id.to_ascii_lowercase().contains(&lower)
            || i.name.to_ascii_lowercase().contains(&lower)
    }) {
        return Ok(by_sub);
    }

    Err(WinctlError::InvalidInput(format!(
        "no CoreAudio endpoint, subunit, or session matched '{selector}'"
    )))
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct MockAudioBackend {
    pub items: std::sync::Mutex<Vec<AudioEndpointItem>>,
    pub app_configs: std::sync::Mutex<Vec<RawAppPropertyStoreRecord>>,
    pub app_default_routes: std::sync::Mutex<Vec<(String, String, Vec<DefaultRole>)>>,
    pub action_log: std::sync::Mutex<Vec<String>>,
    pub queued_wheel_events: std::sync::Mutex<Vec<PointerWheelEvent>>,
    pub inject_hardware_unavailable: std::sync::atomic::AtomicBool,
    pub inject_permission_denied: std::sync::atomic::AtomicBool,
}

#[cfg(any(test, feature = "test-support"))]
impl MockAudioBackend {
    pub fn with_sample_data() -> Self {
        use crate::app_config::{
            POLICY_STORE_KEY_PREFIX, encode_propvar_bool, encode_propvar_r4, encode_propvar_r4_array,
        };

        let items = vec![
            AudioEndpointItem {
                item_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555551}".to_string(),
                name: "Speakers (Realtek High Definition Audio)".to_string(),
                kind: AudioItemKind::Device,
                direction: AudioDirection::Render,
                device_name: "Realtek High Definition Audio".to_string(),
                device_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555551}".to_string(),
                default_roles: vec![DefaultRole::Console, DefaultRole::Multimedia],
                muted: false,
                volume_percent: 65.0,
                volume_db: -6.5,
                min_db: -65.25,
                max_db: 0.0,
                channels: 2,
                channel_volumes: vec![65.0, 65.0],
                peak_level: 0.18,
                pid: None,
                process_name: None,
                state: AudioDeviceState::Active,
                command_line_id: "Realtek High Definition Audio\\Device\\Speakers\\Render".to_string(),
            },
            AudioEndpointItem {
                item_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555552}".to_string(),
                name: "Headphones (USB Audio)".to_string(),
                kind: AudioItemKind::Device,
                direction: AudioDirection::Render,
                device_name: "USB Audio".to_string(),
                device_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555552}".to_string(),
                default_roles: vec![],
                muted: true,
                volume_percent: 40.0,
                volume_db: -14.0,
                min_db: -64.0,
                max_db: 0.0,
                channels: 2,
                channel_volumes: vec![40.0, 40.0],
                peak_level: 0.0,
                pid: None,
                process_name: None,
                state: AudioDeviceState::Active,
                command_line_id: "USB Audio\\Device\\Headphones\\Render".to_string(),
            },
            AudioEndpointItem {
                item_id: "{0.0.1.00000000}.{22222222-3333-4444-5555-666666666661}".to_string(),
                name: "Microphone (Blue Yeti)".to_string(),
                kind: AudioItemKind::Device,
                direction: AudioDirection::Capture,
                device_name: "Blue Yeti".to_string(),
                device_id: "{0.0.1.00000000}.{22222222-3333-4444-5555-666666666661}".to_string(),
                default_roles: vec![DefaultRole::Console, DefaultRole::Communications],
                muted: false,
                volume_percent: 80.0,
                volume_db: -3.2,
                min_db: -48.0,
                max_db: 12.0,
                channels: 2,
                channel_volumes: vec![80.0, 80.0],
                peak_level: 0.05,
                pid: None,
                process_name: None,
                state: AudioDeviceState::Active,
                command_line_id: "Blue Yeti\\Device\\Microphone\\Capture".to_string(),
            },
            AudioEndpointItem {
                item_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555551}\\subunit\\master_mute".to_string(),
                name: "Master Mute".to_string(),
                kind: AudioItemKind::Subunit,
                direction: AudioDirection::Render,
                device_name: "Realtek High Definition Audio".to_string(),
                device_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555551}".to_string(),
                default_roles: vec![],
                muted: false,
                volume_percent: 100.0,
                volume_db: 0.0,
                min_db: -65.25,
                max_db: 0.0,
                channels: 2,
                channel_volumes: vec![100.0, 100.0],
                peak_level: 0.0,
                pid: None,
                process_name: None,
                state: AudioDeviceState::Active,
                command_line_id: "Realtek High Definition Audio\\Subunit\\Master Mute\\Render".to_string(),
            },
            AudioEndpointItem {
                item_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555551}\\subunit\\wave_out".to_string(),
                name: "Wave Out".to_string(),
                kind: AudioItemKind::Subunit,
                direction: AudioDirection::Render,
                device_name: "Realtek High Definition Audio".to_string(),
                device_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555551}".to_string(),
                default_roles: vec![],
                muted: false,
                volume_percent: 65.0,
                volume_db: -6.5,
                min_db: -65.25,
                max_db: 0.0,
                channels: 2,
                channel_volumes: vec![65.0, 65.0],
                peak_level: 0.0,
                pid: None,
                process_name: None,
                state: AudioDeviceState::Active,
                command_line_id: "Realtek High Definition Audio\\Subunit\\Wave Out\\Render".to_string(),
            },
            AudioEndpointItem {
                item_id: "session:4120:vlc.exe".to_string(),
                name: "VLC media player".to_string(),
                kind: AudioItemKind::Application,
                direction: AudioDirection::Render,
                device_name: "Realtek High Definition Audio".to_string(),
                device_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555551}".to_string(),
                default_roles: vec![],
                muted: false,
                volume_percent: 50.0,
                volume_db: -10.5,
                min_db: -96.0,
                max_db: 0.0,
                channels: 2,
                channel_volumes: vec![50.0, 50.0],
                peak_level: 0.12,
                pid: Some(4120),
                process_name: Some("vlc.exe".to_string()),
                state: AudioDeviceState::Active,
                command_line_id: "vlc.exe".to_string(),
            },
            AudioEndpointItem {
                item_id: "session:8240:discord.exe".to_string(),
                name: "Discord".to_string(),
                kind: AudioItemKind::Application,
                direction: AudioDirection::Render,
                device_name: "Realtek High Definition Audio".to_string(),
                device_id: "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555551}".to_string(),
                default_roles: vec![],
                muted: false,
                volume_percent: 90.0,
                volume_db: -1.6,
                min_db: -96.0,
                max_db: 0.0,
                channels: 2,
                channel_volumes: vec![90.0, 90.0],
                peak_level: 0.07,
                pid: Some(8240),
                process_name: Some("discord.exe".to_string()),
                state: AudioDeviceState::Active,
                command_line_id: "discord.exe".to_string(),
            },
        ];

        let app_configs = vec![RawAppPropertyStoreRecord {
            subkey: "186430e1_0".to_string(),
            registry_key: format!("HKCU\\{POLICY_STORE_KEY_PREFIX}\\186430e1_0"),
            default_value: "{2}.\\\\?\\hdaudio#func_01&#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\\topo03/00010001|\\Device\\HarddiskVolume3\\Program Files\\VideoLAN\\VLC\\vlc.exe%b{4533F59D-59EE-00C6-ADB2-C68B501A6655}".to_string(),
            blob_3_volume: Some(encode_propvar_r4(0.50).expect("valid constant scalar")),
            blob_4_channels: Some(encode_propvar_r4_array(&[1.0, 1.0]).expect("valid constant channels")),
            blob_5_mute: Some(encode_propvar_bool(false)),
            raw_1sps_container: None,
        }];

        Self {
            items: std::sync::Mutex::new(items),
            app_configs: std::sync::Mutex::new(app_configs),
            ..Self::default()
        }
    }

    fn check_faults(&self) -> WinctlResult<()> {
        if self
            .inject_hardware_unavailable
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(WinctlError::HardwareUnavailable(
                "mock CoreAudio device unavailable (no endpoint found)".to_string(),
            ));
        }
        if self.inject_permission_denied.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(WinctlError::PermissionDenied(
                "mock ERROR_ACCESS_DENIED on PolicyConfig registry key".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AudioBackend for MockAudioBackend {
    fn list_audio_items(&self) -> WinctlResult<Vec<AudioEndpointItem>> {
        self.check_faults()?;
        Ok(self.items.lock().unwrap().clone())
    }

    fn set_volume_scalar(&self, target_id: &str, scalar: f32) -> WinctlResult<AudioStateSnapshot> {
        self.check_faults()?;
        let clamped = scalar.clamp(0.0, 1.0);
        let mut items = self.items.lock().unwrap();
        let target_key = resolve_audio_target(&items, target_id)?.item_id.clone();
        let item = items.iter_mut().find(|i| i.item_id == target_key).unwrap();
        item.volume_percent = (clamped * 100.0 * 100.0).round() / 100.0;
        item.volume_db = if clamped <= 0.0001 {
            item.min_db
        } else {
            (20.0 * clamped.log10()).clamp(item.min_db, item.max_db)
        };
        item.channel_volumes.fill(item.volume_percent);
        self.action_log
            .lock()
            .unwrap()
            .push(format!("set_volume_scalar:{}:{:.3}", item.item_id, clamped));
        Ok(AudioStateSnapshot {
            item_id: item.item_id.clone(),
            name: item.name.clone(),
            volume_percent: item.volume_percent,
            volume_db: item.volume_db,
            muted: item.muted,
            channel_volumes: item.channel_volumes.clone(),
        })
    }

    fn set_channel_volumes(&self, target_id: &str, channels: &[f32]) -> WinctlResult<AudioStateSnapshot> {
        self.check_faults()?;
        if channels.is_empty() {
            return Err(WinctlError::InvalidInput(
                "channel volumes list must not be empty".to_string(),
            ));
        }
        let mut items = self.items.lock().unwrap();
        let target_key = resolve_audio_target(&items, target_id)?.item_id.clone();
        let item = items.iter_mut().find(|i| i.item_id == target_key).unwrap();
        let clamped_pcts: Vec<f32> = channels
            .iter()
            .map(|c| (c.clamp(0.0, 1.0) * 100.0 * 100.0).round() / 100.0)
            .collect();
        let avg = clamped_pcts.iter().sum::<f32>() / clamped_pcts.len() as f32;
        item.channel_volumes = clamped_pcts;
        item.channels = item.channel_volumes.len() as u32;
        item.volume_percent = avg;
        self.action_log
            .lock()
            .unwrap()
            .push(format!("set_channel_volumes:{}", item.item_id));
        Ok(AudioStateSnapshot {
            item_id: item.item_id.clone(),
            name: item.name.clone(),
            volume_percent: item.volume_percent,
            volume_db: item.volume_db,
            muted: item.muted,
            channel_volumes: item.channel_volumes.clone(),
        })
    }

    fn set_volume_db(&self, target_id: &str, db: f32) -> WinctlResult<AudioStateSnapshot> {
        self.check_faults()?;
        let mut items = self.items.lock().unwrap();
        let target_key = resolve_audio_target(&items, target_id)?.item_id.clone();
        let item = items.iter_mut().find(|i| i.item_id == target_key).unwrap();
        let clamped_db = db.clamp(item.min_db, item.max_db);
        item.volume_db = clamped_db;
        let scalar = if (item.max_db - item.min_db).abs() < f32::EPSILON {
            1.0
        } else {
            10f32.powf((clamped_db - item.max_db) / 20.0).clamp(0.0, 1.0)
        };
        item.volume_percent = (scalar * 100.0 * 100.0).round() / 100.0;
        self.action_log
            .lock()
            .unwrap()
            .push(format!("set_volume_db:{}:{:.2}", item.item_id, clamped_db));
        Ok(AudioStateSnapshot {
            item_id: item.item_id.clone(),
            name: item.name.clone(),
            volume_percent: item.volume_percent,
            volume_db: item.volume_db,
            muted: item.muted,
            channel_volumes: item.channel_volumes.clone(),
        })
    }

    fn set_mute(&self, target_id: &str, mode: MuteAction) -> WinctlResult<AudioStateSnapshot> {
        self.check_faults()?;
        let mut items = self.items.lock().unwrap();
        let target_key = resolve_audio_target(&items, target_id)?.item_id.clone();
        let item = items.iter_mut().find(|i| i.item_id == target_key).unwrap();
        item.muted = match mode {
            MuteAction::Mute => true,
            MuteAction::Unmute => false,
            MuteAction::Toggle => !item.muted,
        };
        self.action_log
            .lock()
            .unwrap()
            .push(format!("set_mute:{}:{}", item.item_id, item.muted));
        Ok(AudioStateSnapshot {
            item_id: item.item_id.clone(),
            name: item.name.clone(),
            volume_percent: item.volume_percent,
            volume_db: item.volume_db,
            muted: item.muted,
            channel_volumes: item.channel_volumes.clone(),
        })
    }

    fn set_default_endpoint(&self, device_id: &str, roles: &[DefaultRole]) -> WinctlResult<()> {
        self.check_faults()?;
        let mut items = self.items.lock().unwrap();
        let target = resolve_audio_target(&items, device_id)?;
        if target.kind != AudioItemKind::Device {
            return Err(WinctlError::InvalidInput(format!(
                "target '{device_id}' is not an audio endpoint device"
            )));
        }
        let target_id = target.item_id.clone();
        let target_dir = target.direction;
        for item in items.iter_mut() {
            if item.kind == AudioItemKind::Device && item.direction == target_dir {
                item.default_roles.retain(|r| !roles.contains(r));
                if item.item_id == target_id {
                    for &r in roles {
                        if !item.default_roles.contains(&r) {
                            item.default_roles.push(r);
                        }
                    }
                    item.default_roles.sort();
                }
            }
        }
        self.action_log
            .lock()
            .unwrap()
            .push(format!("set_default_endpoint:{target_id}:{roles:?}"));
        Ok(())
    }

    fn set_endpoint_enabled(&self, device_id: &str, enabled: bool) -> WinctlResult<()> {
        self.check_faults()?;
        let mut items = self.items.lock().unwrap();
        let target_id = resolve_audio_target(&items, device_id)?.item_id.clone();
        let item = items.iter_mut().find(|i| i.item_id == target_id).unwrap();
        item.state = if enabled {
            AudioDeviceState::Active
        } else {
            AudioDeviceState::Disabled
        };
        self.action_log
            .lock()
            .unwrap()
            .push(format!("set_endpoint_enabled:{target_id}:{enabled}"));
        Ok(())
    }

    fn set_app_default_device(&self, process_or_pid: &str, device_id: &str, roles: &[DefaultRole]) -> WinctlResult<()> {
        self.check_faults()?;
        let items = self.items.lock().unwrap();
        let dev_id = resolve_audio_target(&items, device_id)?.item_id.clone();
        self.app_default_routes
            .lock()
            .unwrap()
            .push((process_or_pid.to_string(), dev_id.clone(), roles.to_vec()));
        self.action_log
            .lock()
            .unwrap()
            .push(format!("set_app_default:{process_or_pid}:{dev_id}:{roles:?}"));
        Ok(())
    }

    fn read_app_audio_configs(&self) -> WinctlResult<Vec<RawAppPropertyStoreRecord>> {
        self.check_faults()?;
        Ok(self.app_configs.lock().unwrap().clone())
    }

    fn write_app_audio_config(&self, subkey: &str, patch: &AppPropertyStorePatch) -> WinctlResult<()> {
        let subkey = crate::app_config::validate_property_store_subkey(subkey)?;
        self.check_faults()?;
        use crate::app_config::{encode_propvar_bool, encode_propvar_r4, encode_propvar_r4_array};

        let mut configs = self.app_configs.lock().unwrap();
        if patch.delete_subkey {
            let before = configs.len();
            configs.retain(|c| !c.subkey.eq_ignore_ascii_case(subkey));
            if configs.len() == before {
                return Err(WinctlError::InvalidInput(format!(
                    "PropertyStore subkey '{subkey}' not found"
                )));
            }
            self.action_log
                .lock()
                .unwrap()
                .push(format!("delete_app_audio_config:{subkey}"));
            return Ok(());
        }
        let rec = configs
            .iter_mut()
            .find(|c| c.subkey.eq_ignore_ascii_case(subkey))
            .ok_or_else(|| WinctlError::InvalidInput(format!("PropertyStore subkey '{subkey}' not found")))?;
        if let Some(vol) = patch.volume_scalar {
            rec.blob_3_volume = Some(encode_propvar_r4(vol)?);
        }
        if let Some(ref chs) = patch.channel_volumes {
            rec.blob_4_channels = Some(encode_propvar_r4_array(chs)?);
        }
        if let Some(muted) = patch.muted {
            rec.blob_5_mute = Some(encode_propvar_bool(muted));
        }
        self.action_log
            .lock()
            .unwrap()
            .push(format!("write_app_audio_config:{subkey}"));
        Ok(())
    }

    fn run_wheel_listener(
        &self,
        rules: &[WheelRule],
        opts: &ListenerOptions,
    ) -> WinctlResult<Vec<WheelActionResult>> {
        self.check_faults()?;
        let events: Vec<PointerWheelEvent> = self
            .queued_wheel_events
            .lock()
            .unwrap()
            .iter()
            .take(opts.max_events)
            .cloned()
            .collect();
        let mut results = Vec::new();
        for ev in &events {
            if let Some(rule) = crate::wheel::find_matching_rule(rules, ev) {
                let target = if rule.target_device.trim().is_empty() {
                    "DefaultRenderDevice".to_string()
                } else {
                    rule.target_device.clone()
                };
                let current = {
                    let items = self.items.lock().unwrap();
                    resolve_audio_target(&items, &target)?.volume_percent
                };
                if let Some(matched) = crate::wheel::apply_wheel_rule(&rule, ev, current) {
                    let snap = self.set_volume_scalar(&target, matched.new_percent / 100.0)?;
                    results.push(WheelActionResult {
                        rule_id: matched.rule_id,
                        target_device: target,
                        previous_percent: current,
                        new_percent: snap.volume_percent,
                        wheel_delta: ev.wheel_delta,
                        applied: true,
                    });
                }
            }
        }
        Ok(results)
    }
}
