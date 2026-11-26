use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::time::{Duration, Instant};

use winctl2rust_core::{WinctlError, WinctlResult};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{
    E_ACCESSDENIED, E_INVALIDARG, E_NOINTERFACE, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER,
    ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_NOT_FOUND, ERROR_PRIVILEGE_NOT_HELD, ERROR_SUCCESS, HANDLE, HWND,
    LPARAM, LRESULT, MAX_PATH, POINT, RECT, RPC_E_CHANGED_MODE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::{
    AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_SERVICE_NOT_RUNNING, AudioSessionStateActive, DEVICE_STATE,
    DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_NOTPRESENT, DEVICE_STATE_UNPLUGGED, DEVICE_STATEMASK_ALL,
    EDataFlow, ERole, IAudioMute, IAudioSessionControl2, IAudioSessionManager2, IAudioVolumeLevel, IChannelAudioVolume,
    IDeviceTopology, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator, IPart, IPerChannelDbLevel,
    ISimpleAudioVolume, MMDeviceEnumerator, PartType, eCapture, eCommunications, eConsole, eMultimedia, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, STGM_READ,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_BINARY, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_LBUTTON, VK_LWIN, VK_MBUTTON, VK_MENU, VK_RBUTTON, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE,
    RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, GetClassNameW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowRect, HWND_MESSAGE, KillTimer, MSG, RI_MOUSE_WHEEL, RegisterClassExW,
    SetTimer, TranslateMessage, WM_INPUT, WM_TIMER, WNDCLASSEXW,
};
use windows::core::{BOOL, GUID, HSTRING, Interface, PCWSTR, PWSTR, w};

use crate::app_config::{
    AUDIO_APP_PROPERTY_BAG_GUID, POLICY_STORE_KEY_PREFIX, encode_propvar_bool, encode_propvar_r4,
    encode_propvar_r4_array, validate_property_store_subkey,
};
use crate::backend::{
    AppPropertyStorePatch, AudioBackend, AudioDeviceState, AudioDirection, AudioEndpointItem, AudioItemKind,
    AudioStateSnapshot, DefaultRole, ListenerOptions, MuteAction, PointerWheelEvent, RawAppPropertyStoreRecord,
    ScreenEdgeKind, WheelActionResult, WheelRule, resolve_audio_target,
};
use crate::wheel::{apply_wheel_rule, find_matching_rule};

const CLSID_CPOLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const CLSID_CPOLICY_CONFIG_VISTA_CLIENT: GUID = GUID::from_u128(0x294935ce_f637_4e7c_a41b_ab255460b862);
const REGDB_E_CLASSNOTREG_HR: i32 = 0x8004_0154_u32 as i32;
const E_NOTFOUND_HR: i32 = 0x8007_0490_u32 as i32;

#[allow(non_snake_case, dead_code)]
mod com_policy {
    use std::ffi::c_void;

    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::ERole;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::core::{BOOL, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR};

    #[windows::core::interface("F8679F50-850A-41CF-9C72-430F290290C8")]
    pub unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, device_id: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        fn GetDeviceFormat(&self, device_id: PCWSTR, default_fmt: BOOL, format: *mut *mut c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, device_id: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(
            &self,
            device_id: PCWSTR,
            endpoint_format: *const c_void,
            mix_format: *const c_void,
        ) -> HRESULT;
        fn GetProcessingPeriod(
            &self,
            device_id: PCWSTR,
            default_period: BOOL,
            pmft_default: *mut i64,
            pmft_minimum: *mut i64,
        ) -> HRESULT;
        fn SetProcessingPeriod(&self, device_id: PCWSTR, pmft_period: *const i64) -> HRESULT;
        fn GetShareMode(&self, device_id: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, device_id: PCWSTR, mode: *const c_void) -> HRESULT;
        fn GetPropertyValue(
            &self,
            device_id: PCWSTR,
            fx_store: BOOL,
            key: *const PROPERTYKEY,
            pv: *mut PROPVARIANT,
        ) -> HRESULT;
        fn SetPropertyValue(
            &self,
            device_id: PCWSTR,
            fx_store: BOOL,
            key: *const PROPERTYKEY,
            pv: *const PROPVARIANT,
        ) -> HRESULT;
        fn SetDefaultEndpoint(&self, device_id: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, device_id: PCWSTR, visible: BOOL) -> HRESULT;
    }

    #[windows::core::interface("568B9108-44BF-40B4-9006-86AFE5B5A620")]
    pub unsafe trait IPolicyConfigVista: IUnknown {
        fn GetMixFormat(&self, device_id: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        fn GetDeviceFormat(&self, device_id: PCWSTR, default_fmt: i32, format: *mut *mut c_void) -> HRESULT;
        fn SetDeviceFormat(
            &self,
            device_id: PCWSTR,
            endpoint_format: *const c_void,
            mix_format: *const c_void,
        ) -> HRESULT;
        fn GetProcessingPeriod(
            &self,
            device_id: PCWSTR,
            default_period: i32,
            pmft_default: *mut i64,
            pmft_minimum: *mut i64,
        ) -> HRESULT;
        fn SetProcessingPeriod(&self, device_id: PCWSTR, pmft_period: *const i64) -> HRESULT;
        fn GetShareMode(&self, device_id: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, device_id: PCWSTR, mode: *const c_void) -> HRESULT;
        fn GetPropertyValue(&self, device_id: PCWSTR, key: *const PROPERTYKEY, pv: *mut PROPVARIANT) -> HRESULT;
        fn SetPropertyValue(&self, device_id: PCWSTR, key: *const PROPERTYKEY, pv: *const PROPVARIANT) -> HRESULT;
        fn SetDefaultEndpoint(&self, device_id: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, device_id: PCWSTR, visible: BOOL) -> HRESULT;
    }

    pub unsafe fn policy_set_default(policy: &IPolicyConfig, device_id: PCWSTR, role: ERole) -> HRESULT {
        unsafe { policy.SetDefaultEndpoint(device_id, role) }
    }

    pub unsafe fn policy_set_visibility(policy: &IPolicyConfig, device_id: PCWSTR, visible: BOOL) -> HRESULT {
        unsafe { policy.SetEndpointVisibility(device_id, visible) }
    }

    pub unsafe fn vista_set_default(policy: &IPolicyConfigVista, device_id: PCWSTR, role: ERole) -> HRESULT {
        unsafe { policy.SetDefaultEndpoint(device_id, role) }
    }
}

use com_policy::{IPolicyConfig, IPolicyConfigVista, policy_set_default, policy_set_visibility, vista_set_default};

#[derive(Debug, Default, Clone, Copy)]
pub struct Win32AudioBackend;

impl Win32AudioBackend {
    pub fn new() -> Self {
        Self
    }
}

struct ComApartmentGuard {
    should_uninit: bool,
}

impl ComApartmentGuard {
    fn init() -> WinctlResult<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            Ok(Self { should_uninit: true })
        } else if hr == RPC_E_CHANGED_MODE {
            Ok(Self { should_uninit: false })
        } else {
            Err(WinctlError::win32(hr.0 as u32, "CoInitializeEx"))
        }
    }
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct OwnedCoTaskPwstr(PWSTR);

impl OwnedCoTaskPwstr {
    fn to_string_lossy(&self) -> String {
        if self.0.is_null() {
            return String::new();
        }
        unsafe { self.0.to_string().unwrap_or_default() }
    }
}

impl Drop for OwnedCoTaskPwstr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CoTaskMemFree(Some(self.0.0.cast::<c_void>()));
            }
        }
    }
}

struct OwnedPropVariant(PROPVARIANT);

