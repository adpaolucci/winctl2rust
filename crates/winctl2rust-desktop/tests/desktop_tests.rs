use std::sync::Arc;

use guap_mcp::{ActionCommand, JsonRpcError};
use serde_json::json;
use tempfile::tempdir;
use winctl2rust_core::WinctlError;
use winctl2rust_desktop::backend::{DesktopBackend, RawVideoFrame, UnsupportedDesktopBackend, VideoPixelFormat};
use winctl2rust_desktop::clip_hist::{ClipHistControlAction, ClipboardArchive};
use winctl2rust_desktop::clipboard::ClipboardControlAction;
use winctl2rust_desktop::props::PropsControlAction;
use winctl2rust_desktop::ctrl_export::{HdItemW32, LvItemW32, TvItemW32, decode_utf16_nul_terminated};
use winctl2rust_desktop::tree::TreeControlAction;
use winctl2rust_desktop::webcam::{WebcamCaptureAction, encode_bmp_24, encode_ppm_p6};
use winctl2rust_desktop::windows::WindowsControlAction;

#[test]
fn unsupported_desktop_backend_and_r4_validation_work_without_test_support() {
    let unsup: Arc<dyn DesktopBackend> = Arc::new(UnsupportedDesktopBackend);
    assert!(matches!(
        unsup.list_window_props(),
        Err(WinctlError::UnsupportedOnPlatform(_))
    ));

    // R4 checks in dry_run=true across every action
    let props_act = PropsControlAction::new(Arc::clone(&unsup));
    assert!(
        props_act
            .execute(&json!({ "operation": "createremotethread", "hwnd": "0x10420" }), true)
            .is_err()
    );

    let win_act = WindowsControlAction::new(Arc::clone(&unsup));
    assert!(
        win_act
            .execute(&json!({ "operation": "setwindowshookex", "hwnd": "0x10420" }), true)
            .is_err()
    );

    let tree_act = TreeControlAction::new(Arc::clone(&unsup));
    assert!(
        tree_act
            .execute(
                &json!({ "operation": "set_text", "hwnd": "0x10422", "text": "mimikatz" }),
                true
            )
            .is_err()
    );

    let clip_act = ClipboardControlAction::new(Arc::clone(&unsup));
    assert!(
        clip_act
            .execute(&json!({ "operation": "set_text", "text": "cred2rust" }), true)
            .is_err()
    );

    let cam_act = WebcamCaptureAction::new(unsup);
    assert!(
        cam_act
            .execute(&json!({ "output_path": "C:\\tmp\\socketsniff.bmp" }), true)
            .is_err()
    );

    // Invalid NV12 odd height rejected by BMP/PPM encoders
    let odd_nv12 = RawVideoFrame {
        width: 4,
        height: 3,
        pixel_format: VideoPixelFormat::Nv12,
        data: vec![0u8; 18],
    };
    assert!(matches!(encode_bmp_24(&odd_nv12), Err(WinctlError::InvalidInput(_))));
    assert!(matches!(encode_ppm_p6(&odd_nv12), Err(WinctlError::InvalidInput(_))));

    // SQLite in-memory archive works cleanly
    let mem_db = ClipboardArchive::open_in_memory().unwrap();
    let e = mem_db.insert_entry(1, "CF_TEXT", b"in-memory\0", "test.exe").unwrap();
    assert_eq!(e.text_preview, "in-memory");
    assert_eq!(mem_db.list_entries().unwrap().len(), 1);
    let _tmp = tempdir().unwrap();
}

#[test]
fn ctrl_export_wow64_struct_sizes_and_utf16_helper_match_win32_abi() {
    assert_eq!(std::mem::size_of::<LvItemW32>(), 60);
    assert_eq!(std::mem::size_of::<TvItemW32>(), 40);
    assert_eq!(std::mem::size_of::<HdItemW32>(), 48);
    assert_eq!(
        decode_utf16_nul_terminated(&[b'O' as u16, b'K' as u16, 0, b'X' as u16]),
        "OK"
    );
}

