#![deny(unsafe_code)]

pub mod app_config;
pub mod backend;
pub mod svcl;
pub mod view;
pub mod volumouse;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod win32_backend;

use std::sync::Arc;

use guap_mcp::ActionCommand;
use guap_table::TableProvider;
use winctl2rust_core::{ToolBundle, ToolCategory, builtin_suite_catalog};

#[cfg(any(test, feature = "test-support"))]
pub use backend::MockAudioBackend;
pub use backend::{AudioBackend, UnsupportedAudioBackend};
#[cfg(windows)]
pub use win32_backend::Win32AudioBackend;

pub fn default_backend() -> Arc<dyn AudioBackend> {
    #[cfg(windows)]
    {
        Arc::new(Win32AudioBackend::new())
    }
    #[cfg(not(windows))]
    {
        Arc::new(UnsupportedAudioBackend)
    }
}

pub fn providers(backend: Arc<dyn AudioBackend>) -> Vec<Arc<dyn TableProvider>> {
    vec![
        Arc::new(view::AudioViewTableProvider::new(Arc::clone(&backend))),
        Arc::new(svcl::SvclStatusTableProvider::new(Arc::clone(&backend))),
        Arc::new(app_config::AppAudioConfigTableProvider::new(Arc::clone(&backend))),
        Arc::new(volumouse::VolumouseRulesTableProvider::default()),
    ]
}

pub fn actions(backend: Arc<dyn AudioBackend>) -> Vec<Arc<dyn ActionCommand>> {
    vec![
        Arc::new(view::AudioViewControlAction::new(Arc::clone(&backend))),
        Arc::new(svcl::SvclAction::new(Arc::clone(&backend))),
        Arc::new(app_config::AppAudioConfigUpdateAction::new(Arc::clone(&backend))),
        Arc::new(volumouse::VolumouseAction::new(backend)),
    ]
}

