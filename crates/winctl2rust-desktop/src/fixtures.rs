use crate::backend::{
    ClipboardFormatEntry, CtrlExportItem, HotkeyEntry, InstalledFontEntry, KeyStateEntry, RawVideoFrame, TopWindowInfo,
    TreeWindowNode, VideoPixelFormat, WebcamDeviceInfo, WindowPropInfo,
};
use crate::clipboard::build_clipboard_entry;
use crate::lnk::{
    FLAG_HAS_ARGUMENTS, FLAG_HAS_LINK_INFO, FLAG_HAS_LINK_TARGET_ID_LIST, FLAG_HAS_NAME, FLAG_HAS_RELATIVE_PATH,
    FLAG_HAS_WORKING_DIR, FLAG_IS_UNICODE, SHELL_LINK_CLSID, SHELL_LINK_HEADER_SIZE,
};
use crate::opentype::calc_sfnt_checksum;

pub fn sample_window_props() -> Vec<WindowPropInfo> {
    vec![
        WindowPropInfo {
            hwnd: 0x0001_0420,
            title: "GUAP Workspace — Editor".to_string(),
            class_name: "GuapEditorFrame".to_string(),
            pid: 4120,
            tid: 4124,
            process_name: "guap_editor.exe".to_string(),
            visible: true,
            enabled: true,
            topmost: false,
            style: 0x14CF_0000,
            ex_style: 0x0004_0100,
            left: 100,
            top: 80,
            right: 1380,
            bottom: 860,
            parent_hwnd: 0,
            opacity: 255,
        },
        WindowPropInfo {
            hwnd: 0x0002_0810,
            title: "Diagnostics Panel".to_string(),
            class_name: "Notepad".to_string(),
            pid: 8824,
            tid: 8828,
            process_name: "notepad.exe".to_string(),
            visible: true,
            enabled: true,
            topmost: true,
            style: 0x14CF_0000,
            ex_style: 0x0008_0008,
            left: 40,
            top: 40,
            right: 760,
            bottom: 540,
            parent_hwnd: 0,
            opacity: 240,
        },
    ]
}

pub fn sample_top_windows() -> Vec<TopWindowInfo> {
    vec![
        TopWindowInfo {
            hwnd: 0x0001_0420,
            title: "GUAP Workspace — Editor".to_string(),
            class_name: "GuapEditorFrame".to_string(),
            pid: 4120,
            process_name: "guap_editor.exe".to_string(),
            state: "Normal".to_string(),
            left: 100,
            top: 80,
            right: 1380,
            bottom: 860,
            visible: true,
            minimized: false,
            maximized: false,
        },
        TopWindowInfo {
            hwnd: 0x0002_0810,
            title: "Diagnostics Panel".to_string(),
            class_name: "Notepad".to_string(),
            pid: 8824,
            process_name: "notepad.exe".to_string(),
            state: "Maximized".to_string(),
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
            visible: true,
            minimized: false,
            maximized: true,
        },
    ]
}

pub fn sample_tree_nodes() -> Vec<TreeWindowNode> {
    vec![
        TreeWindowNode {
            hwnd: 0x0001_0420,
            parent_hwnd: 0,
            depth: 0,
            control_id: 0,
            class_name: "GuapEditorFrame".to_string(),
            text: "GUAP Workspace — Editor".to_string(),
            left: 100,
            top: 80,
            right: 1380,
            bottom: 860,
            visible: true,
            enabled: true,
            pid: 4120,
        },
        TreeWindowNode {
            hwnd: 0x0001_0421,
            parent_hwnd: 0x0001_0420,
            depth: 1,
            control_id: 101,
            class_name: "SysListView32".to_string(),
            text: "Items".to_string(),
            left: 112,
            top: 120,
            right: 780,
            bottom: 820,
            visible: true,
            enabled: true,
            pid: 4120,
        },
        TreeWindowNode {
            hwnd: 0x0001_0422,
            parent_hwnd: 0x0001_0420,
            depth: 1,
            control_id: 102,
            class_name: "Edit".to_string(),
            text: "Filter...".to_string(),
            left: 112,
            top: 90,
            right: 512,
            bottom: 114,
            visible: true,
            enabled: true,
            pid: 4120,
        },
    ]
}