#[test]
fn file_writing_actions_reject_parent_traversal_and_nul_even_in_dry_run() {
    let unsup: Arc<dyn DesktopBackend> = Arc::new(UnsupportedDesktopBackend);
    let clip_act = ClipHistControlAction::new(Arc::clone(&unsup));
    for bad_args in [
        json!({ "operation": "export_json", "path": "../../evil.json" }),
        json!({ "operation": "import_json", "path": "..\\evil.json" }),
        json!({ "operation": "capture_current", "db_path": "../../evil.db" }),
        json!({ "operation": "export_json", "path": "dir/bad\0.json" }),
        json!({ "operation": "capture_current", "db_path": "   " }),
    ] {
        let err = clip_act.execute(&bad_args, true).unwrap_err();
        assert_eq!(JsonRpcError::from(err).code, JsonRpcError::INVALID_PARAMS);
    }

    let cam_act = WebcamCaptureAction::new(unsup);
    for bad_path in ["../../evil.bmp", "sub\\..\\evil.ppm", "bad\0.bmp", "  "] {
        let err = cam_act.execute(&json!({ "output_path": bad_path }), true).unwrap_err();
        assert_eq!(JsonRpcError::from(err).code, JsonRpcError::INVALID_PARAMS);
    }
}

#[cfg(feature = "test-support")]
mod test_support_integration {
    use std::fs;
    use std::sync::Arc;

    use guap_mcp::{ActionCommand, JsonRpcError, McpServerRegistry, PolicyMode};
    use guap_table::TableQuery;
    use serde_json::json;
    use tempfile::tempdir;
    use winctl2rust_core::WinctlError;
    use winctl2rust_desktop::backend::DesktopBackend;
    use winctl2rust_desktop::clip_hist::{ClipboardArchive, ClipboardJsonArchive};
    use winctl2rust_desktop::fixtures::{
        SyntheticLnkSpec, build_synthetic_lnk, build_synthetic_name_table_format0, build_synthetic_os2_v4,
        build_synthetic_sfnt, build_synthetic_ttcf, encode_utf16_be, synthetic_video_frame_bgra32,
        synthetic_video_frame_nv12, synthetic_video_frame_yuy2,
    };
    use winctl2rust_desktop::fonts::font_entries_from_file;
    use winctl2rust_desktop::hotkeys::scan_lnk_hotkeys_in_dir;
    use winctl2rust_desktop::opentype::parse_font_bytes;
    use winctl2rust_desktop::webcam::WebcamCaptureAction;
    use winctl2rust_desktop::{MockDesktopBackend, bundle, providers};

    #[test]
    fn all_props_windows_tree_and_clipboard_operations_mutate_mock_state() {
        let dir = tempdir().unwrap();
        let mock = Arc::new(MockDesktopBackend::with_sample_data().with_clip_hist_db_path(dir.path().join("hist.db")));
        let mut registry = McpServerRegistry::new(PolicyMode::AllowMutating);
        bundle(Arc::clone(&mock) as Arc<dyn DesktopBackend>).register_into(&mut registry);

        for args in [
            json!({ "operation": "hide", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "show", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "disable", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "enable", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "topmost", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "untopmost", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "set_opacity", "hwnd": "0x00010420", "opacity": 180, "confirm_destructive": true }),
            json!({ "operation": "close", "hwnd": "0x00020810", "confirm_destructive": true }),
        ] {
            let res = registry.dispatch("desktop_props_control", &args).unwrap();
            assert_eq!(res["applied"], json!(true));
        }
        assert_eq!(mock.window_props.lock().unwrap().len(), 1);
        assert_eq!(mock.window_props.lock().unwrap()[0].opacity, 180);

        for args in [
            json!({ "operation": "minimize", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "maximize", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "restore", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "focus", "hwnd": "0x00010420", "confirm_destructive": true }),
            json!({ "operation": "close", "hwnd": "0x00020810", "confirm_destructive": true }),
        ] {
            let res = registry.dispatch("desktop_windows_control", &args).unwrap();
            assert_eq!(res["applied"], json!(true));
        }
        assert_eq!(mock.top_windows.lock().unwrap().len(), 1);
        assert_eq!(mock.top_windows.lock().unwrap()[0].state, "Focused");

        for args in [
            json!({ "operation": "set_text", "hwnd": "0x00010422", "text": "New Query", "confirm_destructive": true }),
            json!({ "operation": "resize", "hwnd": "0x00010422", "width": 300, "height": 32, "confirm_destructive": true }),
            json!({ "operation": "move", "hwnd": "0x00010422", "x": 150, "y": 110, "confirm_destructive": true }),
            json!({ "operation": "hide", "hwnd": "0x00010422", "confirm_destructive": true }),
            json!({ "operation": "show", "hwnd": "0x00010422", "confirm_destructive": true }),
        ] {
            let res = registry.dispatch("desktop_tree_control", &args).unwrap();
            assert_eq!(res["applied"], json!(true));
        }
        let edit_node = &mock.tree_nodes.lock().unwrap()[2];
        assert_eq!(edit_node.text, "New Query");
        assert_eq!(
            (edit_node.left, edit_node.top, edit_node.right, edit_node.bottom),
            (150, 110, 450, 142)
        );

        registry
            .dispatch(
                "desktop_clipboard_control",
                &json!({ "operation": "clear", "confirm_destructive": true }),
            )
            .unwrap();
        assert!(mock.clipboard_entries.lock().unwrap().is_empty());

        registry
            .dispatch(
                "desktop_clipboard_control",
                &json!({ "operation": "set_text", "text": "GUAP Verified", "confirm_destructive": true }),
            )
            .unwrap();
        let clip_rows = providers(Arc::clone(&mock) as Arc<dyn DesktopBackend>)[4]
            .query(&TableQuery::default())
            .unwrap();
        assert_eq!(clip_rows.rows.len(), 1);
    }

