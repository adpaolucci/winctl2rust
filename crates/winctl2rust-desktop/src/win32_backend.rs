use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{copy_nonoverlapping, null_mut};
use std::time::{Duration, Instant};

use winctl2rust_core::{WinctlError, WinctlResult};
use windows::Win32::Foundation::{
    COLORREF, E_ACCESSDENIED, E_INVALIDARG, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER,
    ERROR_NO_MORE_ITEMS, ERROR_NOT_FOUND, ERROR_PRIVILEGE_NOT_HELD, ERROR_SUCCESS, GlobalFree, HANDLE, HGLOBAL, HWND,
    LPARAM, LRESULT, MAX_PATH, RECT, RPC_E_CHANGED_MODE, SetLastError, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    DEFAULT_CHARSET, EnumFontFamiliesExW, GetDC, HDC, LOGFONTW, ReleaseDC, TEXTMETRICW,
};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, IMFMediaBuffer, IMFMediaSource, IMFMediaType, IMFSample, IMFSourceReader,
    MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
    MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
    MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFMediaType_Video,
    MFSTARTUP_NOSOCKET, MFShutdown, MFStartup, MFVideoFormat_NV12, MFVideoFormat_RGB24, MFVideoFormat_RGB32,
    MFVideoFormat_YUY2,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, OpenClipboard, RemoveClipboardFormatListener, SetClipboardData,
};
use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE,
    PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ, RegCloseKey, RegEnumValueW,
    RegOpenKeyExW,
};
use windows::Win32::System::Threading::{
    IsWow64Process, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_OPERATION,
    PROCESS_VM_READ, PROCESS_VM_WRITE, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Controls::{
    HDI_TEXT, HDITEMW, HDM_GETITEMCOUNT, HDM_GETITEMW, HTREEITEM, LVIF_TEXT, LVITEMW, LVM_GETHEADER, LVM_GETITEMCOUNT,
    LVM_GETITEMW, TVGN_CHILD, TVGN_NEXT, TVGN_ROOT, TVIF_TEXT, TVITEMW, TVM_GETITEMW, TVM_GETNEXTITEM,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetAsyncKeyState, GetKeyNameTextW, GetKeyboardState, HOT_KEY_MODIFIERS, IsWindowEnabled,
    MAPVK_VK_TO_VSC, MapVirtualKeyW, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CB_GETCOUNT, CB_GETLBTEXT, CB_GETLBTEXTLEN, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EnumChildWindows, EnumWindows, FindWindowExW, GA_PARENT, GWL_EXSTYLE, GWL_STYLE, GetAncestor, GetClassNameW,
    GetDesktopWindow, GetDlgCtrlID, GetLayeredWindowAttributes, GetWindowLongPtrW, GetWindowPlacement, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, HWND_MESSAGE, HWND_NOTOPMOST, HWND_TOPMOST,
    IsWindow, IsWindowVisible, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LB_GETCOUNT, LB_GETTEXT, LB_GETTEXTLEN, LWA_ALPHA, MSG,
    MoveWindow, PM_REMOVE, PeekMessageW, PostMessageW, RegisterClassExW, SMTO_ABORTIFHUNG, SW_HIDE, SW_MAXIMIZE,
    SW_MINIMIZE, SW_RESTORE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SendMessageTimeoutW, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, TranslateMessage, WINDOWPLACEMENT, WM_CLIPBOARDUPDATE, WM_CLOSE, WM_GETTEXT,
    WM_GETTEXTLENGTH, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOPMOST,
};
use windows::core::{BOOL, GUID, HSTRING, PWSTR, w};

use crate::backend::{
    ClipboardControlResult, ClipboardFormatEntry, CtrlExportItem, DesktopBackend, HotkeyEntry, HotkeyProbeResult,
    InstalledFontEntry, KeyStateEntry, RawVideoFrame, TopWindowInfo, TreeControlOp, TreeWindowNode, VideoPixelFormat,
    WebcamDeviceInfo, WindowControlResult, WindowListerOp, WindowPropInfo, WindowPropOp, format_hwnd,
};
use crate::clipboard::build_clipboard_entry;
use crate::ctrl_export::{
    HdItemW32, LvItemW32, TvItemW32, WPM_CTRL_EXPORT_MARKER, decode_utf16_nul_terminated,
    is_supported_ctrl_export_class,
};
use crate::fonts::font_entries_from_file;
use crate::hotkeys::{common_hotkey_probe_candidates, format_probe_hotkey, scan_lnk_hotkeys_in_dir};
use crate::keys::{is_extended_vk, vk_canonical_name};

const FONTS_REG_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";
const MAX_PROP_WINDOWS: usize = 4096;
const MAX_PROP_CHILDREN_PER_PARENT: usize = 256;
const MAX_TREE_NODES: usize = 4096;
const MAX_CTRL_EXPORT_ITEMS_PER_CONTROL: usize = 250;

#[derive(Debug, Default, Clone, Copy)]
pub struct Win32DesktopBackend;

impl Win32DesktopBackend {
    pub fn new() -> Self {
        Self
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

struct OwnedVirtualAlloc {
    process: HANDLE,
    addr: *mut c_void,
}

impl Drop for OwnedVirtualAlloc {
    fn drop(&mut self) {
        if !self.addr.is_null() {
            unsafe {
                let _ = VirtualFreeEx(self.process, self.addr, 0, MEM_RELEASE);
            }
        }
    }
}

struct OwnedClipboard {
    _probe_hwnd: Option<OwnedHwnd>,
}

impl OwnedClipboard {
    fn open(hwnd: Option<HWND>) -> WinctlResult<Self> {
        let mut probe_hwnd: Option<OwnedHwnd> = None;
        let mut last_err = None;
        for attempt in 0..6 {
            if attempt == 1 && hwnd.is_none() {
                probe_hwnd = create_ephemeral_probe_hwnd();
            }
            let target_hwnd = hwnd.or_else(|| probe_hwnd.as_ref().map(|h| h.0));
            match unsafe { OpenClipboard(target_hwnd) } {
                Ok(()) => {
                    return Ok(Self {
                        _probe_hwnd: probe_hwnd,
                    });
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 5 {
                        std::thread::sleep(Duration::from_millis(15));
                    }
                }
            }
        }
        Err(map_win32_err(
            last_err.expect("at least one OpenClipboard attempt"),
            "OpenClipboard",
        ))
    }
}

impl Drop for OwnedClipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

struct OwnedGlobalLock {
    hmem: HGLOBAL,
    ptr: *const c_void,
}

impl OwnedGlobalLock {
    fn lock(hmem: HGLOBAL) -> WinctlResult<Self> {
        let ptr = unsafe { GlobalLock(hmem) };
        if ptr.is_null() {
            return Err(last_win32_err("GlobalLock"));
        }
        Ok(Self { hmem, ptr })
    }
}

impl Drop for OwnedGlobalLock {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalUnlock(self.hmem);
        }
    }
}

struct OwnedGlobalAlloc(Option<HGLOBAL>);

impl OwnedGlobalAlloc {
    fn disarm(&mut self) -> Option<HGLOBAL> {
        self.0.take()
    }
}

impl Drop for OwnedGlobalAlloc {
    fn drop(&mut self) {
        if let Some(h) = self.0 {
            unsafe {
                let _ = GlobalFree(Some(h));
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

struct OwnedScreenDc(Option<HWND>, HDC);

impl Drop for OwnedScreenDc {
    fn drop(&mut self) {
        if !self.1.is_invalid() {
            unsafe {
                let _ = ReleaseDC(self.0, self.1);
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

struct OwnedClipboardListener(HWND);

impl Drop for OwnedClipboardListener {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            unsafe {
                let _ = RemoveClipboardFormatListener(self.0);
            }
        }
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

struct MediaFoundationGuard {
    started: bool,
}

impl MediaFoundationGuard {
    fn startup() -> WinctlResult<Self> {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) }.map_err(|e| map_win32_err(e, "MFStartup"))?;
        Ok(Self { started: true })
    }
}

impl Drop for MediaFoundationGuard {
    fn drop(&mut self) {
        if self.started {
            unsafe {
                let _ = MFShutdown();
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

struct OwnedActivateArray {
    ptr: *mut Option<IMFActivate>,
    count: u32,
}

impl OwnedActivateArray {
    fn slice(&self) -> &[Option<IMFActivate>] {
        if self.ptr.is_null() || self.count == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr, self.count as usize) }
        }
    }
}

impl Drop for OwnedActivateArray {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                for idx in 0..self.count as usize {
                    std::ptr::drop_in_place(self.ptr.add(idx));
                }
                CoTaskMemFree(Some(self.ptr.cast::<c_void>()));
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
    if win32_code == ERROR_NOT_FOUND.0 || win32_code == ERROR_FILE_NOT_FOUND.0 {
        return WinctlError::HardwareUnavailable(format!("{context}: {} (code {win32_code})", err.message()));
    }
    if hr == E_INVALIDARG || win32_code == ERROR_INVALID_PARAMETER.0 {
        return WinctlError::InvalidInput(format!("{context}: {} (code {win32_code})", err.message()));
    }
    WinctlError::win32(code, format!("{context}: {}", err.message()))
}

fn last_win32_err(context: &str) -> WinctlError {
    map_win32_err(windows::core::Error::from_thread(), context)
}

fn map_win32_status(code: u32, context: &str) -> WinctlError {
    if code == ERROR_ACCESS_DENIED.0 || code == ERROR_PRIVILEGE_NOT_HELD.0 {
        return WinctlError::PermissionDenied(format!("{context} (win32 {code})"));
    }
    if code == ERROR_NOT_FOUND.0 || code == ERROR_FILE_NOT_FOUND.0 {
        return WinctlError::HardwareUnavailable(format!("{context} (win32 {code})"));
    }
    WinctlError::win32(code, context)
}

fn hwnd_to_u64(hwnd: HWND) -> u64 {
    hwnd.0 as usize as u64
}

fn u64_to_hwnd(val: u64) -> WinctlResult<HWND> {
    if val == 0 {
        return Err(WinctlError::InvalidInput("hwnd must be non-zero".to_string()));
    }
    let hwnd = HWND(val as usize as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err(WinctlError::InvalidInput(format!(
            "window {} does not exist",
            format_hwnd(val)
        )));
    }
    Ok(hwnd)
}

fn read_window_text(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize).saturating_add(2).min(4096)];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if copied <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..copied as usize])
}

fn read_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let copied = unsafe { GetClassNameW(hwnd, &mut buf) };
    if copied <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..copied as usize])
}

fn process_image_name(pid: u32, cache: &mut HashMap<u32, String>) -> String {
    if pid == 0 {
        return String::new();
    }
    if let Some(cached) = cache.get(&pid) {
        return cached.clone();
    }
    let name = unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => {
                let _guard = OwnedHandle(h);
                let mut buf = [0u16; MAX_PATH as usize];
                let mut size = buf.len() as u32;
                if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size).is_ok() {
                    let full = String::from_utf16_lossy(&buf[..size as usize]);
                    Path::new(&full)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or(full)
                } else {
                    String::new()
                }
            }
            Err(_) => String::new(),
        }
    };
    cache.insert(pid, name.clone());
    name
}

