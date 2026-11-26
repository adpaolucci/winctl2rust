use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};

use winctl2rust_core::{WinctlError, WinctlResult};
use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DestroyPhysicalMonitors,
    DisplayConfigGetDeviceInfo, GetCapabilitiesStringLength, GetDisplayConfigBufferSizes,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply,
    MC_MOMENTARY, MC_VCP_CODE_TYPE, PHYSICAL_MONITOR, QDC_ONLY_ACTIVE_PATHS, QDC_VIRTUAL_MODE_AWARE,
    QueryDisplayConfig, SetVCPFeature,
};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA,
    ERROR_NO_MORE_ITEMS, ERROR_NOT_FOUND, ERROR_PRIVILEGE_NOT_HELD, ERROR_SUCCESS, HMODULE, LPARAM, POINTL, RECT,
    WIN32_ERROR,
};
use windows::Win32::Graphics::Gdi::{
    CDS_NORESET, CDS_RESET, CDS_SET_PRIMARY, CDS_TEST, CDS_TYPE, CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW,
    DEVMODE_DISPLAY_ORIENTATION, DEVMODEW, DISP_CHANGE, DISP_CHANGE_BADDUALVIEW, DISP_CHANGE_BADFLAGS,
    DISP_CHANGE_BADMODE, DISP_CHANGE_BADPARAM, DISP_CHANGE_FAILED, DISP_CHANGE_NOTUPDATED, DISP_CHANGE_RESTART,
    DISP_CHANGE_SUCCESSFUL, DISPLAY_DEVICE_ACTIVE, DISPLAY_DEVICE_MIRRORING_DRIVER, DISPLAY_DEVICE_PRIMARY_DEVICE,
    DISPLAY_DEVICE_RDPUDD, DISPLAY_DEVICEW, DM_BITSPERPEL, DM_DISPLAYFREQUENCY, DM_DISPLAYORIENTATION, DM_PELSHEIGHT,
    DM_PELSWIDTH, DM_POSITION, DMDO_90, DMDO_180, DMDO_270, DMDO_DEFAULT, ENUM_CURRENT_SETTINGS,
    ENUM_DISPLAY_SETTINGS_FLAGS, EnumDisplayDevicesW, EnumDisplayMonitors, EnumDisplaySettingsExW, GetMonitorInfoW,
    HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_BINARY, REG_VALUE_TYPE, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
    RegQueryValueExW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{EDD_GET_DEVICE_INTERFACE_NAME, MONITORINFOF_PRIMARY};
use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR};

use crate::backend::{
    DdcMonitorSnapshot, DdcSetVcpRequest, DdcSetVcpResult, DdcVcpReading, DisplayBackend, MonitorApplyOp,
    MonitorApplyResult, MonitorOrientation, MonitorTopologyInfo, RawEdidEntry, resolve_monitor_index,
    topology_to_config_entries,
};
use crate::ddc::{parse_capabilities_string, vcp_code_info};

const DISPLAY_ENUM_ROOT: &str = r"SYSTEM\CurrentControlSet\Enum\DISPLAY";

#[derive(Debug, Default, Clone, Copy)]
pub struct Win32DisplayBackend;

impl Win32DisplayBackend {
    pub fn new() -> Self {
        Self
    }
}

struct OwnedHKey(HKEY);

impl Drop for OwnedHKey {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
}

struct OwnedPhysicalMonitors(Vec<PHYSICAL_MONITOR>);

impl Drop for OwnedPhysicalMonitors {
    fn drop(&mut self) {
        if !self.0.is_empty() {
            unsafe {
                let _ = DestroyPhysicalMonitors(&self.0);
            }
        }
    }
}

fn map_win32_status(code: u32, context: &str) -> WinctlError {
    if code == ERROR_ACCESS_DENIED.0 || code == ERROR_PRIVILEGE_NOT_HELD.0 {
        return WinctlError::PermissionDenied(format!("{context} (win32 {code})"));
    }
    if code == ERROR_NOT_FOUND.0 || code == ERROR_FILE_NOT_FOUND.0 {
        return WinctlError::HardwareUnavailable(format!("{context} (win32 {code})"));
    }
    if code == ERROR_INVALID_PARAMETER.0 {
        return WinctlError::InvalidInput(format!("{context} (win32 {code})"));
    }
    WinctlError::win32(code, context)
}

fn map_win32_err(err: windows::core::Error, context: &str) -> WinctlError {
    let code = err.code().0 as u32;
    let win32 = code & 0xFFFF;
    if win32 == ERROR_ACCESS_DENIED.0 || win32 == ERROR_PRIVILEGE_NOT_HELD.0 {
        return WinctlError::PermissionDenied(format!("{context}: {}", err.message()));
    }
    if win32 == ERROR_NOT_FOUND.0 || win32 == ERROR_FILE_NOT_FOUND.0 {
        return WinctlError::HardwareUnavailable(format!("{context}: {}", err.message()));
    }
    WinctlError::win32(code, format!("{context}: {}", err.message()))
}