impl Drop for OwnedPropVariant {
    fn drop(&mut self) {
        unsafe {
            let _ = PropVariantClear(&mut self.0);
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
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

struct OwnedHwnd(HWND);

impl Drop for OwnedHwnd {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}

struct RawInputGuard {
    registered: bool,
}

impl Drop for RawInputGuard {
    fn drop(&mut self) {
        if self.registered {
            let dev = RAWINPUTDEVICE {
                usUsagePage: 0x01,
                usUsage: 0x02,
                dwFlags: RIDEV_REMOVE,
                hwndTarget: HWND::default(),
            };
            unsafe {
                let _ = RegisterRawInputDevices(&[dev], size_of::<RAWINPUTDEVICE>() as u32);
            }
        }
    }
}

fn map_win32_err(err: windows::core::Error, context: &str) -> WinctlError {
    let hr = err.code();
    let code = hr.0 as u32;
    let win32_code = code & 0xFFFF;
    if hr == E_ACCESSDENIED || win32_code == ERROR_ACCESS_DENIED.0 || win32_code == ERROR_PRIVILEGE_NOT_HELD.0 {
        return WinctlError::PermissionDenied(format!("{context}: {} (code {win32_code})", err.message()));
    }
    if hr.0 == E_NOTFOUND_HR
        || hr == AUDCLNT_E_DEVICE_INVALIDATED
        || hr == AUDCLNT_E_SERVICE_NOT_RUNNING
        || win32_code == ERROR_NOT_FOUND.0
        || win32_code == ERROR_FILE_NOT_FOUND.0
    {
        return WinctlError::HardwareUnavailable(format!("{context}: {} ({code:#010x})", err.message()));
    }
    if hr == E_INVALIDARG || win32_code == ERROR_INVALID_PARAMETER.0 {
        return WinctlError::InvalidInput(format!("{context}: {} (code {win32_code})", err.message()));
    }
    WinctlError::win32(code, format!("{context}: {}", err.message()))
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

fn erole_from_default(role: DefaultRole) -> ERole {
    match role {
        DefaultRole::Console => eConsole,
        DefaultRole::Multimedia => eMultimedia,
        DefaultRole::Communications => eCommunications,
    }
}

fn device_friendly_name(device: &IMMDevice) -> String {
    let Ok(store): windows::core::Result<IPropertyStore> = (unsafe { device.OpenPropertyStore(STGM_READ) }) else {
        return String::new();
    };
    let Ok(pv) = (unsafe { store.GetValue(&PKEY_Device_FriendlyName) }) else {
        return String::new();
    };
    let owned = OwnedPropVariant(pv);
    unsafe {
        if owned.0.Anonymous.Anonymous.vt == VT_LPWSTR {
            let pw = owned.0.Anonymous.Anonymous.Anonymous.pwszVal;
            if !pw.is_null() {
                return pw.to_string().unwrap_or_default();
            }
        }
    }
    String::new()
}

fn device_state_from_win32(raw: u32) -> AudioDeviceState {
    if raw == DEVICE_STATE_ACTIVE.0 {
        AudioDeviceState::Active
    } else if raw == DEVICE_STATE_DISABLED.0 {
        AudioDeviceState::Disabled
    } else if raw == DEVICE_STATE_UNPLUGGED.0 || raw == DEVICE_STATE_NOTPRESENT.0 {
        AudioDeviceState::Unplugged
    } else {
        AudioDeviceState::Inactive
    }
}

fn get_default_id(enumerator: &IMMDeviceEnumerator, flow: EDataFlow, role: ERole) -> Option<String> {
    let dev = unsafe { enumerator.GetDefaultAudioEndpoint(flow, role) }.ok()?;
    let pw = unsafe { dev.GetId() }.ok()?;
    Some(OwnedCoTaskPwstr(pw).to_string_lossy())
}

fn process_name_map() -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return map;
    };
    let _guard = OwnedHandle(snap);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { zeroed() }
    };
    if unsafe { Process32FirstW(snap, &mut entry) }.is_ok() {
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            map.insert(entry.th32ProcessID, name);
            if unsafe { Process32NextW(snap, &mut entry) }.is_err() {
                break;
            }
        }
    }
    map
}

fn resolve_process_name(pid: u32, fallback_map: &HashMap<u32, String>) -> String {
    if pid == 0 {
        return "System Idle / Kernel".to_string();
    }
    if let Ok(proc_handle) = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        let _guard = OwnedHandle(proc_handle);
        let mut buf = vec![0u16; MAX_PATH as usize * 2];
        let mut size = buf.len() as u32;
        if unsafe { QueryFullProcessImageNameW(proc_handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size) }
            .is_ok()
        {
            let full = String::from_utf16_lossy(&buf[..size as usize]);
            if let Some(file) = full.rsplit(['\\', '/']).next()
                && !file.is_empty()
            {
                return file.to_string();
            }
        }
    }
    fallback_map
        .get(&pid)
        .cloned()
        .unwrap_or_else(|| format!("pid_{pid}.exe"))
}

fn collect_subunits_for_device(
    device: &IMMDevice,
    dev_id: &str,
    dev_name: &str,
    dir: AudioDirection,
    out: &mut Vec<AudioEndpointItem>,
) {
    let Ok(topo): windows::core::Result<IDeviceTopology> = (unsafe { device.Activate(CLSCTX_ALL, None) }) else {
        return;
    };
    let Ok(conn) = (unsafe { topo.GetConnector(0) }) else {
        return;
    };
    let Ok(connected) = (unsafe { conn.GetConnectedTo() }) else {
        return;
    };
    let Ok(part) = connected.cast::<IPart>() else {
        return;
    };
    let Ok(hw_topo) = (unsafe { part.GetTopologyObject() }) else {
        return;
    };

    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![part];
    while let Some(cur) = stack.pop() {
        let gid = unsafe { cur.GetGlobalId() }
            .map(|pw| OwnedCoTaskPwstr(pw).to_string_lossy())
            .unwrap_or_default();
        if !gid.is_empty() && !visited.insert(gid.clone()) {
            continue;
        }
        let ptype = unsafe { cur.GetPartType() }.ok();
        if ptype == Some(PartType(1)) {
            let part_name = unsafe { cur.GetName() }
                .map(|pw| OwnedCoTaskPwstr(pw).to_string_lossy())
                .unwrap_or_else(|_| "Subunit".to_string());
            let mut vol_ptr: *mut c_void = null_mut();
            let mut muted = false;
            let mut mute_ptr: *mut c_void = null_mut();
            let has_mute = unsafe { cur.Activate(CLSCTX_ALL.0, &IAudioMute::IID, Some(&mut mute_ptr)) }.is_ok()
                && !mute_ptr.is_null();
            if has_mute {
                let am = unsafe { IAudioMute::from_raw(mute_ptr) };
                muted = unsafe { am.GetMute() }.map(|b| b.as_bool()).unwrap_or(false);
            }
            let has_vol = unsafe { cur.Activate(CLSCTX_ALL.0, &IAudioVolumeLevel::IID, Some(&mut vol_ptr)) }.is_ok()
                && !vol_ptr.is_null();
            let mut db = 0.0f32;
            let mut min_db = -65.25f32;
            let mut max_db = 0.0f32;
            let mut channels = 0u32;
            let mut ch_vols = Vec::new();
            if has_vol {
                let avl = unsafe { IAudioVolumeLevel::from_raw(vol_ptr) };
                let pcd: IPerChannelDbLevel = avl.into();
                channels = unsafe { pcd.GetChannelCount() }.unwrap_or(0);
                for ch in 0..channels {
                    let mut cmin = 0.0f32;
                    let mut cmax = 0.0f32;
                    let mut cstep = 0.0f32;
                    if unsafe { pcd.GetLevelRange(ch, &mut cmin, &mut cmax, &mut cstep) }.is_ok() {
                        min_db = cmin;
                        max_db = cmax;
                    }
                    if let Ok(lvl) = unsafe { pcd.GetLevel(ch) } {
                        if ch == 0 {
                            db = lvl;
                        }
                        let pct = if (max_db - min_db).abs() > f32::EPSILON {
                            ((lvl - min_db) / (max_db - min_db) * 100.0).clamp(0.0, 100.0)
                        } else {
                            100.0
                        };
                        ch_vols.push(pct);
                    }
                }
            }
            if has_vol || has_mute {
                let item_id = format!("{dev_id}\\subunit\\{gid}");
                let avg_pct = if ch_vols.is_empty() {
                    if muted { 0.0 } else { 100.0 }
                } else {
                    ch_vols.iter().sum::<f32>() / ch_vols.len() as f32
                };
                out.push(AudioEndpointItem {
                    item_id,
                    name: part_name.clone(),
                    kind: AudioItemKind::Subunit,
                    direction: dir,
                    device_name: dev_name.to_string(),
                    device_id: dev_id.to_string(),
                    default_roles: Vec::new(),
                    muted,
                    volume_percent: (avg_pct * 100.0).round() / 100.0,
                    volume_db: db,
                    min_db,
                    max_db,
                    channels,
                    channel_volumes: ch_vols,
                    peak_level: 0.0,
                    pid: None,
                    process_name: None,
                    state: AudioDeviceState::Active,
                    command_line_id: format!("{dev_name}\\Subunit\\{part_name}\\{}", dir.as_str()),
                });
            }
        }
        let next_list = match dir {
            AudioDirection::Render => unsafe { cur.EnumPartsIncoming() },
            _ => unsafe { cur.EnumPartsOutgoing() },
        };
        if let Ok(list) = next_list {
            let count = unsafe { list.GetCount() }.unwrap_or(0);
            for i in 0..count {
                if let Ok(child) = unsafe { list.GetPart(i) } {
                    stack.push(child);
                }
            }
        }
        let _ = &hw_topo;
    }
}