pub fn sample_ctrl_export_items() -> Vec<CtrlExportItem> {
    vec![
        CtrlExportItem {
            source_hwnd: 0x0001_0421,
            control_class: "SysListView32".to_string(),
            process_name: "guap_editor.exe".to_string(),
            row_index: 0,
            col_index: 0,
            column_title: "Service".to_string(),
            item_text: "Winmgmt".to_string(),
        },
        CtrlExportItem {
            source_hwnd: 0x0001_0421,
            control_class: "SysListView32".to_string(),
            process_name: "guap_editor.exe".to_string(),
            row_index: 0,
            col_index: 1,
            column_title: "Status".to_string(),
            item_text: "Running".to_string(),
        },
    ]
}

pub fn sample_clipboard_entries() -> Vec<ClipboardFormatEntry> {
    let utf16_bytes: Vec<u8> = "Hello GUAP Desktop"
        .encode_utf16()
        .chain(std::iter::once(0u16))
        .flat_map(u16::to_le_bytes)
        .collect();
    vec![
        build_clipboard_entry(13, None, utf16_bytes),
        build_clipboard_entry(1, None, b"Hello GUAP Desktop\0".to_vec()),
    ]
}

pub fn sample_hotkeys() -> Vec<HotkeyEntry> {
    vec![
        HotkeyEntry {
            source: "RegisterHotKeyProbe".to_string(),
            hotkey: "PrintScreen".to_string(),
            modifiers: "None".to_string(),
            vk_code: 0x2C,
            status: "Registered".to_string(),
            owner_hint: "Global system/shell binding".to_string(),
            lnk_path: String::new(),
        },
        HotkeyEntry {
            source: "LnkShortcut".to_string(),
            hotkey: "Ctrl+Shift+K".to_string(),
            modifiers: "Ctrl+Shift".to_string(),
            vk_code: 0x4B,
            status: "ShortcutBound".to_string(),
            owner_hint: "C:\\Tools\\guap.exe --status".to_string(),
            lnk_path: "C:\\Users\\Public\\Desktop\\GUAP.lnk".to_string(),
        },
    ]
}

pub fn sample_key_states() -> Vec<KeyStateEntry> {
    vec![
        KeyStateEntry {
            vk_code: 0x14,
            vk_name: "VK_CAPITAL".to_string(),
            scan_code: 0x3A,
            key_label: "Caps Lock".to_string(),
            pressed: false,
            toggled: true,
            extended: false,
        },
        KeyStateEntry {
            vk_code: 0x90,
            vk_name: "VK_NUMLOCK".to_string(),
            scan_code: 0x45,
            key_label: "Num Lock".to_string(),
            pressed: false,
            toggled: true,
            extended: true,
        },
    ]
}

pub fn sample_fonts() -> Vec<InstalledFontEntry> {
    vec![InstalledFontEntry {
        family: "GUAP Sans".to_string(),
        subfamily: "Bold".to_string(),
        full_name: "GUAP Sans Bold".to_string(),
        postscript_name: "GuapSans-Bold".to_string(),
        font_format: "TrueType".to_string(),
        weight_class: 700,
        width_class: 5,
        fs_type: 0x0000,
        panose: "02-0B-08-03-02-02-02-02-02-04".to_string(),
        version: "Version 1.00".to_string(),
        source_path: "C:\\Windows\\Fonts\\GuapSans-Bold.ttf".to_string(),
        face_index: 0,
    }]
}

pub fn sample_webcams() -> Vec<WebcamDeviceInfo> {
    vec![WebcamDeviceInfo {
        device_index: 0,
        friendly_name: "GUAP Virtual HD Camera".to_string(),
        symbolic_link: "\\\\?\\usb#vid_046d&pid_0825#0001#{e5323777-f976-4f5b-9b55-b94699c46e44}\\global".to_string(),
        resolution: "1280x720".to_string(),
        pixel_format: "RGB24".to_string(),
        available: true,
    }]
}

pub fn synthetic_video_frame_rgb24(width: u32, height: u32) -> RawVideoFrame {
    let mut data = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            data.push(((x * 40) % 256) as u8);
            data.push(((y * 80) % 256) as u8);
            data.push((((x + y) * 30) % 256) as u8);
        }
    }
    RawVideoFrame {
        width,
        height,
        pixel_format: VideoPixelFormat::Rgb24,
        data,
    }
}

pub fn synthetic_video_frame_bgra32(width: u32, height: u32) -> RawVideoFrame {
    let rgb = synthetic_video_frame_rgb24(width, height);
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for px in rgb.data.as_chunks::<3>().0 {
        data.push(px[2]); // B
        data.push(px[1]); // G
        data.push(px[0]); // R
        data.push(255); // A
    }
    RawVideoFrame {
        width,
        height,
        pixel_format: VideoPixelFormat::Bgra32,
        data,
    }
}