fn map_disp_change(rc: DISP_CHANGE, context: &str) -> WinctlResult<bool> {
    match rc {
        DISP_CHANGE_SUCCESSFUL => Ok(false),
        DISP_CHANGE_RESTART => Ok(true),
        DISP_CHANGE_BADMODE => Err(WinctlError::InvalidInput(format!(
            "{context}: graphics mode not supported (DISP_CHANGE_BADMODE)"
        ))),
        DISP_CHANGE_BADPARAM => Err(WinctlError::InvalidInput(format!(
            "{context}: invalid parameter (DISP_CHANGE_BADPARAM)"
        ))),
        DISP_CHANGE_BADFLAGS => Err(WinctlError::InvalidInput(format!(
            "{context}: invalid flags (DISP_CHANGE_BADFLAGS)"
        ))),
        DISP_CHANGE_NOTUPDATED => Err(WinctlError::PermissionDenied(format!(
            "{context}: unable to write display settings to registry (DISP_CHANGE_NOTUPDATED)"
        ))),
        DISP_CHANGE_BADDUALVIEW => Err(WinctlError::HardwareUnavailable(format!(
            "{context}: system is DualView capable (DISP_CHANGE_BADDUALVIEW)"
        ))),
        DISP_CHANGE_FAILED => Err(WinctlError::win32(
            rc.0 as u32,
            format!("{context}: DISP_CHANGE_FAILED"),
        )),
        other => Err(WinctlError::win32(
            other.0 as u32,
            format!("{context}: DISP_CHANGE({})", other.0),
        )),
    }
}

fn utf16_cstr(slice: &[u16]) -> String {
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf16_lossy(&slice[..end])
}

fn orientation_from_dmdo(raw: DEVMODE_DISPLAY_ORIENTATION) -> MonitorOrientation {
    match raw {
        DMDO_90 => MonitorOrientation::Deg90,
        DMDO_180 => MonitorOrientation::Deg180,
        DMDO_270 => MonitorOrientation::Deg270,
        _ => MonitorOrientation::Deg0,
    }
}

fn dmdo_from_orientation(ori: MonitorOrientation) -> DEVMODE_DISPLAY_ORIENTATION {
    match ori {
        MonitorOrientation::Deg0 => DMDO_DEFAULT,
        MonitorOrientation::Deg90 => DMDO_90,
        MonitorOrientation::Deg180 => DMDO_180,
        MonitorOrientation::Deg270 => DMDO_270,
    }
}

#[derive(Debug, Clone)]
struct CcdTargetMeta {
    friendly_name: String,
    monitor_device_path: String,
}

fn query_ccd_source_to_target() -> HashMap<String, CcdTargetMeta> {
    let mut map = HashMap::new();
    let flags = QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE;

    let mut num_paths = 0u32;
    let mut num_modes = 0u32;
    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = Vec::new();
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = Vec::new();

    for _ in 0..3 {
        let st = unsafe { GetDisplayConfigBufferSizes(flags, &mut num_paths, &mut num_modes) };
        if st != ERROR_SUCCESS || num_paths == 0 {
            return map;
        }
        paths.resize(num_paths as usize, unsafe { zeroed() });
        modes.resize(num_modes as usize, unsafe { zeroed() });
        let qst = unsafe {
            QueryDisplayConfig(
                flags,
                &mut num_paths,
                paths.as_mut_ptr(),
                &mut num_modes,
                modes.as_mut_ptr(),
                None,
            )
        };
        if qst == ERROR_SUCCESS {
            paths.truncate(num_paths as usize);
            break;
        }
        if qst != ERROR_INSUFFICIENT_BUFFER {
            return map;
        }
    }

    for path in &paths {
        let mut src_name: DISPLAYCONFIG_SOURCE_DEVICE_NAME = unsafe { zeroed() };
        src_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        src_name.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        src_name.header.adapterId = path.sourceInfo.adapterId;
        src_name.header.id = path.sourceInfo.id;
        if unsafe { DisplayConfigGetDeviceInfo(&mut src_name.header) } != ERROR_SUCCESS.0 as i32 {
            continue;
        }
        let gdi_name = utf16_cstr(&src_name.viewGdiDeviceName);
        if gdi_name.is_empty() {
            continue;
        }

        let mut tgt_name: DISPLAYCONFIG_TARGET_DEVICE_NAME = unsafe { zeroed() };
        tgt_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        tgt_name.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        tgt_name.header.adapterId = path.targetInfo.adapterId;
        tgt_name.header.id = path.targetInfo.id;
        if unsafe { DisplayConfigGetDeviceInfo(&mut tgt_name.header) } != ERROR_SUCCESS.0 as i32 {
            continue;
        }

        let friendly = utf16_cstr(&tgt_name.monitorFriendlyDeviceName);
        let dev_path = utf16_cstr(&tgt_name.monitorDevicePath);
        map.entry(gdi_name).or_insert(CcdTargetMeta {
            friendly_name: friendly,
            monitor_device_path: dev_path,
        });
    }

    map
}

#[derive(Debug, Clone)]
struct EnumMonEntry {
    hmonitor: isize,
    device_name: String,
    primary_hint: bool,
    scale_pct: u32,
}