fn find_device_by_id(enumerator: &IMMDeviceEnumerator, dev_id: &str) -> WinctlResult<IMMDevice> {
    let h = HSTRING::from(dev_id);
    unsafe { enumerator.GetDevice(&h) }.map_err(|e| map_win32_err(e, "IMMDeviceEnumerator::GetDevice"))
}

fn extract_device_and_sub_from_item_id(item_id: &str) -> (&str, Option<&str>) {
    if let Some((dev, sub)) = item_id.split_once("\\subunit\\") {
        (dev, Some(sub))
    } else if let Some(rest) = item_id.strip_prefix("session:") {
        if let Some((dev, _)) = rest.split_once('|') {
            (dev, None)
        } else {
            (item_id, None)
        }
    } else {
        (item_id, None)
    }
}

fn extract_session_inst_id(item_id: &str) -> Option<&str> {
    let rest = item_id.strip_prefix("session:")?;
    let (_, after_dev) = rest.split_once('|')?;
    let (_, inst_id) = after_dev.split_once('|')?;
    Some(inst_id)
}

impl AudioBackend for Win32AudioBackend {
    fn list_audio_items(&self) -> WinctlResult<Vec<AudioEndpointItem>> {
        let _com = ComApartmentGuard::init()?;
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| map_win32_err(e, "CoCreateInstance(MMDeviceEnumerator)"))?;

        let def_render = [
            (DefaultRole::Console, get_default_id(&enumerator, eRender, eConsole)),
            (
                DefaultRole::Multimedia,
                get_default_id(&enumerator, eRender, eMultimedia),
            ),
            (
                DefaultRole::Communications,
                get_default_id(&enumerator, eRender, eCommunications),
            ),
        ];
        let def_capture = [
            (DefaultRole::Console, get_default_id(&enumerator, eCapture, eConsole)),
            (
                DefaultRole::Multimedia,
                get_default_id(&enumerator, eCapture, eMultimedia),
            ),
            (
                DefaultRole::Communications,
                get_default_id(&enumerator, eCapture, eCommunications),
            ),
        ];

        let proc_names = process_name_map();
        let mut out = Vec::new();