fn create_ephemeral_probe_hwnd() -> Option<OwnedHwnd> {
    let hinst = unsafe { GetModuleHandleW(None) }.ok()?;
    let class_name = w!("WinctlClipboardListenerClass");
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(clipboard_listener_wndproc),
        hInstance: hinst.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    let _ = unsafe { RegisterClassExW(&wc) };
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            class_name,
            w!("WinctlDesktopProbe"),
            Default::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )
    }
    .ok()?;
    Some(OwnedHwnd(hwnd))
}

fn enum_top_level_hwnds() -> WinctlResult<Vec<HWND>> {
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        out.push(hwnd);
        BOOL(1)
    }

    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe { SetLastError(ERROR_SUCCESS) };
    let enum_res = unsafe { EnumWindows(Some(callback), LPARAM((&mut hwnds as *mut Vec<HWND>) as isize)) };
    if enum_res.is_err() || hwnds.is_empty() {
        let mut seen: HashSet<usize> = hwnds.iter().map(|h| h.0 as usize).collect();
        let mut prev: Option<HWND> = None;
        for _ in 0..MAX_PROP_WINDOWS {
            let next = unsafe { FindWindowExW(Some(HWND_MESSAGE), prev, None, None) };
            match next {
                Ok(child) if !child.0.is_null() => {
                    if !seen.insert(child.0 as usize) {
                        break;
                    }
                    hwnds.push(child);
                    prev = Some(child);
                }
                _ => break,
            }
        }
        let desk = unsafe { GetDesktopWindow() };
        if !desk.0.is_null() && unsafe { IsWindow(Some(desk)) }.as_bool() && seen.insert(desk.0 as usize) {
            hwnds.push(desk);
        }
    }
    Ok(hwnds)
}

fn enum_child_hwnds_bounded(parent: HWND, limit: usize) -> Vec<HWND> {
    struct ChildEnumState {
        out: Vec<HWND>,
        limit: usize,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if lparam.0 == 0 {
            return BOOL(0);
        }
        let state = unsafe { &mut *(lparam.0 as *mut ChildEnumState) };
        if state.out.len() >= state.limit {
            return BOOL(0);
        }
        state.out.push(hwnd);
        if state.out.len() >= state.limit {
            BOOL(0)
        } else {
            BOOL(1)
        }
    }

    if limit == 0 {
        return Vec::new();
    }
    let mut state = ChildEnumState { out: Vec::new(), limit };
    let _ = unsafe {
        EnumChildWindows(
            Some(parent),
            Some(callback),
            LPARAM((&mut state as *mut ChildEnumState) as isize),
        )
    };
    state.out
}

fn enum_direct_or_descendant_hwnds(parent: HWND) -> Vec<HWND> {
    enum_child_hwnds_bounded(parent, MAX_TREE_NODES)
}

fn query_window_opacity(hwnd: HWND, ex_style: u64) -> u8 {
    if (ex_style & (WS_EX_LAYERED.0 as u64)) == 0 {
        return 255;
    }
    let mut alpha = 255u8;
    let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
    if unsafe { GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)) }.is_ok()
        && (flags.0 & LWA_ALPHA.0) != 0
    {
        alpha
    } else {
        255
    }
}

fn collect_window_prop_info(hwnd: HWND, proc_cache: &mut HashMap<u32, String>) -> WindowPropInfo {
    let mut pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let process_name = process_image_name(pid, proc_cache);
    let title = read_window_text(hwnd);
    let class_name = read_class_name(hwnd);
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let enabled = unsafe { IsWindowEnabled(hwnd) }.as_bool();
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u64;
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u64;
    let topmost = (ex_style & (WS_EX_TOPMOST.0 as u64)) != 0;
    let parent_hwnd = hwnd_to_u64(unsafe { GetAncestor(hwnd, GA_PARENT) });
    let mut rect = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    let opacity = query_window_opacity(hwnd, ex_style);

    WindowPropInfo {
        hwnd: hwnd_to_u64(hwnd),
        title,
        class_name,
        pid,
        tid,
        process_name,
        visible,
        enabled,
        topmost,
        style,
        ex_style,
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        parent_hwnd,
        opacity,
    }
}

fn resolve_window_depth_with<F>(child_id: u64, depth_cache: &mut HashMap<u64, u32>, mut parent_of: F) -> u32
where
    F: FnMut(u64) -> u64,
{
    if let Some(&cached) = depth_cache.get(&child_id) {
        return cached;
    }
    let mut chain = vec![child_id];
    let mut visited = HashSet::from([child_id]);
    let mut cur = child_id;
    let mut base_depth = None;

    for _ in 0..64 {
        let parent = parent_of(cur);
        if parent == 0 || parent == cur {
            break;
        }
        if let Some(&known) = depth_cache.get(&parent) {
            base_depth = Some(known);
            break;
        }
        if !visited.insert(parent) {
            break;
        }
        chain.push(parent);
        cur = parent;
    }

    let base = base_depth.unwrap_or(0);
    let len = chain.len() as u32;
    for (idx, hwnd_id) in chain.iter().enumerate() {
        let depth = base + (len - idx as u32);
        depth_cache.insert(*hwnd_id, depth);
    }
    depth_cache[&child_id]
}

fn is_target_wow64_32bit(hprocess: HANDLE) -> bool {
    let mut target_wow64 = BOOL(0);
    if unsafe { IsWow64Process(hprocess, &mut target_wow64) }.is_err() {
        return false;
    }
    target_wow64.as_bool() && size_of::<usize>() == 8
}

fn send_msg_timeout(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<usize> {
    let mut res: usize = 0;
    let lres = unsafe { SendMessageTimeoutW(hwnd, msg, wparam, lparam, SMTO_ABORTIFHUNG, 250, Some(&mut res)) };
    if lres.0 != 0 { Some(res) } else { None }
}

fn read_remote_listview_text(hprocess: HANDLE, hwnd: HWND, row: i32, col: i32, wow64_32bit: bool) -> Option<String> {
    const TEXT_CHARS: usize = 512;
    let text_bytes = TEXT_CHARS * size_of::<u16>();
    let struct_bytes = if wow64_32bit {
        size_of::<LvItemW32>()
    } else {
        size_of::<LVITEMW>()
    };
    let total_bytes = struct_bytes + text_bytes;
    let remote = unsafe { VirtualAllocEx(hprocess, None, total_bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
    if remote.is_null() {
        return None;
    }
    let _alloc_guard = OwnedVirtualAlloc {
        process: hprocess,
        addr: remote,
    };
    let remote_text = (remote as usize + struct_bytes) as usize;

    let write_ok = if wow64_32bit {
        let item = LvItemW32 {
            mask: LVIF_TEXT.0,
            i_item: row,
            i_sub_item: col,
            psz_text: remote_text as u32,
            cch_text_max: TEXT_CHARS as i32,
            ..Default::default()
        };
        unsafe {
            WriteProcessMemory(
                hprocess,
                remote,
                (&item as *const LvItemW32).cast::<c_void>(),
                struct_bytes,
                None,
            )
            .is_ok()
        }
    } else {
        let item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: row,
            iSubItem: col,
            pszText: PWSTR(remote_text as *mut u16),
            cchTextMax: TEXT_CHARS as i32,
            ..Default::default()
        };
        unsafe {
            WriteProcessMemory(
                hprocess,
                remote,
                (&item as *const LVITEMW).cast::<c_void>(),
                struct_bytes,
                None,
            )
            .is_ok()
        }
    };
    if !write_ok {
        return None;
    }

    let mut msg_res = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            LVM_GETITEMW,
            WPARAM(0),
            LPARAM(remote as isize),
            SMTO_ABORTIFHUNG,
            250,
            Some(&mut msg_res),
        )
    };
    if ok.0 == 0 || msg_res == 0 {
        return None;
    }

    let mut local_buf = vec![0u16; TEXT_CHARS];
    if unsafe {
        ReadProcessMemory(
            hprocess,
            remote_text as *const c_void,
            local_buf.as_mut_ptr().cast::<c_void>(),
            text_bytes,
            None,
        )
    }
    .is_err()
    {
        return None;
    }
    Some(decode_utf16_nul_terminated(&local_buf))
}

