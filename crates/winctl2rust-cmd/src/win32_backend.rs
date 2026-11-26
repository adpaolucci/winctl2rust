use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::copy_nonoverlapping;
use std::thread;
use std::time::Duration;

use chrono::Local;
use winctl2rust_core::{WinctlError, WinctlResult};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ERROR_PRIVILEGE_NOT_HELD, ERROR_SUCCESS, GlobalFree,
    HANDLE, HGLOBAL, HWND, LPARAM, RPC_E_CHANGED_MODE, SetLastError, VARIANT_FALSE, VARIANT_TRUE, WPARAM,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::Media::Speech::{ISpVoice, SPF_ASYNC, SPF_DEFAULT, SpVoice};
use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Diagnostics::Debug::Beep;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_WRITE, REG_BINARY, REG_DWORD,
    REG_EXPAND_SZ, REG_MULTI_SZ, REG_OPTION_NON_VOLATILE, REG_QWORD, REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegSetValueExW,
};
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, SC_HANDLE, SC_MANAGER_CONNECT,
    SERVICE_CONTROL_CONTINUE, SERVICE_CONTROL_PAUSE, SERVICE_CONTROL_STOP, SERVICE_PAUSE_CONTINUE, SERVICE_START,
    SERVICE_STATUS, SERVICE_STOP, StartServiceW,
};
use windows::Win32::System::Shutdown::{
    EWX_FORCE, EWX_FORCEIFHUNG, EWX_LOGOFF, EWX_POWEROFF, EWX_REBOOT, EWX_SHUTDOWN, ExitWindowsEx, LockWorkStation,
    SHTDN_REASON_MAJOR_OTHER,
};
use windows::Win32::System::TaskScheduler::{
    IAction, IExecAction, IPrincipal, IRegisteredTask, ITaskDefinition, ITaskService, ITimeTrigger, ITrigger,
    TASK_ACTION_EXEC, TASK_CREATE_OR_UPDATE, TASK_ENUM_HIDDEN, TASK_LOGON_INTERACTIVE_TOKEN, TASK_LOGON_S4U,
    TASK_LOGON_SERVICE_ACCOUNT, TASK_RUNLEVEL_HIGHEST, TASK_RUNLEVEL_LUA, TASK_STATE_DISABLED, TASK_STATE_QUEUED,
    TASK_STATE_READY, TASK_STATE_RUNNING, TASK_TRIGGER_TIME, TaskScheduler,
};
use windows::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, CREATE_NEW_CONSOLE, CREATE_NO_WINDOW,
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, GetProcessId, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, NORMAL_PRIORITY_CLASS, OpenProcess,
    PROCESS_ACCESS_RIGHTS, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, REALTIME_PRIORITY_CLASS, STARTF_USESHOWWINDOW,
    STARTUPINFOEXW, STARTUPINFOW, SetPriorityClass, SetProcessAffinityMask, UpdateProcThreadAttribute,
    WaitForSingleObject,
};
use windows::Win32::System::Variant::{VARIANT, VT_I4, VariantClear};
use windows::Win32::UI::Shell::{
    IShellLinkW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW, ShellExecuteW, ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, HWND_BROADCAST, IsWindowVisible,
    PostMessageW, SC_MONITORPOWER, SHOW_WINDOW_CMD, SW_HIDE, SW_MINIMIZE, SW_RESTORE, SW_SHOW, SW_SHOWMAXIMIZED,
    SW_SHOWNORMAL, SendMessageW, ShowWindow, WM_CLOSE, WM_SETTEXT, WM_SYSCOMMAND,
};
use windows::core::{BOOL, BSTR, HSTRING, Interface, PCWSTR, PWSTR};

use crate::atnow::{AtNowScheduleResult, AtNowTaskInfo, AtNowTaskRequest};
use crate::backend::CmdBackend;
use crate::from_proc::{
    FromProcessRequest, FromProcessResult, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS_VALUE, PROCESS_CREATE_PROCESS_ACCESS,
    ProcessSummary,
};
use crate::script::{
    ClipboardOp, MuteMode, PowerAction, RegValueKind, ScriptCommand, ScriptStepResult, ServiceAction, WinAction,
    WinSelector, apply_ini_operation,
};
use crate::run::{LaunchProfile, LaunchResult, PriorityClass, RunAsMode, WindowState};

#[derive(Debug, Default, Clone, Copy)]
pub struct Win32CmdBackend;