        for (flow, dir, defaults) in [
            (eRender, AudioDirection::Render, &def_render),
            (eCapture, AudioDirection::Capture, &def_capture),
        ] {
            let collection: IMMDeviceCollection =
                match unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE(DEVICE_STATEMASK_ALL)) } {
                    Ok(c) => c,
                    Err(e) => {
                        let mapped = map_win32_err(e, "EnumAudioEndpoints");
                        if matches!(mapped, WinctlError::HardwareUnavailable(_)) {
                            continue;
                        }
                        return Err(mapped);
                    }
                };
            let count = unsafe { collection.GetCount() }.unwrap_or(0);
            for idx in 0..count {
                let Ok(device) = (unsafe { collection.Item(idx) }) else {
                    continue;
                };
                let dev_id = unsafe { device.GetId() }
                    .map(|pw| OwnedCoTaskPwstr(pw).to_string_lossy())
                    .unwrap_or_default();
                let raw_state = unsafe { device.GetState() }.unwrap_or(DEVICE_STATE_ACTIVE);
                let state = device_state_from_win32(raw_state.0);
                let friendly = device_friendly_name(&device);
                let name = if friendly.is_empty() { dev_id.clone() } else { friendly };

                let mut roles = Vec::new();
                for (r, id_opt) in defaults {
                    if id_opt.as_deref() == Some(dev_id.as_str()) {
                        roles.push(*r);
                    }
                }

                let mut muted = false;
                let mut vol_pct = 0.0f32;
                let mut vol_db = 0.0f32;
                let mut min_db = -65.25f32;
                let mut max_db = 0.0f32;
                let mut channels = 0u32;
                let mut ch_vols = Vec::new();
                if let Ok(ep_vol) = unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) } {
                    muted = unsafe { ep_vol.GetMute() }.map(|b| b.as_bool()).unwrap_or(false);
                    vol_pct = unsafe { ep_vol.GetMasterVolumeLevelScalar() }
                        .map(|s| (s * 100.0 * 100.0).round() / 100.0)
                        .unwrap_or(0.0);
                    vol_db = unsafe { ep_vol.GetMasterVolumeLevel() }.unwrap_or(0.0);
                    let mut step_db = 0.0f32;
                    let _ = unsafe { ep_vol.GetVolumeRange(&mut min_db, &mut max_db, &mut step_db) };
                    channels = unsafe { ep_vol.GetChannelCount() }.unwrap_or(0);
                    for ch in 0..channels {
                        let ch_s = unsafe { ep_vol.GetChannelVolumeLevelScalar(ch) }.unwrap_or(vol_pct / 100.0);
                        ch_vols.push((ch_s * 100.0 * 100.0).round() / 100.0);
                    }
                }
                let peak = unsafe { device.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) }
                    .and_then(|m| unsafe { m.GetPeakValue() })
                    .unwrap_or(0.0);

                out.push(AudioEndpointItem {
                    item_id: dev_id.clone(),
                    name: name.clone(),
                    kind: AudioItemKind::Device,
                    direction: dir,
                    device_name: name.clone(),
                    device_id: dev_id.clone(),
                    default_roles: roles,
                    muted,
                    volume_percent: vol_pct,
                    volume_db: vol_db,
                    min_db,
                    max_db,
                    channels,
                    channel_volumes: ch_vols,
                    peak_level: peak,
                    pid: None,
                    process_name: None,
                    state,
                    command_line_id: format!("{name}\\Device\\{}", dir.as_str()),
                });

                if state == AudioDeviceState::Active {
                    collect_subunits_for_device(&device, &dev_id, &name, dir, &mut out);

                    if let Ok(sess_mgr) = unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }
                        && let Ok(sess_enum) = unsafe { sess_mgr.GetSessionEnumerator() }
                    {
                        let scount = unsafe { sess_enum.GetCount() }.unwrap_or(0);
                        for sidx in 0..scount {
                            let Ok(ctrl) = (unsafe { sess_enum.GetSession(sidx) }) else {
                                continue;
                            };
                            let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else {
                                continue;
                            };
                            let pid = unsafe { ctrl2.GetProcessId() }.unwrap_or(0);
                            let proc_name = resolve_process_name(pid, &proc_names);
                            let disp_name = unsafe { ctrl.GetDisplayName() }
                                .map(|pw| OwnedCoTaskPwstr(pw).to_string_lossy())
                                .unwrap_or_default();
                            let sstate = unsafe { ctrl.GetState() }.ok();
                            let is_active = sstate == Some(AudioSessionStateActive);
                            let (s_pct, s_mute) = if let Ok(sav) = ctrl.cast::<ISimpleAudioVolume>() {
                                let v = unsafe { sav.GetMasterVolume() }.unwrap_or(1.0);
                                let m = unsafe { sav.GetMute() }.map(|b| b.as_bool()).unwrap_or(false);
                                ((v * 100.0 * 100.0).round() / 100.0, m)
                            } else {
                                (100.0, false)
                            };
                            let inst_id = unsafe { ctrl2.GetSessionInstanceIdentifier() }
                                .map(|pw| OwnedCoTaskPwstr(pw).to_string_lossy())
                                .unwrap_or_else(|_| format!("{pid}_{sidx}"));
                            let friendly_sess = if disp_name.is_empty() || disp_name.starts_with('@') {
                                proc_name.clone()
                            } else {
                                disp_name
                            };
                            out.push(AudioEndpointItem {
                                item_id: format!("session:{dev_id}|{pid}|{inst_id}"),
                                name: friendly_sess,
                                kind: AudioItemKind::Application,
                                direction: dir,
                                device_name: name.clone(),
                                device_id: dev_id.clone(),
                                default_roles: Vec::new(),
                                muted: s_mute,
                                volume_percent: s_pct,
                                volume_db: if s_pct <= 0.01 {
                                    -96.0
                                } else {
                                    20.0 * (s_pct / 100.0).log10()
                                },
                                min_db: -96.0,
                                max_db: 0.0,
                                channels: 2,
                                channel_volumes: vec![s_pct, s_pct],
                                peak_level: 0.0,
                                pid: Some(pid),
                                process_name: Some(proc_name.clone()),
                                state: if is_active {
                                    AudioDeviceState::Active
                                } else {
                                    AudioDeviceState::Inactive
                                },
                                command_line_id: proc_name,
                            });
                        }
                    }
                }
            }
        }

        Ok(out)
    }

    fn set_volume_scalar(&self, target_id: &str, scalar: f32) -> WinctlResult<AudioStateSnapshot> {
        let _com = ComApartmentGuard::init()?;
        let items = self.list_audio_items()?;
        let item = resolve_audio_target(&items, target_id)?.clone();
        let clamped = scalar.clamp(0.0, 1.0);

        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| map_win32_err(e, "CoCreateInstance(MMDeviceEnumerator)"))?;
        let device = find_device_by_id(&enumerator, &item.device_id)?;

        match item.kind {
            AudioItemKind::Device => {
                let ep: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "Activate(IAudioEndpointVolume)"))?;
                unsafe { ep.SetMasterVolumeLevelScalar(clamped, std::ptr::null()) }
                    .map_err(|e| map_win32_err(e, "SetMasterVolumeLevelScalar"))?;
                let new_db = unsafe { ep.GetMasterVolumeLevel() }.unwrap_or(item.volume_db);
                let muted = unsafe { ep.GetMute() }.map(|b| b.as_bool()).unwrap_or(item.muted);
                Ok(AudioStateSnapshot {
                    item_id: item.item_id,
                    name: item.name,
                    volume_percent: (clamped * 100.0 * 100.0).round() / 100.0,
                    volume_db: new_db,
                    muted,
                    channel_volumes: vec![(clamped * 100.0 * 100.0).round() / 100.0; item.channels.max(1) as usize],
                })
            }
            AudioItemKind::Application => {
                let target_pid = item.pid.unwrap_or(0);
                let target_inst = extract_session_inst_id(&item.item_id);
                let sess_mgr: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "Activate(IAudioSessionManager2)"))?;
                let sess_enum =
                    unsafe { sess_mgr.GetSessionEnumerator() }.map_err(|e| map_win32_err(e, "GetSessionEnumerator"))?;
                let count = unsafe { sess_enum.GetCount() }.unwrap_or(0);
                let mut fallback_sav: Option<ISimpleAudioVolume> = None;
                for idx in 0..count {
                    if let Ok(ctrl) = unsafe { sess_enum.GetSession(idx) }
                        && let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>()
                        && unsafe { ctrl2.GetProcessId() }.unwrap_or(u32::MAX) == target_pid
                        && let Ok(sav) = ctrl.cast::<ISimpleAudioVolume>()
                    {
                        let inst = unsafe { ctrl2.GetSessionInstanceIdentifier() }
                            .map(|pw| OwnedCoTaskPwstr(pw).to_string_lossy())
                            .ok();
                        if target_inst.is_some() && inst.as_deref() == target_inst {
                            fallback_sav = Some(sav);
                            break;
                        }
                        if fallback_sav.is_none() {
                            fallback_sav = Some(sav);
                        }
                    }
                }
                if let Some(sav) = fallback_sav {
                    unsafe { sav.SetMasterVolume(clamped, std::ptr::null()) }
                        .map_err(|e| map_win32_err(e, "ISimpleAudioVolume::SetMasterVolume"))?;
                    let muted = unsafe { sav.GetMute() }.map(|b| b.as_bool()).unwrap_or(item.muted);
                    return Ok(AudioStateSnapshot {
                        item_id: item.item_id,
                        name: item.name,
                        volume_percent: (clamped * 100.0 * 100.0).round() / 100.0,
                        volume_db: if clamped <= 0.0001 {
                            -96.0
                        } else {
                            20.0 * clamped.log10()
                        },
                        muted,
                        channel_volumes: vec![(clamped * 100.0 * 100.0).round() / 100.0; 2],
                    });
                }
                Err(WinctlError::HardwareUnavailable(format!(
                    "audio session for PID {target_pid} is no longer active"
                )))
            }
            AudioItemKind::Subunit => {
                let (_, sub_gid) = extract_device_and_sub_from_item_id(&item.item_id);
                let db = item.min_db + clamped * (item.max_db - item.min_db);
                self.set_subunit_db(&device, sub_gid.unwrap_or_default(), db)?;
                Ok(AudioStateSnapshot {
                    item_id: item.item_id,
                    name: item.name,
                    volume_percent: (clamped * 100.0 * 100.0).round() / 100.0,
                    volume_db: db,
                    muted: item.muted,
                    channel_volumes: vec![(clamped * 100.0 * 100.0).round() / 100.0; item.channels.max(1) as usize],
                })
            }
        }
    }

    fn set_channel_volumes(&self, target_id: &str, channels: &[f32]) -> WinctlResult<AudioStateSnapshot> {
        if channels.is_empty() {
            return Err(WinctlError::InvalidInput(
                "channel volumes list must not be empty".to_string(),
            ));
        }
        let _com = ComApartmentGuard::init()?;
        let items = self.list_audio_items()?;
        let item = resolve_audio_target(&items, target_id)?.clone();
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| map_win32_err(e, "CoCreateInstance(MMDeviceEnumerator)"))?;
        let device = find_device_by_id(&enumerator, &item.device_id)?;
        let pcts: Vec<f32> = channels
            .iter()
            .map(|c| (c.clamp(0.0, 1.0) * 100.0 * 100.0).round() / 100.0)
            .collect();
        let avg = pcts.iter().sum::<f32>() / pcts.len().max(1) as f32;

        match item.kind {
            AudioItemKind::Device => {
                let ep: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "Activate(IAudioEndpointVolume)"))?;
                let dev_channels = unsafe { ep.GetChannelCount() }.unwrap_or(0);
                for (idx, &scalar) in channels.iter().take(dev_channels as usize).enumerate() {
                    unsafe { ep.SetChannelVolumeLevelScalar(idx as u32, scalar.clamp(0.0, 1.0), std::ptr::null()) }
                        .map_err(|e| map_win32_err(e, "SetChannelVolumeLevelScalar"))?;
                }
                Ok(AudioStateSnapshot {
                    item_id: item.item_id,
                    name: item.name,
                    volume_percent: avg,
                    volume_db: unsafe { ep.GetMasterVolumeLevel() }.unwrap_or(item.volume_db),
                    muted: unsafe { ep.GetMute() }.map(|b| b.as_bool()).unwrap_or(item.muted),
                    channel_volumes: pcts,
                })
            }
            AudioItemKind::Application => {
                let target_pid = item.pid.unwrap_or(0);
                let target_inst = extract_session_inst_id(&item.item_id);
                let sess_mgr: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "Activate(IAudioSessionManager2)"))?;
                let sess_enum =
                    unsafe { sess_mgr.GetSessionEnumerator() }.map_err(|e| map_win32_err(e, "GetSessionEnumerator"))?;
                let count = unsafe { sess_enum.GetCount() }.unwrap_or(0);
                let mut fallback_cav: Option<IChannelAudioVolume> = None;
                for idx in 0..count {
                    if let Ok(ctrl) = unsafe { sess_enum.GetSession(idx) }
                        && let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>()
                        && unsafe { ctrl2.GetProcessId() }.unwrap_or(u32::MAX) == target_pid
                        && let Ok(cav) = ctrl.cast::<IChannelAudioVolume>()
                    {
                        let inst = unsafe { ctrl2.GetSessionInstanceIdentifier() }
                            .map(|pw| OwnedCoTaskPwstr(pw).to_string_lossy())
                            .ok();
                        if target_inst.is_some() && inst.as_deref() == target_inst {
                            fallback_cav = Some(cav);
                            break;
                        }
                        if fallback_cav.is_none() {
                            fallback_cav = Some(cav);
                        }
                    }
                }
                if let Some(cav) = fallback_cav {
                    let ch_count = unsafe { cav.GetChannelCount() }.unwrap_or(0);
                    for (idx, &scalar) in channels.iter().take(ch_count as usize).enumerate() {
                        unsafe { cav.SetChannelVolume(idx as u32, scalar.clamp(0.0, 1.0), std::ptr::null()) }
                            .map_err(|e| map_win32_err(e, "IChannelAudioVolume::SetChannelVolume"))?;
                    }
                    return Ok(AudioStateSnapshot {
                        item_id: item.item_id,
                        name: item.name,
                        volume_percent: avg,
                        volume_db: item.volume_db,
                        muted: item.muted,
                        channel_volumes: pcts,
                    });
                }
                Err(WinctlError::HardwareUnavailable(format!(
                    "audio session for PID {target_pid} is no longer active"
                )))
            }
            AudioItemKind::Subunit => {
                let (_, sub_gid) = extract_device_and_sub_from_item_id(&item.item_id);
                let topo: IDeviceTopology = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "Activate(IDeviceTopology)"))?;
                let conn = unsafe { topo.GetConnector(0) }.map_err(|e| map_win32_err(e, "GetConnector"))?;
                let connected = unsafe { conn.GetConnectedTo() }.map_err(|e| map_win32_err(e, "GetConnectedTo"))?;
                let start = connected.cast::<IPart>().map_err(|e| map_win32_err(e, "cast<IPart>"))?;
                let part = find_part_by_gid(start, sub_gid.unwrap_or_default()).ok_or_else(|| {
                    WinctlError::HardwareUnavailable(format!(
                        "subunit '{}' not found in device topology",
                        sub_gid.unwrap_or_default()
                    ))
                })?;
                let mut ptr: *mut c_void = null_mut();
                unsafe { part.Activate(CLSCTX_ALL.0, &IAudioVolumeLevel::IID, Some(&mut ptr)) }
                    .map_err(|e| map_win32_err(e, "IPart::Activate(IAudioVolumeLevel)"))?;
                let avl = unsafe { IAudioVolumeLevel::from_raw(ptr) };
                let pcd: IPerChannelDbLevel = avl.into();
                let ch_count = unsafe { pcd.GetChannelCount() }.unwrap_or(0);
                for (idx, &scalar) in channels.iter().take(ch_count as usize).enumerate() {
                    let db = item.min_db + scalar.clamp(0.0, 1.0) * (item.max_db - item.min_db);
                    unsafe { pcd.SetLevel(idx as u32, db, None) }
                        .map_err(|e| map_win32_err(e, "IPerChannelDbLevel::SetLevel"))?;
                }
                Ok(AudioStateSnapshot {
                    item_id: item.item_id,
                    name: item.name,
                    volume_percent: avg,
                    volume_db: item.volume_db,
                    muted: item.muted,
                    channel_volumes: pcts,
                })
            }
        }
    }

    fn set_volume_db(&self, target_id: &str, db: f32) -> WinctlResult<AudioStateSnapshot> {
        let _com = ComApartmentGuard::init()?;
        let items = self.list_audio_items()?;
        let item = resolve_audio_target(&items, target_id)?.clone();
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| map_win32_err(e, "CoCreateInstance(MMDeviceEnumerator)"))?;
        let device = find_device_by_id(&enumerator, &item.device_id)?;
        match item.kind {
            AudioItemKind::Device => {
                let ep: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "Activate(IAudioEndpointVolume)"))?;
                let clamped = db.clamp(item.min_db, item.max_db);
                unsafe { ep.SetMasterVolumeLevel(clamped, std::ptr::null()) }
                    .map_err(|e| map_win32_err(e, "SetMasterVolumeLevel"))?;
                let scalar = unsafe { ep.GetMasterVolumeLevelScalar() }.unwrap_or(0.0);
                Ok(AudioStateSnapshot {
                    item_id: item.item_id,
                    name: item.name,
                    volume_percent: (scalar * 100.0 * 100.0).round() / 100.0,
                    volume_db: clamped,
                    muted: unsafe { ep.GetMute() }.map(|b| b.as_bool()).unwrap_or(item.muted),
                    channel_volumes: vec![(scalar * 100.0 * 100.0).round() / 100.0; item.channels.max(1) as usize],
                })
            }
            AudioItemKind::Subunit => {
                let (_, sub_gid) = extract_device_and_sub_from_item_id(&item.item_id);
                let clamped = db.clamp(item.min_db, item.max_db);
                self.set_subunit_db(&device, sub_gid.unwrap_or_default(), clamped)?;
                let pct = if (item.max_db - item.min_db).abs() > f32::EPSILON {
                    ((clamped - item.min_db) / (item.max_db - item.min_db) * 100.0).clamp(0.0, 100.0)
                } else {
                    100.0
                };
                Ok(AudioStateSnapshot {
                    item_id: item.item_id,
                    name: item.name,
                    volume_percent: pct,
                    volume_db: clamped,
                    muted: item.muted,
                    channel_volumes: vec![pct; item.channels.max(1) as usize],
                })
            }
            AudioItemKind::Application => {
                let scalar = 10f32.powf(db / 20.0).clamp(0.0, 1.0);
                self.set_volume_scalar(&item.item_id, scalar)
            }
        }
    }

    fn set_mute(&self, target_id: &str, mode: MuteAction) -> WinctlResult<AudioStateSnapshot> {
        let _com = ComApartmentGuard::init()?;
        let items = self.list_audio_items()?;
        let item = resolve_audio_target(&items, target_id)?.clone();
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| map_win32_err(e, "CoCreateInstance(MMDeviceEnumerator)"))?;
        let device = find_device_by_id(&enumerator, &item.device_id)?;

        match item.kind {
            AudioItemKind::Device => {
                let ep: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "Activate(IAudioEndpointVolume)"))?;
                let target_mute = match mode {
                    MuteAction::Mute => true,
                    MuteAction::Unmute => false,
                    MuteAction::Toggle => !unsafe { ep.GetMute() }
                        .map_err(|e| map_win32_err(e, "GetMute"))?
                        .as_bool(),
                };
                unsafe { ep.SetMute(target_mute, std::ptr::null()) }.map_err(|e| map_win32_err(e, "SetMute"))?;
                Ok(AudioStateSnapshot {
                    item_id: item.item_id,
                    name: item.name,
                    volume_percent: item.volume_percent,
                    volume_db: item.volume_db,
                    muted: target_mute,
                    channel_volumes: item.channel_volumes,
                })
            }
            AudioItemKind::Application => {
                let target_pid = item.pid.unwrap_or(0);
                let target_inst = extract_session_inst_id(&item.item_id);
                let sess_mgr: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "Activate(IAudioSessionManager2)"))?;
                let sess_enum =
                    unsafe { sess_mgr.GetSessionEnumerator() }.map_err(|e| map_win32_err(e, "GetSessionEnumerator"))?;
                let count = unsafe { sess_enum.GetCount() }.unwrap_or(0);
                let mut fallback_sav: Option<ISimpleAudioVolume> = None;
                for idx in 0..count {
                    if let Ok(ctrl) = unsafe { sess_enum.GetSession(idx) }
                        && let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>()
                        && unsafe { ctrl2.GetProcessId() }.unwrap_or(u32::MAX) == target_pid
                        && let Ok(sav) = ctrl.cast::<ISimpleAudioVolume>()
                    {
                        let inst = unsafe { ctrl2.GetSessionInstanceIdentifier() }
                            .map(|pw| OwnedCoTaskPwstr(pw).to_string_lossy())
                            .ok();
                        if target_inst.is_some() && inst.as_deref() == target_inst {
                            fallback_sav = Some(sav);
                            break;
                        }
                        if fallback_sav.is_none() {
                            fallback_sav = Some(sav);
                        }
                    }
                }
                if let Some(sav) = fallback_sav {
                    let target_mute = match mode {
                        MuteAction::Mute => true,
                        MuteAction::Unmute => false,
                        MuteAction::Toggle => !unsafe { sav.GetMute() }
                            .map_err(|e| map_win32_err(e, "ISimpleAudioVolume::GetMute"))?
                            .as_bool(),
                    };
                    unsafe { sav.SetMute(target_mute, std::ptr::null()) }
                        .map_err(|e| map_win32_err(e, "ISimpleAudioVolume::SetMute"))?;
                    return Ok(AudioStateSnapshot {
                        item_id: item.item_id,
                        name: item.name,
                        volume_percent: item.volume_percent,
                        volume_db: item.volume_db,
                        muted: target_mute,
                        channel_volumes: item.channel_volumes,
                    });
                }
                Err(WinctlError::HardwareUnavailable(format!(
                    "audio session for PID {target_pid} is no longer active"
                )))
            }
            AudioItemKind::Subunit => {
                let (_, sub_gid) = extract_device_and_sub_from_item_id(&item.item_id);
                let new_mute = self.set_subunit_mute(&device, sub_gid.unwrap_or_default(), mode)?;
                Ok(AudioStateSnapshot {
                    item_id: item.item_id,
                    name: item.name,
                    volume_percent: item.volume_percent,
                    volume_db: item.volume_db,
                    muted: new_mute,
                    channel_volumes: item.channel_volumes,
                })
            }
        }
    }

    fn set_default_endpoint(&self, device_id: &str, roles: &[DefaultRole]) -> WinctlResult<()> {
        let _com = ComApartmentGuard::init()?;
        let items = self.list_audio_items()?;
        let item = resolve_audio_target(&items, device_id)?;
        let dev_h = HSTRING::from(item.device_id.as_str());

        let pc: Result<IPolicyConfig, _> = unsafe { CoCreateInstance(&CLSID_CPOLICY_CONFIG_CLIENT, None, CLSCTX_ALL) };
        match pc {
            Ok(policy) => {
                for &role in roles {
                    let hr = unsafe { policy_set_default(&policy, PCWSTR(dev_h.as_ptr()), erole_from_default(role)) };
                    hr.ok()
                        .map_err(|e| map_win32_err(e, "IPolicyConfig::SetDefaultEndpoint"))?;
                }
                Ok(())
            }
            Err(err) if err.code() == E_NOINTERFACE || err.code().0 == REGDB_E_CLASSNOTREG_HR => {
                let vista: IPolicyConfigVista =
                    unsafe { CoCreateInstance(&CLSID_CPOLICY_CONFIG_VISTA_CLIENT, None, CLSCTX_ALL) }
                        .map_err(|e| map_win32_err(e, "CoCreateInstance(IPolicyConfigVista)"))?;
                for &role in roles {
                    let hr = unsafe { vista_set_default(&vista, PCWSTR(dev_h.as_ptr()), erole_from_default(role)) };
                    hr.ok()
                        .map_err(|e| map_win32_err(e, "IPolicyConfigVista::SetDefaultEndpoint"))?;
                }
                Ok(())
            }
            Err(err) => Err(map_win32_err(err, "CoCreateInstance(IPolicyConfig)")),
        }
    }

    fn set_endpoint_enabled(&self, device_id: &str, enabled: bool) -> WinctlResult<()> {
        let _com = ComApartmentGuard::init()?;
        let items = self.list_audio_items()?;
        let item = resolve_audio_target(&items, device_id)?;
        let dev_h = HSTRING::from(item.device_id.as_str());

        let pc: IPolicyConfig = unsafe { CoCreateInstance(&CLSID_CPOLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
            .map_err(|e| map_win32_err(e, "CoCreateInstance(IPolicyConfig)"))?;
        let hr = unsafe { policy_set_visibility(&pc, PCWSTR(dev_h.as_ptr()), BOOL::from(enabled)) };
        hr.ok()
            .map_err(|e| map_win32_err(e, "IPolicyConfig::SetEndpointVisibility"))
    }

    fn set_app_default_device(&self, process_or_pid: &str, device_id: &str, roles: &[DefaultRole]) -> WinctlResult<()> {
        let _ = (process_or_pid, device_id, roles);
        Err(WinctlError::UnsupportedOnPlatform(
            "Per-application AudioPolicyConfig endpoint routing uses undocumented WinRT IAudioPolicyConfigFactory; use audio_view_control set_default for system roles".to_string(),
        ))
    }

    fn read_app_audio_configs(&self) -> WinctlResult<Vec<RawAppPropertyStoreRecord>> {
        let root_h = HSTRING::from(POLICY_STORE_KEY_PREFIX);
        let mut root_key = HKEY::default();
        let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &root_h, None, KEY_READ, &mut root_key) };
        if status == ERROR_FILE_NOT_FOUND || status == ERROR_NOT_FOUND {
            return Ok(Vec::new());
        }
        if status != ERROR_SUCCESS {
            return Err(map_win32_status(status.0, "RegOpenKeyExW(PolicyConfig\\PropertyStore)"));
        }
        let _root_guard = OwnedHKey(root_key);

        let mut records = Vec::new();
        let mut idx = 0u32;
        loop {
            let mut name_buf = vec![0u16; 256];
            let mut name_len = name_buf.len() as u32;
            let enum_status = unsafe {
                RegEnumKeyExW(
                    root_key,
                    idx,
                    Some(PWSTR(name_buf.as_mut_ptr())),
                    &mut name_len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            idx += 1;
            if enum_status == ERROR_NO_MORE_ITEMS {
                break;
            }
            if enum_status == ERROR_MORE_DATA {
                continue;
            }
            if enum_status != ERROR_SUCCESS {
                continue;
            }
            let subkey_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            let subkey_h = HSTRING::from(subkey_name.as_str());
            let mut sub_hkey = HKEY::default();
            if unsafe { RegOpenKeyExW(root_key, &subkey_h, None, KEY_READ, &mut sub_hkey) } != ERROR_SUCCESS {
                continue;
            }
            let _sub_guard = OwnedHKey(sub_hkey);
            let default_value = query_reg_sz(sub_hkey, None).unwrap_or_default();

            let bag_h = HSTRING::from(AUDIO_APP_PROPERTY_BAG_GUID);
            let mut bag_hkey = HKEY::default();
            let (b3, b4, b5) =
                if unsafe { RegOpenKeyExW(sub_hkey, &bag_h, None, KEY_READ, &mut bag_hkey) } == ERROR_SUCCESS {
                    let _bag_guard = OwnedHKey(bag_hkey);
                    (
                        query_reg_binary(bag_hkey, "3"),
                        query_reg_binary(bag_hkey, "4"),
                        query_reg_binary(bag_hkey, "5"),
                    )
                } else {
                    (None, None, None)
                };

            records.push(RawAppPropertyStoreRecord {
                registry_key: format!("HKCU\\{POLICY_STORE_KEY_PREFIX}\\{subkey_name}"),
                subkey: subkey_name,
                default_value,
                blob_3_volume: b3,
                blob_4_channels: b4,
                blob_5_mute: b5,
                raw_1sps_container: None,
            });
        }
        Ok(records)
    }

    fn write_app_audio_config(&self, subkey: &str, patch: &AppPropertyStorePatch) -> WinctlResult<()> {
        let subkey = validate_property_store_subkey(subkey)?;
        let root_h = HSTRING::from(POLICY_STORE_KEY_PREFIX);
        let mut root_key = HKEY::default();
        let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &root_h, None, KEY_WRITE, &mut root_key) };
        if status != ERROR_SUCCESS {
            return Err(map_win32_status(status.0, "RegOpenKeyExW(PolicyConfig\\PropertyStore)"));
        }
        let _root_guard = OwnedHKey(root_key);

        let subkey_h = HSTRING::from(subkey);
        if patch.delete_subkey {
            let del_status = unsafe { RegDeleteTreeW(root_key, &subkey_h) };
            if del_status != ERROR_SUCCESS {
                return Err(map_win32_status(del_status.0, "RegDeleteTreeW(subkey)"));
            }
            return Ok(());
        }

        let bag_path = format!("{subkey}\\{AUDIO_APP_PROPERTY_BAG_GUID}");
        let bag_h = HSTRING::from(bag_path.as_str());
        let mut bag_key = HKEY::default();
        let create_status = unsafe {
            RegCreateKeyExW(
                root_key,
                &bag_h,
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut bag_key,
                None,
            )
        };
        if create_status != ERROR_SUCCESS {
            return Err(map_win32_status(create_status.0, "RegCreateKeyExW(PropertyStore bag)"));
        }
        let _bag_guard = OwnedHKey(bag_key);

        if let Some(vol) = patch.volume_scalar {
            let bytes = encode_propvar_r4(vol)?;
            let s = unsafe { RegSetValueExW(bag_key, w!("3"), None, REG_BINARY, Some(&bytes)) };
            if s != ERROR_SUCCESS {
                return Err(map_win32_status(s.0, "RegSetValueExW('3')"));
            }
        }
        if let Some(ref chs) = patch.channel_volumes {
            let bytes = encode_propvar_r4_array(chs)?;
            let s = unsafe { RegSetValueExW(bag_key, w!("4"), None, REG_BINARY, Some(&bytes)) };
            if s != ERROR_SUCCESS {
                return Err(map_win32_status(s.0, "RegSetValueExW('4')"));
            }
        }
        if let Some(muted) = patch.muted {
            let bytes = encode_propvar_bool(muted);
            let s = unsafe { RegSetValueExW(bag_key, w!("5"), None, REG_BINARY, Some(&bytes)) };
            if s != ERROR_SUCCESS {
                return Err(map_win32_status(s.0, "RegSetValueExW('5')"));
            }
        }
        Ok(())
    }

    fn run_wheel_listener(
        &self,
        rules: &[WheelRule],
        opts: &ListenerOptions,
    ) -> WinctlResult<Vec<WheelActionResult>> {
        let hinstance = unsafe { GetModuleHandleW(None) }.map_err(|e| map_win32_err(e, "GetModuleHandleW"))?;
        let class_name = w!("Winctl2RustWheelRawInput");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wheel_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        let _atom = unsafe { RegisterClassExW(&wc) };

        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                class_name,
                w!("Winctl2RustWheelSink"),
                Default::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|e| map_win32_err(e, "CreateWindowExW(HWND_MESSAGE)"))?;
        let _hwnd_guard = OwnedHwnd(hwnd);

        let rid = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        unsafe { RegisterRawInputDevices(&[rid], size_of::<RAWINPUTDEVICE>() as u32) }
            .map_err(|e| map_win32_err(e, "RegisterRawInputDevices(RIDEV_INPUTSINK)"))?;
        let _raw_guard = RawInputGuard { registered: true };

        let timer_ms = opts.duration_ms.clamp(1, 60_000) as u32;
        let _timer_id = unsafe { SetTimer(Some(hwnd), 1, timer_ms, None) };
        let deadline = Instant::now() + Duration::from_millis(u64::from(timer_ms));
        let mut results = Vec::new();

        let mut msg = MSG::default();
        while Instant::now() < deadline && results.len() < opts.max_events {
            let ret = unsafe { GetMessageW(&mut msg, Some(hwnd), 0, 0) };
            if ret.0 <= 0 {
                break;
            }
            if msg.message == WM_TIMER {
                break;
            }
            if msg.message == WM_INPUT
                && let Some(wheel_delta) = read_mouse_wheel_delta(HRAWINPUT(msg.lParam.0 as *mut c_void))
            {
                let ev = capture_pointer_context(wheel_delta);
                if let Some(rule) = find_matching_rule(rules, &ev) {
                    let target = if rule.target_device.trim().is_empty() {
                        "DefaultRenderDevice".to_string()
                    } else {
                        rule.target_device.clone()
                    };
                    if let Ok(items) = self.list_audio_items()
                        && let Ok(target_item) = resolve_audio_target(&items, &target)
                    {
                        let current = target_item.volume_percent;
                        if let Some(matched) = apply_wheel_rule(&rule, &ev, current)
                            && let Ok(snap) = self.set_volume_scalar(&target, matched.new_percent / 100.0)
                        {
                            results.push(WheelActionResult {
                                rule_id: matched.rule_id,
                                target_device: target,
                                previous_percent: current,
                                new_percent: snap.volume_percent,
                                wheel_delta,
                                applied: true,
                            });
                        }
                    }
                }
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        unsafe {
            let _ = KillTimer(Some(hwnd), 1);
        }
        Ok(results)
    }
}

