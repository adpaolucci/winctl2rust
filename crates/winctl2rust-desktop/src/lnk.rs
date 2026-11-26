use std::path::Path;

use serde::{Deserialize, Serialize};
use winctl2rust_core::{WinctlError, WinctlResult};

pub const SHELL_LINK_HEADER_SIZE: u32 = 0x0000_004C;
pub const SHELL_LINK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

pub const FLAG_HAS_LINK_TARGET_ID_LIST: u32 = 0x0000_0001;
pub const FLAG_HAS_LINK_INFO: u32 = 0x0000_0002;
pub const FLAG_HAS_NAME: u32 = 0x0000_0004;
pub const FLAG_HAS_RELATIVE_PATH: u32 = 0x0000_0008;
pub const FLAG_HAS_WORKING_DIR: u32 = 0x0000_0010;
pub const FLAG_HAS_ARGUMENTS: u32 = 0x0000_0020;
pub const FLAG_HAS_ICON_LOCATION: u32 = 0x0000_0040;
pub const FLAG_IS_UNICODE: u32 = 0x0000_0080;
pub const FLAG_FORCE_NO_LINK_INFO: u32 = 0x0000_0100;

pub const HOTKEYF_SHIFT: u8 = 0x01;
pub const HOTKEYF_CONTROL: u8 = 0x02;
pub const HOTKEYF_ALT: u8 = 0x04;
pub const HOTKEYF_EXT: u8 = 0x08;

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
pub const MOD_NOREPEAT: u32 = 0x4000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellLinkInfo {
    pub link_flags: u32,
    pub show_command: u32,
    pub hot_key_raw: u16,
    pub vk_code: u8,
    pub hotkeyf_modifiers: u8,
    pub win32_modifiers: u32,
    pub hotkey_label: String,
    pub modifiers_label: String,
    pub target_path: Option<String>,
    pub name_string: Option<String>,
    pub relative_path: Option<String>,
    pub working_dir: Option<String>,
    pub command_line_arguments: Option<String>,
    pub icon_location: Option<String>,
    pub owner_hint: String,
}

pub fn hotkeyf_to_win32_modifiers(hotkeyf: u8) -> u32 {
    let mut mods = 0u32;
    if (hotkeyf & HOTKEYF_ALT) != 0 {
        mods |= MOD_ALT;
    }
    if (hotkeyf & HOTKEYF_CONTROL) != 0 {
        mods |= MOD_CONTROL;
    }
    if (hotkeyf & HOTKEYF_SHIFT) != 0 {
        mods |= MOD_SHIFT;
    }
    mods
}

pub fn format_hotkeyf_modifiers(hotkeyf: u8) -> String {
    let mut parts = Vec::new();
    if (hotkeyf & HOTKEYF_CONTROL) != 0 {
        parts.push("Ctrl");
    }
    if (hotkeyf & HOTKEYF_ALT) != 0 {
        parts.push("Alt");
    }
    if (hotkeyf & HOTKEYF_SHIFT) != 0 {
        parts.push("Shift");
    }
    if (hotkeyf & HOTKEYF_EXT) != 0 {
        parts.push("Ext");
    }
    if parts.is_empty() {
        "None".to_string()
    } else {
        parts.join("+")
    }
}

pub fn format_win32_modifiers(mods: u32) -> String {
    let mut parts = Vec::new();
    if (mods & MOD_CONTROL) != 0 {
        parts.push("Ctrl");
    }
    if (mods & MOD_ALT) != 0 {
        parts.push("Alt");
    }
    if (mods & MOD_SHIFT) != 0 {
        parts.push("Shift");
    }
    if (mods & MOD_WIN) != 0 {
        parts.push("Win");
    }
    if parts.is_empty() {
        "None".to_string()
    } else {
        parts.join("+")
    }
}