pub fn synthetic_video_frame_yuy2(width: u32, height: u32) -> RawVideoFrame {
    let mut data = Vec::with_capacity((width * height * 2) as usize);
    for _y in 0..height {
        for _x in (0..width).step_by(2) {
            // Neutral bright gray-green Y=128, U=128, V=128 -> R~130, G~130, B~130
            data.extend_from_slice(&[200, 128, 100, 128]);
        }
    }
    RawVideoFrame {
        width,
        height,
        pixel_format: VideoPixelFormat::Yuy2,
        data,
    }
}

pub fn synthetic_video_frame_nv12(width: u32, height: u32) -> RawVideoFrame {
    let y_len = (width * height) as usize;
    let uv_len = y_len / 2;
    let mut data = vec![160u8; y_len];
    data.extend(std::iter::repeat_n(128u8, uv_len));
    RawVideoFrame {
        width,
        height,
        pixel_format: VideoPixelFormat::Nv12,
        data,
    }
}

fn encode_utf16le_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn push_string_data(buf: &mut Vec<u8>, s: &str, unicode: bool) {
    if unicode {
        let units: Vec<u16> = s.encode_utf16().collect();
        buf.extend_from_slice(&(units.len() as u16).to_le_bytes());
        for u in units {
            buf.extend_from_slice(&u.to_le_bytes());
        }
    } else {
        let bytes = s.as_bytes();
        buf.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(bytes);
    }
}

pub struct SyntheticLnkSpec<'a> {
    pub hot_key_raw: u16,
    pub unicode: bool,
    pub include_id_list: bool,
    pub include_link_info: bool,
    pub force_no_link_info: bool,
    pub local_base_path: Option<&'a str>,
    pub name: Option<&'a str>,
    pub relative_path: Option<&'a str>,
    pub working_dir: Option<&'a str>,
    pub arguments: Option<&'a str>,
}

