use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use winctl2rust_core::{WinctlError, WinctlResult};

pub fn validate_safe_path(raw: &str, field_name: &str) -> WinctlResult<String> {
    if raw.contains('\0') {
        return Err(WinctlError::InvalidInput(format!(
            "{field_name} must not contain NUL bytes"
        )));
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WinctlError::InvalidInput(format!("{field_name} must not be empty")));
    }
    if Path::new(trimmed)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
        || trimmed.split(['/', '\\']).any(|seg| seg == "..")
    {
        return Err(WinctlError::InvalidInput(format!(
            "{field_name} must not contain parent-directory ('..') traversal"
        )));
    }
    Ok(trimmed.to_string())
}

pub fn format_hwnd(hwnd: u64) -> String {
    format!("0x{hwnd:08X}")
}

pub fn parse_hwnd(raw: &str) -> WinctlResult<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WinctlError::InvalidInput("hwnd must not be empty".to_string()));
    }
    let val = if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|_| WinctlError::InvalidInput(format!("invalid hex hwnd '{raw}'")))?
    } else if trimmed.chars().all(|c| c.is_ascii_digit()) {
        trimmed
            .parse::<u64>()
            .map_err(|_| WinctlError::InvalidInput(format!("invalid decimal hwnd '{raw}'")))?
    } else if trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        u64::from_str_radix(trimmed, 16).map_err(|_| WinctlError::InvalidInput(format!("invalid hex hwnd '{raw}'")))?
    } else {
        return Err(WinctlError::InvalidInput(format!(
            "invalid hwnd '{raw}' (expected 0xHEX or decimal integer)"
        )));
    };
    if val == 0 {
        return Err(WinctlError::InvalidInput("hwnd must be non-zero".to_string()));
    }
    Ok(val)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPropInfo {
    pub hwnd: u64,
    pub title: String,
    pub class_name: String,
    pub pid: u32,
    pub tid: u32,
    pub process_name: String,
    pub visible: bool,
    pub enabled: bool,
    pub topmost: bool,
    pub style: u64,
    pub ex_style: u64,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub parent_hwnd: u64,
    pub opacity: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WindowPropOp {
    Show,
    Hide,
    Enable,
    Disable,
    Topmost,
    Untopmost,
    Close,
    SetOpacity { opacity: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowControlResult {
    pub hwnd: u64,
    pub hwnd_hex: String,
    pub operation: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopWindowInfo {
    pub hwnd: u64,
    pub title: String,
    pub class_name: String,
    pub pid: u32,
    pub process_name: String,
    pub state: String,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub visible: bool,
    pub minimized: bool,
    pub maximized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WindowListerOp {
    Minimize,
    Maximize,
    Restore,
    Close,
    Focus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeWindowNode {
    pub hwnd: u64,
    pub parent_hwnd: u64,
    pub depth: u32,
    pub control_id: u32,
    pub class_name: String,
    pub text: String,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub visible: bool,
    pub enabled: bool,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TreeControlOp {
    SetText { text: String },
    Resize { width: i32, height: i32 },
    Move { x: i32, y: i32 },
    Show,
    Hide,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SysExportItem {
    pub source_hwnd: u64,
    pub control_class: String,
    pub process_name: String,
    pub row_index: u32,
    pub col_index: u32,
    pub column_title: String,
    pub item_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardFormatEntry {
    pub format_id: u32,
    pub format_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub text_preview: String,
    pub is_text: bool,
    pub raw_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardControlResult {
    pub operation: String,
    pub format_id: Option<u32>,
    pub size_bytes: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipHistEntry {
    pub entry_id: u64,
    pub captured_at: DateTime<Utc>,
    pub format_id: u32,
    pub format_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub text_preview: String,
    pub payload: Vec<u8>,
    pub source_app: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyEntry {
    pub source: String,
    pub hotkey: String,
    pub modifiers: String,
    pub vk_code: u32,
    pub status: String,
    pub owner_hint: String,
    pub lnk_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyProbeResult {
    pub vk_code: u32,
    pub modifiers: u32,
    pub available: bool,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyStateEntry {
    pub vk_code: u8,
    pub vk_name: String,
    pub scan_code: u32,
    pub key_label: String,
    pub pressed: bool,
    pub toggled: bool,
    pub extended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFontEntry {
    pub family: String,
    pub subfamily: String,
    pub full_name: String,
    pub postscript_name: String,
    pub font_format: String,
    pub weight_class: u16,
    pub width_class: u16,
    pub fs_type: u16,
    pub panose: String,
    pub version: String,
    pub source_path: String,
    pub face_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoPixelFormat {
    Rgb24,
    Bgra32,
    Yuy2,
    Nv12,
}

impl VideoPixelFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rgb24 => "RGB24",
            Self::Bgra32 => "BGRA32",
            Self::Yuy2 => "YUY2",
            Self::Nv12 => "NV12",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebcamDeviceInfo {
    pub device_index: u32,
    pub friendly_name: String,
    pub symbolic_link: String,
    pub resolution: String,
    pub pixel_format: String,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawVideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixel_format: VideoPixelFormat,
    pub data: Vec<u8>,
}

pub trait DesktopBackend: Send + Sync {
    fn list_window_props(&self) -> WinctlResult<Vec<WindowPropInfo>>;
    fn control_window_prop(&self, hwnd: u64, op: &WindowPropOp) -> WinctlResult<WindowControlResult>;

    fn list_top_windows(&self) -> WinctlResult<Vec<TopWindowInfo>>;
    fn control_top_window(&self, hwnd: u64, op: &WindowListerOp) -> WinctlResult<WindowControlResult>;

    fn list_window_tree(&self) -> WinctlResult<Vec<TreeWindowNode>>;
    fn control_tree_window(&self, hwnd: u64, op: &TreeControlOp) -> WinctlResult<WindowControlResult>;

    fn export_sys_controls(&self) -> WinctlResult<Vec<SysExportItem>>;

    fn inspect_clipboard(&self) -> WinctlResult<Vec<ClipboardFormatEntry>>;
    fn clear_clipboard(&self) -> WinctlResult<ClipboardControlResult>;
    fn set_clipboard_text(&self, text: &str) -> WinctlResult<ClipboardControlResult>;
    fn set_clipboard_payload(&self, format_id: u32, payload: &[u8]) -> WinctlResult<ClipboardControlResult>;
    fn monitor_clipboard_update(&self, timeout_ms: u32) -> WinctlResult<Option<ClipboardFormatEntry>>;

    fn default_clip_hist_db_path(&self) -> Option<PathBuf> {
        None
    }

    fn probe_hotkeys(&self) -> WinctlResult<Vec<HotkeyEntry>>;
    fn probe_single_hotkey(&self, modifiers: u32, vk_code: u32) -> WinctlResult<HotkeyProbeResult>;

    fn query_keyboard_state(&self) -> WinctlResult<Vec<KeyStateEntry>>;

    fn list_installed_fonts(&self) -> WinctlResult<Vec<InstalledFontEntry>>;

    fn list_webcam_devices(&self) -> WinctlResult<Vec<WebcamDeviceInfo>>;
    fn capture_webcam_frame(&self, device_index: u32) -> WinctlResult<RawVideoFrame>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedDesktopBackend;

impl DesktopBackend for UnsupportedDesktopBackend {
    fn list_window_props(&self) -> WinctlResult<Vec<WindowPropInfo>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "GUIPropView EnumWindows requires Windows".to_string(),
        ))
    }

    fn control_window_prop(&self, hwnd: u64, op: &WindowPropOp) -> WinctlResult<WindowControlResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "GUIPropView control ({op:?} on {hwnd:#x}) requires Windows"
        )))
    }

    fn list_top_windows(&self) -> WinctlResult<Vec<TopWindowInfo>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "WinLister EnumWindows requires Windows".to_string(),
        ))
    }

    fn control_top_window(&self, hwnd: u64, op: &WindowListerOp) -> WinctlResult<WindowControlResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "WinLister control ({op:?} on {hwnd:#x}) requires Windows"
        )))
    }

    fn list_window_tree(&self) -> WinctlResult<Vec<TreeWindowNode>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "WinExplorer EnumChildWindows requires Windows".to_string(),
        ))
    }

    fn control_tree_window(&self, hwnd: u64, op: &TreeControlOp) -> WinctlResult<WindowControlResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "WinExplorer control ({op:?} on {hwnd:#x}) requires Windows"
        )))
    }

    fn export_sys_controls(&self) -> WinctlResult<Vec<SysExportItem>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "SysExporter cross-process LVM_GETITEMW/TVM_GETITEMW requires Windows".to_string(),
        ))
    }

    fn inspect_clipboard(&self) -> WinctlResult<Vec<ClipboardFormatEntry>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "InsideClipboard OpenClipboard/EnumClipboardFormats requires Windows".to_string(),
        ))
    }

    fn clear_clipboard(&self) -> WinctlResult<ClipboardControlResult> {
        Err(WinctlError::UnsupportedOnPlatform(
            "InsideClipboard EmptyClipboard requires Windows".to_string(),
        ))
    }

    fn set_clipboard_text(&self, _text: &str) -> WinctlResult<ClipboardControlResult> {
        Err(WinctlError::UnsupportedOnPlatform(
            "InsideClipboard SetClipboardData requires Windows".to_string(),
        ))
    }

    fn set_clipboard_payload(&self, format_id: u32, _payload: &[u8]) -> WinctlResult<ClipboardControlResult> {
        Err(WinctlError::UnsupportedOnPlatform(format!(
            "SetClipboardData(format={format_id}) requires Windows"
        )))
    }

    fn monitor_clipboard_update(&self, _timeout_ms: u32) -> WinctlResult<Option<ClipboardFormatEntry>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "Clipboardic AddClipboardFormatListener requires Windows".to_string(),
        ))
    }

    fn probe_hotkeys(&self) -> WinctlResult<Vec<HotkeyEntry>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "HotKeysList RegisterHotKey probe requires Windows".to_string(),
        ))
    }

    fn probe_single_hotkey(&self, _modifiers: u32, _vk_code: u32) -> WinctlResult<HotkeyProbeResult> {
        Err(WinctlError::UnsupportedOnPlatform(
            "HotKeysList RegisterHotKey probe requires Windows".to_string(),
        ))
    }

    fn query_keyboard_state(&self) -> WinctlResult<Vec<KeyStateEntry>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "KeyboardStateView GetKeyboardState requires Windows".to_string(),
        ))
    }

    fn list_installed_fonts(&self) -> WinctlResult<Vec<InstalledFontEntry>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "WinFontsView EnumFontFamiliesExW requires Windows".to_string(),
        ))
    }

    fn list_webcam_devices(&self) -> WinctlResult<Vec<WebcamDeviceInfo>> {
        Err(WinctlError::UnsupportedOnPlatform(
            "WebCamImageSave Media Foundation MFEnumDeviceSources requires Windows".to_string(),
        ))
    }

    fn capture_webcam_frame(&self, _device_index: u32) -> WinctlResult<RawVideoFrame> {
        Err(WinctlError::UnsupportedOnPlatform(
            "WebCamImageSave Media Foundation IMFSourceReader requires Windows".to_string(),
        ))
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct MockDesktopBackend {
    pub window_props: std::sync::Mutex<Vec<WindowPropInfo>>,
    pub top_windows: std::sync::Mutex<Vec<TopWindowInfo>>,
    pub tree_nodes: std::sync::Mutex<Vec<TreeWindowNode>>,
    pub sysexport_items: std::sync::Mutex<Vec<SysExportItem>>,
    pub clipboard_entries: std::sync::Mutex<Vec<ClipboardFormatEntry>>,
    pub hotkeys: std::sync::Mutex<Vec<HotkeyEntry>>,
    pub key_states: std::sync::Mutex<Vec<KeyStateEntry>>,
    pub fonts: std::sync::Mutex<Vec<InstalledFontEntry>>,
    pub webcams: std::sync::Mutex<Vec<WebcamDeviceInfo>>,
    pub mock_frame: std::sync::Mutex<Option<RawVideoFrame>>,
    pub clip_hist_db_path: std::sync::Mutex<Option<PathBuf>>,
    pub action_log: std::sync::Mutex<Vec<String>>,
    pub inject_hardware_unavailable: std::sync::atomic::AtomicBool,
    pub inject_permission_denied: std::sync::atomic::AtomicBool,
}

#[cfg(any(test, feature = "test-support"))]
impl MockDesktopBackend {
    pub fn with_sample_data() -> Self {
        Self {
            window_props: std::sync::Mutex::new(crate::fixtures::sample_window_props()),
            top_windows: std::sync::Mutex::new(crate::fixtures::sample_top_windows()),
            tree_nodes: std::sync::Mutex::new(crate::fixtures::sample_tree_nodes()),
            sysexport_items: std::sync::Mutex::new(crate::fixtures::sample_sysexport_items()),
            clipboard_entries: std::sync::Mutex::new(crate::fixtures::sample_clipboard_entries()),
            hotkeys: std::sync::Mutex::new(crate::fixtures::sample_hotkeys()),
            key_states: std::sync::Mutex::new(crate::fixtures::sample_key_states()),
            fonts: std::sync::Mutex::new(crate::fixtures::sample_fonts()),
            webcams: std::sync::Mutex::new(crate::fixtures::sample_webcams()),
            mock_frame: std::sync::Mutex::new(Some(crate::fixtures::synthetic_video_frame_rgb24(4, 2))),
            ..Self::default()
        }
    }

    pub fn with_clip_hist_db_path(self, path: PathBuf) -> Self {
        *self.clip_hist_db_path.lock().unwrap() = Some(path);
        self
    }

    fn check_faults(&self) -> WinctlResult<()> {
        if self
            .inject_hardware_unavailable
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(WinctlError::HardwareUnavailable(
                "mock desktop hardware / capture source unavailable".to_string(),
            ));
        }
        if self.inject_permission_denied.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(WinctlError::PermissionDenied(
                "mock ERROR_ACCESS_DENIED (UIPI / privilege boundary)".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
impl DesktopBackend for MockDesktopBackend {
    fn list_window_props(&self) -> WinctlResult<Vec<WindowPropInfo>> {
        self.check_faults()?;
        Ok(self.window_props.lock().unwrap().clone())
    }

    fn control_window_prop(&self, hwnd: u64, op: &WindowPropOp) -> WinctlResult<WindowControlResult> {
        self.check_faults()?;
        let mut props = self.window_props.lock().unwrap();
        let idx = props
            .iter()
            .position(|w| w.hwnd == hwnd)
            .ok_or_else(|| WinctlError::InvalidInput(format!("window {} not found", format_hwnd(hwnd))))?;
        let op_name = match op {
            WindowPropOp::Show => {
                props[idx].visible = true;
                "show"
            }
            WindowPropOp::Hide => {
                props[idx].visible = false;
                "hide"
            }
            WindowPropOp::Enable => {
                props[idx].enabled = true;
                "enable"
            }
            WindowPropOp::Disable => {
                props[idx].enabled = false;
                "disable"
            }
            WindowPropOp::Topmost => {
                props[idx].topmost = true;
                "topmost"
            }
            WindowPropOp::Untopmost => {
                props[idx].topmost = false;
                "untopmost"
            }
            WindowPropOp::Close => {
                props.remove(idx);
                "close"
            }
            WindowPropOp::SetOpacity { opacity } => {
                props[idx].opacity = *opacity;
                "set_opacity"
            }
        };
        self.action_log
            .lock()
            .unwrap()
            .push(format!("props:{op_name}:{}", format_hwnd(hwnd)));
        Ok(WindowControlResult {
            hwnd,
            hwnd_hex: format_hwnd(hwnd),
            operation: op_name.to_string(),
            message: format!("Applied '{op_name}' to {}", format_hwnd(hwnd)),
        })
    }

    fn list_top_windows(&self) -> WinctlResult<Vec<TopWindowInfo>> {
        self.check_faults()?;
        Ok(self.top_windows.lock().unwrap().clone())
    }

    fn control_top_window(&self, hwnd: u64, op: &WindowListerOp) -> WinctlResult<WindowControlResult> {
        self.check_faults()?;
        let mut wins = self.top_windows.lock().unwrap();
        let idx = wins
            .iter()
            .position(|w| w.hwnd == hwnd)
            .ok_or_else(|| WinctlError::InvalidInput(format!("top window {} not found", format_hwnd(hwnd))))?;
        let op_name = match op {
            WindowListerOp::Minimize => {
                wins[idx].minimized = true;
                wins[idx].maximized = false;
                wins[idx].state = "Minimized".to_string();
                "minimize"
            }
            WindowListerOp::Maximize => {
                wins[idx].minimized = false;
                wins[idx].maximized = true;
                wins[idx].state = "Maximized".to_string();
                "maximize"
            }
            WindowListerOp::Restore => {
                wins[idx].minimized = false;
                wins[idx].maximized = false;
                wins[idx].state = "Normal".to_string();
                "restore"
            }
            WindowListerOp::Close => {
                wins.remove(idx);
                "close"
            }
            WindowListerOp::Focus => {
                wins[idx].visible = true;
                wins[idx].minimized = false;
                wins[idx].state = "Focused".to_string();
                "focus"
            }
        };
        self.action_log
            .lock()
            .unwrap()
            .push(format!("windows:{op_name}:{}", format_hwnd(hwnd)));
        Ok(WindowControlResult {
            hwnd,
            hwnd_hex: format_hwnd(hwnd),
            operation: op_name.to_string(),
            message: format!("Applied '{op_name}' to {}", format_hwnd(hwnd)),
        })
    }

    fn list_window_tree(&self) -> WinctlResult<Vec<TreeWindowNode>> {
        self.check_faults()?;
        Ok(self.tree_nodes.lock().unwrap().clone())
    }

    fn control_tree_window(&self, hwnd: u64, op: &TreeControlOp) -> WinctlResult<WindowControlResult> {
        self.check_faults()?;
        let mut nodes = self.tree_nodes.lock().unwrap();
        let node = nodes
            .iter_mut()
            .find(|n| n.hwnd == hwnd)
            .ok_or_else(|| WinctlError::InvalidInput(format!("tree node {} not found", format_hwnd(hwnd))))?;
        let op_name =
            match op {
                TreeControlOp::SetText { text } => {
                    node.text = text.clone();
                    "set_text"
                }
                TreeControlOp::Resize { width, height } => {
                    if *width <= 0 || *height <= 0 {
                        return Err(WinctlError::InvalidInput(
                            "width and height must be positive".to_string(),
                        ));
                    }
                    let new_right = node.left.checked_add(*width).ok_or_else(|| {
                        WinctlError::InvalidInput("resize width causes coordinate overflow".to_string())
                    })?;
                    let new_bottom = node.top.checked_add(*height).ok_or_else(|| {
                        WinctlError::InvalidInput("resize height causes coordinate overflow".to_string())
                    })?;
                    node.right = new_right;
                    node.bottom = new_bottom;
                    "resize"
                }
                TreeControlOp::Move { x, y } => {
                    let w = node
                        .right
                        .checked_sub(node.left)
                        .ok_or_else(|| WinctlError::InvalidInput("window width calculation overflow".to_string()))?;
                    let h = node
                        .bottom
                        .checked_sub(node.top)
                        .ok_or_else(|| WinctlError::InvalidInput("window height calculation overflow".to_string()))?;
                    let new_right = x
                        .checked_add(w)
                        .ok_or_else(|| WinctlError::InvalidInput("move x causes coordinate overflow".to_string()))?;
                    let new_bottom = y
                        .checked_add(h)
                        .ok_or_else(|| WinctlError::InvalidInput("move y causes coordinate overflow".to_string()))?;
                    node.left = *x;
                    node.top = *y;
                    node.right = new_right;
                    node.bottom = new_bottom;
                    "move"
                }
                TreeControlOp::Show => {
                    node.visible = true;
                    "show"
                }
                TreeControlOp::Hide => {
                    node.visible = false;
                    "hide"
                }
            };
        self.action_log
            .lock()
            .unwrap()
            .push(format!("tree:{op_name}:{}", format_hwnd(hwnd)));
        Ok(WindowControlResult {
            hwnd,
            hwnd_hex: format_hwnd(hwnd),
            operation: op_name.to_string(),
            message: format!("Applied '{op_name}' to {}", format_hwnd(hwnd)),
        })
    }

    fn export_sys_controls(&self) -> WinctlResult<Vec<SysExportItem>> {
        self.check_faults()?;
        Ok(self.sysexport_items.lock().unwrap().clone())
    }

    fn inspect_clipboard(&self) -> WinctlResult<Vec<ClipboardFormatEntry>> {
        self.check_faults()?;
        Ok(self.clipboard_entries.lock().unwrap().clone())
    }

    fn clear_clipboard(&self) -> WinctlResult<ClipboardControlResult> {
        self.check_faults()?;
        self.clipboard_entries.lock().unwrap().clear();
        self.action_log.lock().unwrap().push("clipboard:clear".to_string());
        Ok(ClipboardControlResult {
            operation: "clear".to_string(),
            format_id: None,
            size_bytes: 0,
            message: "Cleared clipboard".to_string(),
        })
    }

    fn set_clipboard_text(&self, text: &str) -> WinctlResult<ClipboardControlResult> {
        self.check_faults()?;
        let utf16_bytes: Vec<u8> = text
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .flat_map(u16::to_le_bytes)
            .collect();
        self.set_clipboard_payload(13, &utf16_bytes)
    }

    fn set_clipboard_payload(&self, format_id: u32, payload: &[u8]) -> WinctlResult<ClipboardControlResult> {
        self.check_faults()?;
        let entry = crate::clipboard::build_clipboard_entry(format_id, None, payload.to_vec());
        let size_bytes = entry.size_bytes;
        let mut entries = self.clipboard_entries.lock().unwrap();
        entries.clear();
        entries.push(entry);
        self.action_log
            .lock()
            .unwrap()
            .push(format!("clipboard:set:{format_id}:{size_bytes}"));
        Ok(ClipboardControlResult {
            operation: "set_payload".to_string(),
            format_id: Some(format_id),
            size_bytes,
            message: format!("Set clipboard format {format_id} ({size_bytes} bytes)"),
        })
    }

    fn monitor_clipboard_update(&self, _timeout_ms: u32) -> WinctlResult<Option<ClipboardFormatEntry>> {
        self.check_faults()?;
        Ok(self.clipboard_entries.lock().unwrap().first().cloned())
    }

    fn default_clip_hist_db_path(&self) -> Option<PathBuf> {
        self.clip_hist_db_path.lock().unwrap().clone()
    }

    fn probe_hotkeys(&self) -> WinctlResult<Vec<HotkeyEntry>> {
        self.check_faults()?;
        Ok(self.hotkeys.lock().unwrap().clone())
    }

    fn probe_single_hotkey(&self, modifiers: u32, vk_code: u32) -> WinctlResult<HotkeyProbeResult> {
        self.check_faults()?;
        let in_use = self
            .hotkeys
            .lock()
            .unwrap()
            .iter()
            .any(|h| h.vk_code == vk_code && h.status == "Registered");
        Ok(HotkeyProbeResult {
            vk_code,
            modifiers,
            available: !in_use,
            status: if in_use {
                "InUse".to_string()
            } else {
                "Available".to_string()
            },
        })
    }

    fn query_keyboard_state(&self) -> WinctlResult<Vec<KeyStateEntry>> {
        self.check_faults()?;
        Ok(self.key_states.lock().unwrap().clone())
    }

    fn list_installed_fonts(&self) -> WinctlResult<Vec<InstalledFontEntry>> {
        self.check_faults()?;
        Ok(self.fonts.lock().unwrap().clone())
    }

    fn list_webcam_devices(&self) -> WinctlResult<Vec<WebcamDeviceInfo>> {
        self.check_faults()?;
        Ok(self.webcams.lock().unwrap().clone())
    }

    fn capture_webcam_frame(&self, device_index: u32) -> WinctlResult<RawVideoFrame> {
        self.check_faults()?;
        let cams = self.webcams.lock().unwrap();
        if !cams.iter().any(|c| c.device_index == device_index && c.available) {
            return Err(WinctlError::HardwareUnavailable(format!(
                "webcam device index {device_index} is not available"
            )));
        }
        let frame = self
            .mock_frame
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| WinctlError::HardwareUnavailable("no mock webcam frame configured".to_string()))?;
        self.action_log
            .lock()
            .unwrap()
            .push(format!("webcam:capture:{device_index}"));
        Ok(frame)
    }
}