pub fn vk_short_label(vk: u8) -> String {
    match vk {
        0 => "None".to_string(),
        0x08 => "Backspace".to_string(),
        0x09 => "Tab".to_string(),
        0x0D => "Enter".to_string(),
        0x13 => "Pause".to_string(),
        0x1B => "Esc".to_string(),
        0x20 => "Space".to_string(),
        0x21 => "PageUp".to_string(),
        0x22 => "PageDown".to_string(),
        0x23 => "End".to_string(),
        0x24 => "Home".to_string(),
        0x25 => "Left".to_string(),
        0x26 => "Up".to_string(),
        0x27 => "Right".to_string(),
        0x28 => "Down".to_string(),
        0x2C => "PrintScreen".to_string(),
        0x2D => "Insert".to_string(),
        0x2E => "Delete".to_string(),
        b'0'..=b'9' | b'A'..=b'Z' => (vk as char).to_string(),
        0x60..=0x69 => format!("Num{}", vk - 0x60),
        0x6A => "Num*".to_string(),
        0x6B => "Num+".to_string(),
        0x6D => "Num-".to_string(),
        0x6E => "Num.".to_string(),
        0x6F => "Num/".to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0x90 => "NumLock".to_string(),
        0x91 => "ScrollLock".to_string(),
        other => format!("VK_{other:02X}"),
    }
}

pub fn format_hotkey_flags(vk: u8, hotkeyf: u8) -> String {
    if vk == 0 {
        return "None".to_string();
    }
    let key = vk_short_label(vk);
    let mods = format_hotkeyf_modifiers(hotkeyf);
    if mods == "None" { key } else { format!("{mods}+{key}") }
}

pub fn parse_lnk_file(path: &Path) -> WinctlResult<ShellLinkInfo> {
    let bytes = std::fs::read(path)?;
    let file_hint = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    parse_lnk_bytes(&bytes, Some(&file_hint))
}

pub fn parse_lnk_bytes(bytes: &[u8], fallback_name: Option<&str>) -> WinctlResult<ShellLinkInfo> {
    if bytes.len() < 76 {
        return Err(WinctlError::ParseError(format!(
            "MS-SHLLINK truncated header: {} bytes (need >= 76)",
            bytes.len()
        )));
    }
    let header_size = read_u32_le(bytes, 0)?;
    if header_size != SHELL_LINK_HEADER_SIZE {
        return Err(WinctlError::ParseError(format!(
            "invalid MS-SHLLINK HeaderSize {header_size:#010x} (expected {SHELL_LINK_HEADER_SIZE:#010x})"
        )));
    }
    if bytes[4..20] != SHELL_LINK_CLSID {
        return Err(WinctlError::ParseError(
            "invalid MS-SHLLINK LinkCLSID (expected {00021401-0000-0000-C000-000000000046})".to_string(),
        ));
    }

    let link_flags = read_u32_le(bytes, 0x14)?;
    let show_command = read_u32_le(bytes, 0x3C)?;
    let hot_key_raw = read_u16_le(bytes, 0x40)?;
    let vk_code = (hot_key_raw & 0x00FF) as u8;
    let hotkeyf_modifiers = ((hot_key_raw >> 8) & 0x00FF) as u8;
    let win32_modifiers = hotkeyf_to_win32_modifiers(hotkeyf_modifiers);
    let modifiers_label = format_hotkeyf_modifiers(hotkeyf_modifiers);
    let hotkey_label = format_hotkey_flags(vk_code, hotkeyf_modifiers);

    let mut cursor: usize = 0x4C;

    // 1. Optional LinkTargetIDList
    if (link_flags & FLAG_HAS_LINK_TARGET_ID_LIST) != 0 {
        let id_list_size = read_u16_le(bytes, cursor)? as usize;
        let end = cursor
            .checked_add(2)
            .and_then(|v| v.checked_add(id_list_size))
            .ok_or_else(|| WinctlError::ParseError("MS-SHLLINK LinkTargetIDList length overflow".to_string()))?;
        if end > bytes.len() {
            return Err(WinctlError::ParseError(format!(
                "MS-SHLLINK LinkTargetIDList exceeds buffer ({end} > {})",
                bytes.len()
            )));
        }
        cursor = end;
    }

    // 2. Optional LinkInfo (physically present whenever HAS_LINK_INFO is set; ForceNoLinkInfo suppresses semantic use)
    let mut target_path: Option<String> = None;
    if (link_flags & FLAG_HAS_LINK_INFO) != 0 {
        let info_size = read_u32_le(bytes, cursor)? as usize;
        if info_size < 0x1C {
            return Err(WinctlError::ParseError(format!(
                "MS-SHLLINK LinkInfoSize {info_size} < 28"
            )));
        }
        let end = cursor
            .checked_add(info_size)
            .ok_or_else(|| WinctlError::ParseError("MS-SHLLINK LinkInfoSize overflow".to_string()))?;
        if end > bytes.len() {
            return Err(WinctlError::ParseError(format!(
                "MS-SHLLINK LinkInfo exceeds buffer ({end} > {})",
                bytes.len()
            )));
        }
        let info_slice = &bytes[cursor..end];
        if (link_flags & FLAG_FORCE_NO_LINK_INFO) == 0 {
            target_path = parse_link_info(info_slice)?;
        }
        cursor = end;
    }

    // 3. Optional StringData in strict MS-SHLLINK §2.4 order
    let is_unicode = (link_flags & FLAG_IS_UNICODE) != 0;
    let mut name_string = None;
    let mut relative_path = None;
    let mut working_dir = None;
    let mut command_line_arguments = None;
    let mut icon_location = None;

    if (link_flags & FLAG_HAS_NAME) != 0 {
        name_string = Some(read_string_data(bytes, &mut cursor, is_unicode, "NAME_STRING")?);
    }
    if (link_flags & FLAG_HAS_RELATIVE_PATH) != 0 {
        relative_path = Some(read_string_data(bytes, &mut cursor, is_unicode, "RELATIVE_PATH")?);
    }
    if (link_flags & FLAG_HAS_WORKING_DIR) != 0 {
        working_dir = Some(read_string_data(bytes, &mut cursor, is_unicode, "WORKING_DIR")?);
    }
    if (link_flags & FLAG_HAS_ARGUMENTS) != 0 {
        command_line_arguments = Some(read_string_data(
            bytes,
            &mut cursor,
            is_unicode,
            "COMMAND_LINE_ARGUMENTS",
        )?);
    }
    if (link_flags & FLAG_HAS_ICON_LOCATION) != 0 {
        icon_location = Some(read_string_data(bytes, &mut cursor, is_unicode, "ICON_LOCATION")?);
    }

    let base_hint = target_path
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| relative_path.as_deref().filter(|s| !s.is_empty()))
        .or_else(|| name_string.as_deref().filter(|s| !s.is_empty()))
        .or_else(|| working_dir.as_deref().filter(|s| !s.is_empty()))
        .or(fallback_name)
        .unwrap_or("")
        .to_string();

    let owner_hint = match command_line_arguments
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(args) if !base_hint.is_empty() => format!("{base_hint} {args}"),
        _ => base_hint,
    };

    Ok(ShellLinkInfo {
        link_flags,
        show_command,
        hot_key_raw,
        vk_code,
        hotkeyf_modifiers,
        win32_modifiers,
        hotkey_label,
        modifiers_label,
        target_path,
        name_string,
        relative_path,
        working_dir,
        command_line_arguments,
        icon_location,
        owner_hint,
    })
}