impl Win32CmdBackend {
    pub fn new() -> Self {
        Self
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct OwnedScHandle(SC_HANDLE);

impl Drop for OwnedScHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseServiceHandle(self.0);
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

struct ComApartmentGuard {
    should_uninit: bool,
}

impl ComApartmentGuard {
    fn init(multithreaded: bool) -> WinctlResult<Self> {
        let model = if multithreaded {
            COINIT_MULTITHREADED
        } else {
            COINIT_APARTMENTTHREADED
        };
        let hr = unsafe { CoInitializeEx(None, model) };
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

struct OwnedVariant(VARIANT);

impl OwnedVariant {
    fn empty() -> Self {
        Self(VARIANT::default())
    }

    fn from_i32(value: i32) -> Self {
        let mut v = VARIANT::default();
        unsafe {
            (*v.Anonymous.Anonymous).vt = VT_I4;
            (*v.Anonymous.Anonymous).Anonymous.lVal = value;
        }
        Self(v)
    }

    fn as_ref(&self) -> &VARIANT {
        &self.0
    }
}

impl Drop for OwnedVariant {
    fn drop(&mut self) {
        unsafe {
            let _ = VariantClear(&mut self.0);
        }
    }
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> WinctlResult<Self> {
        unsafe { OpenClipboard(None) }.map_err(|e| map_win32_err(e, "OpenClipboard"))?;
        Ok(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

struct ProcAttrListGuard {
    _storage: Vec<u64>,
    attr_list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl ProcAttrListGuard {
    fn new(attribute_count: u32) -> WinctlResult<Self> {
        let mut size = 0usize;
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, attribute_count, None, &mut size);
        }
        if size == 0 {
            return Err(WinctlError::Win32 {
                code: 0,
                context: "InitializeProcThreadAttributeList returned zero size".to_string(),
            });
        }
        let words = size.div_ceil(size_of::<u64>());
        let mut storage = vec![0u64; words];
        let attr_list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast::<c_void>());
        unsafe { InitializeProcThreadAttributeList(Some(attr_list), attribute_count, None, &mut size) }
            .map_err(|e| map_win32_err(e, "InitializeProcThreadAttributeList"))?;
        Ok(Self {
            _storage: storage,
            attr_list,
        })
    }

    fn raw(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.attr_list
    }
}

impl Drop for ProcAttrListGuard {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.attr_list);
        }
    }
}

fn map_win32_err(err: windows::core::Error, context: &str) -> WinctlError {
    let code = err.code().0 as u32;
    let win32_code = code & 0xFFFF;
    if win32_code == ERROR_ACCESS_DENIED.0 || win32_code == ERROR_PRIVILEGE_NOT_HELD.0 {
        return WinctlError::PermissionDenied(format!("{context}: {} (code {win32_code})", err.message()));
    }
    if win32_code == ERROR_INVALID_PARAMETER.0 {
        return WinctlError::InvalidInput(format!("{context}: {} (code {win32_code})", err.message()));
    }
    WinctlError::win32(code, format!("{context}: {}", err.message()))
}

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn sw_from_window_state(state: WindowState) -> SHOW_WINDOW_CMD {
    match state {
        WindowState::Normal => SW_SHOWNORMAL,
        WindowState::Hidden => SW_HIDE,
        WindowState::Minimized => SW_MINIMIZE,
        WindowState::Maximized => SW_SHOWMAXIMIZED,
    }
}

fn priority_flag(priority: PriorityClass) -> PROCESS_CREATION_FLAGS {
    match priority {
        PriorityClass::Idle => IDLE_PRIORITY_CLASS,
        PriorityClass::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
        PriorityClass::Normal => NORMAL_PRIORITY_CLASS,
        PriorityClass::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
        PriorityClass::High => HIGH_PRIORITY_CLASS,
        PriorityClass::Realtime => REALTIME_PRIORITY_CLASS,
    }
}

impl CmdBackend for Win32CmdBackend {
    fn execute_script(&self, cmd: &ScriptCommand) -> WinctlResult<ScriptStepResult> {
        match cmd {
            ScriptCommand::Speak { text, async_mode } => {
                let _com = ComApartmentGuard::init(false)?;
                let voice: ISpVoice = unsafe { CoCreateInstance(&SpVoice, None, CLSCTX_ALL) }
                    .map_err(|e| map_win32_err(e, "CoCreateInstance(SpVoice)"))?;
                let flags = if *async_mode {
                    SPF_ASYNC.0 as u32
                } else {
                    SPF_DEFAULT.0 as u32
                };
                let wide = HSTRING::from(text.as_str());
                unsafe { voice.Speak(&wide, flags, None) }.map_err(|e| map_win32_err(e, "ISpVoice::Speak"))?;
                Ok(ScriptStepResult {
                    verb: "speak".to_string(),
                    applied: true,
                    detail: format!("Spoke {} chars", text.len()),
                })
            }
            ScriptCommand::Clipboard { op, text } => {
                let _clip = ClipboardGuard::open()?;
                unsafe { EmptyClipboard() }.map_err(|e| map_win32_err(e, "EmptyClipboard"))?;
                if matches!(op, ClipboardOp::Set) {
                    let payload = text.as_deref().unwrap_or("");
                    let wide = to_wide_null(payload);
                    let byte_len = wide.len() * size_of::<u16>();
                    let hglobal: HGLOBAL =
                        unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_len) }.map_err(|e| map_win32_err(e, "GlobalAlloc"))?;
                    let ptr = unsafe { GlobalLock(hglobal) };
                    if ptr.is_null() {
                        unsafe {
                            let _ = GlobalFree(Some(hglobal));
                        }
                        return Err(WinctlError::win32(0, "GlobalLock returned null"));
                    }
                    unsafe {
                        copy_nonoverlapping(wide.as_ptr(), ptr.cast::<u16>(), wide.len());
                        let _ = GlobalUnlock(hglobal);
                    }
                    if let Err(e) = unsafe { SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(hglobal.0))) } {
                        unsafe {
                            let _ = GlobalFree(Some(hglobal));
                        }
                        return Err(map_win32_err(e, "SetClipboardData"));
                    }
                }
                Ok(ScriptStepResult {
                    verb: "clipboard".to_string(),
                    applied: true,
                    detail: format!("Clipboard {op:?} completed"),
                })
            }
            ScriptCommand::MuteSysVolume { mode } => {
                let _com = ComApartmentGuard::init(true)?;
                let enumerator: IMMDeviceEnumerator =
                    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                        .map_err(|e| map_win32_err(e, "CoCreateInstance(MMDeviceEnumerator)"))?;
                let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
                    .map_err(|e| map_win32_err(e, "GetDefaultAudioEndpoint"))?;
                let endpoint: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "IMMDevice::Activate(IAudioEndpointVolume)"))?;
                let target_mute = match mode {
                    MuteMode::Unmute => false,
                    MuteMode::Mute => true,
                    MuteMode::Toggle => !unsafe { endpoint.GetMute() }
                        .map_err(|e| map_win32_err(e, "IAudioEndpointVolume::GetMute"))?
                        .as_bool(),
                };
                unsafe { endpoint.SetMute(target_mute, std::ptr::null()) }
                    .map_err(|e| map_win32_err(e, "IAudioEndpointVolume::SetMute"))?;
                Ok(ScriptStepResult {
                    verb: "mutesysvolume".to_string(),
                    applied: true,
                    detail: format!("Set mute={target_mute}"),
                })
            }
            ScriptCommand::SetVolume { device, left, right } => {
                if *device != 0 {
                    return Err(WinctlError::InvalidInput(format!(
                        "setvolume device index {device} is not supported in Win32CmdBackend (only default device 0 is supported)"
                    )));
                }
                let _com = ComApartmentGuard::init(true)?;
                let enumerator: IMMDeviceEnumerator =
                    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                        .map_err(|e| map_win32_err(e, "CoCreateInstance(MMDeviceEnumerator)"))?;
                let endpoint_device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
                    .map_err(|e| map_win32_err(e, "GetDefaultAudioEndpoint"))?;
                let endpoint: IAudioEndpointVolume = unsafe { endpoint_device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| map_win32_err(e, "IMMDevice::Activate(IAudioEndpointVolume)"))?;
                let avg = (f32::from(*left) + f32::from(*right)) / (2.0 * 65535.0);
                let scalar = avg.clamp(0.0, 1.0);
                unsafe { endpoint.SetMasterVolumeLevelScalar(scalar, std::ptr::null()) }
                    .map_err(|e| map_win32_err(e, "SetMasterVolumeLevelScalar"))?;
                Ok(ScriptStepResult {
                    verb: "setvolume".to_string(),
                    applied: true,
                    detail: format!("Set master volume scalar={scalar:.3}"),
                })
            }
            ScriptCommand::Monitor { state } => {
                unsafe {
                    PostMessageW(
                        Some(HWND_BROADCAST),
                        WM_SYSCOMMAND,
                        WPARAM(SC_MONITORPOWER as usize),
                        LPARAM(state.lparam_code()),
                    )
                }
                .map_err(|e| map_win32_err(e, "PostMessageW(SC_MONITORPOWER)"))?;
                Ok(ScriptStepResult {
                    verb: "monitor".to_string(),
                    applied: true,
                    detail: format!("Broadcast monitor state {state:?}"),
                })
            }
            ScriptCommand::ScreenSaver => {
                const SC_SCREENSAVE_CMD: usize = 0xF140;
                unsafe {
                    PostMessageW(
                        Some(HWND_BROADCAST),
                        WM_SYSCOMMAND,
                        WPARAM(SC_SCREENSAVE_CMD),
                        LPARAM(0),
                    )
                }
                .map_err(|e| map_win32_err(e, "PostMessageW(SC_SCREENSAVE)"))?;
                Ok(ScriptStepResult {
                    verb: "screensaver".to_string(),
                    applied: true,
                    detail: "Broadcast SC_SCREENSAVE".to_string(),
                })
            }
            ScriptCommand::Shortcut {
                target,
                folder,
                title,
                args,
                working_dir,
                icon_file,
                icon_idx,
                show_cmd,
                hotkey,
                description,
            } => {
                let _com = ComApartmentGuard::init(false)?;
                let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
                    .map_err(|e| map_win32_err(e, "CoCreateInstance(ShellLink)"))?;
                unsafe {
                    link.SetPath(&HSTRING::from(target.as_str()))
                        .map_err(|e| map_win32_err(e, "IShellLinkW::SetPath"))?;
                    if let Some(a) = args {
                        link.SetArguments(&HSTRING::from(a.as_str()))
                            .map_err(|e| map_win32_err(e, "IShellLinkW::SetArguments"))?;
                    }
                    if let Some(wd) = working_dir {
                        link.SetWorkingDirectory(&HSTRING::from(wd.as_str()))
                            .map_err(|e| map_win32_err(e, "IShellLinkW::SetWorkingDirectory"))?;
                    }
                    if let Some(desc) = description {
                        link.SetDescription(&HSTRING::from(desc.as_str()))
                            .map_err(|e| map_win32_err(e, "IShellLinkW::SetDescription"))?;
                    }
                    if let Some(icon) = icon_file {
                        link.SetIconLocation(&HSTRING::from(icon.as_str()), icon_idx.unwrap_or(0))
                            .map_err(|e| map_win32_err(e, "IShellLinkW::SetIconLocation"))?;
                    }
                    link.SetShowCmd(sw_from_window_state(*show_cmd))
                        .map_err(|e| map_win32_err(e, "IShellLinkW::SetShowCmd"))?;
                    if let Some(hk) = hotkey {
                        link.SetHotkey(*hk)
                            .map_err(|e| map_win32_err(e, "IShellLinkW::SetHotkey"))?;
                    }
                }
                let mut lnk_path = PathBuf::from(folder);
                lnk_path.push(format!("{title}.lnk"));
                let persist: IPersistFile = link.cast().map_err(|e| map_win32_err(e, "IPersistFile cast"))?;
                let path_h = HSTRING::from(lnk_path.to_string_lossy().as_ref());
                unsafe { persist.Save(&path_h, true) }.map_err(|e| map_win32_err(e, "IPersistFile::Save"))?;
                Ok(ScriptStepResult {
                    verb: "shortcut".to_string(),
                    applied: true,
                    detail: format!("Saved shortcut {}", lnk_path.display()),
                })
            }
            ScriptCommand::IniFile {
                op,
                file,
                section,
                key,
                value,
            } => {
                apply_ini_operation(*op, file, section, key.as_deref(), value.as_deref())?;
                Ok(ScriptStepResult {
                    verb: "inifile".to_string(),
                    applied: true,
                    detail: format!("Updated INI {}", file.display()),
                })
            }
            ScriptCommand::RegSetVal {
                reg_type,
                key,
                value_name,
                data,
            } => {
                let (root, subkey) = split_registry_key(key)?;
                let subkey_h = HSTRING::from(subkey);
                let mut hkey = HKEY::default();
                let status = unsafe {
                    RegCreateKeyExW(
                        root,
                        &subkey_h,
                        None,
                        None,
                        REG_OPTION_NON_VOLATILE,
                        KEY_WRITE,
                        None,
                        &mut hkey,
                        None,
                    )
                };
                if status != ERROR_SUCCESS {
                    return Err(map_win32_status(status.0, "RegCreateKeyExW"));
                }
                let _guard = OwnedHKey(hkey);
                let (win_type, bytes) = encode_registry_bytes(*reg_type, data)?;
                let val_h = HSTRING::from(value_name.as_str());
                let set_status = unsafe { RegSetValueExW(hkey, &val_h, None, win_type, Some(&bytes)) };
                if set_status != ERROR_SUCCESS {
                    return Err(map_win32_status(set_status.0, "RegSetValueExW"));
                }
                Ok(ScriptStepResult {
                    verb: "regsetval".to_string(),
                    applied: true,
                    detail: format!("Wrote {key}\\{value_name}"),
                })
            }
            ScriptCommand::Service { action, name } => {
                let scm = unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) }
                    .map_err(|e| map_win32_err(e, "OpenSCManagerW"))?;
                let _scm_guard = OwnedScHandle(scm);
                let desired = match action {
                    ServiceAction::Start => SERVICE_START,
                    ServiceAction::Stop => SERVICE_STOP,
                    ServiceAction::Pause | ServiceAction::Continue => SERVICE_PAUSE_CONTINUE,
                    ServiceAction::Restart => SERVICE_START | SERVICE_STOP,
                };
                let svc = unsafe { OpenServiceW(scm, &HSTRING::from(name.as_str()), desired) }
                    .map_err(|e| map_win32_err(e, "OpenServiceW"))?;
                let _svc_guard = OwnedScHandle(svc);
                let mut status = SERVICE_STATUS::default();
                match action {
                    ServiceAction::Start => {
                        unsafe { StartServiceW(svc, None) }.map_err(|e| map_win32_err(e, "StartServiceW"))?
                    }
                    ServiceAction::Stop => unsafe { ControlService(svc, SERVICE_CONTROL_STOP, &mut status) }
                        .map_err(|e| map_win32_err(e, "ControlService(STOP)"))?,
                    ServiceAction::Pause => unsafe { ControlService(svc, SERVICE_CONTROL_PAUSE, &mut status) }
                        .map_err(|e| map_win32_err(e, "ControlService(PAUSE)"))?,
                    ServiceAction::Continue => unsafe { ControlService(svc, SERVICE_CONTROL_CONTINUE, &mut status) }
                        .map_err(|e| map_win32_err(e, "ControlService(CONTINUE)"))?,
                    ServiceAction::Restart => {
                        let _ = unsafe { ControlService(svc, SERVICE_CONTROL_STOP, &mut status) };
                        thread::sleep(Duration::from_millis(250));
                        unsafe { StartServiceW(svc, None) }.map_err(|e| map_win32_err(e, "StartServiceW"))?;
                    }
                }
                Ok(ScriptStepResult {
                    verb: "service".to_string(),
                    applied: true,
                    detail: format!("Service '{name}' {action:?} dispatched"),
                })
            }
            ScriptCommand::Win {
                action,
                find_by,
                target,
                extra,
            } => {
                let hwnds = match find_by {
                    WinSelector::Title => {
                        let hwnd = unsafe { FindWindowW(None, &HSTRING::from(target.as_str())) }
                            .map_err(|e| map_win32_err(e, "FindWindowW(title)"))?;
                        vec![hwnd]
                    }
                    WinSelector::Class => {
                        let hwnd = unsafe { FindWindowW(&HSTRING::from(target.as_str()), None) }
                            .map_err(|e| map_win32_err(e, "FindWindowW(class)"))?;
                        vec![hwnd]
                    }
                    WinSelector::Handle => {
                        let parsed = parse_usize_handle(target)?;
                        if parsed == 0 {
                            return Err(WinctlError::InvalidInput(
                                "HWND(0) is not a valid window handle".to_string(),
                            ));
                        }
                        vec![HWND(parsed as *mut c_void)]
                    }
                    WinSelector::Active => {
                        let fg = unsafe { GetForegroundWindow() };
                        if fg.0.is_null() {
                            return Err(WinctlError::InvalidInput(
                                "no active foreground window found".to_string(),
                            ));
                        }
                        vec![fg]
                    }
                    WinSelector::AllTop => {
                        let list = enumerate_top_level_windows(true)?;
                        if list.is_empty() {
                            return Err(WinctlError::InvalidInput(
                                "no visible top-level windows found".to_string(),
                            ));
                        }
                        list
                    }
                    WinSelector::Process => {
                        let matching_pids = if let Ok(pid) = target.trim().parse::<u32>() {
                            let mut set = HashSet::new();
                            set.insert(pid);
                            set
                        } else {
                            let target_lower = target.trim().to_ascii_lowercase();
                            let procs = self.list_candidate_processes()?;
                            procs
                                .into_iter()
                                .filter(|p| {
                                    let name_lower = p.process_name.to_ascii_lowercase();
                                    let stem_lower = name_lower.strip_suffix(".exe").unwrap_or(&name_lower);
                                    name_lower == target_lower || stem_lower == target_lower
                                })
                                .map(|p| p.pid)
                                .collect()
                        };
                        if matching_pids.is_empty() {
                            return Err(WinctlError::InvalidInput(format!(
                                "no running process matched '{target}'"
                            )));
                        }
                        let mut matched = Vec::new();
                        for hwnd in enumerate_top_level_windows(false)? {
                            let mut pid = 0u32;
                            unsafe {
                                let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
                            }
                            if matching_pids.contains(&pid) {
                                matched.push(hwnd);
                            }
                        }
                        if matched.is_empty() {
                            return Err(WinctlError::InvalidInput(format!(
                                "no top-level windows found for process '{target}'"
                            )));
                        }
                        matched
                    }
                };
                let settext_payload = if matches!(find_by, WinSelector::Active | WinSelector::AllTop)
                    && extra.is_none()
                    && !target.is_empty()
                {
                    target.as_str()
                } else {
                    extra.as_deref().unwrap_or("")
                };
                let txt = (*action == WinAction::SetText).then(|| to_wide_null(settext_payload));
                for hwnd in &hwnds {
                    match action {
                        WinAction::Show => unsafe {
                            let _ = ShowWindow(*hwnd, SW_SHOW);
                        },
                        WinAction::Hide => unsafe {
                            let _ = ShowWindow(*hwnd, SW_HIDE);
                        },
                        WinAction::Min => unsafe {
                            let _ = ShowWindow(*hwnd, SW_MINIMIZE);
                        },
                        WinAction::Max => unsafe {
                            let _ = ShowWindow(*hwnd, SW_SHOWMAXIMIZED);
                        },
                        WinAction::Normal | WinAction::Activate => unsafe {
                            let _ = ShowWindow(*hwnd, SW_RESTORE);
                        },
                        WinAction::Close => unsafe {
                            PostMessageW(Some(*hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))
                                .map_err(|e| map_win32_err(e, "PostMessageW(WM_CLOSE)"))?;
                        },
                        WinAction::SetText => {
                            if let Some(ref wide) = txt {
                                unsafe {
                                    let _ = SendMessageW(
                                        *hwnd,
                                        WM_SETTEXT,
                                        Some(WPARAM(0)),
                                        Some(LPARAM(wide.as_ptr() as isize)),
                                    );
                                }
                            }
                        }
                    }
                }
                Ok(ScriptStepResult {
                    verb: "win".to_string(),
                    applied: true,
                    detail: format!(
                        "Window action {action:?} applied to {} window(s) for '{target}'",
                        hwnds.len()
                    ),
                })
            }
            ScriptCommand::Beep { freq_hz, duration_ms } => {
                unsafe { Beep(*freq_hz, *duration_ms) }.map_err(|e| map_win32_err(e, "Beep"))?;
                Ok(ScriptStepResult {
                    verb: "beep".to_string(),
                    applied: true,
                    detail: format!("Beeped {freq_hz}Hz for {duration_ms}ms"),
                })
            }
            ScriptCommand::Wait { ms } => {
                thread::sleep(Duration::from_millis(*ms));
                Ok(ScriptStepResult {
                    verb: "wait".to_string(),
                    applied: true,
                    detail: format!("Waited {ms}ms"),
                })
            }
            ScriptCommand::Exec { show_mode, exe, args } => {
                let profile = LaunchProfile {
                    exe_path: exe.clone(),
                    args: args.clone(),
                    window_state: *show_mode,
                    ..LaunchProfile::default()
                };
                let res = self.launch_process(&profile)?;
                Ok(ScriptStepResult {
                    verb: "exec".to_string(),
                    applied: true,
                    detail: format!("Launched PID {:?}", res.pid),
                })
            }
            ScriptCommand::ShellExec {
                verb,
                target,
                params,
                dir,
            } => {
                let verb_h = HSTRING::from(verb.as_str());
                let target_h = HSTRING::from(target.as_str());
                let params_h = params.as_deref().map(HSTRING::from);
                let dir_h = dir.as_deref().map(HSTRING::from);
                let hinst = unsafe {
                    ShellExecuteW(
                        None,
                        &verb_h,
                        &target_h,
                        params_h.as_ref().map(|h| PCWSTR(h.as_ptr())).unwrap_or(PCWSTR::null()),
                        dir_h.as_ref().map(|h| PCWSTR(h.as_ptr())).unwrap_or(PCWSTR::null()),
                        SW_SHOWNORMAL,
                    )
                };
                let code = hinst.0 as usize;
                if code <= 32 {
                    return Err(WinctlError::win32(code as u32, "ShellExecuteW"));
                }
                Ok(ScriptStepResult {
                    verb: "shellexec".to_string(),
                    applied: true,
                    detail: format!("ShellExecuteW('{verb}', '{target}') succeeded"),
                })
            }
            ScriptCommand::Power { action } => {
                match action {
                    PowerAction::LockWorkstation => {
                        unsafe { LockWorkStation() }.map_err(|e| map_win32_err(e, "LockWorkStation"))?
                    }
                    PowerAction::Logoff { force } => {
                        let mut flags = EWX_LOGOFF;
                        if *force {
                            flags |= EWX_FORCE | EWX_FORCEIFHUNG;
                        }
                        unsafe { ExitWindowsEx(flags, SHTDN_REASON_MAJOR_OTHER) }
                            .map_err(|e| map_win32_err(e, "ExitWindowsEx(LOGOFF)"))?;
                    }
                    PowerAction::Reboot { force } => {
                        let mut flags = EWX_REBOOT;
                        if *force {
                            flags |= EWX_FORCE | EWX_FORCEIFHUNG;
                        }
                        unsafe { ExitWindowsEx(flags, SHTDN_REASON_MAJOR_OTHER) }
                            .map_err(|e| map_win32_err(e, "ExitWindowsEx(REBOOT)"))?;
                    }
                    PowerAction::Shutdown { force } => {
                        let mut flags = EWX_SHUTDOWN | EWX_POWEROFF;
                        if *force {
                            flags |= EWX_FORCE | EWX_FORCEIFHUNG;
                        }
                        unsafe { ExitWindowsEx(flags, SHTDN_REASON_MAJOR_OTHER) }
                            .map_err(|e| map_win32_err(e, "ExitWindowsEx(SHUTDOWN)"))?;
                    }
                }
                Ok(ScriptStepResult {
                    verb: "power".to_string(),
                    applied: true,
                    detail: format!("Executed {action:?}"),
                })
            }
        }
    }

    fn launch_process(&self, profile: &LaunchProfile) -> WinctlResult<LaunchResult> {
        profile.validate()?;
        let cmd_line = profile.build_command_line();

        if profile.run_as_mode == RunAsMode::Administrator {
            let verb_h = HSTRING::from("runas");
            let file_h = HSTRING::from(profile.exe_path.as_str());
            let joined_args = profile
                .args
                .iter()
                .map(|a| crate::run::quote_windows_arg(a))
                .collect::<Vec<_>>()
                .join(" ");
            let params_h = (!joined_args.is_empty()).then(|| HSTRING::from(joined_args));
            let dir_h = profile.working_dir.as_deref().map(HSTRING::from);

            let mut sei = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS,
                lpVerb: PCWSTR(verb_h.as_ptr()),
                lpFile: PCWSTR(file_h.as_ptr()),
                lpParameters: params_h.as_ref().map(|h| PCWSTR(h.as_ptr())).unwrap_or(PCWSTR::null()),
                lpDirectory: dir_h.as_ref().map(|h| PCWSTR(h.as_ptr())).unwrap_or(PCWSTR::null()),
                nShow: sw_from_window_state(profile.window_state).0,
                ..Default::default()
            };
            unsafe { ShellExecuteExW(&mut sei) }.map_err(|e| map_win32_err(e, "ShellExecuteExW(runas)"))?;
            let _proc_guard = OwnedHandle(sei.hProcess);
            let pid = if !sei.hProcess.is_invalid() {
                unsafe {
                    let _ = SetPriorityClass(sei.hProcess, priority_flag(profile.priority_class));
                    if let Some(mask) = profile.affinity_mask {
                        let _ = SetProcessAffinityMask(sei.hProcess, mask);
                    }
                    let raw_pid = GetProcessId(sei.hProcess);
                    (raw_pid != 0).then_some(raw_pid)
                }
            } else {
                None
            };
            let exit_code = if profile.wait_for_exit && !sei.hProcess.is_invalid() {
                wait_and_read_exit_code(sei.hProcess)
            } else {
                None
            };
            return Ok(LaunchResult {
                pid,
                command_line: cmd_line,
                run_as: "Administrator".to_string(),
                exit_code,
            });
        }

        let mut cmd_wide = to_wide_null(&cmd_line);
        let dir_h = profile.working_dir.as_deref().map(HSTRING::from);
        let env_block = profile.build_utf16_env_block();
        let mut flags = CREATE_UNICODE_ENVIRONMENT | priority_flag(profile.priority_class);
        if profile.new_console {
            flags |= CREATE_NEW_CONSOLE;
        }
        if profile.no_window {
            flags |= CREATE_NO_WINDOW;
        }
        if profile.detached {
            flags |= DETACHED_PROCESS;
        }

        let si = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            dwFlags: STARTF_USESHOWWINDOW,
            wShowWindow: sw_from_window_state(profile.window_state).0 as u16,
            ..Default::default()
        };
        let mut pi = PROCESS_INFORMATION::default();
        unsafe {
            CreateProcessW(
                None,
                Some(PWSTR(cmd_wide.as_mut_ptr())),
                None,
                None,
                false,
                flags,
                Some(env_block.as_ptr().cast::<c_void>()),
                dir_h.as_ref().map(|h| PCWSTR(h.as_ptr())).unwrap_or(PCWSTR::null()),
                &si,
                &mut pi,
            )
        }
        .map_err(|e| map_win32_err(e, "CreateProcessW"))?;

        let _proc_guard = OwnedHandle(pi.hProcess);
        let _thread_guard = OwnedHandle(pi.hThread);

        if let Some(mask) = profile.affinity_mask {
            unsafe { SetProcessAffinityMask(pi.hProcess, mask) }
                .map_err(|e| map_win32_err(e, "SetProcessAffinityMask"))?;
        }
        let exit_code = if profile.wait_for_exit {
            wait_and_read_exit_code(pi.hProcess)
        } else {
            None
        };

        Ok(LaunchResult {
            pid: Some(pi.dwProcessId),
            command_line: cmd_line,
            run_as: "CurrentUser".to_string(),
            exit_code,
        })
    }

    fn list_candidate_processes(&self) -> WinctlResult<Vec<ProcessSummary>> {
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| map_win32_err(e, "CreateToolhelp32Snapshot"))?;
        let _guard = OwnedHandle(snap);

        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut out = Vec::new();

        if unsafe { Process32FirstW(snap, &mut entry) }.is_ok() {
            loop {
                let nul = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..nul]);
                let hint = if entry.th32ProcessID <= 4 || name.eq_ignore_ascii_case("services.exe") {
                    "SYSTEM/Protected (requires Elevation/SeDebugPrivilege)".to_string()
                } else {
                    "Standard/Desktop".to_string()
                };
                out.push(ProcessSummary {
                    pid: entry.th32ProcessID,
                    ppid: entry.th32ParentProcessID,
                    process_name: name,
                    threads: entry.cntThreads,
                    elevation_hint: hint,
                });
                if unsafe { Process32NextW(snap, &mut entry) }.is_err() {
                    break;
                }
            }
        }

        Ok(out)
    }

    fn spawn_from_parent(&self, req: &FromProcessRequest) -> WinctlResult<FromProcessResult> {
        req.validate()?;
        let parent_handle = unsafe {
            OpenProcess(
                PROCESS_ACCESS_RIGHTS(PROCESS_CREATE_PROCESS_ACCESS),
                false,
                req.parent_pid,
            )
        }
        .map_err(|e| {
            map_win32_err(
                e,
                &format!("OpenProcess(PROCESS_CREATE_PROCESS, pid={})", req.parent_pid),
            )
        })?;
        let _parent_guard = OwnedHandle(parent_handle);

        let attr_guard = ProcAttrListGuard::new(1)?;
        let raw_parent: HANDLE = parent_handle;
        unsafe {
            UpdateProcThreadAttribute(
                attr_guard.raw(),
                0,
                PROC_THREAD_ATTRIBUTE_PARENT_PROCESS_VALUE,
                Some((&raw_parent as *const HANDLE).cast::<c_void>()),
                size_of::<HANDLE>(),
                None,
                None,
            )
        }
        .map_err(|e| map_win32_err(e, "UpdateProcThreadAttribute(PROC_THREAD_ATTRIBUTE_PARENT_PROCESS)"))?;

        let cmd_line = req.build_command_line();
        let mut cmd_wide = to_wide_null(&cmd_line);
        let dir_h = req.working_dir.as_deref().map(HSTRING::from);

        let mut flags = EXTENDED_STARTUPINFO_PRESENT;
        if req.new_console {
            flags |= CREATE_NEW_CONSOLE;
        }

        let mut si_ex: STARTUPINFOEXW = unsafe { zeroed() };
        si_ex.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        si_ex.lpAttributeList = attr_guard.raw();

        let mut pi = PROCESS_INFORMATION::default();
        unsafe {
            CreateProcessW(
                None,
                Some(PWSTR(cmd_wide.as_mut_ptr())),
                None,
                None,
                req.inherit_handles,
                flags,
                None,
                dir_h.as_ref().map(|h| PCWSTR(h.as_ptr())).unwrap_or(PCWSTR::null()),
                &si_ex.StartupInfo,
                &mut pi,
            )
        }
        .map_err(|e| map_win32_err(e, "CreateProcessW(EXTENDED_STARTUPINFO_PRESENT)"))?;

        let _child_proc = OwnedHandle(pi.hProcess);
        let _child_thread = OwnedHandle(pi.hThread);

        Ok(FromProcessResult {
            parent_pid: req.parent_pid,
            child_pid: pi.dwProcessId,
            child_tid: pi.dwThreadId,
            command_line: cmd_line,
        })
    }

    fn list_scheduled_tasks(&self, folder_path: &str) -> WinctlResult<Vec<AtNowTaskInfo>> {
        let _com = ComApartmentGuard::init(true)?;
        let service: ITaskService = unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| map_win32_err(e, "CoCreateInstance(TaskScheduler)"))?;
        let empty = OwnedVariant::empty();
        unsafe { service.Connect(empty.as_ref(), empty.as_ref(), empty.as_ref(), empty.as_ref()) }
            .map_err(|e| map_win32_err(e, "ITaskService::Connect"))?;

        let folder = unsafe { service.GetFolder(&BSTR::from(folder_path)) }
            .map_err(|e| map_win32_err(e, "ITaskService::GetFolder"))?;
        let collection =
            unsafe { folder.GetTasks(TASK_ENUM_HIDDEN.0) }.map_err(|e| map_win32_err(e, "ITaskFolder::GetTasks"))?;
        let count = unsafe { collection.Count() }.map_err(|e| map_win32_err(e, "IRegisteredTaskCollection::Count"))?;

        let mut tasks = Vec::new();
        for idx in 1..=count {
            let v_idx = OwnedVariant::from_i32(idx);
            let task: IRegisteredTask = unsafe { collection.get_Item(v_idx.as_ref()) }
                .map_err(|e| map_win32_err(e, "IRegisteredTaskCollection::get_Item"))?;
            let name = unsafe { task.Name() }.map(|b| b.to_string()).unwrap_or_default();
            let path = unsafe { task.Path() }.map(|b| b.to_string()).unwrap_or_default();
            let state_raw = unsafe { task.State() }.unwrap_or(TASK_STATE_READY);
            let state = match state_raw {
                x if x == TASK_STATE_DISABLED => "Disabled",
                x if x == TASK_STATE_QUEUED => "Queued",
                x if x == TASK_STATE_READY => "Ready",
                x if x == TASK_STATE_RUNNING => "Running",
                _ => "Unknown",
            }
            .to_string();
            let enabled = unsafe { task.Enabled() }.map(|b| b != VARIANT_FALSE).unwrap_or(false);
            let last_result = unsafe { task.LastTaskResult() }.unwrap_or(0);
            let command = extract_task_command(&task).unwrap_or_default();
            tasks.push(AtNowTaskInfo {
                task_name: name,
                folder: path,
                state,
                enabled,
                next_run: None,
                last_run: None,
                last_result,
                command,
            });
        }
        Ok(tasks)
    }

    fn schedule_task(&self, req: &AtNowTaskRequest) -> WinctlResult<AtNowScheduleResult> {
        req.validate()?;
        let _com = ComApartmentGuard::init(true)?;
        let service: ITaskService = unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| map_win32_err(e, "CoCreateInstance(TaskScheduler)"))?;
        let empty = OwnedVariant::empty();
        unsafe { service.Connect(empty.as_ref(), empty.as_ref(), empty.as_ref(), empty.as_ref()) }
            .map_err(|e| map_win32_err(e, "ITaskService::Connect"))?;

        let folder = unsafe { service.GetFolder(&BSTR::from(req.folder_path.as_str())) }
            .map_err(|e| map_win32_err(e, "ITaskService::GetFolder"))?;
        let def: ITaskDefinition =
            unsafe { service.NewTask(0) }.map_err(|e| map_win32_err(e, "ITaskService::NewTask"))?;

        let principal: IPrincipal =
            unsafe { def.Principal() }.map_err(|e| map_win32_err(e, "ITaskDefinition::Principal"))?;
        let logon_type = if req.run_as_system {
            unsafe { principal.SetUserId(&BSTR::from("SYSTEM")) }
                .map_err(|e| map_win32_err(e, "IPrincipal::SetUserId(SYSTEM)"))?;
            TASK_LOGON_SERVICE_ACCOUNT
        } else if req.interactive {
            TASK_LOGON_INTERACTIVE_TOKEN
        } else {
            TASK_LOGON_S4U
        };
        unsafe {
            principal
                .SetLogonType(logon_type)
                .map_err(|e| map_win32_err(e, "IPrincipal::SetLogonType"))?;
            principal
                .SetRunLevel(if req.highest_privileges {
                    TASK_RUNLEVEL_HIGHEST
                } else {
                    TASK_RUNLEVEL_LUA
                })
                .map_err(|e| map_win32_err(e, "IPrincipal::SetRunLevel"))?;
        }

        let start_boundary = req.run_at.resolve_start_boundary(Local::now())?;
        let triggers = unsafe { def.Triggers() }.map_err(|e| map_win32_err(e, "ITaskDefinition::Triggers"))?;
        let trigger: ITrigger = unsafe { triggers.Create(TASK_TRIGGER_TIME) }
            .map_err(|e| map_win32_err(e, "ITriggerCollection::Create(TASK_TRIGGER_TIME)"))?;
        let time_trigger: ITimeTrigger = trigger
            .cast()
            .map_err(|e| map_win32_err(e, "ITrigger::cast<ITimeTrigger>"))?;
        unsafe {
            time_trigger
                .SetStartBoundary(&BSTR::from(start_boundary.as_str()))
                .map_err(|e| map_win32_err(e, "ITimeTrigger::SetStartBoundary"))?;
            time_trigger
                .SetEnabled(VARIANT_TRUE)
                .map_err(|e| map_win32_err(e, "ITimeTrigger::SetEnabled"))?;
        }

        let actions = unsafe { def.Actions() }.map_err(|e| map_win32_err(e, "ITaskDefinition::Actions"))?;
        let action: IAction = unsafe { actions.Create(TASK_ACTION_EXEC) }
            .map_err(|e| map_win32_err(e, "IActionCollection::Create(TASK_ACTION_EXEC)"))?;
        let exec: IExecAction = action
            .cast()
            .map_err(|e| map_win32_err(e, "IAction::cast<IExecAction>"))?;
        unsafe {
            exec.SetPath(&BSTR::from(req.command.as_str()))
                .map_err(|e| map_win32_err(e, "IExecAction::SetPath"))?;
            if let Some(ref a) = req.arguments {
                exec.SetArguments(&BSTR::from(a.as_str()))
                    .map_err(|e| map_win32_err(e, "IExecAction::SetArguments"))?;
            }
            if let Some(ref wd) = req.working_dir {
                exec.SetWorkingDirectory(&BSTR::from(wd.as_str()))
                    .map_err(|e| map_win32_err(e, "IExecAction::SetWorkingDirectory"))?;
            }
        }

        let registered = unsafe {
            folder.RegisterTaskDefinition(
                &BSTR::from(req.task_name.as_str()),
                &def,
                TASK_CREATE_OR_UPDATE.0,
                empty.as_ref(),
                empty.as_ref(),
                logon_type,
                empty.as_ref(),
            )
        }
        .map_err(|e| map_win32_err(e, "ITaskFolder::RegisterTaskDefinition"))?;

        let state = match unsafe { registered.State() } {
            Ok(s) if s == TASK_STATE_READY => "Ready".to_string(),
            Ok(s) if s == TASK_STATE_RUNNING => "Running".to_string(),
            Ok(s) if s == TASK_STATE_QUEUED => "Queued".to_string(),
            _ => "Registered".to_string(),
        };

        Ok(AtNowScheduleResult {
            task_name: req.task_name.clone(),
            folder: req.folder_path.clone(),
            start_boundary,
            state,
        })
    }
}