impl Win32AudioBackend {
    fn set_subunit_db(&self, device: &IMMDevice, sub_gid: &str, db: f32) -> WinctlResult<()> {
        let topo: IDeviceTopology =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|e| map_win32_err(e, "Activate(IDeviceTopology)"))?;
        let conn = unsafe { topo.GetConnector(0) }.map_err(|e| map_win32_err(e, "GetConnector"))?;
        let connected = unsafe { conn.GetConnectedTo() }.map_err(|e| map_win32_err(e, "GetConnectedTo"))?;
        let start = connected.cast::<IPart>().map_err(|e| map_win32_err(e, "cast<IPart>"))?;
        if let Some(part) = find_part_by_gid(start, sub_gid) {
            let mut ptr: *mut c_void = null_mut();
            unsafe { part.Activate(CLSCTX_ALL.0, &IAudioVolumeLevel::IID, Some(&mut ptr)) }
                .map_err(|e| map_win32_err(e, "IPart::Activate(IAudioVolumeLevel)"))?;
            let avl = unsafe { IAudioVolumeLevel::from_raw(ptr) };
            let pcd: IPerChannelDbLevel = avl.into();
            unsafe { pcd.SetLevelUniform(db, None) }.map_err(|e| map_win32_err(e, "SetLevelUniform"))?;
            return Ok(());
        }
        Err(WinctlError::HardwareUnavailable(format!(
            "subunit '{sub_gid}' not found in device topology"
        )))
    }

    fn set_subunit_mute(&self, device: &IMMDevice, sub_gid: &str, mode: MuteAction) -> WinctlResult<bool> {
        let topo: IDeviceTopology =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|e| map_win32_err(e, "Activate(IDeviceTopology)"))?;
        let conn = unsafe { topo.GetConnector(0) }.map_err(|e| map_win32_err(e, "GetConnector"))?;
        let connected = unsafe { conn.GetConnectedTo() }.map_err(|e| map_win32_err(e, "GetConnectedTo"))?;
        let start = connected.cast::<IPart>().map_err(|e| map_win32_err(e, "cast<IPart>"))?;
        if let Some(part) = find_part_by_gid(start, sub_gid) {
            let mut ptr: *mut c_void = null_mut();
            unsafe { part.Activate(CLSCTX_ALL.0, &IAudioMute::IID, Some(&mut ptr)) }
                .map_err(|e| map_win32_err(e, "IPart::Activate(IAudioMute)"))?;
            let am = unsafe { IAudioMute::from_raw(ptr) };
            let target = match mode {
                MuteAction::Mute => true,
                MuteAction::Unmute => false,
                MuteAction::Toggle => !unsafe { am.GetMute() }
                    .map_err(|e| map_win32_err(e, "IAudioMute::GetMute"))?
                    .as_bool(),
            };
            unsafe { am.SetMute(target, None) }.map_err(|e| map_win32_err(e, "IAudioMute::SetMute"))?;
            return Ok(target);
        }
        Err(WinctlError::HardwareUnavailable(format!(
            "subunit '{sub_gid}' not found in device topology"
        )))
    }
}