fn parse_link_info(info: &[u8]) -> WinctlResult<Option<String>> {
    if info.len() < 0x1C {
        return Err(WinctlError::ParseError("LinkInfo smaller than 28 bytes".to_string()));
    }
    let header_size = read_u32_le(info, 4)? as usize;
    if header_size < 0x1C || header_size > info.len() {
        return Err(WinctlError::ParseError(format!(
            "invalid LinkInfoHeaderSize {header_size} for {} byte LinkInfo",
            info.len()
        )));
    }
    let info_flags = read_u32_le(info, 8)?;
    let local_base_off = read_u32_le(info, 16)? as usize;
    let net_off = read_u32_le(info, 20)? as usize;
    let suffix_off = read_u32_le(info, 24)? as usize;

    let (local_uni_off, suffix_uni_off) = if header_size >= 0x24 && info.len() >= 36 {
        (
            Some(read_u32_le(info, 28)? as usize),
            Some(read_u32_le(info, 32)? as usize),
        )
    } else {
        (None, None)
    };

    if (info_flags & 0x01) != 0 {
        let base = if let Some(uoff) = local_uni_off.filter(|&o| o > 0 && o < info.len()) {
            read_cwstr_bounded(info, uoff)
        } else if local_base_off > 0 && local_base_off < info.len() {
            read_cstr_bounded(info, local_base_off)
        } else {
            String::new()
        };

        let suffix = if let Some(uoff) = suffix_uni_off.filter(|&o| o > 0 && o < info.len()) {
            read_cwstr_bounded(info, uoff)
        } else if suffix_off > 0 && suffix_off < info.len() {
            read_cstr_bounded(info, suffix_off)
        } else {
            String::new()
        };

        if !base.is_empty() || !suffix.is_empty() {
            return Ok(Some(if suffix.is_empty() {
                base
            } else if base.is_empty() {
                suffix
            } else {
                format!("{base}{suffix}")
            }));
        }
    }

    if (info_flags & 0x02) != 0 && net_off >= 0x1C && net_off + 20 <= info.len() {
        let net_name_off = read_u32_le(info, net_off + 8)? as usize;
        let share = if net_name_off > 0x14 && net_off + 24 <= info.len() {
            let uni_rel = read_u32_le(info, net_off + 20)? as usize;
            let abs = net_off.saturating_add(uni_rel);
            if uni_rel > 0 && abs < info.len() {
                read_cwstr_bounded(info, abs)
            } else {
                read_cstr_bounded(info, net_off.saturating_add(net_name_off))
            }
        } else if net_name_off > 0 && net_off.saturating_add(net_name_off) < info.len() {
            read_cstr_bounded(info, net_off + net_name_off)
        } else {
            String::new()
        };
        if !share.is_empty() {
            return Ok(Some(share));
        }
    }

    Ok(None)
}