fn wait_and_read_exit_code(handle: HANDLE) -> Option<u32> {
    unsafe {
        let _ = WaitForSingleObject(handle, INFINITE);
        let mut code = 0u32;
        if GetExitCodeProcess(handle, &mut code).is_ok() {
            Some(code)
        } else {
            None
        }
    }
}

fn extract_task_command(task: &IRegisteredTask) -> Option<String> {
    let def = unsafe { task.Definition().ok()? };
    let actions = unsafe { def.Actions().ok()? };
    let first = unsafe { actions.get_Item(1).ok()? };
    let exec: IExecAction = first.cast().ok()?;
    let mut path_bstr = BSTR::new();
    unsafe { exec.Path(&mut path_bstr).ok()? };
    let mut args_bstr = BSTR::new();
    let _ = unsafe { exec.Arguments(&mut args_bstr) };
    let path_s = path_bstr.to_string();
    let args_s = args_bstr.to_string();
    if args_s.is_empty() {
        Some(path_s)
    } else {
        Some(format!("{path_s} {args_s}"))
    }
}

fn map_win32_status(code: u32, context: &str) -> WinctlError {
    if code == ERROR_ACCESS_DENIED.0 || code == ERROR_PRIVILEGE_NOT_HELD.0 {
        return WinctlError::PermissionDenied(format!("{context}: access denied (code {code})"));
    }
    WinctlError::win32(code, context)
}

