#![deny(unsafe_code)]

pub mod backend;
pub mod ddc;
pub mod edid;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
pub mod info;
pub mod monitors;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod win32_backend;

use std::sync::Arc;

use guap_mcp::ActionCommand;
use guap_table::TableProvider;
use winctl2rust_core::{ToolBundle, ToolCategory, builtin_suite_catalog};

#[cfg(any(test, feature = "test-support"))]
pub use backend::MockDisplayBackend;
pub use backend::{DisplayBackend, UnsupportedDisplayBackend};
#[cfg(windows)]
pub use win32_backend::Win32DisplayBackend;

pub fn default_backend() -> Arc<dyn DisplayBackend> {
    #[cfg(windows)]
    {
        Arc::new(Win32DisplayBackend::new())
    }
    #[cfg(not(windows))]
    {
        Arc::new(UnsupportedDisplayBackend)
    }
}

pub fn providers(backend: Arc<dyn DisplayBackend>) -> Vec<Arc<dyn TableProvider>> {
    vec![
        Arc::new(monitors::MonitorsTableProvider::new(Arc::clone(&backend))),
        Arc::new(ddc::DdcVcpTableProvider::new(Arc::clone(&backend))),
        Arc::new(info::MonitorInfoTableProvider::new(Arc::clone(&backend))),
        Arc::new(edid::EdidTableProvider::new(backend)),
    ]
}

pub fn actions(backend: Arc<dyn DisplayBackend>) -> Vec<Arc<dyn ActionCommand>> {
    vec![
        Arc::new(monitors::MonitorsApplyAction::new(Arc::clone(&backend))),
        Arc::new(ddc::DdcSetVcpAction::new(backend)),
    ]
}