fn read_string_data(bytes: &[u8], cursor: &mut usize, is_unicode: bool, label: &str) -> WinctlResult<String> {
    let char_count = read_u16_le(bytes, *cursor)? as usize;
    let start = cursor
        .checked_add(2)
        .ok_or_else(|| WinctlError::ParseError(format!("MS-SHLLINK {label} offset overflow")))?;
    let byte_len = if is_unicode {
        char_count
            .checked_mul(2)
            .ok_or_else(|| WinctlError::ParseError(format!("MS-SHLLINK {label} byte length overflow")))?
    } else {
        char_count
    };
    let end = start
        .checked_add(byte_len)
        .ok_or_else(|| WinctlError::ParseError(format!("MS-SHLLINK {label} end overflow")))?;
    if end > bytes.len() {
        return Err(WinctlError::ParseError(format!(
            "MS-SHLLINK {label} truncated ({end} > {})",
            bytes.len()
        )));
    }
    let slice = &bytes[start..end];
    *cursor = end;
    if is_unicode {
        let units: Vec<u16> = slice
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        Ok(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
    } else {
        Ok(decode_cp1252(slice).trim_end_matches('\0').to_string())
    }
}

fn read_u16_le(bytes: &[u8], offset: usize) -> WinctlResult<u16> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| WinctlError::ParseError("offset overflow reading u16".to_string()))?;
    if end > bytes.len() {
        return Err(WinctlError::ParseError(format!(
            "unexpected EOF reading u16 at {offset} (len {})",
            bytes.len()
        )));
    }
    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> WinctlResult<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| WinctlError::ParseError("offset overflow reading u32".to_string()))?;
    if end > bytes.len() {
        return Err(WinctlError::ParseError(format!(
            "unexpected EOF reading u32 at {offset} (len {})",
            bytes.len()
        )));
    }
    Ok(u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

fn read_cstr_bounded(bytes: &[u8], start: usize) -> String {
    if start >= bytes.len() {
        return String::new();
    }
    let slice = &bytes[start..];
    let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    decode_cp1252(&slice[..len])
}

fn read_cwstr_bounded(bytes: &[u8], start: usize) -> String {
    if start >= bytes.len() {
        return String::new();
    }
    let mut units = Vec::new();
    for chunk in bytes[start..].as_chunks::<2>().0 {
        let u = u16::from_le_bytes(*chunk);
        if u == 0 {
            break;
        }
        units.push(u);
    }
    String::from_utf16_lossy(&units)
}

pub fn decode_cp1252(bytes: &[u8]) -> String {
    const HIGH_CP1252: [char; 32] = [
        '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{02C6}',
        '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}', '\u{017D}', '\u{FFFD}', '\u{FFFD}', '\u{2018}',
        '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}',
        '\u{203A}', '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
    ];
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            0x00..=0x7F => out.push(b as char),
            0x80..=0x9F => out.push(HIGH_CP1252[(b - 0x80) as usize]),
            0xA0..=0xFF => out.push(char::from(b)),
        }
    }
    out
}