fn split_registry_key(full: &str) -> WinctlResult<(HKEY, &str)> {
    let (hive, sub) = full
        .split_once('\\')
        .ok_or_else(|| WinctlError::InvalidInput(format!("registry path '{full}' missing hive prefix")))?;
    let root = match hive.to_ascii_uppercase().as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCR" | "HKEY_CLASSES_ROOT" => HKEY_CLASSES_ROOT,
        "HKU" | "HKEY_USERS" => HKEY_USERS,
        other => {
            return Err(WinctlError::InvalidInput(format!(
                "unsupported registry hive '{other}'"
            )));
        }
    };
    Ok((root, sub))
}

fn encode_registry_bytes(
    kind: RegValueKind,
    data: &str,
) -> WinctlResult<(windows::Win32::System::Registry::REG_VALUE_TYPE, Vec<u8>)> {
    match kind {
        RegValueKind::Sz => {
            let bytes = to_wide_null(data).into_iter().flat_map(u16::to_le_bytes).collect();
            Ok((REG_SZ, bytes))
        }
        RegValueKind::ExpandSz => {
            let bytes = to_wide_null(data).into_iter().flat_map(u16::to_le_bytes).collect();
            Ok((REG_EXPAND_SZ, bytes))
        }
        RegValueKind::Dword => {
            let val: u32 = if let Some(hex) = data.strip_prefix("0x").or_else(|| data.strip_prefix("0X")) {
                u32::from_str_radix(hex, 16)
            } else {
                data.parse()
            }
            .map_err(|_| WinctlError::ParseError(format!("invalid DWORD value '{data}'")))?;
            Ok((REG_DWORD, val.to_le_bytes().to_vec()))
        }
        RegValueKind::Qword => {
            let val: u64 = if let Some(hex) = data.strip_prefix("0x").or_else(|| data.strip_prefix("0X")) {
                u64::from_str_radix(hex, 16)
            } else {
                data.parse()
            }
            .map_err(|_| WinctlError::ParseError(format!("invalid QWORD value '{data}'")))?;
            Ok((REG_QWORD, val.to_le_bytes().to_vec()))
        }
        RegValueKind::Binary => {
            let trimmed = data.trim();
            let stripped = trimmed
                .strip_prefix("0x")
                .or_else(|| trimmed.strip_prefix("0X"))
                .unwrap_or(trimmed);
            let cleaned: String = stripped
                .chars()
                .filter(|c| !c.is_ascii_whitespace() && *c != ',' && *c != '-')
                .collect();
            if !cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(WinctlError::ParseError(format!(
                    "invalid binary hex character in '{data}'"
                )));
            }
            if !cleaned.len().is_multiple_of(2) {
                return Err(WinctlError::ParseError(
                    "binary hex string must have even length".to_string(),
                ));
            }
            let mut out = Vec::with_capacity(cleaned.len() / 2);
            for i in (0..cleaned.len()).step_by(2) {
                let b = u8::from_str_radix(&cleaned[i..i + 2], 16)
                    .map_err(|_| WinctlError::ParseError("invalid hex byte".to_string()))?;
                out.push(b);
            }
            Ok((REG_BINARY, out))
        }
        RegValueKind::MultiSz => {
            let mut units = Vec::new();
            for part in data.split('|') {
                units.extend(part.encode_utf16());
                units.push(0);
            }
            units.push(0);
            Ok((REG_MULTI_SZ, units.into_iter().flat_map(u16::to_le_bytes).collect()))
        }
    }
}