unsafe extern "system" fn collect_hmonitors_proc(hmon: HMONITOR, _hdc: HDC, _rect: *mut RECT, lparam: LPARAM) -> BOOL {
    let vec_ptr = lparam.0 as *mut Vec<EnumMonEntry>;
    if vec_ptr.is_null() {
        return BOOL(0);
    }
    let mut mi_ex: MONITORINFOEXW = unsafe { zeroed() };
    mi_ex.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(hmon, (&raw mut mi_ex).cast::<MONITORINFO>()) }.as_bool() {
        let device_name = utf16_cstr(&mi_ex.szDevice);
        let primary_hint = (mi_ex.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0;
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let scale_pct =
            if unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_ok() && dpi_x > 0 {
                ((dpi_x as f32 / 96.0) * 100.0).round() as u32
            } else {
                100
            };
        let list = unsafe { &mut *vec_ptr };
        list.push(EnumMonEntry {
            hmonitor: hmon.0 as isize,
            device_name,
            primary_hint,
            scale_pct,
        });
    }
    BOOL(1)
}

fn enumerate_hmonitors() -> Vec<EnumMonEntry> {
    let mut list: Vec<EnumMonEntry> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_hmonitors_proc),
            LPARAM((&raw mut list) as isize),
        );
    }
    list
}

fn extract_pnp_id_from_device_path(path: &str) -> Option<String> {
    // Example: \\?\DISPLAY#ACI27CC#5&2de1bb75&0&UID4352#{e6f07b5f-...}
    let upper = path.to_ascii_uppercase();
    if let Some(after) = upper.split("DISPLAY#").nth(1)
        && let Some(pnp) = after.split('#').next()
        && !pnp.is_empty()
    {
        return Some(pnp.to_string());
    }
    if let Some(after) = upper.split("MONITOR\\").nth(1)
        && let Some(pnp) = after.split('\\').next()
        && !pnp.is_empty()
    {
        return Some(pnp.to_string());
    }
    None
}

