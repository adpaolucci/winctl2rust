use std::fmt;

use serde::{Deserialize, Serialize};
use winctl2rust_core::{WinctlError, WinctlResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum MonitorOrientation {
    #[default]
    Deg0 = 0,
    Deg90 = 90,
    Deg180 = 180,
    Deg270 = 270,
}

impl MonitorOrientation {
    pub fn degrees(self) -> u32 {
        match self {
            Self::Deg0 => 0,
            Self::Deg90 => 90,
            Self::Deg180 => 180,
            Self::Deg270 => 270,
        }
    }

    pub fn from_degrees(deg: u32) -> WinctlResult<Self> {
        match deg {
            0 => Ok(Self::Deg0),
            90 | 1 => Ok(Self::Deg90),
            180 | 2 => Ok(Self::Deg180),
            270 | 3 => Ok(Self::Deg270),
            other => Err(WinctlError::InvalidInput(format!(
                "invalid monitor orientation '{other}' (expected 0, 90, 180, or 270)"
            ))),
        }
    }

    pub fn is_portrait_rotated(self) -> bool {
        matches!(self, Self::Deg90 | Self::Deg270)
    }
}

impl fmt::Display for MonitorOrientation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} deg", self.degrees())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorTopologyInfo {
    pub monitor_id: String,
    pub device_name: String,
    pub friendly_name: String,
    pub device_interface_id: String,
    pub primary: bool,
    pub active: bool,
    pub width: u32,
    pub height: u32,
    pub position_x: i32,
    pub position_y: i32,
    pub orientation: MonitorOrientation,
    pub bpp: u32,
    pub refresh_hz: u32,
    pub scale_pct: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorConfigEntry {
    pub monitor_id: String,
    pub name: String,
    pub friendly_name: String,
    pub active: bool,
    pub primary: bool,
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub display_frequency: u32,
    pub position_x: i32,
    pub position_y: i32,
    pub orientation: MonitorOrientation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorConfigProfile {
    pub monitors: Vec<MonitorConfigEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MonitorApplyOp {
    SetPrimary {
        target: String,
    },
    Enable {
        target: String,
    },
    Disable {
        target: String,
    },
    Move {
        target: String,
        x: i32,
        y: i32,
    },
    SetResolution {
        target: String,
        width: u32,
        height: u32,
        #[serde(default)]
        refresh_hz: Option<u32>,
        #[serde(default)]
        bpp: Option<u32>,
    },
    SetOrientation {
        target: String,
        orientation: MonitorOrientation,
    },
    SaveConfig {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        format: Option<String>,
    },
    LoadConfig {
        profile: MonitorConfigProfile,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorApplyResult {
    pub operation: String,
    pub target: String,
    pub message: String,
    pub restart_required: bool,
    pub topology: Vec<MonitorConfigEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawEdidEntry {
    pub registry_path: String,
    pub monitor_id: String,
    pub instance_id: String,
    pub device_interface_id: String,
    pub active: bool,
    pub raw_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DdcVcpReading {
    pub vcp_code: u8,
    pub vcp_name: String,
    pub current_value: u32,
    pub max_value: u32,
    pub is_momentary: bool,
    pub possible_values: Vec<u16>,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DdcMonitorSnapshot {
    pub monitor_id: String,
    pub monitor_name: String,
    pub device_name: String,
    pub capabilities_raw: Option<String>,
    pub vcp_readings: Vec<DdcVcpReading>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DdcSetVcpRequest {
    pub monitor: String,
    pub vcp_code: u8,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DdcSetVcpResult {
    pub monitor_id: String,
    pub monitor_name: String,
    pub vcp_code: u8,
    pub vcp_name: String,
    pub previous_value: Option<u32>,
    pub new_value: u32,
}

pub trait DisplayBackend: Send + Sync {
    fn list_monitors(&self) -> WinctlResult<Vec<MonitorTopologyInfo>>;
    fn apply_monitor_action(&self, op: &MonitorApplyOp) -> WinctlResult<MonitorApplyResult>;
    fn list_edid_records(&self) -> WinctlResult<Vec<RawEdidEntry>>;
    fn query_ddc_vcp(&self, monitor_selector: Option<&str>) -> WinctlResult<Vec<DdcMonitorSnapshot>>;
    fn set_ddc_vcp(&self, req: &DdcSetVcpRequest) -> WinctlResult<DdcSetVcpResult>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedDisplayBackend;

impl DisplayBackend for UnsupportedDisplayBackend {
    fn list_monitors(&self) -> WinctlResult<Vec<MonitorTopologyInfo>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "CCD / GDI monitor enumeration requires Windows".to_string(),
        ))
    }

    fn apply_monitor_action(&self, op: &MonitorApplyOp) -> WinctlResult<MonitorApplyResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "MultiMonitorTool display configuration ({op:?}) requires Windows"
        )))
    }

    fn list_edid_records(&self) -> WinctlResult<Vec<RawEdidEntry>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "HKLM\\SYSTEM\\CurrentControlSet\\Enum\\DISPLAY EDID enumeration requires Windows".to_string(),
        ))
    }

    fn query_ddc_vcp(&self, _monitor_selector: Option<&str>) -> WinctlResult<Vec<DdcMonitorSnapshot>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "DDC/CI dxva2.dll VCP query requires Windows".to_string(),
        ))
    }

    fn set_ddc_vcp(&self, req: &DdcSetVcpRequest) -> WinctlResult<DdcSetVcpResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "DDC/CI SetVCPFeature({:#04x}) requires Windows",
            req.vcp_code
        )))
    }
}