fn parse_usize_handle(raw: &str) -> WinctlResult<usize> {
    let trimmed = raw.trim();
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16)
    } else {
        trimmed.parse::<usize>()
    }
    .map_err(|_| WinctlError::InvalidInput(format!("invalid HWND value '{raw}'")))
}

struct EnumWindowsState {
    only_visible: bool,
    hwnds: Vec<HWND>,
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = unsafe { &mut *(lparam.0 as *mut EnumWindowsState) };
    if !state.only_visible || unsafe { IsWindowVisible(hwnd) }.as_bool() {
        state.hwnds.push(hwnd);
    }
    BOOL::from(true)
}

fn enumerate_top_level_windows(only_visible: bool) -> WinctlResult<Vec<HWND>> {
    let mut state = EnumWindowsState {
        only_visible,
        hwnds: Vec::new(),
    };
    let lparam = LPARAM((&mut state as *mut EnumWindowsState) as isize);
    unsafe { SetLastError(ERROR_SUCCESS) };
    unsafe { EnumWindows(Some(enum_windows_proc), lparam) }.map_err(|e| map_win32_err(e, "EnumWindows"))?;
    Ok(state.hwnds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_registry_encoding_strips_0x_prefix_and_rejects_bad_hex() {
        let (kind, bytes) = encode_registry_bytes(RegValueKind::Binary, "0xdeadbeef").unwrap();
        assert_eq!(kind, REG_BINARY);
        assert_eq!(bytes, vec![0xde, 0xad, 0xbe, 0xef]);

        let (_, single) = encode_registry_bytes(RegValueKind::Binary, "0X0a").unwrap();
        assert_eq!(single, vec![0x0a]);

        assert!(encode_registry_bytes(RegValueKind::Binary, "0x0").is_err());
        assert!(encode_registry_bytes(RegValueKind::Binary, "0xzz").is_err());
    }

    #[test]
    fn win32_backend_rejects_nonzero_setvolume_device_and_zero_hwnd() {
        let backend = Win32CmdBackend::new();
        assert!(matches!(
            backend.execute_script(&ScriptCommand::SetVolume {
                device: 1,
                left: 1000,
                right: 1000
            }),
            Err(WinctlError::InvalidInput(_))
        ));
        assert!(matches!(
            backend.execute_script(&ScriptCommand::Win {
                action: WinAction::Show,
                find_by: WinSelector::Handle,
                target: "0".to_string(),
                extra: None
            }),
            Err(WinctlError::InvalidInput(_))
        ));
    }
}