    #[test]
    fn clip_hist_restore_delete_clear_and_import_roundtrip_work() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("hist.db");
        let json_path = dir.path().join("archive.json");

        let mock = Arc::new(MockDesktopBackend::with_sample_data().with_clip_hist_db_path(db_path.clone()));
        let mut registry = McpServerRegistry::new(PolicyMode::AllowMutating);
        bundle(Arc::clone(&mock) as Arc<dyn DesktopBackend>).register_into(&mut registry);

        let cap = registry
            .dispatch(
                "desktop_clip_hist_control",
                &json!({ "operation": "capture_current", "confirm_destructive": true }),
            )
            .unwrap();
        assert_eq!(cap["captured_count"], json!(2));

        registry
            .dispatch(
                "desktop_clip_hist_control",
                &json!({ "operation": "export_json", "path": json_path, "confirm_destructive": true }),
            )
            .unwrap();

        registry
            .dispatch(
                "desktop_clipboard_control",
                &json!({ "operation": "clear", "confirm_destructive": true }),
            )
            .unwrap();
        assert!(mock.clipboard_entries.lock().unwrap().is_empty());

        registry
            .dispatch(
                "desktop_clip_hist_control",
                &json!({ "operation": "clear", "confirm_destructive": true }),
            )
            .unwrap();

        let imp = registry
            .dispatch(
                "desktop_clip_hist_control",
                &json!({ "operation": "import_json", "path": json_path, "confirm_destructive": true }),
            )
            .unwrap();
        assert_eq!(imp["imported_count"], json!(2));

        let archive = ClipboardArchive::open(&db_path).unwrap();
        let entries = archive.list_entries().unwrap();
        assert_eq!(entries.len(), 2);
        let restore_id = entries[0].entry_id;

        let restored = registry
            .dispatch(
                "desktop_clip_hist_control",
                &json!({ "operation": "restore", "entry_id": restore_id, "confirm_destructive": true }),
            )
            .unwrap();
        assert_eq!(restored["applied"], json!(true));
        assert_eq!(mock.clipboard_entries.lock().unwrap().len(), 1);

        registry
            .dispatch(
                "desktop_clip_hist_control",
                &json!({ "operation": "delete", "entry_id": restore_id, "confirm_destructive": true }),
            )
            .unwrap();
        assert_eq!(archive.list_entries().unwrap().len(), 1);