fn find_part_by_gid(start: IPart, target_gid: &str) -> Option<IPart> {
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![start];
    while let Some(part) = stack.pop() {
        let gid = unsafe { part.GetGlobalId() }
            .map(|pw| OwnedCoTaskPwstr(pw).to_string_lossy())
            .unwrap_or_default();
        if gid.eq_ignore_ascii_case(target_gid) {
            return Some(part);
        }
        if !gid.is_empty() && !visited.insert(gid) {
            continue;
        }
        for list in [unsafe { part.EnumPartsIncoming() }, unsafe { part.EnumPartsOutgoing() }]
            .into_iter()
            .flatten()
        {
            let count = unsafe { list.GetCount() }.unwrap_or(0);
            for i in 0..count {
                if let Ok(child) = unsafe { list.GetPart(i) } {
                    stack.push(child);
                }
            }
        }
    }
    None
}

fn query_reg_sz(hkey: HKEY, name: Option<&str>) -> Option<String> {
    let name_h = name.map(HSTRING::from);
    let pc = name_h.as_ref().map(|h| PCWSTR(h.as_ptr())).unwrap_or(PCWSTR::null());
    let mut reg_type = REG_VALUE_TYPE(0);
    let mut byte_len = 0u32;
    let st = unsafe { RegQueryValueExW(hkey, pc, None, Some(&mut reg_type), None, Some(&mut byte_len)) };
    if st != ERROR_SUCCESS || reg_type != REG_SZ || byte_len == 0 {
        return None;
    }
    let mut buf = vec![0u8; byte_len as usize];
    if unsafe {
        RegQueryValueExW(
            hkey,
            pc,
            None,
            Some(&mut reg_type),
            Some(buf.as_mut_ptr()),
            Some(&mut byte_len),
        )
    } != ERROR_SUCCESS
    {
        return None;
    }
    let units: Vec<u16> = buf[..byte_len as usize]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
}