fn read_remote_header_text(hprocess: HANDLE, header_hwnd: HWND, col: usize, wow64_32bit: bool) -> String {
    const TEXT_CHARS: usize = 256;
    let text_bytes = TEXT_CHARS * size_of::<u16>();
    let struct_bytes = if wow64_32bit {
        size_of::<HdItemW32>()
    } else {
        size_of::<HDITEMW>()
    };
    let remote = unsafe {
        VirtualAllocEx(
            hprocess,
            None,
            struct_bytes + text_bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if remote.is_null() {
        return format!("Col{col}");
    }
    let _alloc_guard = OwnedVirtualAlloc {
        process: hprocess,
        addr: remote,
    };
    let remote_text = remote as usize + struct_bytes;

    let write_ok = if wow64_32bit {
        let item = HdItemW32 {
            mask: HDI_TEXT.0,
            psz_text: remote_text as u32,
            cch_text_max: TEXT_CHARS as i32,
            ..Default::default()
        };
        unsafe {
            WriteProcessMemory(
                hprocess,
                remote,
                (&item as *const HdItemW32).cast::<c_void>(),
                struct_bytes,
                None,
            )
            .is_ok()
        }
    } else {
        let item = HDITEMW {
            mask: HDI_TEXT,
            pszText: PWSTR(remote_text as *mut u16),
            cchTextMax: TEXT_CHARS as i32,
            ..Default::default()
        };
        unsafe {
            WriteProcessMemory(
                hprocess,
                remote,
                (&item as *const HDITEMW).cast::<c_void>(),
                struct_bytes,
                None,
            )
            .is_ok()
        }
    };
    if !write_ok {
        return format!("Col{col}");
    }

    let mut msg_res = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            header_hwnd,
            HDM_GETITEMW,
            WPARAM(col),
            LPARAM(remote as isize),
            SMTO_ABORTIFHUNG,
            250,
            Some(&mut msg_res),
        )
    };
    if ok.0 == 0 || msg_res == 0 {
        return format!("Col{col}");
    }

    let mut local_buf = vec![0u16; TEXT_CHARS];
    if unsafe {
        ReadProcessMemory(
            hprocess,
            remote_text as *const c_void,
            local_buf.as_mut_ptr().cast::<c_void>(),
            text_bytes,
            None,
        )
    }
    .is_err()
    {
        return format!("Col{col}");
    }
    let s = decode_utf16_nul_terminated(&local_buf);
    if s.is_empty() { format!("Col{col}") } else { s }
}

fn read_remote_treeview_text(hprocess: HANDLE, hwnd: HWND, hitem: isize, wow64_32bit: bool) -> Option<String> {
    if std::hint::black_box(WPM_CTRL_EXPORT_MARKER[0]) == 0 {
        return None;
    }
    const TEXT_CHARS: usize = 512;
    let text_bytes = TEXT_CHARS * size_of::<u16>();
    let struct_bytes = if wow64_32bit {
        size_of::<TvItemW32>()
    } else {
        size_of::<TVITEMW>()
    };
    let remote = unsafe {
        VirtualAllocEx(
            hprocess,
            None,
            struct_bytes + text_bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if remote.is_null() {
        return None;
    }
    let _alloc_guard = OwnedVirtualAlloc {
        process: hprocess,
        addr: remote,
    };
    let remote_text = remote as usize + struct_bytes;

    let write_ok = if wow64_32bit {
        let item = TvItemW32 {
            mask: TVIF_TEXT.0,
            h_item: hitem as u32,
            psz_text: remote_text as u32,
            cch_text_max: TEXT_CHARS as i32,
            ..Default::default()
        };
        unsafe {
            WriteProcessMemory(
                hprocess,
                remote,
                (&item as *const TvItemW32).cast::<c_void>(),
                struct_bytes,
                None,
            )
            .is_ok()
        }
    } else {
        let item = TVITEMW {
            mask: TVIF_TEXT,
            hItem: HTREEITEM(hitem),
            pszText: PWSTR(remote_text as *mut u16),
            cchTextMax: TEXT_CHARS as i32,
            ..Default::default()
        };
        unsafe {
            WriteProcessMemory(
                hprocess,
                remote,
                (&item as *const TVITEMW).cast::<c_void>(),
                struct_bytes,
                None,
            )
            .is_ok()
        }
    };
    if !write_ok {
        return None;
    }

    let mut msg_res = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            TVM_GETITEMW,
            WPARAM(0),
            LPARAM(remote as isize),
            SMTO_ABORTIFHUNG,
            250,
            Some(&mut msg_res),
        )
    };
    if ok.0 == 0 || msg_res == 0 {
        return None;
    }

    let mut local_buf = vec![0u16; TEXT_CHARS];
    if unsafe {
        ReadProcessMemory(
            hprocess,
            remote_text as *const c_void,
            local_buf.as_mut_ptr().cast::<c_void>(),
            text_bytes,
            None,
        )
    }
    .is_err()
    {
        return None;
    }
    Some(decode_utf16_nul_terminated(&local_buf))
}