        let raw_json = fs::read_to_string(&json_path).unwrap();
        let mut parsed: ClipboardJsonArchive = serde_json::from_str(&raw_json).unwrap();
        parsed.entries[0].payload_hex = "deadbeef".to_string();
        fs::write(&json_path, serde_json::to_string(&parsed).unwrap()).unwrap();
        let err = registry
            .dispatch(
                "desktop_clip_hist_control",
                &json!({ "operation": "import_json", "path": json_path, "confirm_destructive": true }),
            )
            .unwrap_err();
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
    }

    #[test]
    fn disk_lnk_scanner_and_font_reader_work_offline() {
        let dir = tempdir().unwrap();

        let bound = build_synthetic_lnk(&SyntheticLnkSpec {
            hot_key_raw: (0x02 << 8) | u16::from(b'G'),
            unicode: true,
            include_id_list: false,
            include_link_info: true,
            force_no_link_info: false,
            local_base_path: Some("C:\\Apps\\guap.exe"),
            name: Some("GUAP"),
            relative_path: None,
            working_dir: None,
            arguments: None,
        });
        let unbound = build_synthetic_lnk(&SyntheticLnkSpec {
            hot_key_raw: 0,
            unicode: true,
            include_id_list: false,
            include_link_info: false,
            force_no_link_info: false,
            local_base_path: None,
            name: Some("NoHotkey"),
            relative_path: None,
            working_dir: None,
            arguments: None,
        });
        fs::write(dir.path().join("guap.lnk"), bound).unwrap();
        fs::write(dir.path().join("plain.lnk"), unbound).unwrap();

        let scanned = scan_lnk_hotkeys_in_dir(dir.path(), 2);
        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].hotkey, "Ctrl+G");
        assert_eq!(scanned[0].owner_hint, "C:\\Apps\\guap.exe");

        let name_t = build_synthetic_name_table_format0(&[
            (3, 1, 0x0409, 1, encode_utf16_be("DiskFont")),
            (3, 1, 0x0409, 2, encode_utf16_be("Regular")),
            (3, 1, 0x0409, 4, encode_utf16_be("DiskFont Regular")),
            (3, 1, 0x0409, 6, encode_utf16_be("DiskFont-Regular")),
        ]);
        let os2 = build_synthetic_os2_v4(400, 5, 0, 0x0040, *b"DISK");
        let ttc = build_synthetic_ttcf(vec![(
            vec![(*b"name", name_t), (*b"OS/2", os2)],
            [0x00, 0x01, 0x00, 0x00],
        )]);
        let ttc_path = dir.path().join("diskfont.ttc");
        fs::write(&ttc_path, &ttc).unwrap();
        let font_entries = font_entries_from_file(&ttc_path).unwrap();
        assert_eq!(font_entries.len(), 1);
        assert_eq!(font_entries[0].family, "DiskFont");
        assert_eq!(font_entries[0].postscript_name, "DiskFont-Regular");
    }

    #[test]
    fn opentype_rejects_overlapping_tables_and_duplicate_tags() {
        let name_t = build_synthetic_name_table_format0(&[(3, 1, 0x0409, 1, encode_utf16_be("Dup"))]);
        let mut dup_sfnt = build_synthetic_sfnt(
            [0x00, 0x01, 0x00, 0x00],
            vec![(*b"name", name_t.clone()), (*b"head", vec![0u8; 54])],
        );
        dup_sfnt[12..16].copy_from_slice(b"name");
        assert!(matches!(parse_font_bytes(&dup_sfnt), Err(WinctlError::ParseError(_))));

        let mut overlap_sfnt = build_synthetic_sfnt([0x00, 0x01, 0x00, 0x00], vec![(*b"name", name_t)]);
        overlap_sfnt[12 + 8..12 + 12].copy_from_slice(&4u32.to_be_bytes());
        assert!(matches!(
            parse_font_bytes(&overlap_sfnt),
            Err(WinctlError::ParseError(_))
        ));
    }

    #[test]
    fn webcam_yuy2_nv12_bgra32_capture_work() {
        let dir = tempdir().unwrap();
        let mock = Arc::new(MockDesktopBackend::with_sample_data());

        for (frame, ext) in [
            (synthetic_video_frame_bgra32(4, 2), "bmp"),
            (synthetic_video_frame_yuy2(4, 2), "ppm"),
            (synthetic_video_frame_nv12(4, 2), "bmp"),
        ] {
            *mock.mock_frame.lock().unwrap() = Some(frame);
            let out_path = dir.path().join(format!("out.{ext}"));
            let action = WebcamCaptureAction::new(Arc::clone(&mock) as Arc<dyn DesktopBackend>);
            let res = action
                .execute(&json!({ "device_index": 0, "output_path": out_path }), false)
                .unwrap();
            assert_eq!(res["format"], json!(ext));
            assert!(out_path.exists());
        }
    }

    #[test]
    fn tree_control_rejects_i32_overflow_without_panicking() {
        use winctl2rust_desktop::tree::TreeControlAction;

        let mock = Arc::new(MockDesktopBackend::with_sample_data());
        let before = mock.tree_nodes.lock().unwrap()[2].clone();
        let tree_act = TreeControlAction::new(Arc::clone(&mock) as Arc<dyn DesktopBackend>);

        let err_resize = tree_act
            .execute(
                &json!({ "operation": "resize", "hwnd": "0x00010422", "width": i32::MAX, "height": 10 }),
                false,
            )
            .unwrap_err();
        assert_eq!(JsonRpcError::from(err_resize).code, JsonRpcError::INVALID_PARAMS);

        let err_move = tree_act
            .execute(
                &json!({ "operation": "move", "hwnd": "0x00010422", "x": i32::MAX, "y": 10 }),
                false,
            )
            .unwrap_err();
        assert_eq!(JsonRpcError::from(err_move).code, JsonRpcError::INVALID_PARAMS);

        let after = mock.tree_nodes.lock().unwrap()[2].clone();
        assert_eq!(before, after);
    }
}