pub fn bundle(backend: Arc<dyn AudioBackend>) -> ToolBundle {
    let tools = builtin_suite_catalog()
        .iter()
        .filter(|t| t.category == ToolCategory::Audio)
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

    use guap_mcp::{JsonRpcError, McpServerRegistry, PolicyMode};
    use guap_table::TableQuery;
    use serde_json::json;
    use winctl2rust_core::{ToolCategory, WinctlError, builtin_suite_catalog};

    use crate::app_config::{
        FMTID_AUDIO_POLICY_BAG, FMTID_USER_DEFINED_PROPERTIES, PropertyKeyId, SerializedPropertyEntry,
        SerializedPropertyStorage, SerializedPropertyValue, VT_BOOL, VT_LPWSTR, VT_R4, VT_VECTOR_R4,
        decode_propvar_bool, decode_propvar_r4, decode_propvar_r4_array, decode_raw_record, encode_propvar_bool,
        encode_propvar_r4, encode_propvar_r4_array, encode_serialized_prop_storage, parse_policy_default_string,
        parse_serialized_prop_storage,
    };
    use crate::backend::{
        AudioBackend, DefaultRole, MockAudioBackend, PointerWheelEvent, RawAppPropertyStoreRecord, ScreenEdgeKind,
        UnsupportedAudioBackend,
    };
    use crate::svcl::{SaveFileEncoding, SvclVerb, execute_svcl, parse_svcl_command};
    use crate::volumouse::{default_volumouse_rules, evaluate_event_stream, evaluate_wheel_event};
    use crate::{bundle, providers};

    #[test]
    fn bundle_matches_core_suite_catalog_exact_ids() {
        let mock = Arc::new(MockAudioBackend::with_sample_data());
        let audio_bundle = bundle(mock);
        let expected: Vec<_> = builtin_suite_catalog()
            .iter()
            .filter(|t| t.category == ToolCategory::Audio)
            .collect();
        assert_eq!(audio_bundle.tools.len(), 4);
        assert_eq!(audio_bundle.providers.len(), 4);
        assert_eq!(audio_bundle.actions.len(), 4);

        for tool in expected {
            for tid in tool.table_ids {
                assert!(
                    audio_bundle.find_provider(tid).is_some(),
                    "missing TableProvider '{tid}'"
                );
            }
            for aid in tool.action_ids {
                let action = audio_bundle
                    .find_action(aid)
                    .unwrap_or_else(|| panic!("missing ActionCommand '{aid}'"));
                assert_eq!(action.risk_level(), tool.risk_level);
            }
        }
    }

    #[test]
    fn svcl_grammar_parses_all_verbs_and_rejects_invalid_inputs() {
        let cases = [
            (
                "/SetVolume \"Speakers\" 75",
                SvclVerb::SetVolume {
                    target: "Speakers".into(),
                    percent: 75.0,
                },
            ),
            (
                "/SetVolumeChannels \"Speakers\" 50 80",
                SvclVerb::SetVolumeChannels {
                    target: "Speakers".into(),
                    channels: vec![50.0, 80.0],
                },
            ),
            (
                "/ChangeVolume \"Speakers\" -15",
                SvclVerb::ChangeVolume {
                    target: "Speakers".into(),
                    delta_percent: -15.0,
                },
            ),
            (
                "/SetVolumeDecibel \"Speakers\" -6.5",
                SvclVerb::SetVolumeDecibel {
                    target: "Speakers".into(),
                    db: -6.5,
                },
            ),
            (
                "/ChangeVolumeDecibel \"Speakers\" +3.0",
                SvclVerb::ChangeVolumeDecibel {
                    target: "Speakers".into(),
                    delta_db: 3.0,
                },
            ),
            (
                "/Mute vlc.exe",
                SvclVerb::Mute {
                    target: "vlc.exe".into(),
                },
            ),
            (
                "/Unmute vlc.exe",
                SvclVerb::Unmute {
                    target: "vlc.exe".into(),
                },
            ),
            (
                "/Switch \"Headphones\"",
                SvclVerb::Switch {
                    target: "Headphones".into(),
                },
            ),
            (
                "/SetDefault \"Headphones\" 0,1,2",
                SvclVerb::SetDefault {
                    target: "Headphones".into(),
                    roles: vec![
                        DefaultRole::Console,
                        DefaultRole::Multimedia,
                        DefaultRole::Communications,
                    ],
                },
            ),
            (
                "/SetDefault \"Speakers\" all",
                SvclVerb::SetDefault {
                    target: "Speakers".into(),
                    roles: vec![
                        DefaultRole::Console,
                        DefaultRole::Multimedia,
                        DefaultRole::Communications,
                    ],
                },
            ),
            (
                "/SetAppDefault \"Headphones\" all vlc.exe",
                SvclVerb::SetAppDefault {
                    target: "Headphones".into(),
                    roles: vec![
                        DefaultRole::Console,
                        DefaultRole::Multimedia,
                        DefaultRole::Communications,
                    ],
                    process_or_pid: "vlc.exe".into(),
                },
            ),
            (
                "/Enable \"Headphones\"",
                SvclVerb::Enable {
                    target: "Headphones".into(),
                },
            ),
            (
                "/Disable \"Headphones\"",
                SvclVerb::Disable {
                    target: "Headphones".into(),
                },
            ),
            (
                "/GetPercent DefaultRenderDevice",
                SvclVerb::GetPercent {
                    target: "DefaultRenderDevice".into(),
                },
            ),
            (
                "/GetMute \"Headphones\"",
                SvclVerb::GetMute {
                    target: "Headphones".into(),
                },
            ),
            (
                "/GetDecibel \"Speakers\"",
                SvclVerb::GetDecibel {
                    target: "Speakers".into(),
                },
            ),
        ];

        for (cmd, expected) in cases {
            let parsed = parse_svcl_command(cmd).unwrap_or_else(|e| panic!("failed '{cmd}': {e}"));
            assert_eq!(parsed.verb, expected);
        }

        let with_opts = parse_svcl_command("/SaveFileEncoding utf8 /Wait 25 /Mute vlc.exe").unwrap();
        assert_eq!(with_opts.save_file_encoding, Some(SaveFileEncoding::Utf8));
        assert_eq!(with_opts.wait_ms, Some(25));

        assert!(parse_svcl_command("").is_err());
        assert!(parse_svcl_command("/SetVolume \"Speakers\" 150").is_err());
        assert!(parse_svcl_command("/SetVolume \"unterminated").is_err());
        assert!(parse_svcl_command("/Wait 999999 /Mute vlc.exe").is_err());
        assert!(parse_svcl_command("/UnknownVerb Speakers").is_err());
        assert!(matches!(
            parse_svcl_command("/Mute runasdate"),
            Err(WinctlError::PolicyDenied(_))
        ));
    }

    #[test]
    fn svcl_executes_against_mock_backend_including_get_exit_codes() {
        let mock = MockAudioBackend::with_sample_data();
        let get_pct = parse_svcl_command("/GetPercent DefaultRenderDevice").unwrap();
        let res = execute_svcl(&mock, &get_pct).unwrap();
        assert!(res.read_only);
        assert!(!res.applied);
        assert_eq!(res.exit_code, 650);

        let change = parse_svcl_command("/ChangeVolume \"Speakers\" -15").unwrap();
        let res2 = execute_svcl(&mock, &change).unwrap();
        assert!(res2.applied);
        assert_eq!(res2.result_value, json!(50.0));

        let get_mute = parse_svcl_command("/GetMute \"Headphones\"").unwrap();
        let res3 = execute_svcl(&mock, &get_mute).unwrap();
        assert_eq!(res3.exit_code, 1);

        let set_def = parse_svcl_command("/SetDefault \"Headphones\" 0,1").unwrap();
        execute_svcl(&mock, &set_def).unwrap();
        let app_def = parse_svcl_command("/SetAppDefault \"Headphones\" all 4120").unwrap();
        execute_svcl(&mock, &app_def).unwrap();
        assert_eq!(mock.app_default_routes.lock().unwrap().len(), 1);
    }

    #[test]
    fn propvariant_and_1sps_binary_codecs_roundtrip_and_match_live_windows_bytes() {
        // Real-world Windows 11 PolicyConfig blobs:
        // "3" VT_R4 (24 bytes): 0.216 -> 1C 2F 5D 3E
        let mut live_vol_blob = vec![0u8; 24];
        live_vol_blob[0..2].copy_from_slice(&[0x04, 0x00]);
        live_vol_blob[8..12].copy_from_slice(&[0x1C, 0x2F, 0x5D, 0x3E]);
        let decoded_v = decode_propvar_r4(&live_vol_blob).unwrap();
        assert!((decoded_v - 0.216).abs() < 1e-4);

        let enc_v = encode_propvar_r4(0.75).unwrap();
        assert_eq!(enc_v.len(), 24);
        assert!((decode_propvar_r4(&enc_v).unwrap() - 0.75).abs() < 1e-6);

        // "4" VT_ARRAY|VT_R4 (32 bytes for 2ch)
        let enc_ch = encode_propvar_r4_array(&[0.5, 0.85]).unwrap();
        assert_eq!(enc_ch.len(), 32);
        assert_eq!(&enc_ch[0..2], &[0x04, 0x20]);
        assert_eq!(u64::from_le_bytes(enc_ch[8..16].try_into().unwrap()), 24);
        let dec_ch = decode_propvar_r4_array(&enc_ch).unwrap();
        assert_eq!(dec_ch.len(), 2);
        assert!((dec_ch[0] - 0.5).abs() < 1e-6);
        assert!((dec_ch[1] - 0.85).abs() < 1e-6);

        // "5" VT_BOOL (24 bytes)
        let enc_mute = encode_propvar_bool(true);
        assert_eq!(enc_mute.len(), 24);
        assert_eq!(&enc_mute[8..10], &[0xFF, 0xFF]);
        assert!(decode_propvar_bool(&enc_mute).unwrap());
        assert!(!decode_propvar_bool(&encode_propvar_bool(false)).unwrap());

        // 1SPS SERIALIZEDPROPSTORAGE round-trip (numeric PIDs + named user-defined properties)
        let storages = vec![
            SerializedPropertyStorage {
                fmtid: FMTID_AUDIO_POLICY_BAG,
                fmtid_guid: "{219ED5A0-9CBF-4F3A-B927-37C9E5C5F14F}".to_string(),
                entries: vec![
                    SerializedPropertyEntry {
                        key: PropertyKeyId::Pid(3),
                        vt: VT_R4,
                        value: SerializedPropertyValue::R4(0.42),
                    },
                    SerializedPropertyEntry {
                        key: PropertyKeyId::Pid(4),
                        vt: VT_VECTOR_R4,
                        value: SerializedPropertyValue::R4List(vec![0.42, 0.84]),
                    },
                    SerializedPropertyEntry {
                        key: PropertyKeyId::Pid(5),
                        vt: VT_BOOL,
                        value: SerializedPropertyValue::Bool(true),
                    },
                ],
            },
            SerializedPropertyStorage {
                fmtid: FMTID_USER_DEFINED_PROPERTIES,
                fmtid_guid: "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}".to_string(),
                entries: vec![SerializedPropertyEntry {
                    key: PropertyKeyId::Name("AppLabel".to_string()),
                    vt: VT_LPWSTR,
                    value: SerializedPropertyValue::Lpwstr("VLC media player".to_string()),
                }],
            },
        ];
        let encoded_1sps = encode_serialized_prop_storage(&storages);
        assert_eq!(&encoded_1sps[4..8], b"1SPS");
        let decoded_1sps = parse_serialized_prop_storage(&encoded_1sps).unwrap();
        assert_eq!(decoded_1sps.len(), 2);
        assert_eq!(decoded_1sps[0].entries.len(), 3);
        assert_eq!(
            decoded_1sps[1].entries[0].key,
            PropertyKeyId::Name("AppLabel".to_string())
        );

        let rec = RawAppPropertyStoreRecord {
            subkey: "test_1sps".to_string(),
            registry_key: "HKCU\\...".to_string(),
            default_value: "{2}.\\device|\\Program Files\\VLC\\vlc.exe%b{GUID}".to_string(),
            blob_3_volume: None,
            blob_4_channels: None,
            blob_5_mute: None,
            raw_1sps_container: Some(encoded_1sps),
        };
        let decoded_rec = decode_raw_record(&rec);
        assert_eq!(decoded_rec.app_name, "vlc.exe");
        assert_eq!(decoded_rec.raw_format, "1SPS_SERIALIZEDPROPSTORAGE");
        assert_eq!(decoded_rec.muted, Some(true));
        assert!((decoded_rec.volume_percent.unwrap() - 42.0).abs() < 0.05);

        let (dev, path, name, guid) = parse_policy_default_string(&rec.default_value);
        assert_eq!(dev, "{2}.\\device");
        assert_eq!(path, "\\Program Files\\VLC\\vlc.exe");
        assert_eq!(name, "vlc.exe");
        assert_eq!(guid, "{GUID}");
    }

    #[test]
    fn volumouse_rule_priority_clamping_and_stream_evaluation() {
        let rules = default_volumouse_rules();
        // Ctrl+Alt beats Alt alone (priority 200 vs 100)
        let ev_ctrl_alt = PointerWheelEvent {
            wheel_delta: 120,
            alt: true,
            ctrl: true,
            shift: false,
            win: false,
            left_button: false,
            right_button: false,
            middle_button: false,
            over_taskbar: false,
            over_desktop: false,
            over_window_title: false,
            screen_edge: None,
            window_class: String::new(),
        };
        let m1 = evaluate_wheel_event(&rules, &ev_ctrl_alt, 50.0).unwrap();
        assert_eq!(m1.rule_id, "ctrl_alt_fine");
        assert_eq!(m1.new_percent, 51.0);

        // OverTaskbar clamps [10.0, 80.0]
        let stream = vec![
            PointerWheelEvent {
                wheel_delta: 240,
                over_taskbar: true,
                ..ev_ctrl_alt.clone()
            },
            PointerWheelEvent {
                wheel_delta: -1200,
                over_taskbar: true,
                alt: false,
                ctrl: false,
                ..ev_ctrl_alt.clone()
            },
            PointerWheelEvent {
                wheel_delta: 120,
                alt: false,
                ctrl: false,
                screen_edge: Some(ScreenEdgeKind::Left),
                ..ev_ctrl_alt
            },
        ];
        // Event 1: ctrl+alt+over_taskbar -> matches ctrl_alt_fine (+2% => 79 -> 81)
        // Event 2: over_taskbar (-20% from 81 => 61)
        // Event 3: left screen edge (+4% => 65)
        let results = evaluate_event_stream(&rules, &stream, 79.0);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].rule_id, "ctrl_alt_fine");
        assert_eq!(results[0].new_percent, 81.0);
        assert_eq!(results[1].rule_id, "over_taskbar_wheel");
        assert_eq!(results[1].new_percent, 61.0);
        assert_eq!(results[2].rule_id, "left_edge_headphones");
        assert_eq!(results[2].new_percent, 65.0);
    }

    #[test]
    fn mock_backend_fault_injection_and_unsupported_backend_work() {
        let unsup = UnsupportedAudioBackend;
        assert!(matches!(
            unsup.list_audio_items(),
            Err(WinctlError::UnsupportedOnPlatform(_))
        ));

        let mock = MockAudioBackend::with_sample_data();
        mock.inject_hardware_unavailable.store(true, Ordering::Relaxed);
        assert!(matches!(
            mock.list_audio_items(),
            Err(WinctlError::HardwareUnavailable(_))
        ));

        mock.inject_hardware_unavailable.store(false, Ordering::Relaxed);
        mock.inject_permission_denied.store(true, Ordering::Relaxed);
        assert!(matches!(
            mock.read_app_audio_configs(),
            Err(WinctlError::PermissionDenied(_))
        ));
    }

    #[test]
    fn mcp_policy_gates_and_dry_run_work_end_to_end() {
        let mock = Arc::new(MockAudioBackend::with_sample_data());
        mock.queued_wheel_events.lock().unwrap().push(PointerWheelEvent {
            wheel_delta: 120,
            alt: true,
            ctrl: false,
            shift: false,
            win: false,
            left_button: false,
            right_button: false,
            middle_button: false,
            over_taskbar: false,
            over_desktop: false,
            over_window_title: false,
            screen_edge: None,
            window_class: String::new(),
        });

        let audio_bundle = bundle(Arc::clone(&mock) as Arc<dyn AudioBackend>);
        let mut registry = McpServerRegistry::new(PolicyMode::ReadOnlyOnly);
        audio_bundle.register_into(&mut registry);

        let list = registry.list_tools();
        assert_eq!(list.as_array().unwrap().len(), 8);

        for table_id in [
            "audio_view",
            "audio_svcl_status",
            "audio_app_config",
            "audio_volumouse_rules",
        ] {
            let res = registry.dispatch(table_id, &json!({})).unwrap();
            assert!(res["rows"].is_array(), "missing rows for {table_id}");
        }

        let mutating_calls = [
            (
                "audio_view_control",
                json!({ "action": "set_volume", "target": "Speakers", "volume_percent": 45.0, "dry_run": true }),
            ),
            (
                "audio_svcl",
                json!({ "command_line": "/Mute vlc.exe", "dry_run": true }),
            ),
            (
                "audio_app_config_update",
                json!({ "subkey": "186430e1_0", "volume_percent": 30.0, "dry_run": true }),
            ),
            (
                "audio_volumouse",
                json!({ "mode": "start_listener", "duration_ms": 50, "max_events": 1, "dry_run": true }),
            ),
        ];

        // ReadOnlyOnly -> -32003
        for (name, args) in &mutating_calls {
            let err = registry.dispatch(name, args).unwrap_err();
            assert_eq!(err.code, JsonRpcError::POLICY_DENIED);
        }

        // AllowMutating without confirm_destructive -> -32003
        registry.set_policy(PolicyMode::AllowMutating);
        for (name, args) in &mutating_calls {
            let err = registry.dispatch(name, args).unwrap_err();
            assert_eq!(err.code, JsonRpcError::POLICY_DENIED);
        }

        // AllowMutating + confirm_destructive + dry_run -> preview without mutation
        for (name, args) in &mutating_calls {
            let mut payload = args.clone();
            payload["confirm_destructive"] = json!(true);
            let res = registry.dispatch(name, &payload).unwrap();
            assert_eq!(res["dry_run"], json!(true));
            assert_eq!(res["applied"], json!(false));
        }
        assert!(mock.action_log.lock().unwrap().is_empty());

        // Real mutations against MockAudioBackend
        let res_view = registry
            .dispatch(
                "audio_view_control",
                &json!({ "action": "set_volume", "target": "Speakers", "volume_percent": 45.0, "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_view["applied"], json!(true));

        let res_svcl = registry
            .dispatch(
                "audio_svcl",
                &json!({ "command_line": "/Mute vlc.exe", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_svcl["applied"], json!(true));

        let res_cfg = registry
            .dispatch(
                "audio_app_config_update",
                &json!({ "subkey": "186430e1_0", "volume_percent": 30.0, "muted": true, "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_cfg["applied"], json!(true));

        let res_vm = registry
            .dispatch(
                "audio_volumouse",
                &json!({ "mode": "start_listener", "duration_ms": 50, "max_events": 1, "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_vm["applied"], json!(true));
        assert!(!mock.action_log.lock().unwrap().is_empty());

        let provs = providers(mock);
        let data = provs[2].query(&TableQuery::default()).unwrap();
        assert_eq!(data.rows.len(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn win32_audio_backend_read_only_smoke() {
        let b = crate::default_bundle();
        assert_eq!(b.tools.len(), 4);
        assert_eq!(b.providers.len(), 4);
        assert_eq!(b.actions.len(), 4);

        let backend = crate::Win32AudioBackend::new();
        // Enumerate CoreAudio endpoints (graceful empty/HardwareUnavailable on headless CI)
        match backend.list_audio_items() {
            Ok(items) => {
                for item in &items {
                    assert!(!item.item_id.is_empty());
                }
            }
            Err(WinctlError::HardwareUnavailable(_)) => {}
            Err(other) => panic!("unexpected Win32AudioBackend::list_audio_items error: {other}"),
        }
        // Enumerate HKCU PolicyConfig\PropertyStore read-only
        let configs = backend.read_app_audio_configs().unwrap();
        for rec in &configs {
            assert!(!rec.subkey.is_empty());
            let _ = decode_raw_record(rec);
        }

        // Direct trait validation rejects empty/traversal subkey before touching Registry
        let bad_patch = crate::backend::AppPropertyStorePatch {
            delete_subkey: true,
            ..Default::default()
        };
        assert!(matches!(
            backend.write_app_audio_config("", &bad_patch),
            Err(WinctlError::InvalidInput(_))
        ));

        // Repeated Volumouse listener calls on the same thread must not poison the queue with WM_QUIT
        let opts = crate::backend::ListenerOptions {
            duration_ms: 15,
            max_events: 1,
        };
        let r1 = backend.run_volumouse_listener(&[], &opts).unwrap();
        let r2 = backend.run_volumouse_listener(&[], &opts).unwrap();
        assert!(r1.is_empty());
        assert!(r2.is_empty());
    }

    #[test]
    fn regression_volumouse_resolves_non_first_target_and_guards_invalid_bounds_and_nan() {
        let mock = MockAudioBackend::with_sample_data();
        // Speakers is first at 65%; Headphones is second at 40%.
        // left_edge_headphones targets "Headphones (USB Audio)".
        mock.queued_wheel_events.lock().unwrap().push(PointerWheelEvent {
            wheel_delta: 120,
            alt: false,
            ctrl: false,
            shift: false,
            win: false,
            left_button: false,
            right_button: false,
            middle_button: false,
            over_taskbar: false,
            over_desktop: false,
            over_window_title: false,
            screen_edge: Some(ScreenEdgeKind::Left),
            window_class: String::new(),
        });
        let rules = default_volumouse_rules();
        let results = mock
            .run_volumouse_listener(
                &rules,
                &crate::backend::ListenerOptions {
                    duration_ms: 50,
                    max_events: 4,
                },
            )
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rule_id, "left_edge_headphones");
        assert_eq!(results[0].target_device, "Headphones (USB Audio)");
        assert_eq!(results[0].previous_percent, 40.0);
        assert_eq!(results[0].new_percent, 44.0);

        let items = mock.list_audio_items().unwrap();
        assert_eq!(items[0].volume_percent, 65.0);
        assert_eq!(items[1].volume_percent, 44.0);

        // Invalid bounds / NaN / non-positive step must never panic in evaluate_wheel_event / evaluate_event_stream
        let ev = PointerWheelEvent {
            wheel_delta: 120,
            alt: true,
            ctrl: false,
            shift: false,
            win: false,
            left_button: false,
            right_button: false,
            middle_button: false,
            over_taskbar: false,
            over_desktop: false,
            over_window_title: false,
            screen_edge: None,
            window_class: String::new(),
        };
        let bad_rules = vec![
            crate::backend::VolumouseRule {
                rule_id: "inverted_bounds".to_string(),
                enabled: true,
                priority: 100,
                conditions: vec![crate::backend::VolumouseCondition::Alt],
                target_device: "DefaultRenderDevice".to_string(),
                step_percent: 5.0,
                min_percent: 80.0,
                max_percent: 20.0,
                consume_wheel: false,
                invert_direction: false,
            },
            crate::backend::VolumouseRule {
                rule_id: "nan_step".to_string(),
                enabled: true,
                priority: 90,
                conditions: vec![crate::backend::VolumouseCondition::Alt],
                target_device: "DefaultRenderDevice".to_string(),
                step_percent: f32::NAN,
                min_percent: 0.0,
                max_percent: 100.0,
                consume_wheel: false,
                invert_direction: false,
            },
            crate::backend::VolumouseRule {
                rule_id: "zero_step".to_string(),
                enabled: true,
                priority: 80,
                conditions: vec![crate::backend::VolumouseCondition::Alt],
                target_device: "DefaultRenderDevice".to_string(),
                step_percent: 0.0,
                min_percent: 0.0,
                max_percent: 100.0,
                consume_wheel: false,
                invert_direction: false,
            },
        ];
        assert!(evaluate_wheel_event(&bad_rules, &ev, 50.0).is_none());
        assert!(evaluate_wheel_event(&rules, &ev, f32::NAN).is_none());
        assert!(evaluate_wheel_event(&rules, &ev, f32::INFINITY).is_none());
        assert!(evaluate_event_stream(&rules, std::slice::from_ref(&ev), f32::NAN).is_empty());
        assert!(evaluate_event_stream(&rules, &[ev], f32::NEG_INFINITY).is_empty());
    }

    #[test]
    fn regression_app_config_rejects_nan_trims_subkey_and_validates_direct_trait() {
        assert!(matches!(encode_propvar_r4(f32::NAN), Err(WinctlError::InvalidInput(_))));
        assert!(matches!(
            encode_propvar_r4(f32::INFINITY),
            Err(WinctlError::InvalidInput(_))
        ));
        assert!(matches!(
            encode_propvar_r4_array(&[0.5, f32::NAN]),
            Err(WinctlError::InvalidInput(_))
        ));
        assert!(matches!(
            encode_propvar_r4_array(&[f32::NEG_INFINITY]),
            Err(WinctlError::InvalidInput(_))
        ));

        let mock = Arc::new(MockAudioBackend::with_sample_data());
        let action = crate::app_config::AppAudioConfigUpdateAction::new(Arc::clone(&mock) as Arc<dyn AudioBackend>);
        let preview =
            guap_mcp::ActionCommand::execute(&action, &json!({ "subkey": "  186430e1_0  ", "muted": true }), true)
                .unwrap();
        assert_eq!(preview["plan"]["subkey"], json!("186430e1_0"));

        let applied =
            guap_mcp::ActionCommand::execute(&action, &json!({ "subkey": "  186430e1_0  ", "muted": true }), false)
                .unwrap();
        assert_eq!(applied["subkey"], json!("186430e1_0"));
        assert!(
            mock.action_log
                .lock()
                .unwrap()
                .iter()
                .any(|s| s == "write_app_audio_config:186430e1_0")
        );

        let bad_patch = crate::backend::AppPropertyStorePatch {
            delete_subkey: true,
            ..Default::default()
        };
        assert!(matches!(
            mock.write_app_audio_config("", &bad_patch),
            Err(WinctlError::InvalidInput(_))
        ));
        assert!(matches!(
            mock.write_app_audio_config("../bad", &bad_patch),
            Err(WinctlError::InvalidInput(_))
        ));
    }

    #[test]
    fn regression_svcl_quoting_empty_targets_percent_roles_and_view_control() {
        let q1 = parse_svcl_command(r#"/SetVolume "/dev/audio" 50"#).unwrap();
        assert_eq!(
            q1.verb,
            SvclVerb::SetVolume {
                target: "/dev/audio".to_string(),
                percent: 50.0,
            }
        );
        let q2 = parse_svcl_command(r#"/Mute "--my-speaker""#).unwrap();
        assert_eq!(
            q2.verb,
            SvclVerb::Mute {
                target: "--my-speaker".to_string(),
            }
        );

        assert!(parse_svcl_command(r#"/SetVolume "" 50"#).is_err());
        assert!(parse_svcl_command(r#"/SetVolumeChannels "  " 50 50"#).is_err());
        assert!(parse_svcl_command(r#"/ChangeVolume "" 10"#).is_err());
        assert!(parse_svcl_command(r#"/SetVolumeDecibel "" -6"#).is_err());
        assert!(parse_svcl_command(r#"/ChangeVolumeDecibel "" 3"#).is_err());
        assert!(parse_svcl_command(r#"/SetDefault "" all"#).is_err());
        assert!(parse_svcl_command(r#"/SetAppDefault "" all vlc.exe"#).is_err());
        assert!(parse_svcl_command(r#"/SetAppDefault "Speakers" all """#).is_err());

        assert!(parse_svcl_command("/SetVolume Speakers 50%").is_ok());
        assert!(parse_svcl_command("/SetVolume Speakers 50%%").is_err());

        assert!(matches!(
            DefaultRole::parse_role_list("0,all,invalid"),
            Err(WinctlError::InvalidInput(_))
        ));

        let mock = Arc::new(MockAudioBackend::with_sample_data());
        let view_action = crate::view::AudioViewControlAction::new(Arc::clone(&mock) as Arc<dyn AudioBackend>);
        assert!(
            guap_mcp::ActionCommand::execute(
                &view_action,
                &json!({ "action": "set_volume", "target": "", "volume_percent": 50.0 }),
                true,
            )
            .is_err()
        );
        assert!(
            guap_mcp::ActionCommand::execute(
                &view_action,
                &json!({ "action": "set_volume", "target": "   ", "volume_percent": 50.0 }),
                true,
            )
            .is_err()
        );

        let svcl_action = crate::svcl::SvclAction::new(mock);
        let structured = guap_mcp::ActionCommand::execute(
            &svcl_action,
            &json!({ "verb": "/SetVolume", "target": "/dev/audio", "args": ["50"] }),
            true,
        )
        .unwrap();
        assert_eq!(structured["plan"]["target"], json!("/dev/audio"));
    }
}