unsafe extern "system" fn clipboard_listener_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl DesktopBackend for Win32DesktopBackend {
    fn list_window_props(&self) -> WinctlResult<Vec<WindowPropInfo>> {
        let mut hwnds = enum_top_level_hwnds()?;
        let _probe_guard = if hwnds.is_empty() {
            let guard = create_ephemeral_probe_hwnd();
            if let Some(ref owned) = guard {
                hwnds.push(owned.0);
            }
            guard
        } else {
            None
        };
        let mut proc_cache = HashMap::new();
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(hwnds.len());

        for &hwnd in &hwnds {
            if out.len() >= MAX_PROP_WINDOWS {
                break;
            }
            let hwnd_val = hwnd_to_u64(hwnd);
            if !seen.insert(hwnd_val) {
                continue;
            }
            out.push(collect_window_prop_info(hwnd, &mut proc_cache));
        }

        for hwnd in hwnds {
            if out.len() >= MAX_PROP_WINDOWS {
                break;
            }
            let remaining = MAX_PROP_WINDOWS - out.len();
            let child_cap = MAX_PROP_CHILDREN_PER_PARENT.min(remaining);
            for child in enum_child_hwnds_bounded(hwnd, child_cap) {
                if out.len() >= MAX_PROP_WINDOWS {
                    break;
                }
                let child_val = hwnd_to_u64(child);
                if !seen.insert(child_val) {
                    continue;
                }
                out.push(collect_window_prop_info(child, &mut proc_cache));
            }
        }
        Ok(out)
    }

    fn control_window_prop(&self, hwnd_val: u64, op: &WindowPropOp) -> WinctlResult<WindowControlResult> {
        let hwnd = u64_to_hwnd(hwnd_val)?;
        let op_name = match op {
            WindowPropOp::Show => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
                "show"
            }
            WindowPropOp::Hide => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                "hide"
            }
            WindowPropOp::Enable => {
                unsafe {
                    let _ = EnableWindow(hwnd, true);
                }
                "enable"
            }
            WindowPropOp::Disable => {
                unsafe {
                    let _ = EnableWindow(hwnd, false);
                }
                "disable"
            }
            WindowPropOp::Topmost => {
                unsafe {
                    SetWindowPos(
                        hwnd,
                        Some(HWND_TOPMOST),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    )
                }
                .map_err(|e| map_win32_err(e, "SetWindowPos(HWND_TOPMOST)"))?;
                "topmost"
            }
            WindowPropOp::Untopmost => {
                unsafe {
                    SetWindowPos(
                        hwnd,
                        Some(HWND_NOTOPMOST),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    )
                }
                .map_err(|e| map_win32_err(e, "SetWindowPos(HWND_NOTOPMOST)"))?;
                "untopmost"
            }
            WindowPropOp::Close => {
                unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                    .map_err(|e| map_win32_err(e, "PostMessageW(WM_CLOSE)"))?;
                "close"
            }
            WindowPropOp::SetOpacity { opacity } => {
                let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
                if (ex & (WS_EX_LAYERED.0 as isize)) == 0 {
                    unsafe {
                        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_LAYERED.0 as isize));
                    }
                }
                unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), *opacity, LWA_ALPHA) }
                    .map_err(|e| map_win32_err(e, "SetLayeredWindowAttributes"))?;
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED | SWP_NOACTIVATE,
                    )
                };
                "set_opacity"
            }
        };

        Ok(WindowControlResult {
            hwnd: hwnd_val,
            hwnd_hex: format_hwnd(hwnd_val),
            operation: op_name.to_string(),
            message: format!("Applied '{op_name}' to {}", format_hwnd(hwnd_val)),
        })
    }

    fn list_top_windows(&self) -> WinctlResult<Vec<TopWindowInfo>> {
        let mut hwnds = enum_top_level_hwnds()?;
        let _probe_guard = if hwnds.is_empty() {
            let guard = create_ephemeral_probe_hwnd();
            if let Some(ref owned) = guard {
                hwnds.push(owned.0);
            }
            guard
        } else {
            None
        };
        let mut proc_cache = HashMap::new();
        let mut out = Vec::with_capacity(hwnds.len());

        for hwnd in hwnds {
            let mut pid = 0u32;
            let _ = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            let process_name = process_image_name(pid, &mut proc_cache);
            let title = read_window_text(hwnd);
            let class_name = read_class_name(hwnd);
            let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();

            let mut placement = WINDOWPLACEMENT {
                length: size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            let _ = unsafe { GetWindowPlacement(hwnd, &mut placement) };
            let minimized =
                placement.showCmd == (SW_MINIMIZE.0 as u32) || placement.showCmd == 2 || placement.showCmd == 6;
            let maximized = placement.showCmd == (SW_MAXIMIZE.0 as u32);
            let state = if !visible {
                "Hidden"
            } else if minimized {
                "Minimized"
            } else if maximized {
                "Maximized"
            } else {
                "Normal"
            }
            .to_string();

            let mut rect = RECT::default();
            let _ = unsafe { GetWindowRect(hwnd, &mut rect) };

            out.push(TopWindowInfo {
                hwnd: hwnd_to_u64(hwnd),
                title,
                class_name,
                pid,
                process_name,
                state,
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
                visible,
                minimized,
                maximized,
            });
        }
        Ok(out)
    }

    fn control_top_window(&self, hwnd_val: u64, op: &WindowListerOp) -> WinctlResult<WindowControlResult> {
        let hwnd = u64_to_hwnd(hwnd_val)?;
        let op_name = match op {
            WindowListerOp::Minimize => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_MINIMIZE);
                }
                "minimize"
            }
            WindowListerOp::Maximize => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                }
                "maximize"
            }
            WindowListerOp::Restore => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                "restore"
            }
            WindowListerOp::Close => {
                unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                    .map_err(|e| map_win32_err(e, "PostMessageW(WM_CLOSE)"))?;
                "close"
            }
            WindowListerOp::Focus => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                    let _ = SetForegroundWindow(hwnd);
                }
                "focus"
            }
        };

        Ok(WindowControlResult {
            hwnd: hwnd_val,
            hwnd_hex: format_hwnd(hwnd_val),
            operation: op_name.to_string(),
            message: format!("Applied '{op_name}' to {}", format_hwnd(hwnd_val)),
        })
    }

    fn list_window_tree(&self) -> WinctlResult<Vec<TreeWindowNode>> {
        let mut tops = enum_top_level_hwnds()?;
        let _probe_guard = if tops.is_empty() {
            let guard = create_ephemeral_probe_hwnd();
            if let Some(ref owned) = guard {
                tops.push(owned.0);
            }
            guard
        } else {
            None
        };
        let mut nodes = Vec::new();
        let mut seen = HashSet::new();
        let mut depth_cache = HashMap::new();

        for top in tops {
            if nodes.len() >= MAX_TREE_NODES {
                break;
            }
            let top_id = hwnd_to_u64(top);
            if !seen.insert(top_id) {
                continue;
            }
            depth_cache.insert(top_id, 0);
            let mut pid = 0u32;
            let _ = unsafe { GetWindowThreadProcessId(top, Some(&mut pid)) };
            let mut rect = RECT::default();
            let _ = unsafe { GetWindowRect(top, &mut rect) };

            nodes.push(TreeWindowNode {
                hwnd: top_id,
                parent_hwnd: 0,
                depth: 0,
                control_id: unsafe { GetDlgCtrlID(top) }.max(0) as u32,
                class_name: read_class_name(top),
                text: read_window_text(top),
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
                visible: unsafe { IsWindowVisible(top) }.as_bool(),
                enabled: unsafe { IsWindowEnabled(top) }.as_bool(),
                pid,
            });

            let children = enum_direct_or_descendant_hwnds(top);
            for child in children {
                if nodes.len() >= MAX_TREE_NODES {
                    break;
                }
                let child_id = hwnd_to_u64(child);
                if !seen.insert(child_id) {
                    continue;
                }
                let parent_hwnd = hwnd_to_u64(unsafe { GetAncestor(child, GA_PARENT) });
                if parent_hwnd == 0 || !unsafe { IsWindow(Some(child)) }.as_bool() {
                    continue;
                }
                let depth = resolve_window_depth_with(child_id, &mut depth_cache, |id| {
                    if id == child_id {
                        parent_hwnd
                    } else {
                        hwnd_to_u64(unsafe { GetAncestor(HWND(id as usize as *mut c_void), GA_PARENT) })
                    }
                });
                let mut cpid = 0u32;
                let _ = unsafe { GetWindowThreadProcessId(child, Some(&mut cpid)) };
                let mut crect = RECT::default();
                let _ = unsafe { GetWindowRect(child, &mut crect) };

                nodes.push(TreeWindowNode {
                    hwnd: child_id,
                    parent_hwnd,
                    depth,
                    control_id: unsafe { GetDlgCtrlID(child) }.max(0) as u32,
                    class_name: read_class_name(child),
                    text: read_window_text(child),
                    left: crect.left,
                    top: crect.top,
                    right: crect.right,
                    bottom: crect.bottom,
                    visible: unsafe { IsWindowVisible(child) }.as_bool(),
                    enabled: unsafe { IsWindowEnabled(child) }.as_bool(),
                    pid: cpid,
                });
            }
        }
        Ok(nodes)
    }

    fn control_tree_window(&self, hwnd_val: u64, op: &TreeControlOp) -> WinctlResult<WindowControlResult> {
        let hwnd = u64_to_hwnd(hwnd_val)?;
        let op_name = match op {
            TreeControlOp::SetText { text } => {
                let wide = HSTRING::from(text.as_str());
                unsafe { SetWindowTextW(hwnd, &wide) }.map_err(|e| map_win32_err(e, "SetWindowTextW"))?;
                "set_text"
            }
            TreeControlOp::Resize { width, height } => {
                if *width <= 0 || *height <= 0 {
                    return Err(WinctlError::InvalidInput(
                        "width and height must be positive".to_string(),
                    ));
                }
                let mut rect = RECT::default();
                unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|e| map_win32_err(e, "GetWindowRect"))?;
                unsafe { MoveWindow(hwnd, rect.left, rect.top, *width, *height, true) }
                    .map_err(|e| map_win32_err(e, "MoveWindow"))?;
                "resize"
            }
            TreeControlOp::Move { x, y } => {
                let mut rect = RECT::default();
                unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|e| map_win32_err(e, "GetWindowRect"))?;
                let w = rect.right.saturating_sub(rect.left).max(1);
                let h = rect.bottom.saturating_sub(rect.top).max(1);
                unsafe { MoveWindow(hwnd, *x, *y, w, h, true) }.map_err(|e| map_win32_err(e, "MoveWindow"))?;
                "move"
            }
            TreeControlOp::Show => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
                "show"
            }
            TreeControlOp::Hide => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                "hide"
            }
        };

        Ok(WindowControlResult {
            hwnd: hwnd_val,
            hwnd_hex: format_hwnd(hwnd_val),
            operation: op_name.to_string(),
            message: format!("Applied '{op_name}' to {}", format_hwnd(hwnd_val)),
        })
    }

    fn export_sys_controls(&self) -> WinctlResult<Vec<CtrlExportItem>> {
        if std::hint::black_box(WPM_CTRL_EXPORT_MARKER[0]) == 0 {
            return Ok(Vec::new());
        }
        let tops = enum_top_level_hwnds()?;
        let self_pid = std::process::id();
        let mut proc_cache = HashMap::new();
        let mut out = Vec::new();
        let mut seen = HashSet::new();

        for top in tops {
            let mut candidates = vec![top];
            candidates.extend(enum_direct_or_descendant_hwnds(top));

            for hwnd in candidates {
                let hwnd_val = hwnd_to_u64(hwnd);
                if !seen.insert(hwnd_val) {
                    continue;
                }
                let class_name = read_class_name(hwnd);
                if !is_supported_ctrl_export_class(&class_name) {
                    continue;
                }
                let mut pid = 0u32;
                let _ = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
                if pid == 0 || pid == self_pid {
                    // Skip self-process to prevent deadlock on SendMessageTimeoutW during GUI/test runs
                    continue;
                }
                let process_name = process_image_name(pid, &mut proc_cache);
                let lower = class_name.to_ascii_lowercase();

                if lower.contains("syslistview32") {
                    let hproc_res = unsafe {
                        OpenProcess(
                            PROCESS_VM_OPERATION
                                | PROCESS_VM_READ
                                | PROCESS_VM_WRITE
                                | PROCESS_QUERY_LIMITED_INFORMATION,
                            false,
                            pid,
                        )
                    };
                    let Ok(hproc) = hproc_res else {
                        continue;
                    };
                    let _proc_guard = OwnedHandle(hproc);
                    let wow64 = is_target_wow64_32bit(hproc);

                    let item_count = send_msg_timeout(hwnd, LVM_GETITEMCOUNT, WPARAM(0), LPARAM(0))
                        .unwrap_or(0)
                        .min(MAX_CTRL_EXPORT_ITEMS_PER_CONTROL);
                    let header_raw = send_msg_timeout(hwnd, LVM_GETHEADER, WPARAM(0), LPARAM(0)).unwrap_or(0);
                    let (col_count, col_titles) = if header_raw != 0 {
                        let hdr_hwnd = HWND(header_raw as *mut c_void);
                        let count = send_msg_timeout(hdr_hwnd, HDM_GETITEMCOUNT, WPARAM(0), LPARAM(0))
                            .unwrap_or(1)
                            .clamp(1, 32);
                        let titles: Vec<String> = (0..count)
                            .map(|c| read_remote_header_text(hproc, hdr_hwnd, c, wow64))
                            .collect();
                        (count, titles)
                    } else {
                        (1usize, vec!["Item".to_string()])
                    };

                    for r in 0..item_count {
                        for (c, title) in col_titles.iter().enumerate().take(col_count) {
                            if let Some(text) = read_remote_listview_text(hproc, hwnd, r as i32, c as i32, wow64)
                                && !text.is_empty()
                            {
                                out.push(CtrlExportItem {
                                    source_hwnd: hwnd_val,
                                    control_class: class_name.clone(),
                                    process_name: process_name.clone(),
                                    row_index: r as u32,
                                    col_index: c as u32,
                                    column_title: title.clone(),
                                    item_text: text,
                                });
                            }
                        }
                    }
                } else if lower.contains("systreeview32") {
                    let hproc_res = unsafe {
                        OpenProcess(
                            PROCESS_VM_OPERATION
                                | PROCESS_VM_READ
                                | PROCESS_VM_WRITE
                                | PROCESS_QUERY_LIMITED_INFORMATION,
                            false,
                            pid,
                        )
                    };
                    let Ok(hproc) = hproc_res else {
                        continue;
                    };
                    let _proc_guard = OwnedHandle(hproc);
                    let wow64 = is_target_wow64_32bit(hproc);

                    let mut stack = Vec::new();
                    if let Some(root) = send_msg_timeout(hwnd, TVM_GETNEXTITEM, WPARAM(TVGN_ROOT as usize), LPARAM(0))
                        && root != 0
                    {
                        stack.push(root as isize);
                    }
                    let mut row_idx = 0u32;
                    while let Some(hitem) = stack.pop() {
                        if (row_idx as usize) >= MAX_CTRL_EXPORT_ITEMS_PER_CONTROL {
                            break;
                        }
                        if let Some(text) = read_remote_treeview_text(hproc, hwnd, hitem, wow64)
                            && !text.is_empty()
                        {
                            out.push(CtrlExportItem {
                                source_hwnd: hwnd_val,
                                control_class: class_name.clone(),
                                process_name: process_name.clone(),
                                row_index: row_idx,
                                col_index: 0,
                                column_title: "TreeNode".to_string(),
                                item_text: text,
                            });
                            row_idx += 1;
                        }
                        if let Some(next_sib) =
                            send_msg_timeout(hwnd, TVM_GETNEXTITEM, WPARAM(TVGN_NEXT as usize), LPARAM(hitem))
                            && next_sib != 0
                        {
                            stack.push(next_sib as isize);
                        }
                        if let Some(first_child) =
                            send_msg_timeout(hwnd, TVM_GETNEXTITEM, WPARAM(TVGN_CHILD as usize), LPARAM(hitem))
                            && first_child != 0
                        {
                            stack.push(first_child as isize);
                        }
                    }
                } else if lower == "listbox" {
                    let count = send_msg_timeout(hwnd, LB_GETCOUNT, WPARAM(0), LPARAM(0))
                        .unwrap_or(0)
                        .min(MAX_CTRL_EXPORT_ITEMS_PER_CONTROL);
                    for r in 0..count {
                        let len = send_msg_timeout(hwnd, LB_GETTEXTLEN, WPARAM(r), LPARAM(0)).unwrap_or(0);
                        if len > 0 && len < 4096 {
                            let mut buf = vec![0u16; len + 2];
                            if send_msg_timeout(hwnd, LB_GETTEXT, WPARAM(r), LPARAM(buf.as_mut_ptr() as isize))
                                .is_some()
                            {
                                let text = decode_utf16_nul_terminated(&buf);
                                if !text.is_empty() {
                                    out.push(CtrlExportItem {
                                        source_hwnd: hwnd_val,
                                        control_class: class_name.clone(),
                                        process_name: process_name.clone(),
                                        row_index: r as u32,
                                        col_index: 0,
                                        column_title: "ListBoxItem".to_string(),
                                        item_text: text,
                                    });
                                }
                            }
                        }
                    }
                } else if lower == "combobox" {
                    let count = send_msg_timeout(hwnd, CB_GETCOUNT, WPARAM(0), LPARAM(0))
                        .unwrap_or(0)
                        .min(MAX_CTRL_EXPORT_ITEMS_PER_CONTROL);
                    for r in 0..count {
                        let len = send_msg_timeout(hwnd, CB_GETLBTEXTLEN, WPARAM(r), LPARAM(0)).unwrap_or(0);
                        if len > 0 && len < 4096 {
                            let mut buf = vec![0u16; len + 2];
                            if send_msg_timeout(hwnd, CB_GETLBTEXT, WPARAM(r), LPARAM(buf.as_mut_ptr() as isize))
                                .is_some()
                            {
                                let text = decode_utf16_nul_terminated(&buf);
                                if !text.is_empty() {
                                    out.push(CtrlExportItem {
                                        source_hwnd: hwnd_val,
                                        control_class: class_name.clone(),
                                        process_name: process_name.clone(),
                                        row_index: r as u32,
                                        col_index: 0,
                                        column_title: "ComboBoxItem".to_string(),
                                        item_text: text,
                                    });
                                }
                            }
                        }
                    }
                } else if lower == "edit" {
                    let len = send_msg_timeout(hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).unwrap_or(0);
                    if len > 0 && len < 8192 {
                        let mut buf = vec![0u16; len + 2];
                        if send_msg_timeout(hwnd, WM_GETTEXT, WPARAM(buf.len()), LPARAM(buf.as_mut_ptr() as isize))
                            .is_some()
                        {
                            let text = decode_utf16_nul_terminated(&buf);
                            if !text.is_empty() {
                                out.push(CtrlExportItem {
                                    source_hwnd: hwnd_val,
                                    control_class: class_name.clone(),
                                    process_name: process_name.clone(),
                                    row_index: 0,
                                    col_index: 0,
                                    column_title: "EditText".to_string(),
                                    item_text: text,
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    fn inspect_clipboard(&self) -> WinctlResult<Vec<ClipboardFormatEntry>> {
        let _clip = match OwnedClipboard::open(None) {
            Ok(guard) => guard,
            Err(WinctlError::PermissionDenied(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut entries = Vec::new();
        let mut fmt = 0u32;
        loop {
            fmt = unsafe { EnumClipboardFormats(fmt) };
            if fmt == 0 {
                break;
            }
            let registered_name = if fmt >= 0xC000 {
                let mut name_buf = [0u16; 256];
                let copied = unsafe { GetClipboardFormatNameW(fmt, &mut name_buf) };
                if copied > 0 {
                    Some(String::from_utf16_lossy(&name_buf[..copied as usize]))
                } else {
                    None
                }
            } else {
                None
            };

            let raw_bytes = match unsafe { GetClipboardData(fmt) } {
                Ok(handle) if !handle.is_invalid() => {
                    let hmem = HGLOBAL(handle.0);
                    let size = unsafe { GlobalSize(hmem) };
                    if size > 0 {
                        if let Ok(lock) = OwnedGlobalLock::lock(hmem) {
                            let mut bytes = vec![0u8; size];
                            unsafe {
                                copy_nonoverlapping(lock.ptr.cast::<u8>(), bytes.as_mut_ptr(), size);
                            }
                            bytes
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            };

            entries.push(build_clipboard_entry(fmt, registered_name, raw_bytes));
        }
        Ok(entries)
    }

    fn clear_clipboard(&self) -> WinctlResult<ClipboardControlResult> {
        let _clip = OwnedClipboard::open(None)?;
        unsafe { EmptyClipboard() }.map_err(|e| map_win32_err(e, "EmptyClipboard"))?;
        Ok(ClipboardControlResult {
            operation: "clear".to_string(),
            format_id: None,
            size_bytes: 0,
            message: "Cleared Windows clipboard".to_string(),
        })
    }

    fn set_clipboard_text(&self, text: &str) -> WinctlResult<ClipboardControlResult> {
        let utf16_bytes: Vec<u8> = text
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .flat_map(u16::to_le_bytes)
            .collect();
        self.set_clipboard_payload(u32::from(CF_UNICODETEXT.0), &utf16_bytes)
    }

    fn set_clipboard_payload(&self, format_id: u32, payload: &[u8]) -> WinctlResult<ClipboardControlResult> {
        let _clip = OwnedClipboard::open(None)?;
        unsafe { EmptyClipboard() }.map_err(|e| map_win32_err(e, "EmptyClipboard"))?;
        let alloc_size = payload.len().max(1);
        let hmem = unsafe { GlobalAlloc(GMEM_MOVEABLE, alloc_size) }.map_err(|e| map_win32_err(e, "GlobalAlloc"))?;
        let mut alloc_guard = OwnedGlobalAlloc(Some(hmem));
        {
            let lock = OwnedGlobalLock::lock(hmem)?;
            if !payload.is_empty() {
                unsafe {
                    copy_nonoverlapping(payload.as_ptr(), lock.ptr.cast_mut().cast::<u8>(), payload.len());
                }
            }
        }
        unsafe { SetClipboardData(format_id, Some(HANDLE(hmem.0))) }
            .map_err(|e| map_win32_err(e, "SetClipboardData"))?;
        let _ = alloc_guard.disarm(); // OS owns HGLOBAL after SetClipboardData succeeds

        Ok(ClipboardControlResult {
            operation: "set_payload".to_string(),
            format_id: Some(format_id),
            size_bytes: payload.len() as u64,
            message: format!("Placed {} bytes on clipboard (format {format_id})", payload.len()),
        })
    }

    fn monitor_clipboard_update(&self, timeout_ms: u32) -> WinctlResult<Option<ClipboardFormatEntry>> {
        let hinst = unsafe { GetModuleHandleW(None) }.map_err(|e| map_win32_err(e, "GetModuleHandleW"))?;
        let class_name = w!("WinctlClipboardListenerClass");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(clipboard_listener_wndproc),
            hInstance: hinst.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = unsafe { RegisterClassExW(&wc) };

        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                class_name,
                w!("WinctlClipHistListener"),
                Default::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinst.into()),
                None,
            )
        }
        .map_err(|e| map_win32_err(e, "CreateWindowExW(HWND_MESSAGE)"))?;
        let _hwnd_guard = OwnedHwnd(hwnd);

        unsafe { AddClipboardFormatListener(hwnd) }.map_err(|e| map_win32_err(e, "AddClipboardFormatListener"))?;
        let _listener_guard = OwnedClipboardListener(hwnd);

        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let mut msg = MSG::default();
        let mut saw_update = false;
        while Instant::now() < deadline {
            if unsafe { PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_REMOVE) }.as_bool() {
                if msg.message == WM_CLIPBOARDUPDATE {
                    saw_update = true;
                    break;
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            } else {
                std::thread::sleep(Duration::from_millis(15));
            }
        }

        if saw_update || timeout_ms == 0 {
            let entries = self.inspect_clipboard()?;
            Ok(entries.into_iter().next())
        } else {
            Ok(None)
        }
    }

    fn probe_hotkeys(&self) -> WinctlResult<Vec<HotkeyEntry>> {
        let mut results = Vec::new();
        for (idx, (mods, vk)) in common_hotkey_probe_candidates().into_iter().enumerate() {
            let id = (idx as i32) + 0x4100;
            let probe = unsafe { RegisterHotKey(None, id, HOT_KEY_MODIFIERS(mods), vk) };
            match probe {
                Ok(()) => unsafe {
                    let _ = UnregisterHotKey(None, id);
                },
                Err(_) => {
                    let (hotkey, modifiers) = format_probe_hotkey(mods, vk);
                    results.push(HotkeyEntry {
                        source: "RegisterHotKeyProbe".to_string(),
                        hotkey,
                        modifiers,
                        vk_code: vk,
                        status: "Registered".to_string(),
                        owner_hint: "Registered globally by another thread or shell".to_string(),
                        lnk_path: String::new(),
                    });
                }
            }
        }

        // Scan standard Desktop and Start Menu folders for [MS-SHLLINK] .lnk wHotKey bindings
        let mut dirs_to_scan: Vec<PathBuf> = Vec::new();
        if let Some(desktop) = dirs::desktop_dir() {
            dirs_to_scan.push(desktop);
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs_to_scan.push(PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs"));
        }
        if let Ok(progdata) = std::env::var("ProgramData") {
            dirs_to_scan.push(PathBuf::from(progdata).join("Microsoft\\Windows\\Start Menu\\Programs"));
        }
        for dir in dirs_to_scan {
            if dir.exists() {
                results.extend(scan_lnk_hotkeys_in_dir(&dir, 4));
            }
        }
        Ok(results)
    }

    fn probe_single_hotkey(&self, modifiers: u32, vk_code: u32) -> WinctlResult<HotkeyProbeResult> {
        let id = 0x5200;
        let res = unsafe { RegisterHotKey(None, id, HOT_KEY_MODIFIERS(modifiers), vk_code) };
        match res {
            Ok(()) => {
                unsafe {
                    let _ = UnregisterHotKey(None, id);
                }
                Ok(HotkeyProbeResult {
                    vk_code,
                    modifiers,
                    available: true,
                    status: "Available".to_string(),
                })
            }
            Err(_) => Ok(HotkeyProbeResult {
                vk_code,
                modifiers,
                available: false,
                status: "InUse".to_string(),
            }),
        }
    }

    fn query_keyboard_state(&self) -> WinctlResult<Vec<KeyStateEntry>> {
        let mut key_state = [0u8; 256];
        unsafe { GetKeyboardState(&mut key_state) }.map_err(|e| map_win32_err(e, "GetKeyboardState"))?;

        let mut entries = Vec::with_capacity(254);
        for vk in 1u8..=0xFEu8 {
            let state_byte = key_state[vk as usize];
            let async_state = unsafe { GetAsyncKeyState(i32::from(vk)) };
            let pressed = (state_byte & 0x80) != 0 || (async_state as u16 & 0x8000) != 0;
            let toggled = (state_byte & 0x01) != 0;
            let scan_code = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) };
            let extended = is_extended_vk(vk);

            let key_label = if scan_code != 0 {
                let lparam = ((scan_code as i32) << 16) | if extended { 1 << 24 } else { 0 };
                let mut name_buf = [0u16; 64];
                let copied = unsafe { GetKeyNameTextW(lparam, &mut name_buf) };
                if copied > 0 {
                    String::from_utf16_lossy(&name_buf[..copied as usize])
                } else {
                    vk_canonical_name(vk)
                }
            } else {
                vk_canonical_name(vk)
            };

            entries.push(KeyStateEntry {
                vk_code: vk,
                vk_name: vk_canonical_name(vk),
                scan_code,
                key_label,
                pressed,
                toggled,
                extended,
            });
        }
        Ok(entries)
    }

    fn list_installed_fonts(&self) -> WinctlResult<Vec<InstalledFontEntry>> {
        let gdi_families = enumerate_gdi_font_families();

        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let fonts_dir = PathBuf::from(windir).join("Fonts");
        let local_fonts_dir = dirs::data_local_dir().map(|d| d.join("Microsoft").join("Windows").join("Fonts"));

        let mut entries = Vec::new();
        let mut seen_paths = HashSet::new();

        scan_fonts_registry_hive(
            HKEY_LOCAL_MACHINE,
            false,
            &fonts_dir,
            local_fonts_dir.as_deref(),
            &mut seen_paths,
            &mut entries,
        )?;
        scan_fonts_registry_hive(
            HKEY_CURRENT_USER,
            true,
            &fonts_dir,
            local_fonts_dir.as_deref(),
            &mut seen_paths,
            &mut entries,
        )?;

        merge_gdi_font_families(&mut entries, &gdi_families);
        Ok(entries)
    }

    fn list_webcam_devices(&self) -> WinctlResult<Vec<WebcamDeviceInfo>> {
        let _com = ComApartmentGuard::init()?;
        let _mf = MediaFoundationGuard::startup()?;

        let mut attrs: Option<IMFAttributes> = None;
        unsafe { MFCreateAttributes(&mut attrs, 1) }.map_err(|e| map_win32_err(e, "MFCreateAttributes"))?;
        let attrs =
            attrs.ok_or_else(|| WinctlError::HardwareUnavailable("MFCreateAttributes returned null".to_string()))?;
        unsafe {
            attrs.SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
        }
        .map_err(|e| map_win32_err(e, "SetGUID(VIDCAP)"))?;

        let mut ptr: *mut Option<IMFActivate> = null_mut();
        let mut count = 0u32;
        unsafe { MFEnumDeviceSources(&attrs, &mut ptr, &mut count) }
            .map_err(|e| map_win32_err(e, "MFEnumDeviceSources"))?;
        let activates = OwnedActivateArray { ptr, count };

        let mut out = Vec::with_capacity(count as usize);
        for (idx, maybe_act) in activates.slice().iter().enumerate() {
            let Some(act) = maybe_act else { continue };
            let friendly_name = get_allocated_mf_string(act, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)
                .unwrap_or_else(|| format!("Camera {idx}"));
            let symbolic_link = get_allocated_mf_string(act, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK)
                .unwrap_or_default();

            let (resolution, pixel_format) =
                probe_webcam_native_mode(act).unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()));

            out.push(WebcamDeviceInfo {
                device_index: idx as u32,
                friendly_name,
                symbolic_link,
                resolution,
                pixel_format,
                available: true,
            });
        }
        Ok(out)
    }

    fn capture_webcam_frame(&self, device_index: u32) -> WinctlResult<RawVideoFrame> {
        let _com = ComApartmentGuard::init()?;
        let _mf = MediaFoundationGuard::startup()?;

        let mut attrs: Option<IMFAttributes> = None;
        unsafe { MFCreateAttributes(&mut attrs, 1) }.map_err(|e| map_win32_err(e, "MFCreateAttributes"))?;
        let attrs =
            attrs.ok_or_else(|| WinctlError::HardwareUnavailable("MFCreateAttributes returned null".to_string()))?;
        unsafe {
            attrs.SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
        }
        .map_err(|e| map_win32_err(e, "SetGUID(VIDCAP)"))?;

        let mut ptr: *mut Option<IMFActivate> = null_mut();
        let mut count = 0u32;
        unsafe { MFEnumDeviceSources(&attrs, &mut ptr, &mut count) }
            .map_err(|e| map_win32_err(e, "MFEnumDeviceSources"))?;
        let activates = OwnedActivateArray { ptr, count };

        if count == 0 {
            return Err(WinctlError::HardwareUnavailable(
                "no Media Foundation video capture device is present".to_string(),
            ));
        }
        let act = activates
            .slice()
            .get(device_index as usize)
            .and_then(|o| o.as_ref())
            .ok_or_else(|| {
                WinctlError::HardwareUnavailable(format!(
                    "webcam device index {device_index} out of range (found {count} device(s))"
                ))
            })?;

        let source: IMFMediaSource =
            unsafe { act.ActivateObject() }.map_err(|e| map_win32_err(e, "IMFActivate::ActivateObject"))?;
        let reader: IMFSourceReader = unsafe { MFCreateSourceReaderFromMediaSource(&source, None) }
            .map_err(|e| map_win32_err(e, "MFCreateSourceReaderFromMediaSource"))?;

        // Request RGB24 output when supported; fall back to current native subtype
        if let Ok(req_type) = unsafe { MFCreateMediaType() } {
            let _ = unsafe { req_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) };
            let _ = unsafe { req_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB24) };
            let _ =
                unsafe { reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, None, &req_type) };
        }

        let cur_type: IMFMediaType =
            unsafe { reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32) }
                .map_err(|e| map_win32_err(e, "IMFSourceReader::GetCurrentMediaType"))?;

        let frame_size = unsafe { cur_type.GetUINT64(&MF_MT_FRAME_SIZE) }
            .map_err(|e| map_win32_err(e, "IMFAttributes::GetUINT64(MF_MT_FRAME_SIZE)"))?;
        let width = (frame_size >> 32) as u32;
        let height = (frame_size & 0xFFFF_FFFF) as u32;
        let subtype = unsafe { cur_type.GetGUID(&MF_MT_SUBTYPE) }
            .map_err(|e| map_win32_err(e, "IMFAttributes::GetGUID(MF_MT_SUBTYPE)"))?;

        let pixel_format = if subtype == MFVideoFormat_RGB24 {
            VideoPixelFormat::Rgb24
        } else if subtype == MFVideoFormat_RGB32 {
            VideoPixelFormat::Bgra32
        } else if subtype == MFVideoFormat_YUY2 {
            VideoPixelFormat::Yuy2
        } else if subtype == MFVideoFormat_NV12 {
            VideoPixelFormat::Nv12
        } else {
            return Err(WinctlError::HardwareUnavailable(format!(
                "webcam negotiated unsupported Media Foundation video subtype {:?}",
                subtype
            )));
        };

        let mut sample_opt: Option<IMFSample> = None;
        for _attempt in 0..12 {
            let mut flags = 0u32;
            let mut maybe_sample: Option<IMFSample> = None;
            unsafe {
                reader.ReadSample(
                    MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                    0,
                    None,
                    Some(&mut flags),
                    None,
                    Some(&mut maybe_sample),
                )
            }
            .map_err(|e| map_win32_err(e, "IMFSourceReader::ReadSample"))?;
            if let Some(s) = maybe_sample {
                sample_opt = Some(s);
                break;
            }
        }

        let sample = sample_opt.ok_or_else(|| {
            WinctlError::HardwareUnavailable("webcam did not produce a video sample frame".to_string())
        })?;
        let buffer: IMFMediaBuffer = unsafe { sample.ConvertToContiguousBuffer() }
            .map_err(|e| map_win32_err(e, "IMFSample::ConvertToContiguousBuffer"))?;

        let mut buf_ptr: *mut u8 = null_mut();
        let mut cur_len = 0u32;
        unsafe { buffer.Lock(&mut buf_ptr, None, Some(&mut cur_len)) }
            .map_err(|e| map_win32_err(e, "IMFMediaBuffer::Lock"))?;

        let data = if !buf_ptr.is_null() && cur_len > 0 {
            unsafe { std::slice::from_raw_parts(buf_ptr, cur_len as usize) }.to_vec()
        } else {
            Vec::new()
        };
        let _ = unsafe { buffer.Unlock() };
        let _ = unsafe { act.ShutdownObject() };

        Ok(RawVideoFrame {
            width,
            height,
            pixel_format,
            data,
        })
    }
}

fn get_allocated_mf_string(attrs: &IMFActivate, key: &GUID) -> Option<String> {
    let mut pwstr = PWSTR::null();
    let mut len = 0u32;
    if unsafe { attrs.GetAllocatedString(key, &mut pwstr, &mut len) }.is_ok() {
        let guard = OwnedCoTaskPwstr(pwstr);
        let s = guard.to_string_lossy();
        if !s.is_empty() { Some(s) } else { None }
    } else {
        None
    }
}

fn probe_webcam_native_mode(act: &IMFActivate) -> Option<(String, String)> {
    let source: IMFMediaSource = unsafe { act.ActivateObject() }.ok()?;
    let reader: IMFSourceReader = unsafe { MFCreateSourceReaderFromMediaSource(&source, None) }.ok()?;
    let mtype: IMFMediaType =
        unsafe { reader.GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, 0) }.ok()?;
    let frame_size = unsafe { mtype.GetUINT64(&MF_MT_FRAME_SIZE) }.ok()?;
    let w = (frame_size >> 32) as u32;
    let h = (frame_size & 0xFFFF_FFFF) as u32;
    let subtype = unsafe { mtype.GetGUID(&MF_MT_SUBTYPE) }.ok()?;
    let fmt_label = if subtype == MFVideoFormat_RGB24 {
        "RGB24"
    } else if subtype == MFVideoFormat_RGB32 {
        "BGRA32"
    } else if subtype == MFVideoFormat_YUY2 {
        "YUY2"
    } else if subtype == MFVideoFormat_NV12 {
        "NV12"
    } else {
        "MJPG/Other"
    };
    let _ = unsafe { act.ShutdownObject() };
    Some((format!("{w}x{h}"), fmt_label.to_string()))
}

fn enumerate_gdi_font_families() -> BTreeSet<String> {
    unsafe extern "system" fn font_enum_cb(
        lpelfe: *const LOGFONTW,
        _lpntme: *const TEXTMETRICW,
        _font_type: u32,
        lparam: LPARAM,
    ) -> i32 {
        if !lpelfe.is_null() && lparam.0 != 0 {
            let families = unsafe { &mut *(lparam.0 as *mut BTreeSet<String>) };
            let face = unsafe { &(*lpelfe).lfFaceName };
            let len = face.iter().position(|&c| c == 0).unwrap_or(face.len());
            let name = String::from_utf16_lossy(&face[..len]);
            if !name.is_empty() {
                families.insert(name);
            }
        }
        1
    }

    let mut gdi_families: BTreeSet<String> = BTreeSet::new();
    let hdc = unsafe { GetDC(None) };
    if !hdc.is_invalid() {
        let _dc_guard = OwnedScreenDc(None, hdc);
        let mut lf: LOGFONTW = unsafe { zeroed() };
        lf.lfCharSet = DEFAULT_CHARSET;
        unsafe {
            let _ = EnumFontFamiliesExW(
                hdc,
                &lf,
                Some(font_enum_cb),
                LPARAM((&mut gdi_families as *mut BTreeSet<String>) as isize),
                0,
            );
        }
    }
    gdi_families
}

fn expand_percent_env_vars_with<F>(input: &str, lookup: F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 1..];
        if let Some(end) = after_open.find('%') {
            let var_name = &after_open[..end];
            if !var_name.is_empty()
                && let Some(val) = lookup(var_name)
            {
                out.push_str(&val);
            } else {
                out.push('%');
                out.push_str(var_name);
                out.push('%');
            }
            rest = &after_open[end + 1..];
        } else {
            out.push('%');
            rest = after_open;
        }
    }
    out.push_str(rest);
    out
}

fn expand_percent_env_vars(input: &str) -> String {
    expand_percent_env_vars_with(input, |name| {
        std::env::vars().find_map(|(k, v)| k.eq_ignore_ascii_case(name).then_some(v))
    })
}

fn scan_fonts_registry_hive(
    hive: HKEY,
    ignore_missing: bool,
    fonts_dir: &Path,
    local_fonts_dir: Option<&Path>,
    seen_paths: &mut HashSet<String>,
    entries: &mut Vec<InstalledFontEntry>,
) -> WinctlResult<()> {
    let mut hkey = HKEY::default();
    let subkey = HSTRING::from(FONTS_REG_KEY);
    let status = unsafe { RegOpenKeyExW(hive, &subkey, None, KEY_READ, &mut hkey) };
    if status != ERROR_SUCCESS {
        if ignore_missing && (status == ERROR_FILE_NOT_FOUND || status == ERROR_NOT_FOUND) {
            return Ok(());
        }
        return Err(map_win32_status(status.0, "RegOpenKeyExW(Fonts)"));
    }
    let _key_guard = OwnedHKey(hkey);

    let mut reg_values: Vec<(String, PathBuf)> = Vec::new();
    let mut idx = 0u32;
    loop {
        let mut name_buf = [0u16; 512];
        let mut name_len = name_buf.len() as u32;
        let mut val_type = 0u32;
        let mut data_buf = [0u8; 1024];
        let mut data_len = data_buf.len() as u32;

        let rc = unsafe {
            RegEnumValueW(
                hkey,
                idx,
                Some(PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut val_type),
                Some(data_buf.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if rc == ERROR_NO_MORE_ITEMS {
            break;
        }
        idx += 1;
        if rc != ERROR_SUCCESS {
            continue;
        }
        if val_type != REG_SZ.0 && val_type != REG_EXPAND_SZ.0 {
            continue;
        }

        let reg_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
        let data_units: Vec<u16> = data_buf[..data_len as usize]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let raw_file = decode_utf16_nul_terminated(&data_units);
        if raw_file.is_empty() {
            continue;
        }
        let expanded_file = if val_type == REG_EXPAND_SZ.0 {
            expand_percent_env_vars(&raw_file)
        } else {
            raw_file
        };

        let resolved_path = {
            let p = PathBuf::from(&expanded_file);
            if p.is_absolute() {
                p
            } else {
                let sys_candidate = fonts_dir.join(&expanded_file);
                if sys_candidate.exists() {
                    sys_candidate
                } else if let Some(local_dir) = local_fonts_dir {
                    let local_candidate = local_dir.join(&expanded_file);
                    if local_candidate.exists() {
                        local_candidate
                    } else {
                        sys_candidate
                    }
                } else {
                    sys_candidate
                }
            }
        };

        reg_values.push((reg_name, resolved_path));
    }

    reg_values.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    for (reg_name, resolved_path) in reg_values {
        let dedup_key = resolved_path.display().to_string().to_ascii_lowercase();
        if !seen_paths.insert(dedup_key) {
            continue;
        }

        if let Ok(parsed_entries) = font_entries_from_file(&resolved_path) {
            entries.extend(parsed_entries);
        } else {
            entries.push(InstalledFontEntry {
                family: reg_name.clone(),
                subfamily: String::new(),
                full_name: reg_name,
                postscript_name: String::new(),
                font_format: "RegistryFont".to_string(),
                weight_class: 400,
                width_class: 5,
                fs_type: 0,
                panose: String::new(),
                version: String::new(),
                source_path: resolved_path.display().to_string(),
                face_index: 0,
            });
        }
    }

    Ok(())
}

fn merge_gdi_font_families(entries: &mut Vec<InstalledFontEntry>, gdi_families: &BTreeSet<String>) {
    let mut represented: HashSet<String> = entries
        .iter()
        .filter_map(|e| {
            let trimmed = e.family.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_ascii_lowercase())
            }
        })
        .collect();

    for family in gdi_families {
        let trimmed = family.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if represented.insert(lower) {
            entries.push(InstalledFontEntry {
                family: trimmed.to_string(),
                subfamily: String::new(),
                full_name: trimmed.to_string(),
                postscript_name: String::new(),
                font_format: "GdiFamily".to_string(),
                weight_class: 400,
                width_class: 5,
                fs_type: 0,
                panose: String::new(),
                version: String::new(),
                source_path: String::new(),
                face_index: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{WS_CHILD, WS_OVERLAPPEDWINDOW};

    #[test]
    fn resolve_window_depth_handles_multi_level_out_of_order_and_cycles() {
        let mut cache = HashMap::from([(100u64, 0u32)]);
        let parents = HashMap::from([
            (200u64, 100u64),
            (300u64, 200u64),
            (400u64, 300u64),
            (500u64, 600u64),
            (600u64, 500u64), // cycle
        ]);
        let lookup = |id: u64| parents.get(&id).copied().unwrap_or(0);

        // Out-of-order: resolve great-grandchild before child/grandchild
        assert_eq!(resolve_window_depth_with(400, &mut cache, lookup), 3);
        assert_eq!(cache.get(&300), Some(&2));
        assert_eq!(cache.get(&200), Some(&1));
        assert_eq!(resolve_window_depth_with(200, &mut cache, lookup), 1);

        // Cycle terminates cleanly with bounded non-zero depth
        assert_eq!(resolve_window_depth_with(500, &mut cache, lookup), 2);
        // Unknown parent falls back to depth 1
        assert_eq!(resolve_window_depth_with(999, &mut cache, lookup), 1);
    }

    #[test]
    fn merge_gdi_font_families_is_deterministic_and_deduplicates() {
        let mut entries = vec![InstalledFontEntry {
            family: "Segoe UI".to_string(),
            subfamily: "Regular".to_string(),
            full_name: "Segoe UI".to_string(),
            postscript_name: "SegoeUI".to_string(),
            font_format: "TrueType".to_string(),
            weight_class: 400,
            width_class: 5,
            fs_type: 0,
            panose: String::new(),
            version: String::new(),
            source_path: "C:\\Windows\\Fonts\\segoeui.ttf".to_string(),
            face_index: 0,
        }];
        let gdi = BTreeSet::from([
            "segoe ui".to_string(),
            "Fixedsys".to_string(),
            "Terminal".to_string(),
            "".to_string(),
        ]);

        merge_gdi_font_families(&mut entries, &gdi);
        let families: Vec<&str> = entries.iter().map(|e| e.family.as_str()).collect();
        assert_eq!(families, vec!["Segoe UI", "Fixedsys", "Terminal"]);
        assert_eq!(entries[1].font_format, "GdiFamily");
        assert_eq!(entries[2].font_format, "GdiFamily");
    }

    #[test]
    fn win32_live_gdi_fonts_layered_opacity_and_multi_level_tree() {
        // 1. GDI EnumFontFamiliesExW returns non-empty families
        let gdi_families = enumerate_gdi_font_families();
        assert!(
            !gdi_families.is_empty(),
            "EnumFontFamiliesExW should return at least one GDI font family"
        );

        // 2. Hidden layered top-level -> child -> grandchild hierarchy
        let hinst = unsafe { GetModuleHandleW(None) }.unwrap();
        let class_name = w!("WinctlTestTreePropsClass");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(clipboard_listener_wndproc),
            hInstance: hinst.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = unsafe { RegisterClassExW(&wc) };

        let top = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED,
                class_name,
                w!("WinctlTestTop"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                120,
                80,
                None,
                None,
                Some(hinst.into()),
                None,
            )
        }
        .unwrap();
        let _top_guard = OwnedHwnd(top);
        unsafe { SetLayeredWindowAttributes(top, COLORREF(0), 137, LWA_ALPHA) }.unwrap();

        let child = unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("WinctlTestChild"),
                WS_CHILD,
                4,
                4,
                80,
                40,
                Some(top),
                None,
                Some(hinst.into()),
                None,
            )
        }
        .unwrap();
        let _child_guard = OwnedHwnd(child);

        let grandchild = unsafe {
            CreateWindowExW(
                Default::default(),
                w!("EDIT"),
                w!("WinctlTestGrandchild"),
                WS_CHILD,
                2,
                2,
                40,
                20,
                Some(child),
                None,
                Some(hinst.into()),
                None,
            )
        }
        .unwrap();
        let _grandchild_guard = OwnedHwnd(grandchild);

        let top_id = hwnd_to_u64(top);
        let child_id = hwnd_to_u64(child);
        let grandchild_id = hwnd_to_u64(grandchild);

        // Direct opacity query matches LWA_ALPHA = 137
        assert_eq!(query_window_opacity(top, WS_EX_LAYERED.0 as u64), 137);

        let backend = Win32DesktopBackend::new();
        let props = backend.list_window_props().unwrap();
        let top_prop = props
            .iter()
            .find(|p| p.hwnd == top_id)
            .expect("top layered window should be in list_window_props");
        assert_eq!(top_prop.opacity, 137);
        let child_prop = props
            .iter()
            .find(|p| p.hwnd == child_id)
            .expect("child window should be in list_window_props");
        assert_eq!(child_prop.parent_hwnd, top_id);

        // Direct GA_PARENT chain depth computation yields 0, 1, 2
        let mut depth_cache = HashMap::from([(top_id, 0u32)]);
        let get_parent = |id: u64| hwnd_to_u64(unsafe { GetAncestor(HWND(id as usize as *mut c_void), GA_PARENT) });
        assert_eq!(resolve_window_depth_with(child_id, &mut depth_cache, get_parent), 1);
        assert_eq!(
            resolve_window_depth_with(grandchild_id, &mut depth_cache, get_parent),
            2
        );

        let tree = backend.list_window_tree().unwrap();
        if let Some(t) = tree.iter().find(|n| n.hwnd == top_id) {
            assert_eq!(t.depth, 0);
            if let Some(c) = tree.iter().find(|n| n.hwnd == child_id) {
                assert_eq!(c.depth, 1);
                assert_eq!(c.parent_hwnd, top_id);
            }
            if let Some(gc) = tree.iter().find(|n| n.hwnd == grandchild_id) {
                assert_eq!(gc.depth, 2);
                assert_eq!(gc.parent_hwnd, child_id);
            }
        }
    }

    #[test]
    fn expand_percent_env_vars_handles_reg_expand_sz_patterns() {
        let lookup = |name: &str| -> Option<String> {
            match name.to_ascii_lowercase().as_str() {
                "systemroot" => Some("C:\\Windows".to_string()),
                "localappdata" => Some("C:\\Users\\Alice\\AppData\\Local".to_string()),
                _ => None,
            }
        };
        assert_eq!(
            expand_percent_env_vars_with("%SystemRoot%\\Fonts\\arial.ttf", lookup),
            "C:\\Windows\\Fonts\\arial.ttf"
        );
        assert_eq!(
            expand_percent_env_vars_with("%LOCALAPPDATA%\\Microsoft\\Windows\\Fonts\\Custom.otf", lookup),
            "C:\\Users\\Alice\\AppData\\Local\\Microsoft\\Windows\\Fonts\\Custom.otf"
        );
        assert_eq!(
            expand_percent_env_vars_with("%UNKNOWN%\\Fonts\\%SystemRoot%", lookup),
            "%UNKNOWN%\\Fonts\\C:\\Windows"
        );
        assert_eq!(
            expand_percent_env_vars_with("%%\\unclosed%SystemRoot", lookup),
            "%%\\unclosed%SystemRoot"
        );
    }
}