pub fn bundle(backend: Arc<dyn DisplayBackend>) -> ToolBundle {
    let tools = builtin_suite_catalog()
        .iter()
        .filter(|t| t.category == ToolCategory::Display)
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
    use tempfile::tempdir;
    use winctl2rust_core::{ToolCategory, WinctlError, builtin_suite_catalog};

    use crate::backend::{
        DisplayBackend, MockDisplayBackend, MonitorApplyOp, MonitorConfigProfile, MonitorOrientation,
        UnsupportedDisplayBackend, topology_to_config_entries,
    };
    use crate::ddc::{parse_capabilities_string, parse_vcp_code, vcp_code_info, vcp_dictionary};
    use crate::edid::{
        compute_edid_checksum_byte, edid_table_from_bytes, edid_table_from_hex, parse_edid_bytes, parse_edid_input,
        verify_edid_block_checksum,
    };
    use crate::fixtures::{
        SYNTHETIC_DDC_CAPABILITIES, SYNTHETIC_EDID_256_BLOCK0_HEX, SYNTHETIC_EDID_256_BLOCK1_HEX, sample_monitors,
        synthetic_edid_256_bytes,
    };
    use crate::monitors::{parse_monitor_config_auto, parse_monitor_config_ini, serialize_monitor_config_ini};
    use crate::{bundle, providers};

    #[test]
    fn bundle_matches_core_suite_catalog_exact_ids() {
        let mock = Arc::new(MockDisplayBackend::with_sample_data());
        let display_bundle = bundle(mock);
        let expected: Vec<_> = builtin_suite_catalog()
            .iter()
            .filter(|t| t.category == ToolCategory::Display)
            .collect();
        assert_eq!(display_bundle.tools.len(), 4);
        assert_eq!(display_bundle.providers.len(), 4);
        assert_eq!(display_bundle.actions.len(), 2);

        for tool in expected {
            for tid in tool.table_ids {
                assert!(
                    display_bundle.find_provider(tid).is_some(),
                    "missing TableProvider '{tid}'"
                );
            }
            for aid in tool.action_ids {
                let action = display_bundle
                    .find_action(aid)
                    .unwrap_or_else(|| panic!("missing ActionCommand '{aid}'"));
                assert_eq!(action.risk_level(), tool.risk_level);
            }
        }
    }

    #[test]
    fn edid_256_byte_fixture_checksums_and_fields_parse_accurately() {
        let bytes = synthetic_edid_256_bytes();
        assert_eq!(bytes.len(), 256);
        assert!(verify_edid_block_checksum(&bytes[0..128]));
        assert!(verify_edid_block_checksum(&bytes[128..256]));
        assert_eq!(compute_edid_checksum_byte(&bytes[0..127]), bytes[127]);
        assert_eq!(compute_edid_checksum_byte(&bytes[128..255]), bytes[255]);

        let parsed = parse_edid_bytes(&bytes).unwrap();
        assert_eq!(parsed.manufacturer_id, "GUP");
        assert_eq!(parsed.product_code, 0x2701);
        assert_eq!(parsed.product_code_hex, "0x2701");
        assert_eq!(parsed.serial_number_u32, 0x1234_5678);
        assert_eq!(parsed.manufacture_week, Some(12));
        assert_eq!(parsed.manufacture_year, 2026);
        assert_eq!(parsed.edid_version, "1.4");
        assert!(parsed.video_input.is_digital);
        assert_eq!(parsed.video_input.color_bit_depth, Some(8));
        assert_eq!(parsed.video_input.digital_interface.as_deref(), Some("DisplayPort"));
        assert_eq!(parsed.width_cm, Some(60));
        assert_eq!(parsed.height_cm, Some(34));
        assert_eq!(parsed.gamma, Some(2.2));
        assert_eq!(parsed.monitor_name.as_deref(), Some("GUAP-U27Q"));
        assert_eq!(parsed.ascii_serial.as_deref(), Some("SN20260001"));
        assert!(parsed.all_checksums_valid);
        assert_eq!(parsed.cea_extensions.len(), 1);

        let cea = &parsed.cea_extensions[0];
        assert_eq!(cea.revision, 3);
        assert!(cea.basic_audio);
        assert!(cea.ycbcr_444);
        assert!(cea.ycbcr_422);
        assert_eq!(cea.video_descriptors.len(), 3);
        assert_eq!(cea.video_descriptors[0].vic, 16);
        assert!(cea.video_descriptors[0].native);
        assert_eq!(cea.audio_descriptors.len(), 1);
        assert_eq!(cea.audio_descriptors[0].format_name, "LPCM");
        assert_eq!(cea.vendor_specific[0].ieee_oui, "00-0C-03");
        assert_eq!(cea.detailed_timings.len(), 1);
        assert_eq!(cea.detailed_timings[0].h_active, 2560);
        assert_eq!(cea.detailed_timings[0].v_active, 1440);

        let (max_w, max_h, _) = parsed.max_resolution();
        assert_eq!((max_w, max_h), (2560, 1440));

        // Flip 1 byte in unused descriptor padding -> checksum must report false
        let mut bad_chk = bytes;
        bad_chk[125] ^= 0x01;
        let parsed_bad = parse_edid_bytes(&bad_chk).unwrap();
        assert!(!parsed_bad.base_checksum_valid);
        assert!(!parsed_bad.all_checksums_valid);
    }

    #[test]
    fn edid_input_parser_handles_bin_hex_dumpedid_and_reg_formats() {
        let raw = synthetic_edid_256_bytes();
        assert_eq!(parse_edid_input(&raw).unwrap(), raw);

        // .reg format with line continuations
        let block0_commas: Vec<String> = raw[..128].iter().map(|b| format!("{b:02x}")).collect();
        let reg_text = format!(
            "Windows Registry Editor Version 5.00\n\n[HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\GUP2701\\0\\Device Parameters]\n\"EDID\"=hex:{},\\\n  {}\n",
            block0_commas[..64].join(","),
            block0_commas[64..].join(","),
        );
        let parsed_reg = parse_edid_input(reg_text.as_bytes()).unwrap();
        assert_eq!(parsed_reg, raw[..128]);

        // DumpEDID offset format with trailing ASCII column
        let mut dump_lines = String::new();
        for (row_idx, chunk) in raw.chunks(16).enumerate() {
            let hex_cols: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
            dump_lines.push_str(&format!(
                "{:08X}: {} | ................\n",
                row_idx * 16,
                hex_cols.join(" ")
            ));
        }
        let parsed_dump = parse_edid_input(dump_lines.as_bytes()).unwrap();
        assert_eq!(parsed_dump, raw);

        // Offline table helpers
        let t_bin = edid_table_from_bytes("fixture.bin", &raw, &TableQuery::default()).unwrap();
        assert_eq!(t_bin.rows.len(), 1);
        let combined_hex = format!("{SYNTHETIC_EDID_256_BLOCK0_HEX}{SYNTHETIC_EDID_256_BLOCK1_HEX}");
        let t_hex = edid_table_from_hex("fixture.hex", &combined_hex, &TableQuery::default()).unwrap();
        assert_eq!(t_hex.rows.len(), 1);

        // Corrupted magic rejected
        let mut bad = raw;
        bad[0] = 0xAA;
        assert!(matches!(parse_edid_bytes(&bad), Err(WinctlError::ParseError(_))));
    }

    #[test]
    fn ddc_capabilities_parser_and_vcp_dictionary_work() {
        assert!(!vcp_dictionary().is_empty());
        assert_eq!(vcp_code_info(0x10).unwrap().name, "Luminance (Brightness)");
        assert!(vcp_code_info(0xDF).unwrap().read_only);
        assert_eq!(parse_vcp_code("0x10").unwrap(), 0x10);
        assert_eq!(parse_vcp_code("16").unwrap(), 16);
        assert_eq!(parse_vcp_code("DF").unwrap(), 0xDF);
        assert!(parse_vcp_code("").is_err());

        let caps = parse_capabilities_string(SYNTHETIC_DDC_CAPABILITIES).unwrap();
        assert_eq!(caps.prot.as_deref(), Some("monitor"));
        assert_eq!(caps.display_type.as_deref(), Some("lcd"));
        assert_eq!(caps.model.as_deref(), Some("GUAP-U27Q"));
        assert_eq!(caps.mccs_ver.as_deref(), Some("2.2"));
        assert!(caps.cmds.contains(&0x01));
        assert_eq!(caps.vcp.get(&0x14).unwrap(), &vec![0x01, 0x04, 0x05, 0x08, 0x0B]);
        assert_eq!(caps.vcp.get(&0x60).unwrap(), &vec![0x0F, 0x11, 0x12, 0x1B]);
        assert!(caps.vcp.get(&0x10).unwrap().is_empty());

        assert!(parse_capabilities_string("(prot(monitor").is_err());
    }

    #[test]
    fn multimonitortool_ini_and_json_profiles_roundtrip() {
        let profile = MonitorConfigProfile {
            monitors: topology_to_config_entries(&sample_monitors()),
        };
        let ini = serialize_monitor_config_ini(&profile);
        let parsed_ini = parse_monitor_config_ini(&ini).unwrap();
        assert_eq!(parsed_ini, profile);

        let json_str = serde_json::to_string_pretty(&profile).unwrap();
        let parsed_json = parse_monitor_config_auto(&json_str).unwrap();
        assert_eq!(parsed_json, profile);

        // Reject disabling the last active monitor
        let mock = MockDisplayBackend::with_sample_data();
        mock.apply_monitor_action(&MonitorApplyOp::Disable {
            target: "\\\\.\\DISPLAY2".to_string(),
        })
        .unwrap();
        let err = mock
            .apply_monitor_action(&MonitorApplyOp::Disable {
                target: "\\\\.\\DISPLAY1".to_string(),
            })
            .unwrap_err();
        assert!(matches!(err, WinctlError::InvalidInput(_)));

        // Orientation swap 0 -> 90 swaps width/height
        let res = mock
            .apply_monitor_action(&MonitorApplyOp::SetOrientation {
                target: "\\\\.\\DISPLAY1".to_string(),
                orientation: MonitorOrientation::Deg90,
            })
            .unwrap();
        assert_eq!(res.topology[0].width, 1080);
        assert_eq!(res.topology[0].height, 1920);
    }

    #[test]
    fn mock_backend_fault_injection_and_unsupported_backend_work() {
        let unsup = UnsupportedDisplayBackend;
        assert!(matches!(
            unsup.list_monitors(),
            Err(WinctlError::UnsupportedOnPlatform(_))
        ));

        let mock = MockDisplayBackend::with_sample_data();
        mock.inject_hardware_unavailable.store(true, Ordering::Relaxed);
        assert!(matches!(
            mock.query_ddc_vcp(None),
            Err(WinctlError::HardwareUnavailable(_))
        ));

        mock.inject_hardware_unavailable.store(false, Ordering::Relaxed);
        mock.inject_permission_denied.store(true, Ordering::Relaxed);
        assert!(matches!(
            mock.list_edid_records(),
            Err(WinctlError::PermissionDenied(_))
        ));
    }

    #[test]
    fn mcp_policy_gates_and_dry_run_work_end_to_end() {
        let mock = Arc::new(MockDisplayBackend::with_sample_data());
        let display_bundle = bundle(Arc::clone(&mock) as Arc<dyn DisplayBackend>);
        let mut registry = McpServerRegistry::new(PolicyMode::ReadOnlyOnly);
        display_bundle.register_into(&mut registry);

        let list = registry.list_tools();
        assert_eq!(list.as_array().unwrap().len(), 6);

        for table_id in ["display_monitors", "display_ddc_vcp", "display_info", "display_edid"] {
            let res = registry.dispatch(table_id, &json!({})).unwrap();
            assert!(res["rows"].is_array(), "missing rows for {table_id}");
            assert!(!res["rows"].as_array().unwrap().is_empty());
        }

        let info_res = registry.dispatch("display_info", &json!({})).unwrap();
        let info_row = &info_res["rows"][0];
        assert_eq!(info_row["monitor_name"], json!("GUAP-U27Q"));
        assert_eq!(info_row["vendor_id"], json!("GUP"));
        assert_eq!(info_row["product_id"], json!("0x2701"));
        assert_eq!(info_row["active"], json!(true));

        let dir = tempdir().unwrap();
        let profile_path = dir.path().join("monitors.ini");

        let mutating_calls = [
            (
                "display_monitors_apply",
                json!({ "operation": "set_primary", "target": "\\\\.\\DISPLAY2", "dry_run": true }),
            ),
            (
                "display_monitors_apply",
                json!({ "operation": "save_config", "path": profile_path, "format": "ini", "dry_run": true }),
            ),
            (
                "display_ddc_set",
                json!({ "monitor": "GUAP-U27Q", "vcp_code": "0x10", "value": 85, "dry_run": true }),
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
        assert!(!profile_path.exists());

        // Reject read-only VCP code write
        let ro_err = registry
            .dispatch(
                "display_ddc_set",
                &json!({ "monitor": "GUAP-U27Q", "vcp_code": "0xDF", "value": 1, "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap_err();
        assert_eq!(ro_err.code, JsonRpcError::INVALID_PARAMS);

        // Execute real mock mutations
        let res_save = registry
            .dispatch(
                "display_monitors_apply",
                &json!({ "operation": "save_config", "path": profile_path, "format": "ini", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_save["applied"], json!(true));
        assert!(profile_path.exists());

        let res_pri = registry
            .dispatch(
                "display_monitors_apply",
                &json!({ "operation": "set_primary", "target": "\\\\.\\DISPLAY2", "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_pri["applied"], json!(true));

        let res_ddc = registry
            .dispatch(
                "display_ddc_set",
                &json!({ "monitor": "GUAP-U27Q", "vcp_code": "0x10", "value": 85, "confirm_destructive": true, "dry_run": false }),
            )
            .unwrap();
        assert_eq!(res_ddc["applied"], json!(true));
        assert_eq!(res_ddc["result"]["new_value"], json!(85));

        let ddc_rows = providers(mock)[1].query(&TableQuery::default()).unwrap();
        assert!(!ddc_rows.rows.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn win32_display_backend_read_only_smoke() {
        let b = crate::default_bundle();
        assert_eq!(b.tools.len(), 4);
        assert_eq!(b.providers.len(), 4);
        assert_eq!(b.actions.len(), 2);

        let backend = crate::Win32DisplayBackend::new();
        let mons = backend.list_monitors().unwrap();
        for m in &mons {
            assert!(!m.device_name.is_empty());
        }

        let edids = backend.list_edid_records().unwrap();
        for rec in &edids {
            assert!(!rec.registry_path.is_empty());
            if rec.raw_bytes.len() >= 128 && rec.raw_bytes[0..8] == crate::edid::EDID_HEADER_MAGIC {
                let parsed = parse_edid_bytes(&rec.raw_bytes).unwrap();
                assert!(!parsed.manufacturer_id.is_empty());
            }
        }

        // Defense-in-depth: LoadConfig with zero active monitors is rejected directly by Win32DisplayBackend
        let mut inactive = topology_to_config_entries(&sample_monitors());
        for m in &mut inactive {
            m.active = false;
        }
        assert!(matches!(
            backend.apply_monitor_action(&MonitorApplyOp::LoadConfig {
                profile: MonitorConfigProfile { monitors: inactive },
            }),
            Err(WinctlError::InvalidInput(_))
        ));
    }

    #[test]
    fn regression_edid_non_ascii_colon_hex3_and_missing_extensions() {
        // Non-ASCII single token >= 256 bytes must return ParseError without panicking
        let non_ascii = format!("a{}b", "\u{00E9}".repeat(127));
        assert!(matches!(
            crate::edid::parse_edid_hex_text(&non_ascii),
            Err(WinctlError::ParseError(_))
        ));

        // Colon-separated 128-byte and 256-byte EDID hex strings must not drop byte 0 ("00:")
        let raw = synthetic_edid_256_bytes();
        let colon_128 = raw[..128]
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":");
        assert_eq!(crate::edid::parse_edid_hex_text(&colon_128).unwrap(), raw[..128]);

        let colon_256 = raw.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
        assert_eq!(crate::edid::parse_edid_hex_text(&colon_256).unwrap(), raw);

        // .reg hex(3): explicit REG_BINARY format
        let block0_commas: Vec<String> = raw[..128].iter().map(|b| format!("{b:02x}")).collect();
        let reg_hex3 = format!(
            "Windows Registry Editor Version 5.00\n\n[HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\GUP2701\\0\\Device Parameters]\n\"EDID\"=hex(3):{},\\\n  {}\n",
            block0_commas[..64].join(","),
            block0_commas[64..].join(","),
        );
        assert_eq!(parse_edid_input(reg_hex3.as_bytes()).unwrap(), raw[..128]);

        // Declared extension blocks missing from buffer -> all_checksums_valid must be false
        let parsed_trunc_ext = parse_edid_bytes(&raw[..128]).unwrap();
        assert_eq!(parsed_trunc_ext.extension_count, 1);
        assert!(parsed_trunc_ext.base_checksum_valid);
        assert!(!parsed_trunc_ext.all_checksums_valid);
    }

    #[test]
    fn regression_ddc_enforces_max_value_and_possible_values() {
        let mock = Arc::new(MockDisplayBackend::with_sample_data());
        let ddc_action = crate::ddc::DdcSetVcpAction::new(Arc::clone(&mock) as Arc<dyn DisplayBackend>);

        // 0x10 Luminance has max_value = 100
        assert!(
            guap_mcp::ActionCommand::execute(
                &ddc_action,
                &json!({ "monitor": "GUAP-U27Q", "vcp_code": "0x10", "value": 150 }),
                false,
            )
            .is_err()
        );
        assert!(
            guap_mcp::ActionCommand::execute(
                &ddc_action,
                &json!({ "monitor": "GUAP-U27Q", "vcp_code": "0x10", "value": 150 }),
                true,
            )
            .is_err()
        );
        assert!(matches!(
            mock.set_ddc_vcp(&crate::backend::DdcSetVcpRequest {
                monitor: "GUAP-U27Q".to_string(),
                vcp_code: 0x10,
                value: 150,
            }),
            Err(WinctlError::InvalidInput(_))
        ));

        // 0x60 Input Source has possible_values = [0x0F, 0x11, 0x12, 0x1B]
        assert!(
            guap_mcp::ActionCommand::execute(
                &ddc_action,
                &json!({ "monitor": "GUAP-U27Q", "vcp_code": "0x60", "value": 0x99 }),
                false,
            )
            .is_err()
        );
        assert!(
            guap_mcp::ActionCommand::execute(
                &ddc_action,
                &json!({ "monitor": "GUAP-U27Q", "vcp_code": "0x60", "value": 0x99 }),
                true,
            )
            .is_err()
        );
        assert!(matches!(
            mock.set_ddc_vcp(&crate::backend::DdcSetVcpRequest {
                monitor: "GUAP-U27Q".to_string(),
                vcp_code: 0x60,
                value: 0x99,
            }),
            Err(WinctlError::InvalidInput(_))
        ));

        // Valid discrete value succeeds
        let ok_res = guap_mcp::ActionCommand::execute(
            &ddc_action,
            &json!({ "monitor": "GUAP-U27Q", "vcp_code": "0x60", "value": 0x11 }),
            false,
        )
        .unwrap();
        assert_eq!(ok_res["result"]["new_value"], json!(0x11));
    }

    #[test]
    fn regression_ini_preserves_order_for_eleven_plus_monitors() {
        let mut entries = Vec::new();
        for i in 0..11u32 {
            entries.push(crate::backend::MonitorConfigEntry {
                monitor_id: format!("GUP27{i:02}"),
                name: format!("\\\\.\\DISPLAY{i}"),
                friendly_name: format!("Monitor {i}"),
                active: true,
                primary: i == 0,
                width: 1920,
                height: 1080,
                bpp: 32,
                display_frequency: 60,
                position_x: (i as i32) * 1920,
                position_y: 0,
                orientation: MonitorOrientation::Deg0,
            });
        }
        let profile = MonitorConfigProfile { monitors: entries };
        let ini = serialize_monitor_config_ini(&profile);
        let parsed = parse_monitor_config_ini(&ini).unwrap();
        assert_eq!(parsed.monitors.len(), 11);
        assert_eq!(parsed.monitors[2].name, "\\\\.\\DISPLAY2");
        assert_eq!(parsed.monitors[10].name, "\\\\.\\DISPLAY10");
        assert_eq!(parsed, profile);
    }

    #[test]
    fn regression_correlate_monitor_info_does_not_mark_historical_duplicate_active() {
        let monitors = sample_monitors();
        let mut edids = crate::fixtures::sample_edid_entries();
        edids.push(crate::backend::RawEdidEntry {
            registry_path:
                "HKLM\\SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\GUP2701\\5&old&0&UID9999\\Device Parameters\\EDID"
                    .to_string(),
            monitor_id: "GUP2701".to_string(),
            instance_id: "5&old&0&UID9999".to_string(),
            device_interface_id: "\\\\?\\DISPLAY#GUP2701#5&old&0&UID9999#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
                .to_string(),
            active: false,
            raw_bytes: synthetic_edid_256_bytes(),
        });

        let correlated = crate::info::correlate_monitor_info(&edids, &monitors);
        assert_eq!(correlated.len(), 2);
        assert!(correlated[0].0.active);
        assert!(!correlated[1].0.active);
    }
}