pub fn resolve_monitor_index(monitors: &[MonitorTopologyInfo], selector: &str) -> WinctlResult<usize> {
    let trimmed = selector.trim();
    if trimmed.is_empty() {
        return Err(WinctlError::InvalidInput(
            "monitor selector must not be empty".to_string(),
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower == "primary" {
        return monitors
            .iter()
            .position(|m| m.primary)
            .ok_or_else(|| WinctlError::HardwareUnavailable("no primary monitor present".to_string()));
    }
    if let Some(idx) = monitors.iter().position(|m| {
        m.monitor_id.eq_ignore_ascii_case(trimmed)
            || m.device_name.eq_ignore_ascii_case(trimmed)
            || m.friendly_name.eq_ignore_ascii_case(trimmed)
            || m.device_interface_id.eq_ignore_ascii_case(trimmed)
    }) {
        return Ok(idx);
    }
    if let Some(idx) = monitors.iter().position(|m| {
        m.monitor_id.to_ascii_lowercase().contains(&lower)
            || m.device_name.to_ascii_lowercase().contains(&lower)
            || m.friendly_name.to_ascii_lowercase().contains(&lower)
            || m.device_interface_id.to_ascii_lowercase().contains(&lower)
    }) {
        return Ok(idx);
    }
    Err(WinctlError::InvalidInput(format!(
        "no display matched monitor selector '{selector}'"
    )))
}

pub fn topology_to_config_entries(monitors: &[MonitorTopologyInfo]) -> Vec<MonitorConfigEntry> {
    monitors
        .iter()
        .map(|m| MonitorConfigEntry {
            monitor_id: m.monitor_id.clone(),
            name: m.device_name.clone(),
            friendly_name: m.friendly_name.clone(),
            active: m.active,
            primary: m.primary,
            width: m.width,
            height: m.height,
            bpp: m.bpp,
            display_frequency: m.refresh_hz,
            position_x: m.position_x,
            position_y: m.position_y,
            orientation: m.orientation,
        })
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct MockDisplayBackend {
    pub monitors: std::sync::Mutex<Vec<MonitorTopologyInfo>>,
    pub edid_records: std::sync::Mutex<Vec<RawEdidEntry>>,
    pub ddc_monitors: std::sync::Mutex<Vec<DdcMonitorSnapshot>>,
    pub action_log: std::sync::Mutex<Vec<String>>,
    pub inject_hardware_unavailable: std::sync::atomic::AtomicBool,
    pub inject_permission_denied: std::sync::atomic::AtomicBool,
}

#[cfg(any(test, feature = "test-support"))]
impl MockDisplayBackend {
    pub fn with_sample_data() -> Self {
        Self {
            monitors: std::sync::Mutex::new(crate::fixtures::sample_monitors()),
            edid_records: std::sync::Mutex::new(crate::fixtures::sample_edid_entries()),
            ddc_monitors: std::sync::Mutex::new(crate::fixtures::sample_ddc_snapshots()),
            ..Self::default()
        }
    }

    fn check_faults(&self) -> WinctlResult<()> {
        if self
            .inject_hardware_unavailable
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(WinctlError::HardwareUnavailable(
                "mock display hardware / DDC/CI monitor unavailable".to_string(),
            ));
        }
        if self.inject_permission_denied.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(WinctlError::PermissionDenied(
                "mock ERROR_ACCESS_DENIED on display registry / CCD".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
impl DisplayBackend for MockDisplayBackend {
    fn list_monitors(&self) -> WinctlResult<Vec<MonitorTopologyInfo>> {
        self.check_faults()?;
        Ok(self.monitors.lock().unwrap().clone())
    }

    fn apply_monitor_action(&self, op: &MonitorApplyOp) -> WinctlResult<MonitorApplyResult> {
        self.check_faults()?;
        let mut mons = self.monitors.lock().unwrap();
        let (op_name, target_label, msg) = match op {
            MonitorApplyOp::SetPrimary { target } => {
                let idx = resolve_monitor_index(&mons, target)?;
                if !mons[idx].active {
                    return Err(WinctlError::InvalidInput(format!(
                        "cannot set inactive monitor '{}' as primary",
                        mons[idx].device_name
                    )));
                }
                let shift_x = mons[idx].position_x;
                let shift_y = mons[idx].position_y;
                for m in mons.iter_mut() {
                    m.primary = false;
                    m.position_x -= shift_x;
                    m.position_y -= shift_y;
                }
                mons[idx].primary = true;
                mons[idx].position_x = 0;
                mons[idx].position_y = 0;
                (
                    "set_primary".to_string(),
                    mons[idx].device_name.clone(),
                    format!("Set {} as primary display", mons[idx].device_name),
                )
            }
            MonitorApplyOp::Enable { target } => {
                let idx = resolve_monitor_index(&mons, target)?;
                mons[idx].active = true;
                (
                    "enable".to_string(),
                    mons[idx].device_name.clone(),
                    format!("Enabled display {}", mons[idx].device_name),
                )
            }
            MonitorApplyOp::Disable { target } => {
                let idx = resolve_monitor_index(&mons, target)?;
                let active_count = mons.iter().filter(|m| m.active).count();
                if mons[idx].active && active_count <= 1 {
                    return Err(WinctlError::InvalidInput(
                        "refusing to disable the last active monitor".to_string(),
                    ));
                }
                mons[idx].active = false;
                if mons[idx].primary {
                    mons[idx].primary = false;
                    if let Some(next_active) = mons.iter_mut().find(|m| m.active) {
                        next_active.primary = true;
                    }
                }
                (
                    "disable".to_string(),
                    mons[idx].device_name.clone(),
                    format!("Disabled display {}", mons[idx].device_name),
                )
            }
            MonitorApplyOp::Move { target, x, y } => {
                let idx = resolve_monitor_index(&mons, target)?;
                mons[idx].position_x = *x;
                mons[idx].position_y = *y;
                (
                    "move".to_string(),
                    mons[idx].device_name.clone(),
                    format!("Moved {} to ({x}, {y})", mons[idx].device_name),
                )
            }
            MonitorApplyOp::SetResolution {
                target,
                width,
                height,
                refresh_hz,
                bpp,
            } => {
                if *width == 0 || *height == 0 {
                    return Err(WinctlError::InvalidInput(
                        "width and height must be positive".to_string(),
                    ));
                }
                let idx = resolve_monitor_index(&mons, target)?;
                mons[idx].width = *width;
                mons[idx].height = *height;
                if let Some(hz) = refresh_hz {
                    mons[idx].refresh_hz = *hz;
                }
                if let Some(bits) = bpp {
                    mons[idx].bpp = *bits;
                }
                (
                    "set_resolution".to_string(),
                    mons[idx].device_name.clone(),
                    format!("Set {} to {}x{}", mons[idx].device_name, width, height),
                )
            }
            MonitorApplyOp::SetOrientation { target, orientation } => {
                let idx = resolve_monitor_index(&mons, target)?;
                let was_portrait = mons[idx].orientation.is_portrait_rotated();
                let now_portrait = orientation.is_portrait_rotated();
                if was_portrait != now_portrait {
                    let m = &mut mons[idx];
                    std::mem::swap(&mut m.width, &mut m.height);
                }
                mons[idx].orientation = *orientation;
                (
                    "set_orientation".to_string(),
                    mons[idx].device_name.clone(),
                    format!("Rotated {} to {}", mons[idx].device_name, orientation),
                )
            }
            MonitorApplyOp::SaveConfig { .. } => (
                "save_config".to_string(),
                "all".to_string(),
                "Captured monitor topology profile".to_string(),
            ),
            MonitorApplyOp::LoadConfig { profile } => {
                if profile.monitors.is_empty() || !profile.monitors.iter().any(|m| m.active) {
                    return Err(WinctlError::InvalidInput(
                        "monitor profile must include at least one active monitor".to_string(),
                    ));
                }
                for entry in &profile.monitors {
                    if let Ok(idx) = resolve_monitor_index(&mons, &entry.name)
                        .or_else(|_| resolve_monitor_index(&mons, &entry.monitor_id))
                    {
                        mons[idx].active = entry.active;
                        mons[idx].primary = entry.primary;
                        mons[idx].width = entry.width;
                        mons[idx].height = entry.height;
                        mons[idx].bpp = entry.bpp;
                        mons[idx].refresh_hz = entry.display_frequency;
                        mons[idx].position_x = entry.position_x;
                        mons[idx].position_y = entry.position_y;
                        mons[idx].orientation = entry.orientation;
                    }
                }
                (
                    "load_config".to_string(),
                    "profile".to_string(),
                    format!("Loaded monitor profile ({} entries)", profile.monitors.len()),
                )
            }
        };

        self.action_log
            .lock()
            .unwrap()
            .push(format!("{op_name}:{target_label}"));
        Ok(MonitorApplyResult {
            operation: op_name,
            target: target_label,
            message: msg,
            restart_required: false,
            topology: topology_to_config_entries(&mons),
        })
    }

    fn list_edid_records(&self) -> WinctlResult<Vec<RawEdidEntry>> {
        self.check_faults()?;
        Ok(self.edid_records.lock().unwrap().clone())
    }

    fn query_ddc_vcp(&self, monitor_selector: Option<&str>) -> WinctlResult<Vec<DdcMonitorSnapshot>> {
        self.check_faults()?;
        let snaps = self.ddc_monitors.lock().unwrap();
        if let Some(sel) = monitor_selector {
            let lower = sel.trim().to_ascii_lowercase();
            let filtered: Vec<_> = snaps
                .iter()
                .filter(|s| {
                    s.monitor_id.to_ascii_lowercase().contains(&lower)
                        || s.monitor_name.to_ascii_lowercase().contains(&lower)
                        || s.device_name.to_ascii_lowercase().contains(&lower)
                })
                .cloned()
                .collect();
            if filtered.is_empty() {
                return Err(WinctlError::HardwareUnavailable(format!(
                    "no DDC/CI monitor matched '{sel}'"
                )));
            }
            return Ok(filtered);
        }
        Ok(snaps.clone())
    }

    fn set_ddc_vcp(&self, req: &DdcSetVcpRequest) -> WinctlResult<DdcSetVcpResult> {
        self.check_faults()?;
        let mut snaps = self.ddc_monitors.lock().unwrap();
        let lower = req.monitor.trim().to_ascii_lowercase();
        let mon = snaps
            .iter_mut()
            .find(|s| {
                s.monitor_id.to_ascii_lowercase() == lower
                    || s.device_name.to_ascii_lowercase() == lower
                    || s.monitor_name.to_ascii_lowercase().contains(&lower)
                    || s.monitor_id.to_ascii_lowercase().contains(&lower)
            })
            .ok_or_else(|| WinctlError::HardwareUnavailable(format!("no DDC/CI monitor matched '{}'", req.monitor)))?;

        let vcp_name = crate::ddc::vcp_code_info(req.vcp_code)
            .map(|d| d.name.to_string())
            .unwrap_or_else(|| format!("VCP {:#04X}", req.vcp_code));

        let prev = if let Some(reading) = mon.vcp_readings.iter_mut().find(|r| r.vcp_code == req.vcp_code) {
            if reading.read_only {
                return Err(WinctlError::InvalidInput(format!(
                    "VCP code {:#04X} ({}) is read-only",
                    req.vcp_code, reading.vcp_name
                )));
            }
            if reading.max_value > 0 && req.value > reading.max_value {
                return Err(WinctlError::InvalidInput(format!(
                    "VCP code {:#04X} ({}) value {} exceeds maximum {}",
                    req.vcp_code, reading.vcp_name, req.value, reading.max_value
                )));
            }
            let val_u16 = u16::try_from(req.value).map_err(|_| {
                WinctlError::InvalidInput(format!("DDC/CI VCP value {} exceeds 16-bit maximum (65535)", req.value))
            })?;
            if !reading.possible_values.is_empty() && !reading.possible_values.contains(&val_u16) {
                return Err(WinctlError::InvalidInput(format!(
                    "VCP code {:#04X} ({}) value {:#04X} not in supported values {:?}",
                    req.vcp_code, reading.vcp_name, req.value, reading.possible_values
                )));
            }
            let old = reading.current_value;
            reading.current_value = req.value;
            Some(old)
        } else {
            mon.vcp_readings.push(DdcVcpReading {
                vcp_code: req.vcp_code,
                vcp_name: vcp_name.clone(),
                current_value: req.value,
                max_value: 100,
                is_momentary: false,
                possible_values: Vec::new(),
                read_only: false,
            });
            None
        };

        self.action_log.lock().unwrap().push(format!(
            "set_ddc_vcp:{}:{:#04x}:{}",
            mon.monitor_id, req.vcp_code, req.value
        ));

        Ok(DdcSetVcpResult {
            monitor_id: mon.monitor_id.clone(),
            monitor_name: mon.monitor_name.clone(),
            vcp_code: req.vcp_code,
            vcp_name,
            previous_value: prev,
            new_value: req.value,
        })
    }
}
