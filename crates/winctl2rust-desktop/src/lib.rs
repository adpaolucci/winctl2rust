#![deny(unsafe_code)]

pub mod backend;
pub mod clip_hist;
pub mod clipboard;
pub mod ctrl_export;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
pub mod fonts;
pub mod hotkeys;
pub mod keys;
pub mod lnk;
pub mod opentype;
pub mod props;
pub mod tree;
pub mod webcam;
pub mod windows;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod win32_backend;

use std::sync::Arc;

use guap_mcp::ActionCommand;
use guap_table::TableProvider;
use winctl2rust_core::{ToolBundle, ToolCategory, builtin_suite_catalog};

#[cfg(any(test, feature = "test-support"))]
pub use backend::MockDesktopBackend;
pub use backend::{DesktopBackend, UnsupportedDesktopBackend};
#[cfg(windows)]
pub use win32_backend::Win32DesktopBackend;

pub fn default_backend() -> Arc<dyn DesktopBackend> {
    #[cfg(windows)]
    {
        Arc::new(Win32DesktopBackend::new())
    }
    #[cfg(not(windows))]
    {
        Arc::new(UnsupportedDesktopBackend)
    }
}

pub fn providers(backend: Arc<dyn DesktopBackend>) -> Vec<Arc<dyn TableProvider>> {
    vec![
        Arc::new(props::PropsTableProvider::new(Arc::clone(&backend))),
        Arc::new(windows::WindowsTableProvider::new(Arc::clone(&backend))),
        Arc::new(tree::TreeTableProvider::new(Arc::clone(&backend))),
        Arc::new(ctrl_export::CtrlExportTableProvider::new(Arc::clone(&backend))),
        Arc::new(clipboard::ClipboardTableProvider::new(Arc::clone(&backend))),
        Arc::new(clip_hist::ClipHistTableProvider::new(Arc::clone(&backend))),
        Arc::new(hotkeys::HotkeysTableProvider::new(Arc::clone(&backend))),
        Arc::new(keys::KeysTableProvider::new(Arc::clone(&backend))),
        Arc::new(fonts::FontsTableProvider::new(Arc::clone(&backend))),
        Arc::new(webcam::WebcamTableProvider::new(backend)),
    ]
}

pub fn actions(backend: Arc<dyn DesktopBackend>) -> Vec<Arc<dyn ActionCommand>> {
    vec![
        Arc::new(props::PropsControlAction::new(Arc::clone(&backend))),
        Arc::new(windows::WindowsControlAction::new(Arc::clone(&backend))),
        Arc::new(tree::TreeControlAction::new(Arc::clone(&backend))),
        Arc::new(clipboard::ClipboardControlAction::new(Arc::clone(&backend))),
        Arc::new(clip_hist::ClipHistControlAction::new(Arc::clone(&backend))),
        Arc::new(webcam::WebcamCaptureAction::new(backend)),
    ]
}

pub fn bundle(backend: Arc<dyn DesktopBackend>) -> ToolBundle {
    let tools = builtin_suite_catalog()
        .iter()
        .filter(|t| t.category == ToolCategory::Desktop)
        .cloned()
        .collect();
    ToolBundle {
        providers: providers(Arc::clone(&backend)),
        actions: actions(backend),
        tools,
    }
}

pub fn default_providers() -> Vec<Arc<dyn TableProvider>> {
    providers(default_backend())
}

pub fn default_actions() -> Vec<Arc<dyn ActionCommand>> {
    actions(default_backend())
}