fn current_devmode_for_device(device_name: &str) -> WinctlResult<DEVMODEW> {
    let dev_h = HSTRING::from(device_name);
    let mut dm: DEVMODEW = unsafe { zeroed() };
    dm.dmSize = size_of::<DEVMODEW>() as u16;
    let ok = unsafe {
        EnumDisplaySettingsExW(
            PCWSTR(dev_h.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut dm,
            ENUM_DISPLAY_SETTINGS_FLAGS(0),
        )
    };
    if !ok.as_bool() {
        return Err(WinctlError::HardwareUnavailable(format!(
            "EnumDisplaySettingsExW failed for '{device_name}'"
        )));
    }
    Ok(dm)
}

fn query_reg_binary_opt(hkey: HKEY, value_name: &str) -> Option<Vec<u8>> {
    let name_h = HSTRING::from(value_name);
    let mut reg_type = REG_VALUE_TYPE(0);
    let mut byte_len = 0u32;
    let st = unsafe { RegQueryValueExW(hkey, &name_h, None, Some(&mut reg_type), None, Some(&mut byte_len)) };
    if st != ERROR_SUCCESS || reg_type != REG_BINARY || byte_len == 0 {
        return None;
    }
    let mut buf = vec![0u8; byte_len as usize];
    if unsafe {
        RegQueryValueExW(
            hkey,
            &name_h,
            None,
            Some(&mut reg_type),
            Some(buf.as_mut_ptr()),
            Some(&mut byte_len),
        )
    } != ERROR_SUCCESS
    {
        return None;
    }
    buf.truncate(byte_len as usize);
    Some(buf)
}

fn enum_reg_subkeys(hkey: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    let mut idx = 0u32;
    loop {
        let mut buf = vec![0u16; 256];
        let mut len = buf.len() as u32;
        let st = unsafe {
            RegEnumKeyExW(
                hkey,
                idx,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        idx += 1;
        if st == ERROR_NO_MORE_ITEMS {
            break;
        }
        if st == ERROR_MORE_DATA || st != ERROR_SUCCESS {
            continue;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    out
}

fn active_display_interface_paths() -> HashSet<String> {
    let mut set = HashSet::new();
    for meta in query_ccd_source_to_target().into_values() {
        if !meta.monitor_device_path.is_empty() {
            set.insert(meta.monitor_device_path.to_ascii_lowercase());
        }
    }
    let mut dev_idx = 0u32;
    loop {
        let mut dd: DISPLAY_DEVICEW = unsafe { zeroed() };
        dd.cb = size_of::<DISPLAY_DEVICEW>() as u32;
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), dev_idx, &mut dd, 0) }.as_bool() {
            break;
        }
        dev_idx += 1;
        let active = (dd.StateFlags & DISPLAY_DEVICE_ACTIVE).0 != 0;
        if !active {
            continue;
        }
        let adapter_name = utf16_cstr(&dd.DeviceName);
        let adapter_h = HSTRING::from(adapter_name.as_str());
        let mut mon_idx = 0u32;
        loop {
            let mut mdd: DISPLAY_DEVICEW = unsafe { zeroed() };
            mdd.cb = size_of::<DISPLAY_DEVICEW>() as u32;
            if !unsafe {
                EnumDisplayDevicesW(
                    PCWSTR(adapter_h.as_ptr()),
                    mon_idx,
                    &mut mdd,
                    EDD_GET_DEVICE_INTERFACE_NAME,
                )
            }
            .as_bool()
            {
                break;
            }
            mon_idx += 1;
            if (mdd.StateFlags & DISPLAY_DEVICE_ACTIVE).0 != 0 {
                let id = utf16_cstr(&mdd.DeviceID);
                if !id.is_empty() {
                    set.insert(id.to_ascii_lowercase());
                }
            }
        }
    }
    set
}

impl DisplayBackend for Win32DisplayBackend {
    fn list_monitors(&self) -> WinctlResult<Vec<MonitorTopologyInfo>> {
        let ccd_map = query_ccd_source_to_target();
        let hmonitors = enumerate_hmonitors();
        let mut out = Vec::new();
        let mut idx = 0u32;

        loop {
            let mut dd: DISPLAY_DEVICEW = unsafe { zeroed() };
            dd.cb = size_of::<DISPLAY_DEVICEW>() as u32;
            if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), idx, &mut dd, 0) }.as_bool() {
                break;
            }
            idx += 1;

            // Skip virtual mirroring drivers; surface real/RDP outputs gracefully
            if (dd.StateFlags & DISPLAY_DEVICE_MIRRORING_DRIVER).0 != 0
                && (dd.StateFlags & DISPLAY_DEVICE_RDPUDD).0 == 0
            {
                continue;
            }

            let device_name = utf16_cstr(&dd.DeviceName);
            let adapter_string = utf16_cstr(&dd.DeviceString);
            let active = (dd.StateFlags & DISPLAY_DEVICE_ACTIVE).0 != 0;
            let primary_flag = (dd.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE).0 != 0;

            let dev_h = HSTRING::from(device_name.as_str());
            let mut mon_dd: DISPLAY_DEVICEW = unsafe { zeroed() };
            mon_dd.cb = size_of::<DISPLAY_DEVICEW>() as u32;
            let has_mon =
                unsafe { EnumDisplayDevicesW(PCWSTR(dev_h.as_ptr()), 0, &mut mon_dd, EDD_GET_DEVICE_INTERFACE_NAME) }
                    .as_bool();
            let dev_iface = if has_mon {
                utf16_cstr(&mon_dd.DeviceID)
            } else {
                String::new()
            };
            let mon_string = if has_mon {
                utf16_cstr(&mon_dd.DeviceString)
            } else {
                String::new()
            };

            let ccd = ccd_map.get(&device_name);
            let device_interface_id = ccd
                .map(|m| m.monitor_device_path.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or(dev_iface);
            let friendly_name = ccd
                .map(|m| m.friendly_name.clone())
                .filter(|s| !s.is_empty())
                .or_else(|| (!mon_string.is_empty()).then_some(mon_string))
                .unwrap_or_else(|| adapter_string.clone());
            let monitor_id =
                extract_pnp_id_from_device_path(&device_interface_id).unwrap_or_else(|| utf16_cstr(&dd.DeviceID));

            let mut dm: DEVMODEW = unsafe { zeroed() };
            dm.dmSize = size_of::<DEVMODEW>() as u16;
            let has_mode = unsafe {
                EnumDisplaySettingsExW(
                    PCWSTR(dev_h.as_ptr()),
                    ENUM_CURRENT_SETTINGS,
                    &mut dm,
                    ENUM_DISPLAY_SETTINGS_FLAGS(0),
                )
            }
            .as_bool();

            let (width, height, pos_x, pos_y, orientation, bpp, refresh_hz) = if has_mode {
                let pos = unsafe { dm.Anonymous1.Anonymous2.dmPosition };
                let ori = unsafe { dm.Anonymous1.Anonymous2.dmDisplayOrientation };
                (
                    dm.dmPelsWidth,
                    dm.dmPelsHeight,
                    pos.x,
                    pos.y,
                    orientation_from_dmdo(ori),
                    dm.dmBitsPerPel,
                    dm.dmDisplayFrequency,
                )
            } else {
                (0, 0, 0, 0, MonitorOrientation::Deg0, 0, 0)
            };

            let hmon_match = hmonitors
                .iter()
                .find(|h| h.device_name.eq_ignore_ascii_case(&device_name));
            let scale_pct = hmon_match.map(|h| h.scale_pct).unwrap_or(100);
            let primary = primary_flag || hmon_match.is_some_and(|h| h.primary_hint);

            out.push(MonitorTopologyInfo {
                monitor_id,
                device_name,
                friendly_name,
                device_interface_id,
                primary,
                active,
                width,
                height,
                position_x: pos_x,
                position_y: pos_y,
                orientation,
                bpp,
                refresh_hz,
                scale_pct,
            });
        }

        Ok(out)
    }

    fn apply_monitor_action(&self, op: &MonitorApplyOp) -> WinctlResult<MonitorApplyResult> {
        let monitors = self.list_monitors()?;
        if monitors.is_empty() {
            return Err(WinctlError::HardwareUnavailable(
                "no display outputs available".to_string(),
            ));
        }

        let mut restart_required = false;
        let (op_name, target_label, msg) = match op {
            MonitorApplyOp::SetPrimary { target } => {
                let idx = resolve_monitor_index(&monitors, target)?;
                let target_mon = &monitors[idx];
                if !target_mon.active {
                    return Err(WinctlError::InvalidInput(format!(
                        "cannot set inactive monitor '{}' as primary",
                        target_mon.device_name
                    )));
                }
                let shift_x = target_mon.position_x;
                let shift_y = target_mon.position_y;

                for m in monitors.iter().filter(|m| m.active) {
                    let dev_h = HSTRING::from(m.device_name.as_str());
                    let mut dm = current_devmode_for_device(&m.device_name)?;
                    dm.dmFields |= DM_POSITION;
                    dm.Anonymous1.Anonymous2.dmPosition = POINTL {
                        x: m.position_x - shift_x,
                        y: m.position_y - shift_y,
                    };
                    let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
                    if m.device_name.eq_ignore_ascii_case(&target_mon.device_name) {
                        flags |= CDS_SET_PRIMARY;
                    }
                    let rc = unsafe { ChangeDisplaySettingsExW(PCWSTR(dev_h.as_ptr()), Some(&dm), None, flags, None) };
                    restart_required |= map_disp_change(rc, "ChangeDisplaySettingsExW(SetPrimary stage)")?;
                }
                let commit = unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None) };
                restart_required |= map_disp_change(commit, "ChangeDisplaySettingsExW(SetPrimary commit)")?;
                (
                    "set_primary".to_string(),
                    target_mon.device_name.clone(),
                    format!("Set {} as primary monitor", target_mon.device_name),
                )
            }
            MonitorApplyOp::Enable { target } => {
                let idx = resolve_monitor_index(&monitors, target)?;
                let m = &monitors[idx];
                let dev_h = HSTRING::from(m.device_name.as_str());
                let mut dm: DEVMODEW = unsafe { zeroed() };
                dm.dmSize = size_of::<DEVMODEW>() as u16;
                if !unsafe {
                    EnumDisplaySettingsExW(
                        PCWSTR(dev_h.as_ptr()),
                        ENUM_CURRENT_SETTINGS,
                        &mut dm,
                        ENUM_DISPLAY_SETTINGS_FLAGS(0),
                    )
                }
                .as_bool()
                {
                    let _ = unsafe {
                        EnumDisplaySettingsExW(
                            PCWSTR(dev_h.as_ptr()),
                            windows::Win32::Graphics::Gdi::ENUM_REGISTRY_SETTINGS,
                            &mut dm,
                            ENUM_DISPLAY_SETTINGS_FLAGS(0),
                        )
                    };
                }
                if dm.dmPelsWidth == 0 || dm.dmPelsHeight == 0 {
                    dm.dmPelsWidth = if m.width > 0 { m.width } else { 1920 };
                    dm.dmPelsHeight = if m.height > 0 { m.height } else { 1080 };
                }
                dm.dmFields |= DM_PELSWIDTH | DM_PELSHEIGHT | DM_POSITION;
                let test_rc =
                    unsafe { ChangeDisplaySettingsExW(PCWSTR(dev_h.as_ptr()), Some(&dm), None, CDS_TEST, None) };
                let _ = map_disp_change(test_rc, "ChangeDisplaySettingsExW(Enable test)")?;
                let apply_rc = unsafe {
                    ChangeDisplaySettingsExW(
                        PCWSTR(dev_h.as_ptr()),
                        Some(&dm),
                        None,
                        CDS_UPDATEREGISTRY | CDS_RESET,
                        None,
                    )
                };
                restart_required = map_disp_change(apply_rc, "ChangeDisplaySettingsExW(Enable)")?;
                (
                    "enable".to_string(),
                    m.device_name.clone(),
                    format!("Enabled {}", m.device_name),
                )
            }
            MonitorApplyOp::Disable { target } => {
                let idx = resolve_monitor_index(&monitors, target)?;
                let m = &monitors[idx];
                let active_count = monitors.iter().filter(|x| x.active).count();
                if m.active && active_count <= 1 {
                    return Err(WinctlError::InvalidInput(
                        "refusing to disable the last active monitor".to_string(),
                    ));
                }
                let dev_h = HSTRING::from(m.device_name.as_str());
                let mut dm: DEVMODEW = unsafe { zeroed() };
                dm.dmSize = size_of::<DEVMODEW>() as u16;
                dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_POSITION;
                dm.dmPelsWidth = 0;
                dm.dmPelsHeight = 0;
                let apply_rc = unsafe {
                    ChangeDisplaySettingsExW(
                        PCWSTR(dev_h.as_ptr()),
                        Some(&dm),
                        None,
                        CDS_UPDATEREGISTRY | CDS_RESET,
                        None,
                    )
                };
                restart_required = map_disp_change(apply_rc, "ChangeDisplaySettingsExW(Disable)")?;
                (
                    "disable".to_string(),
                    m.device_name.clone(),
                    format!("Disabled {}", m.device_name),
                )
            }
            MonitorApplyOp::Move { target, x, y } => {
                let idx = resolve_monitor_index(&monitors, target)?;
                let m = &monitors[idx];
                let dev_h = HSTRING::from(m.device_name.as_str());
                let mut dm = current_devmode_for_device(&m.device_name)?;
                dm.dmFields |= DM_POSITION;
                dm.Anonymous1.Anonymous2.dmPosition = POINTL { x: *x, y: *y };
                let test_rc =
                    unsafe { ChangeDisplaySettingsExW(PCWSTR(dev_h.as_ptr()), Some(&dm), None, CDS_TEST, None) };
                let _ = map_disp_change(test_rc, "ChangeDisplaySettingsExW(Move test)")?;
                let apply_rc = unsafe {
                    ChangeDisplaySettingsExW(
                        PCWSTR(dev_h.as_ptr()),
                        Some(&dm),
                        None,
                        CDS_UPDATEREGISTRY | CDS_RESET,
                        None,
                    )
                };
                restart_required = map_disp_change(apply_rc, "ChangeDisplaySettingsExW(Move)")?;
                (
                    "move".to_string(),
                    m.device_name.clone(),
                    format!("Moved {} to ({x}, {y})", m.device_name),
                )
            }
            MonitorApplyOp::SetResolution {
                target,
                width,
                height,
                refresh_hz,
                bpp,
            } => {
                let idx = resolve_monitor_index(&monitors, target)?;
                let m = &monitors[idx];
                let dev_h = HSTRING::from(m.device_name.as_str());
                let mut dm = current_devmode_for_device(&m.device_name)?;
                dm.dmFields |= DM_PELSWIDTH | DM_PELSHEIGHT;
                dm.dmPelsWidth = *width;
                dm.dmPelsHeight = *height;
                if let Some(hz) = refresh_hz {
                    dm.dmFields |= DM_DISPLAYFREQUENCY;
                    dm.dmDisplayFrequency = *hz;
                }
                if let Some(bits) = bpp {
                    dm.dmFields |= DM_BITSPERPEL;
                    dm.dmBitsPerPel = *bits;
                }
                let test_rc =
                    unsafe { ChangeDisplaySettingsExW(PCWSTR(dev_h.as_ptr()), Some(&dm), None, CDS_TEST, None) };
                let _ = map_disp_change(test_rc, "ChangeDisplaySettingsExW(SetResolution test)")?;
                let apply_rc = unsafe {
                    ChangeDisplaySettingsExW(
                        PCWSTR(dev_h.as_ptr()),
                        Some(&dm),
                        None,
                        CDS_UPDATEREGISTRY | CDS_RESET,
                        None,
                    )
                };
                restart_required = map_disp_change(apply_rc, "ChangeDisplaySettingsExW(SetResolution)")?;
                (
                    "set_resolution".to_string(),
                    m.device_name.clone(),
                    format!("Set {} resolution to {}x{}", m.device_name, width, height),
                )
            }
            MonitorApplyOp::SetOrientation { target, orientation } => {
                let idx = resolve_monitor_index(&monitors, target)?;
                let m = &monitors[idx];
                let dev_h = HSTRING::from(m.device_name.as_str());
                let mut dm = current_devmode_for_device(&m.device_name)?;
                let old_ori = orientation_from_dmdo(unsafe { dm.Anonymous1.Anonymous2.dmDisplayOrientation });
                if old_ori.is_portrait_rotated() != orientation.is_portrait_rotated() {
                    std::mem::swap(&mut dm.dmPelsWidth, &mut dm.dmPelsHeight);
                    dm.dmFields |= DM_PELSWIDTH | DM_PELSHEIGHT;
                }
                dm.dmFields |= DM_DISPLAYORIENTATION;
                dm.Anonymous1.Anonymous2.dmDisplayOrientation = dmdo_from_orientation(*orientation);
                let test_rc =
                    unsafe { ChangeDisplaySettingsExW(PCWSTR(dev_h.as_ptr()), Some(&dm), None, CDS_TEST, None) };
                let _ = map_disp_change(test_rc, "ChangeDisplaySettingsExW(SetOrientation test)")?;
                let apply_rc = unsafe {
                    ChangeDisplaySettingsExW(
                        PCWSTR(dev_h.as_ptr()),
                        Some(&dm),
                        None,
                        CDS_UPDATEREGISTRY | CDS_RESET,
                        None,
                    )
                };
                restart_required = map_disp_change(apply_rc, "ChangeDisplaySettingsExW(SetOrientation)")?;
                (
                    "set_orientation".to_string(),
                    m.device_name.clone(),
                    format!("Rotated {} to {}", m.device_name, orientation),
                )
            }
            MonitorApplyOp::SaveConfig { .. } => (
                "save_config".to_string(),
                "all".to_string(),
                "Captured live monitor configuration".to_string(),
            ),
            MonitorApplyOp::LoadConfig { profile } => {
                if profile.monitors.is_empty() || !profile.monitors.iter().any(|m| m.active) {
                    return Err(WinctlError::InvalidInput(
                        "monitor profile must include at least one active monitor".to_string(),
                    ));
                }
                for entry in &profile.monitors {
                    let dev_h = HSTRING::from(entry.name.as_str());
                    let mut dm = current_devmode_for_device(&entry.name).unwrap_or_else(|_| {
                        let mut d: DEVMODEW = unsafe { zeroed() };
                        d.dmSize = size_of::<DEVMODEW>() as u16;
                        d
                    });
                    dm.dmFields |= DM_POSITION
                        | DM_DISPLAYORIENTATION
                        | DM_PELSWIDTH
                        | DM_PELSHEIGHT
                        | DM_BITSPERPEL
                        | DM_DISPLAYFREQUENCY;
                    dm.Anonymous1.Anonymous2.dmPosition = POINTL {
                        x: entry.position_x,
                        y: entry.position_y,
                    };
                    dm.Anonymous1.Anonymous2.dmDisplayOrientation = dmdo_from_orientation(entry.orientation);
                    dm.dmPelsWidth = if entry.active { entry.width } else { 0 };
                    dm.dmPelsHeight = if entry.active { entry.height } else { 0 };
                    dm.dmBitsPerPel = entry.bpp;
                    dm.dmDisplayFrequency = entry.display_frequency;

                    let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
                    if entry.primary && entry.active {
                        flags |= CDS_SET_PRIMARY;
                    }
                    let rc = unsafe { ChangeDisplaySettingsExW(PCWSTR(dev_h.as_ptr()), Some(&dm), None, flags, None) };
                    restart_required |= map_disp_change(rc, "ChangeDisplaySettingsExW(LoadConfig stage)")?;
                }
                let commit = unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None) };
                restart_required |= map_disp_change(commit, "ChangeDisplaySettingsExW(LoadConfig commit)")?;
                (
                    "load_config".to_string(),
                    "profile".to_string(),
                    format!("Applied monitor configuration ({} monitors)", profile.monitors.len()),
                )
            }
        };

        let updated = self.list_monitors().unwrap_or(monitors);
        Ok(MonitorApplyResult {
            operation: op_name,
            target: target_label,
            message: msg,
            restart_required,
            topology: topology_to_config_entries(&updated),
        })
    }

    fn list_edid_records(&self) -> WinctlResult<Vec<RawEdidEntry>> {
        let root_h = HSTRING::from(DISPLAY_ENUM_ROOT);
        let mut root_key = HKEY::default();
        let st = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, &root_h, None, KEY_READ, &mut root_key) };
        if st == ERROR_FILE_NOT_FOUND || st == ERROR_NOT_FOUND {
            return Ok(Vec::new());
        }
        if st != ERROR_SUCCESS {
            return Err(map_win32_status(st.0, "RegOpenKeyExW(Enum\\DISPLAY)"));
        }
        let _root_guard = OwnedHKey(root_key);

        let active_paths = active_display_interface_paths();
        let mut records = Vec::new();

        for monitor_id in enum_reg_subkeys(root_key) {
            let mon_h = HSTRING::from(monitor_id.as_str());
            let mut mon_key = HKEY::default();
            if unsafe { RegOpenKeyExW(root_key, &mon_h, None, KEY_READ, &mut mon_key) } != ERROR_SUCCESS {
                continue;
            }
            let _mon_guard = OwnedHKey(mon_key);

            for instance_id in enum_reg_subkeys(mon_key) {
                let dp_subpath = format!("{instance_id}\\Device Parameters");
                let dp_h = HSTRING::from(dp_subpath.as_str());
                let mut dp_key = HKEY::default();
                if unsafe { RegOpenKeyExW(mon_key, &dp_h, None, KEY_READ, &mut dp_key) } != ERROR_SUCCESS {
                    continue;
                }
                let _dp_guard = OwnedHKey(dp_key);

                let Some(raw_bytes) =
                    query_reg_binary_opt(dp_key, "EDID").or_else(|| query_reg_binary_opt(dp_key, "BAD_EDID"))
                else {
                    continue;
                };

                let marker = format!("display#{monitor_id}#{instance_id}").to_ascii_lowercase();
                let alt_marker = format!("display\\{monitor_id}\\{instance_id}").to_ascii_lowercase();
                let matched_path = active_paths
                    .iter()
                    .find(|p| p.contains(&marker) || p.contains(&alt_marker))
                    .cloned();
                let active = matched_path.is_some();

                records.push(RawEdidEntry {
                    registry_path: format!("HKLM\\{DISPLAY_ENUM_ROOT}\\{monitor_id}\\{dp_subpath}\\EDID"),
                    monitor_id: monitor_id.clone(),
                    instance_id,
                    device_interface_id: matched_path.unwrap_or_default(),
                    active,
                    raw_bytes,
                });
            }
        }

        Ok(records)
    }

    fn query_ddc_vcp(&self, monitor_selector: Option<&str>) -> WinctlResult<Vec<DdcMonitorSnapshot>> {
        let topology = self.list_monitors().unwrap_or_default();
        let hmons = enumerate_hmonitors();
        if hmons.is_empty() {
            return Err(WinctlError::HardwareUnavailable(
                "no HMONITOR instances available for DDC/CI".to_string(),
            ));
        }

        let probe_codes: &[u8] = &[0x10, 0x12, 0x14, 0x16, 0x18, 0x1A, 0x60, 0x62, 0x8D, 0xB6, 0xD6, 0xDF];
        let mut out = Vec::new();

        for hm in hmons {
            let topo_match = topology
                .iter()
                .find(|m| m.device_name.eq_ignore_ascii_case(&hm.device_name));
            let monitor_id = topo_match
                .map(|m| m.monitor_id.clone())
                .unwrap_or_else(|| hm.device_name.clone());
            let friendly = topo_match
                .map(|m| m.friendly_name.clone())
                .unwrap_or_else(|| hm.device_name.clone());

            if let Some(sel) = monitor_selector {
                let lower = sel.trim().to_ascii_lowercase();
                if !monitor_id.to_ascii_lowercase().contains(&lower)
                    && !friendly.to_ascii_lowercase().contains(&lower)
                    && !hm.device_name.to_ascii_lowercase().contains(&lower)
                {
                    continue;
                }
            }

            let hmonitor = HMONITOR(hm.hmonitor as *mut c_void);
            let mut count = 0u32;
            if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count) }.is_err() || count == 0 {
                continue;
            }
            let mut phys = vec![PHYSICAL_MONITOR::default(); count as usize];
            if unsafe { GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut phys) }.is_err() {
                continue;
            }
            let _guard = OwnedPhysicalMonitors(phys.clone());

            for pm in &phys {
                let hphys = pm.hPhysicalMonitor;
                let desc_buf = pm.szPhysicalMonitorDescription;
                let desc = utf16_cstr(&desc_buf);
                let mon_name = if friendly.is_empty() { desc } else { friendly.clone() };

                let mut cap_len = 0u32;
                let capabilities_raw =
                    if unsafe { GetCapabilitiesStringLength(hphys, &mut cap_len) } != 0 && cap_len > 1 {
                        let mut buf = vec![0u8; cap_len as usize];
                        if unsafe { CapabilitiesRequestAndCapabilitiesReply(hphys, &mut buf) } != 0 {
                            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                            Some(String::from_utf8_lossy(&buf[..end]).to_string())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                let parsed_caps = capabilities_raw
                    .as_deref()
                    .and_then(|s| parse_capabilities_string(s).ok());

                let mut codes: Vec<u8> = if let Some(ref pc) = parsed_caps {
                    if pc.vcp.is_empty() {
                        probe_codes.to_vec()
                    } else {
                        pc.vcp.keys().copied().collect()
                    }
                } else {
                    probe_codes.to_vec()
                };
                codes.sort_unstable();
                codes.dedup();

                let mut readings = Vec::new();
                for code in codes {
                    let mut vct = MC_VCP_CODE_TYPE(0);
                    let mut cur = 0u32;
                    let mut max = 0u32;
                    if unsafe { GetVCPFeatureAndVCPFeatureReply(hphys, code, Some(&mut vct), &mut cur, Some(&mut max)) }
                        != 0
                    {
                        let dict = vcp_code_info(code);
                        let possible = parsed_caps
                            .as_ref()
                            .and_then(|c| c.vcp.get(&code))
                            .cloned()
                            .unwrap_or_default();
                        readings.push(DdcVcpReading {
                            vcp_code: code,
                            vcp_name: dict
                                .map(|d| d.name.to_string())
                                .unwrap_or_else(|| format!("VCP {code:#04X}")),
                            current_value: cur,
                            max_value: max,
                            is_momentary: vct == MC_MOMENTARY,
                            possible_values: possible,
                            read_only: dict.is_some_and(|d| d.read_only),
                        });
                    }
                }

                out.push(DdcMonitorSnapshot {
                    monitor_id: monitor_id.clone(),
                    monitor_name: mon_name,
                    device_name: hm.device_name.clone(),
                    capabilities_raw,
                    vcp_readings: readings,
                });
            }
        }

        if out.is_empty() {
            return Err(WinctlError::HardwareUnavailable(
                "no DDC/CI physical monitors responded on this display session".to_string(),
            ));
        }
        Ok(out)
    }

    fn set_ddc_vcp(&self, req: &DdcSetVcpRequest) -> WinctlResult<DdcSetVcpResult> {
        let topology = self.list_monitors().unwrap_or_default();
        let hmons = enumerate_hmonitors();
        let lower = req.monitor.trim().to_ascii_lowercase();

        for hm in hmons {
            let topo_match = topology
                .iter()
                .find(|m| m.device_name.eq_ignore_ascii_case(&hm.device_name));
            let monitor_id = topo_match
                .map(|m| m.monitor_id.clone())
                .unwrap_or_else(|| hm.device_name.clone());
            let friendly = topo_match
                .map(|m| m.friendly_name.clone())
                .unwrap_or_else(|| hm.device_name.clone());

            if !monitor_id.to_ascii_lowercase().contains(&lower)
                && !friendly.to_ascii_lowercase().contains(&lower)
                && !hm.device_name.to_ascii_lowercase().contains(&lower)
            {
                continue;
            }

            let hmonitor = HMONITOR(hm.hmonitor as *mut c_void);
            let mut count = 0u32;
            unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count) }
                .map_err(|e| map_win32_err(e, "GetNumberOfPhysicalMonitorsFromHMONITOR"))?;
            if count == 0 {
                continue;
            }
            let mut phys = vec![PHYSICAL_MONITOR::default(); count as usize];
            unsafe { GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut phys) }
                .map_err(|e| map_win32_err(e, "GetPhysicalMonitorsFromHMONITOR"))?;
            let _guard = OwnedPhysicalMonitors(phys.clone());

            if let Some(pm) = phys.first() {
                let hphys = pm.hPhysicalMonitor;
                let mut prev = 0u32;
                let prev_opt =
                    if unsafe { GetVCPFeatureAndVCPFeatureReply(hphys, req.vcp_code, None, &mut prev, None) } != 0 {
                        Some(prev)
                    } else {
                        None
                    };

                if unsafe { SetVCPFeature(hphys, req.vcp_code, req.value) } == 0 {
                    let err = windows::core::Error::from_thread();
                    return Err(WinctlError::HardwareUnavailable(format!(
                        "SetVCPFeature({:#04X}, {}) failed on '{}': {}",
                        req.vcp_code,
                        req.value,
                        friendly,
                        err.message()
                    )));
                }

                let vcp_name = vcp_code_info(req.vcp_code)
                    .map(|d| d.name.to_string())
                    .unwrap_or_else(|| format!("VCP {:#04X}", req.vcp_code));

                return Ok(DdcSetVcpResult {
                    monitor_id,
                    monitor_name: friendly,
                    vcp_code: req.vcp_code,
                    vcp_name,
                    previous_value: prev_opt,
                    new_value: req.value,
                });
            }
        }

        let _ = HMODULE::default();
        let _ = WIN32_ERROR(0);
        Err(WinctlError::HardwareUnavailable(format!(
            "no DDC/CI physical monitor matched '{}'",
            req.monitor
        )))
    }
}