fn query_reg_binary(hkey: HKEY, name: &str) -> Option<Vec<u8>> {
    let name_h = HSTRING::from(name);
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

unsafe extern "system" fn wheel_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn read_mouse_wheel_delta(hraw: HRAWINPUT) -> Option<i16> {
    let mut size = 0u32;
    let header_size = size_of::<RAWINPUTHEADER>() as u32;
    let r = unsafe { GetRawInputData(hraw, RID_INPUT, None, &mut size, header_size) };
    if r == u32::MAX || (size as usize) < size_of::<RAWINPUT>() {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    let copied = unsafe {
        GetRawInputData(
            hraw,
            RID_INPUT,
            Some(buf.as_mut_ptr().cast::<c_void>()),
            &mut size,
            header_size,
        )
    };
    if copied == u32::MAX || (copied as usize) < size_of::<RAWINPUT>() {
        return None;
    }
    let raw: RAWINPUT = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast::<RAWINPUT>()) };
    if raw.header.dwType != RIM_TYPEMOUSE.0 {
        return None;
    }
    let flags = unsafe { raw.data.mouse.Anonymous.Anonymous.usButtonFlags };
    if (flags & (RI_MOUSE_WHEEL as u16)) == 0 {
        return None;
    }
    let delta = unsafe { raw.data.mouse.Anonymous.Anonymous.usButtonData as i16 };
    Some(delta)
}