pub fn default_bundle() -> ToolBundle {
    bundle(default_backend())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use guap_mcp::{ActionCommand, JsonRpcError, McpServerRegistry, PolicyMode};
    use guap_table::TableQuery;
    use serde_json::json;
    use tempfile::tempdir;
    use winctl2rust_core::{ToolCategory, WinctlError, builtin_suite_catalog};

    use crate::backend::{
        DesktopBackend, MockDesktopBackend, RawVideoFrame, UnsupportedDesktopBackend, VideoPixelFormat, parse_hwnd,
    };
    use crate::clip_hist::{ClipboardArchive, ClipboardJsonArchive};
    use crate::fixtures::{
        SyntheticLnkSpec, build_synthetic_lnk, build_synthetic_name_table_format0, build_synthetic_name_table_format1,
        build_synthetic_os2_v4, build_synthetic_sfnt, build_synthetic_ttcf, encode_utf16_be,
        synthetic_video_frame_bgra32, synthetic_video_frame_nv12, synthetic_video_frame_rgb24,
        synthetic_video_frame_yuy2,
    };
    use crate::lnk::{
        HOTKEYF_ALT, HOTKEYF_CONTROL, HOTKEYF_EXT, HOTKEYF_SHIFT, MOD_ALT, MOD_CONTROL, MOD_SHIFT,
        hotkeyf_to_win32_modifiers, parse_lnk_bytes,
    };
    use crate::opentype::{decode_mac_roman, parse_font_bytes, parse_os2_table};
    use crate::webcam::{encode_bmp_24, encode_ppm_p6, frame_to_rgb24};
    use crate::{bundle, providers};

    #[test]
    fn bundle_matches_core_suite_catalog_exact_ids() {
        let mock = Arc::new(MockDesktopBackend::with_sample_data());
        let desktop_bundle = bundle(mock);
        let expected: Vec<_> = builtin_suite_catalog()
            .iter()
            .filter(|t| t.category == ToolCategory::Desktop)
            .collect();

        assert_eq!(desktop_bundle.tools.len(), 10);
        assert_eq!(desktop_bundle.providers.len(), 10);
        assert_eq!(desktop_bundle.actions.len(), 6);

        for tool in expected {
            for tid in tool.table_ids {
                assert!(
                    desktop_bundle.find_provider(tid).is_some(),
                    "missing TableProvider '{tid}'"
                );
            }
            for aid in tool.action_ids {
                let action = desktop_bundle
                    .find_action(aid)
                    .unwrap_or_else(|| panic!("missing ActionCommand '{aid}'"));
                assert_eq!(action.risk_level(), tool.risk_level);
            }
        }
    }

    #[test]
    fn lnk_parser_handles_unicode_force_no_link_info_ansi_unbound_and_malformed() {
        // 1. Ctrl+Shift+K (0x034B) Unicode with IDList + LinkInfo + StringData
        let hk_ctrl_shift_k = ((u16::from(HOTKEYF_CONTROL | HOTKEYF_SHIFT)) << 8) | u16::from(b'K');
        let bytes = build_synthetic_lnk(&SyntheticLnkSpec {
            hot_key_raw: hk_ctrl_shift_k,
            unicode: true,
            include_id_list: true,
            include_link_info: true,
            force_no_link_info: false,
            local_base_path: Some("C:\\Tools\\guap.exe"),
            name: Some("GUAP Tool"),
            relative_path: Some(".\\guap.exe"),
            working_dir: Some("C:\\Tools"),
            arguments: Some("--status"),
        });
        let parsed = parse_lnk_bytes(&bytes, None).unwrap();
        assert_eq!(parsed.vk_code, b'K');
        assert_eq!(parsed.hotkeyf_modifiers, HOTKEYF_CONTROL | HOTKEYF_SHIFT);
        assert_eq!(parsed.win32_modifiers, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(parsed.hotkey_label, "Ctrl+Shift+K");
        assert_eq!(parsed.target_path.as_deref(), Some("C:\\Tools\\guap.exe"));
        assert_eq!(parsed.owner_hint, "C:\\Tools\\guap.exe --status");

        // 2. Alt+Ext+F12 with HasLinkInfo | ForceNoLinkInfo both set -> skips LinkInfo physically, reads StringData cleanly
        let hk_alt_ext_f12 = ((u16::from(HOTKEYF_ALT | HOTKEYF_EXT)) << 8) | 0x7Bu16;
        let bytes2 = build_synthetic_lnk(&SyntheticLnkSpec {
            hot_key_raw: hk_alt_ext_f12,
            unicode: true,
            include_id_list: false,
            include_link_info: true,
            force_no_link_info: true,
            local_base_path: Some("C:\\Ignored\\ignored.exe"),
            name: Some("F12 Action"),
            relative_path: Some(".\\f12.exe"),
            working_dir: None,
            arguments: None,
        });
        let parsed2 = parse_lnk_bytes(&bytes2, None).unwrap();
        assert_eq!(parsed2.vk_code, 0x7B);
        assert_eq!(parsed2.hotkey_label, "Alt+Ext+F12");
        assert!(parsed2.target_path.is_none());
        assert_eq!(parsed2.relative_path.as_deref(), Some(".\\f12.exe"));
        assert_eq!(parsed2.owner_hint, ".\\f12.exe");

        // 3. ANSI-only + unbound hotkey
        let bytes3 = build_synthetic_lnk(&SyntheticLnkSpec {
            hot_key_raw: 0,
            unicode: false,
            include_id_list: false,
            include_link_info: false,
            force_no_link_info: false,
            local_base_path: None,
            name: Some("AnsiShortcut"),
            relative_path: None,
            working_dir: Some("C:\\Work"),
            arguments: None,
        });
        let parsed3 = parse_lnk_bytes(&bytes3, Some("fallback.lnk")).unwrap();
        assert_eq!(parsed3.vk_code, 0);
        assert_eq!(parsed3.hotkey_label, "None");
        assert_eq!(parsed3.name_string.as_deref(), Some("AnsiShortcut"));

        // Explicit HOTKEYF_* -> MOD_* mapping check
        assert_eq!(
            hotkeyf_to_win32_modifiers(HOTKEYF_ALT | HOTKEYF_CONTROL | HOTKEYF_SHIFT),
            MOD_ALT | MOD_CONTROL | MOD_SHIFT
        );

        // 4. Malformed checks
        assert!(matches!(
            parse_lnk_bytes(&[0u8; 20], None),
            Err(WinctlError::ParseError(_))
        ));
        let mut bad_hdr = bytes.clone();
        bad_hdr[0] = 0x4B;
        assert!(matches!(
            parse_lnk_bytes(&bad_hdr, None),
            Err(WinctlError::ParseError(_))
        ));
        let mut bad_clsid = bytes.clone();
        bad_clsid[5] ^= 0xFF;
        assert!(matches!(
            parse_lnk_bytes(&bad_clsid, None),
            Err(WinctlError::ParseError(_))
        ));
        let trunc = &bytes[..bytes.len() - 4];
        assert!(matches!(parse_lnk_bytes(trunc, None), Err(WinctlError::ParseError(_))));
    }

    #[test]
    fn opentype_parser_handles_sfnt_ttcf_format0_format1_os2_and_malformed() {
        // Format 0 name table with MacRoman + Windows BMP en-US + Windows Full Unicode en-US priority check
        let name_f0 = build_synthetic_name_table_format0(&[
            (1, 0, 0, 1, b"Mac Family".to_vec()),
            (3, 1, 0x0409, 1, encode_utf16_be("GUAP Sans")),
            (3, 10, 0x0409, 1, encode_utf16_be("GUAP Sans FullUnicode")),
            (3, 1, 0x0409, 2, encode_utf16_be("Bold")),
            (3, 1, 0x0409, 4, encode_utf16_be("GUAP Sans Bold")),
            (3, 1, 0x0409, 5, encode_utf16_be("Version 2.10")),
            (3, 1, 0x0409, 6, encode_utf16_be("GuapSans-Bold")),
            (1, 0, 0, 0, vec![b'(', 0xA9, b')']), // MacRoman copyright symbol 0xA9
        ]);
        let os2 = build_synthetic_os2_v4(700, 5, 0x0000, 0x0020, *b"GUAP");
        let sfnt = build_synthetic_sfnt(
            [0x00, 0x01, 0x00, 0x00],
            vec![(*b"name", name_f0.clone()), (*b"OS/2", os2.clone())],
        );

        let faces = parse_font_bytes(&sfnt).unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].family(), "GUAP Sans FullUnicode");
        assert_eq!(faces[0].subfamily(), "Bold");
        assert_eq!(faces[0].full_name(), "GUAP Sans Bold");
        assert_eq!(faces[0].postscript_name(), "GuapSans-Bold");
        assert_eq!(faces[0].version(), "Version 2.10");
        assert_eq!(faces[0].name.as_ref().unwrap().resolve_name(0), Some("(\u{00A9})"));
        let parsed_os2 = faces[0].os2.as_ref().unwrap();
        assert_eq!(parsed_os2.us_weight_class, 700);
        assert_eq!(parsed_os2.weight_label, "Bold");
        assert_eq!(parsed_os2.width_label, "Normal");
        assert_eq!(parsed_os2.ach_vend_id, "GUAP");
        assert_eq!(parsed_os2.s_typo_ascender, 800);
        assert_eq!(parsed_os2.s_typo_descender, -200);
        assert_eq!(decode_mac_roman(&[0xA9, 0xAA, 0xD2]), "\u{00A9}\u{2122}\u{201C}");

        // Format 1 name table + OTTO + ttcf collection
        let name_f1 = build_synthetic_name_table_format1(
            &[
                (3, 1, 0x8000, 1, encode_utf16_be("GUAP Serif")),
                (3, 1, 0x8000, 2, encode_utf16_be("Regular")),
                (3, 1, 0x8000, 16, encode_utf16_be("GUAP Serif Typo")),
            ],
            &["en-US"],
        );
        let os2_reg = build_synthetic_os2_v4(400, 5, 0x0008, 0x0040, *b"GUAP");
        let ttcf = build_synthetic_ttcf(vec![
            (vec![(*b"name", name_f0), (*b"OS/2", os2)], [0x00, 0x01, 0x00, 0x00]),
            (vec![(*b"name", name_f1), (*b"OS/2", os2_reg)], *b"OTTO"),
        ]);
        let col_faces = parse_font_bytes(&ttcf).unwrap();
        assert_eq!(col_faces.len(), 2);
        assert_eq!(col_faces[0].format.label(), "TrueType");
        assert_eq!(col_faces[1].format.label(), "OpenType CFF");
        assert_eq!(col_faces[1].family(), "GUAP Serif Typo");
        assert_eq!(col_faces[1].name.as_ref().unwrap().lang_tags, vec!["en-US"]);

        // Malformed checks: bad magic, truncated table, numFonts DoS, short OS/2
        let mut bad_magic = sfnt.clone();
        bad_magic[0..4].copy_from_slice(b"XXXX");
        assert!(matches!(parse_font_bytes(&bad_magic), Err(WinctlError::ParseError(_))));

        let mut dos_ttcf = vec![0u8; 20];
        dos_ttcf[0..4].copy_from_slice(b"ttcf");
        dos_ttcf[4..6].copy_from_slice(&1u16.to_be_bytes());
        dos_ttcf[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(parse_font_bytes(&dos_ttcf), Err(WinctlError::ParseError(_))));

        let bad_os2_sfnt = build_synthetic_sfnt(*b"OTTO", vec![(*b"OS/2", vec![0u8; 20])]);
        assert!(matches!(
            parse_font_bytes(&bad_os2_sfnt),
            Err(WinctlError::ParseError(_))
        ));
    }

    #[test]
    fn clip_hist_sqlite_and_json_roundtrip_work() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("clip.db");
        let archive = ClipboardArchive::open(&db_path).unwrap();

        let utf16_hello: Vec<u8> = "Hello SQLite"
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .flat_map(u16::to_le_bytes)
            .collect();
        let e1 = archive
            .insert_entry(13, "CF_UNICODETEXT", &utf16_hello, "notepad.exe")
            .unwrap();
        let e2 = archive.insert_entry(8, "CF_DIB", &[1, 2, 3, 4], "mspaint.exe").unwrap();
        assert_eq!(e1.text_preview, "Hello SQLite");
        assert_eq!(e2.text_preview, "[binary 4 bytes]");

        let list = archive.list_entries().unwrap();
        assert_eq!(list.len(), 2);

        let exported = archive.export_json().unwrap();
        assert_eq!(exported.entries.len(), 2);
        let json_text = serde_json::to_string_pretty(&exported).unwrap();

        assert!(archive.delete_entry(e1.entry_id).unwrap());
        assert_eq!(archive.list_entries().unwrap().len(), 1);
        archive.clear().unwrap();
        assert!(archive.list_entries().unwrap().is_empty());

        let parsed: ClipboardJsonArchive = serde_json::from_str(&json_text).unwrap();
        let imported = archive.import_json(&parsed).unwrap();
        assert_eq!(imported, 2);
        assert_eq!(archive.list_entries().unwrap().len(), 2);

        // Tampered SHA-256 rejected
        let mut tampered = parsed;
        tampered.entries[0].sha256 = "0".repeat(64);
        assert!(matches!(
            archive.import_json(&tampered),
            Err(WinctlError::ParseError(_))
        ));
    }

    #[test]
    fn clip_hist_import_json_is_atomic_and_rejects_invalid_invariants() {
        let src = ClipboardArchive::open_in_memory().unwrap();
        src.insert_entry(1, "CF_TEXT", b"entry-one\0", "a.exe").unwrap();
        src.insert_entry(8, "CF_DIB", &[1, 2, 3, 4], "b.exe").unwrap();
        let exported = src.export_json().unwrap();
        assert_eq!(exported.entries.len(), 2);

        let dst = ClipboardArchive::open_in_memory().unwrap();
        dst.insert_entry(1, "CF_TEXT", b"sentinel\0", "init.exe").unwrap();
        assert_eq!(dst.list_entries().unwrap().len(), 1);

        // 1. Entry 0 valid, Entry 1 tampered SHA-256 -> atomic rollback leaves only sentinel row
        let mut partial_tamper = exported.clone();
        partial_tamper.entries[1].sha256 = "0".repeat(64);
        assert!(matches!(
            dst.import_json(&partial_tamper),
            Err(WinctlError::ParseError(_))
        ));
        assert_eq!(dst.list_entries().unwrap().len(), 1);

        // 2. Unsupported schema_version
        let mut bad_ver = exported.clone();
        bad_ver.schema_version = 999;
        assert!(matches!(dst.import_json(&bad_ver), Err(WinctlError::ParseError(_))));

        // 3. Empty sha256 bypass attempt
        let mut empty_sha = exported.clone();
        empty_sha.entries[0].sha256.clear();
        assert!(matches!(dst.import_json(&empty_sha), Err(WinctlError::ParseError(_))));

        // 4. Mismatched size_bytes
        let mut bad_size = exported.clone();
        bad_size.entries[0].size_bytes = 9999;
        assert!(matches!(dst.import_json(&bad_size), Err(WinctlError::ParseError(_))));

        // 5. Empty format / zero format_id
        let mut bad_fmt = exported.clone();
        bad_fmt.entries[0].format.clear();
        assert!(matches!(dst.import_json(&bad_fmt), Err(WinctlError::ParseError(_))));
        let mut zero_fmt_id = exported;
        zero_fmt_id.entries[0].format_id = 0;
        assert!(matches!(dst.import_json(&zero_fmt_id), Err(WinctlError::ParseError(_))));

        assert_eq!(dst.list_entries().unwrap().len(), 1);
    }

    #[test]
    fn opentype_rejects_ttcf_header_overlap_duplicate_faces_and_short_os2() {
        let name_t = build_synthetic_name_table_format0(&[(3, 1, 0x0409, 1, encode_utf16_be("TtcTest"))]);
        let os2 = build_synthetic_os2_v4(400, 5, 0, 0x0040, *b"TEST");
        let base_ttcf = build_synthetic_ttcf(vec![
            (vec![(*b"name", name_t.clone()), (*b"OS/2", os2.clone())], *b"OTTO"),
            (vec![(*b"name", name_t), (*b"OS/2", os2)], *b"OTTO"),
        ]);

        // 1. Table record pointing to 0..12 (ttcf header) must be rejected
        let face0_off = u32::from_be_bytes(base_ttcf[12..16].try_into().unwrap()) as usize;
        let mut hdr_overlap_ttcf = base_ttcf.clone();
        // Table 0 record offset/length at face0_off + 12 + 8..16
        hdr_overlap_ttcf[face0_off + 12 + 8..face0_off + 12 + 12].copy_from_slice(&0u32.to_be_bytes());
        hdr_overlap_ttcf[face0_off + 12 + 12..face0_off + 12 + 16].copy_from_slice(&12u32.to_be_bytes());
        assert!(matches!(
            parse_font_bytes(&hdr_overlap_ttcf),
            Err(WinctlError::ParseError(_))
        ));

        // 2. Duplicate/overlapping face sfnt_offset in ttcf must be rejected
        let mut dup_face_ttcf = base_ttcf.clone();
        dup_face_ttcf[16..20].copy_from_slice(&(face0_off as u32).to_be_bytes());
        assert!(matches!(
            parse_font_bytes(&dup_face_ttcf),
            Err(WinctlError::ParseError(_))
        ));

        // 3. Face sfnt_offset pointing inside ttcf header (0..20) must be rejected
        let mut bad_face_off_ttcf = base_ttcf;
        bad_face_off_ttcf[12..16].copy_from_slice(&4u32.to_be_bytes());
        assert!(matches!(
            parse_font_bytes(&bad_face_off_ttcf),
            Err(WinctlError::ParseError(_))
        ));

        // 4. OS/2 version length enforcement and ach_vend_id NUL trimming
        let mut os2_v0 = vec![0u8; 78];
        os2_v0[0..2].copy_from_slice(&0u16.to_be_bytes());
        let parsed_v0 = parse_os2_table(&os2_v0).unwrap();
        assert_eq!(parsed_v0.ach_vend_id, "");

        os2_v0[0x3A..0x3E].copy_from_slice(b"AB\0\0");
        assert_eq!(parse_os2_table(&os2_v0).unwrap().ach_vend_id, "AB");

        let mut os2_v1_short = vec![0u8; 78];
        os2_v1_short[0..2].copy_from_slice(&1u16.to_be_bytes());
        assert!(matches!(
            parse_os2_table(&os2_v1_short),
            Err(WinctlError::ParseError(_))
        ));

        let mut os2_v1_ok = vec![0u8; 86];
        os2_v1_ok[0..2].copy_from_slice(&1u16.to_be_bytes());
        assert!(parse_os2_table(&os2_v1_ok).unwrap().ul_code_page_range1.is_some());

        let mut os2_v2_short = vec![0u8; 86];
        os2_v2_short[0..2].copy_from_slice(&2u16.to_be_bytes());
        assert!(matches!(
            parse_os2_table(&os2_v2_short),
            Err(WinctlError::ParseError(_))
        ));

        let mut os2_v5_short = vec![0u8; 96];
        os2_v5_short[0..2].copy_from_slice(&5u16.to_be_bytes());
        assert!(matches!(
            parse_os2_table(&os2_v5_short),
            Err(WinctlError::ParseError(_))
        ));

        let mut os2_v5_ok = vec![0u8; 100];
        os2_v5_ok[0..2].copy_from_slice(&5u16.to_be_bytes());
        assert_eq!(parse_os2_table(&os2_v5_ok).unwrap().version, 5);
    }

    #[test]
    fn webcam_bmp_and_ppm_encoders_handle_all_pixel_formats_and_padding() {
        let rgb = synthetic_video_frame_rgb24(4, 2);
        let bgra = synthetic_video_frame_bgra32(4, 2);
        let yuy2 = synthetic_video_frame_yuy2(4, 2);
        let nv12 = synthetic_video_frame_nv12(4, 2);

        assert_eq!(frame_to_rgb24(&rgb).unwrap(), frame_to_rgb24(&bgra).unwrap());
        assert_eq!(frame_to_rgb24(&yuy2).unwrap().len(), 4 * 2 * 3);
        assert_eq!(frame_to_rgb24(&nv12).unwrap().len(), 4 * 2 * 3);

        // Odd width = 3 -> row_bytes = 9 -> 4-byte aligned stride = 12 -> 2 rows * 12 + 54 = 78 bytes
        let odd_rgb = synthetic_video_frame_rgb24(3, 2);
        let bmp = encode_bmp_24(&odd_rgb).unwrap();
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()), 78);
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
        assert_eq!(u16::from_le_bytes(bmp[28..30].try_into().unwrap()), 24);
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), 2);
        assert_eq!(bmp.len(), 78);

        let ppm = encode_ppm_p6(&odd_rgb).unwrap();
        assert!(ppm.starts_with(b"P6\n3 2\n255\n"));

        // Invalid dimensions / truncated buffers
        let bad_yuy2 = RawVideoFrame {
            width: 3,
            height: 2,
            pixel_format: VideoPixelFormat::Yuy2,
            data: vec![0u8; 12],
        };
        assert!(matches!(encode_bmp_24(&bad_yuy2), Err(WinctlError::InvalidInput(_))));

        let trunc = RawVideoFrame {
            width: 4,
            height: 2,
            pixel_format: VideoPixelFormat::Rgb24,
            data: vec![0u8; 10],
        };
        assert!(matches!(encode_ppm_p6(&trunc), Err(WinctlError::ParseError(_))));
    }

    #[test]
    fn mock_backend_fault_injection_unsupported_and_input_validation_work() {
        let unsup = UnsupportedDesktopBackend;
        assert!(matches!(
            unsup.list_window_props(),
            Err(WinctlError::UnsupportedOnPlatform(_))
        ));
        assert!(matches!(
            unsup.capture_webcam_frame(0),
            Err(WinctlError::UnsupportedOnPlatform(_))
        ));

        let mock = MockDesktopBackend::with_sample_data();
        mock.inject_hardware_unavailable.store(true, Ordering::Relaxed);
        assert!(matches!(
            mock.capture_webcam_frame(0),
            Err(WinctlError::HardwareUnavailable(_))
        ));

        mock.inject_hardware_unavailable.store(false, Ordering::Relaxed);
        mock.inject_permission_denied.store(true, Ordering::Relaxed);
        assert!(matches!(
            mock.list_window_props(),
            Err(WinctlError::PermissionDenied(_))
        ));

        assert_eq!(parse_hwnd("0x00010420").unwrap(), 0x0001_0420);
        assert_eq!(parse_hwnd("66592").unwrap(), 66592);
        assert!(matches!(parse_hwnd("0"), Err(WinctlError::InvalidInput(_))));
        assert!(matches!(parse_hwnd("not-hwnd"), Err(WinctlError::InvalidInput(_))));
    }

    #[test]
    fn mcp_policy_gates_r4_compliance_and_dry_run_work_end_to_end() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("hist.db");
        let export_json_path = dir.path().join("hist.json");
        let bmp_path = dir.path().join("frame.bmp");

        let mock = Arc::new(MockDesktopBackend::with_sample_data().with_clip_hist_db_path(db_path.clone()));
        let desktop_bundle = bundle(Arc::clone(&mock) as Arc<dyn DesktopBackend>);
        let mut registry = McpServerRegistry::new(PolicyMode::ReadOnlyOnly);
        desktop_bundle.register_into(&mut registry);

        let list = registry.list_tools();
        assert_eq!(list.as_array().unwrap().len(), 16);

        for table_id in [
            "desktop_props",
            "desktop_windows",
            "desktop_tree",
            "desktop_ctrl_export",
            "desktop_clipboard",
            "desktop_clip_hist",
            "desktop_hotkeys",
            "desktop_keys",
            "desktop_fonts",
            "desktop_webcam",
        ] {
            let res = registry.dispatch(table_id, &json!({})).unwrap();
            assert!(res["rows"].is_array(), "missing rows for {table_id}");
        }

        let mutating_calls = [
            (
                "desktop_props_control",
                json!({ "operation": "set_opacity", "hwnd": "0x00010420", "opacity": 200, "dry_run": true }),
            ),
            (
                "desktop_windows_control",
                json!({ "operation": "minimize", "hwnd": "0x00010420", "dry_run": true }),
            ),
            (
                "desktop_tree_control",
                json!({ "operation": "set_text", "hwnd": "0x00010422", "text": "Updated", "dry_run": true }),
            ),
            (
                "desktop_clipboard_control",
                json!({ "operation": "set_text", "text": "Safe Text", "dry_run": true }),
            ),
            (
                "desktop_clip_hist_control",
                json!({ "operation": "capture_current", "dry_run": true }),
            ),
            (
                "desktop_webcam_capture",
                json!({ "device_index": 0, "output_path": bmp_path, "format": "bmp", "dry_run": true }),
            ),
        ];

        for (name, args) in &mutating_calls {
            let err = registry.dispatch(name, args).unwrap_err();
            assert_eq!(err.code, JsonRpcError::POLICY_DENIED);
        }

        registry.set_policy(PolicyMode::AllowMutating);
        for (name, args) in &mutating_calls {
            let err = registry.dispatch(name, args).unwrap_err();
            assert_eq!(err.code, JsonRpcError::POLICY_DENIED);
        }

        for (name, args) in &mutating_calls {
            let mut payload = args.clone();
            payload["confirm_destructive"] = json!(true);
            let res = registry.dispatch(name, &payload).unwrap();
            assert_eq!(res["dry_run"], json!(true));
            assert_eq!(res["applied"], json!(false));
        }
        assert!(mock.action_log.lock().unwrap().is_empty());
        assert!(!bmp_path.exists());

        // Validation & R4 compliance must fail even during dry_run=true
        let props_action = crate::props::PropsControlAction::new(Arc::clone(&mock) as Arc<dyn DesktopBackend>);
        assert!(
            props_action
                .execute(
                    &json!({ "operation": "set_opacity", "hwnd": "0x10420", "opacity": 300 }),
                    true
                )
                .is_err()
        );
        let clip_action = crate::clipboard::ClipboardControlAction::new(Arc::clone(&mock) as Arc<dyn DesktopBackend>);
        let r4_err = clip_action
            .execute(&json!({ "operation": "set_text", "text": "runasdate /hook" }), true)
            .unwrap_err();
        assert_eq!(JsonRpcError::from(r4_err).code, JsonRpcError::POLICY_DENIED);

        // Execute real mock actions
        let res_capture = registry
            .dispatch(
                "desktop_clip_hist_control",
                &json!({ "operation": "capture_current", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_capture["applied"], json!(true));
        assert_eq!(res_capture["captured_count"], json!(2));

        let hist_rows = providers(Arc::clone(&mock) as Arc<dyn DesktopBackend>)[5]
            .query(&TableQuery::default())
            .unwrap();
        assert_eq!(hist_rows.rows.len(), 2);

        let res_exp = registry
            .dispatch(
                "desktop_clip_hist_control",
                &json!({ "operation": "export_json", "path": export_json_path, "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_exp["entry_count"], json!(2));
        assert!(export_json_path.exists());

        let res_cam = registry
            .dispatch(
                "desktop_webcam_capture",
                &json!({ "device_index": 0, "output_path": bmp_path, "format": "bmp", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_cam["applied"], json!(true));
        assert!(bmp_path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn win32_desktop_backend_read_only_smoke() {
        use std::collections::HashSet;

        let b = crate::default_bundle();
        assert_eq!(b.tools.len(), 10);
        assert_eq!(b.providers.len(), 10);
        assert_eq!(b.actions.len(), 6);

        let backend = crate::Win32DesktopBackend::new();
        let props = backend.list_window_props().unwrap();
        let mut seen_props = HashSet::new();
        for w in &props {
            assert_ne!(w.hwnd, 0);
            assert!(seen_props.insert(w.hwnd), "duplicate hwnd in list_window_props");
        }

        let top_wins = backend.list_top_windows().unwrap();
        for w in &top_wins {
            assert_ne!(w.hwnd, 0);
        }

        let tree = backend.list_window_tree().unwrap();
        let mut seen_tree = HashSet::new();
        for node in &tree {
            assert_ne!(node.hwnd, 0);
            assert!(seen_tree.insert(node.hwnd), "duplicate hwnd in list_window_tree");
            if node.parent_hwnd == 0 {
                assert_eq!(node.depth, 0);
            } else {
                assert!(node.depth >= 1);
            }
        }

        let keys = backend.query_keyboard_state().unwrap();
        assert_eq!(keys.len(), 254);

        let fonts = backend.list_installed_fonts().unwrap();
        assert!(!fonts.is_empty());
        assert!(fonts.iter().all(|f| !f.family.is_empty()));

        // Webcam: either lists devices or gracefully returns empty; capturing out-of-range returns HardwareUnavailable
        let _cams = backend.list_webcam_devices().unwrap();
        assert!(matches!(
            backend.capture_webcam_frame(9999),
            Err(WinctlError::HardwareUnavailable(_))
        ));
    }
}