pub fn build_synthetic_lnk(spec: &SyntheticLnkSpec<'_>) -> Vec<u8> {
    let mut flags = 0u32;
    if spec.include_id_list {
        flags |= FLAG_HAS_LINK_TARGET_ID_LIST;
    }
    if spec.include_link_info {
        flags |= FLAG_HAS_LINK_INFO;
    }
    if spec.force_no_link_info {
        flags |= 0x0000_0100; // FLAG_FORCE_NO_LINK_INFO
    }
    if spec.name.is_some() {
        flags |= FLAG_HAS_NAME;
    }
    if spec.relative_path.is_some() {
        flags |= FLAG_HAS_RELATIVE_PATH;
    }
    if spec.working_dir.is_some() {
        flags |= FLAG_HAS_WORKING_DIR;
    }
    if spec.arguments.is_some() {
        flags |= FLAG_HAS_ARGUMENTS;
    }
    if spec.unicode {
        flags |= FLAG_IS_UNICODE;
    }

    let mut out = Vec::new();
    out.extend_from_slice(&SHELL_LINK_HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&SHELL_LINK_CLSID);
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // FileAttributes
    out.extend_from_slice(&0u64.to_le_bytes()); // CreationTime
    out.extend_from_slice(&0u64.to_le_bytes()); // AccessTime
    out.extend_from_slice(&0u64.to_le_bytes()); // WriteTime
    out.extend_from_slice(&0u32.to_le_bytes()); // FileSize
    out.extend_from_slice(&0i32.to_le_bytes()); // IconIndex
    out.extend_from_slice(&1u32.to_le_bytes()); // ShowCommand = SW_SHOWNORMAL
    out.extend_from_slice(&spec.hot_key_raw.to_le_bytes());
    out.extend_from_slice(&[0u8; 10]); // Reserved1..3

    if spec.include_id_list {
        // Minimal ItemIDList: 2-byte size (6) + 4-byte item + 2-byte terminal 0
        out.extend_from_slice(&6u16.to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&[0x1F, 0x50]);
        out.extend_from_slice(&0u16.to_le_bytes());
    }

    if spec.include_link_info {
        let base = spec.local_base_path.unwrap_or("C:\\Tools\\app.exe");
        let mut base_bytes = base.as_bytes().to_vec();
        base_bytes.push(0);
        let mut suffix_bytes = vec![0u8]; // empty common suffix NUL
        let header_size = 0x1Cu32;
        let vol_offset = 0x1Cu32;
        let vol_table = [16u32.to_le_bytes().as_slice(), &[0u8; 12]].concat();
        let local_base_off = vol_offset + vol_table.len() as u32;
        let suffix_off = local_base_off + base_bytes.len() as u32;
        let total_size = suffix_off + suffix_bytes.len() as u32;

        out.extend_from_slice(&total_size.to_le_bytes());
        out.extend_from_slice(&header_size.to_le_bytes());
        out.extend_from_slice(&0x01u32.to_le_bytes()); // VolumeIDAndLocalBasePath
        out.extend_from_slice(&vol_offset.to_le_bytes());
        out.extend_from_slice(&local_base_off.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // CommonNetworkRelativeLinkOffset
        out.extend_from_slice(&suffix_off.to_le_bytes());
        out.extend_from_slice(&vol_table);
        out.extend_from_slice(&base_bytes);
        out.append(&mut suffix_bytes);
    }

    if let Some(s) = spec.name {
        push_string_data(&mut out, s, spec.unicode);
    }
    if let Some(s) = spec.relative_path {
        push_string_data(&mut out, s, spec.unicode);
    }
    if let Some(s) = spec.working_dir {
        push_string_data(&mut out, s, spec.unicode);
    }
    if let Some(s) = spec.arguments {
        push_string_data(&mut out, s, spec.unicode);
    }

    out
}

pub fn build_synthetic_name_table_format0(records: &[(u16, u16, u16, u16, Vec<u8>)]) -> Vec<u8> {
    let storage_offset = 6 + records.len() * 12;
    let mut table = Vec::new();
    table.extend_from_slice(&0u16.to_be_bytes()); // format 0
    table.extend_from_slice(&(records.len() as u16).to_be_bytes());
    table.extend_from_slice(&(storage_offset as u16).to_be_bytes());

    let mut string_blob = Vec::new();
    for (plat, enc, lang, name_id, raw_str) in records {
        let off = string_blob.len() as u16;
        let len = raw_str.len() as u16;
        string_blob.extend_from_slice(raw_str);
        table.extend_from_slice(&plat.to_be_bytes());
        table.extend_from_slice(&enc.to_be_bytes());
        table.extend_from_slice(&lang.to_be_bytes());
        table.extend_from_slice(&name_id.to_be_bytes());
        table.extend_from_slice(&len.to_be_bytes());
        table.extend_from_slice(&off.to_be_bytes());
    }
    table.extend_from_slice(&string_blob);
    table
}

pub fn build_synthetic_name_table_format1(records: &[(u16, u16, u16, u16, Vec<u8>)], lang_tags: &[&str]) -> Vec<u8> {
    let storage_offset = 6 + records.len() * 12 + 2 + lang_tags.len() * 4;
    let mut table = Vec::new();
    table.extend_from_slice(&1u16.to_be_bytes()); // format 1
    table.extend_from_slice(&(records.len() as u16).to_be_bytes());
    table.extend_from_slice(&(storage_offset as u16).to_be_bytes());

    let mut string_blob = Vec::new();
    for (plat, enc, lang, name_id, raw_str) in records {
        let off = string_blob.len() as u16;
        let len = raw_str.len() as u16;
        string_blob.extend_from_slice(raw_str);
        table.extend_from_slice(&plat.to_be_bytes());
        table.extend_from_slice(&enc.to_be_bytes());
        table.extend_from_slice(&lang.to_be_bytes());
        table.extend_from_slice(&name_id.to_be_bytes());
        table.extend_from_slice(&len.to_be_bytes());
        table.extend_from_slice(&off.to_be_bytes());
    }
    table.extend_from_slice(&(lang_tags.len() as u16).to_be_bytes());
    for tag in lang_tags {
        let encoded: Vec<u8> = tag.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let off = string_blob.len() as u16;
        let len = encoded.len() as u16;
        string_blob.extend_from_slice(&encoded);
        table.extend_from_slice(&len.to_be_bytes());
        table.extend_from_slice(&off.to_be_bytes());
    }
    table.extend_from_slice(&string_blob);
    table
}

pub fn build_synthetic_os2_v4(weight: u16, width: u16, fs_type: u16, fs_selection: u16, vend_id: [u8; 4]) -> Vec<u8> {
    let mut buf = vec![0u8; 96];
    buf[0..2].copy_from_slice(&4u16.to_be_bytes()); // version 4
    buf[2..4].copy_from_slice(&500i16.to_be_bytes()); // xAvgCharWidth
    buf[4..6].copy_from_slice(&weight.to_be_bytes());
    buf[6..8].copy_from_slice(&width.to_be_bytes());
    buf[8..10].copy_from_slice(&fs_type.to_be_bytes());
    buf[0x20..0x2A].copy_from_slice(&[2, 11, 8, 3, 2, 2, 2, 2, 2, 4]); // panose
    buf[0x3A..0x3E].copy_from_slice(&vend_id);
    buf[0x3E..0x40].copy_from_slice(&fs_selection.to_be_bytes());
    buf[0x44..0x46].copy_from_slice(&800i16.to_be_bytes()); // sTypoAscender
    buf[0x46..0x48].copy_from_slice(&(-200i16).to_be_bytes()); // sTypoDescender
    buf[0x48..0x4A].copy_from_slice(&100i16.to_be_bytes()); // sTypoLineGap
    buf[0x4E..0x52].copy_from_slice(&0x0000_0001u32.to_be_bytes()); // ulCodePageRange1
    buf[0x52..0x56].copy_from_slice(&0x0000_0000u32.to_be_bytes()); // ulCodePageRange2
    buf
}

pub fn encode_utf16_be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

pub fn build_synthetic_sfnt(magic: [u8; 4], mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| *tag);
    let num_tables = tables.len();
    let header_len = 12 + num_tables * 16;
    let mut out = vec![0u8; header_len];

    out[0..4].copy_from_slice(&magic);
    out[4..6].copy_from_slice(&(num_tables as u16).to_be_bytes());
    // searchRange/entrySelector/rangeShift (nominal values)
    let entry_selector = (num_tables as u16).ilog2() as u16;
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = (num_tables as u16) * 16 - search_range;
    out[6..8].copy_from_slice(&search_range.to_be_bytes());
    out[8..10].copy_from_slice(&entry_selector.to_be_bytes());
    out[10..12].copy_from_slice(&range_shift.to_be_bytes());

    for (idx, (tag, data)) in tables.into_iter().enumerate() {
        let offset = out.len() as u32;
        let length = data.len() as u32;
        let checksum = calc_sfnt_checksum(&data, &tag == b"head");
        let rec_off = 12 + idx * 16;
        out[rec_off..rec_off + 4].copy_from_slice(&tag);
        out[rec_off + 4..rec_off + 8].copy_from_slice(&checksum.to_be_bytes());
        out[rec_off + 8..rec_off + 12].copy_from_slice(&offset.to_be_bytes());
        out[rec_off + 12..rec_off + 16].copy_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    out
}

pub type SyntheticTtcfFaceSpec = (Vec<([u8; 4], Vec<u8>)>, [u8; 4]);

pub fn build_synthetic_ttcf(faces: Vec<SyntheticTtcfFaceSpec>) -> Vec<u8> {
    let num_fonts = faces.len();
    let dir_header_end = 12 + num_fonts * 4;
    let mut face_headers_end = dir_header_end;
    let mut face_offsets = Vec::with_capacity(num_fonts);
    for (tables, _) in &faces {
        face_offsets.push(face_headers_end as u32);
        face_headers_end += 12 + tables.len() * 16;
    }

    let mut out = vec![0u8; face_headers_end];
    out[0..4].copy_from_slice(b"ttcf");
    out[4..6].copy_from_slice(&1u16.to_be_bytes());
    out[6..8].copy_from_slice(&0u16.to_be_bytes());
    out[8..12].copy_from_slice(&(num_fonts as u32).to_be_bytes());
    for (i, off) in face_offsets.iter().enumerate() {
        out[12 + i * 4..16 + i * 4].copy_from_slice(&off.to_be_bytes());
    }

    for (face_idx, (mut tables, magic)) in faces.into_iter().enumerate() {
        tables.sort_by_key(|(tag, _)| *tag);
        let sfnt_off = face_offsets[face_idx] as usize;
        out[sfnt_off..sfnt_off + 4].copy_from_slice(&magic);
        out[sfnt_off + 4..sfnt_off + 6].copy_from_slice(&(tables.len() as u16).to_be_bytes());

        for (t_idx, (tag, data)) in tables.into_iter().enumerate() {
            let offset = out.len() as u32;
            let length = data.len() as u32;
            let checksum = calc_sfnt_checksum(&data, &tag == b"head");
            let rec_off = sfnt_off + 12 + t_idx * 16;
            out[rec_off..rec_off + 4].copy_from_slice(&tag);
            out[rec_off + 4..rec_off + 8].copy_from_slice(&checksum.to_be_bytes());
            out[rec_off + 8..rec_off + 12].copy_from_slice(&offset.to_be_bytes());
            out[rec_off + 12..rec_off + 16].copy_from_slice(&length.to_be_bytes());
            out.extend_from_slice(&data);
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        }
    }
    let _ = encode_utf16le_bytes;
    out
}