fn is_vk_down(vk: u16) -> bool {
    (unsafe { GetAsyncKeyState(i32::from(vk)) } as u16 & 0x8000) != 0
}

fn capture_pointer_context(wheel_delta: i16) -> PointerWheelEvent {
    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };

    let mut over_taskbar = false;
    if let Ok(tray) = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) } {
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(tray, &mut rect) }.is_ok()
            && pt.x >= rect.left
            && pt.x < rect.right
            && pt.y >= rect.top
            && pt.y < rect.bottom
        {
            over_taskbar = true;
        }
    }

    let mut screen_edge = None;
    let hmon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    if !hmon.is_invalid() {
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
            const EDGE_PX: i32 = 8;
            if pt.x <= mi.rcMonitor.left + EDGE_PX {
                screen_edge = Some(ScreenEdgeKind::Left);
            } else if pt.x >= mi.rcMonitor.right - EDGE_PX {
                screen_edge = Some(ScreenEdgeKind::Right);
            } else if pt.y <= mi.rcMonitor.top + EDGE_PX {
                screen_edge = Some(ScreenEdgeKind::Top);
            } else if pt.y >= mi.rcMonitor.bottom - EDGE_PX {
                screen_edge = Some(ScreenEdgeKind::Bottom);
            }
        }
    }

    let fg = unsafe { GetForegroundWindow() };
    let mut class_buf = [0u16; 128];
    let class_len = if !fg.0.is_null() {
        unsafe { GetClassNameW(fg, &mut class_buf) }
    } else {
        0
    };
    let window_class = if class_len > 0 {
        String::from_utf16_lossy(&class_buf[..class_len as usize])
    } else {
        String::new()
    };
    let over_desktop = window_class.eq_ignore_ascii_case("Progman") || window_class.eq_ignore_ascii_case("WorkerW");

    PointerWheelEvent {
        wheel_delta,
        alt: is_vk_down(VK_MENU.0),
        ctrl: is_vk_down(VK_CONTROL.0),
        shift: is_vk_down(VK_SHIFT.0),
        win: is_vk_down(VK_LWIN.0) || is_vk_down(VK_RWIN.0),
        left_button: is_vk_down(VK_LBUTTON.0),
        right_button: is_vk_down(VK_RBUTTON.0),
        middle_button: is_vk_down(VK_MBUTTON.0),
        over_taskbar,
        over_desktop,
        over_window_title: false,
        screen_edge,
        window_class,
    }
}
