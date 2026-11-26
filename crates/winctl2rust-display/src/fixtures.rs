use crate::backend::{DdcMonitorSnapshot, DdcVcpReading, MonitorOrientation, MonitorTopologyInfo, RawEdidEntry};
use crate::edid::parse_edid_hex_text;

pub const SYNTHETIC_EDID_256_BLOCK0_HEX: &str = "00FFFFFFFFFFFF001EB00127785634120C240104A53C2278EEEE91A3544C99260F5054210800D1C081C0010101010101010101010101023A801871382D40582C450056502100001E000000FD0030901EA51E000A202020202020000000FC00475541502D553237510A202020000000FF00534E32303236303030310A202001DE";
pub const SYNTHETIC_EDID_256_BLOCK1_HEX: &str = "02031671439004612309070765030C00100083010000565E00A0A0A029503020350056502100001E00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000083";

pub const SYNTHETIC_DDC_CAPABILITIES: &str = "(prot(monitor)type(lcd)model(GUAP-U27Q)cmds(01 02 03 07 0C E3 F3)vcp(02 04 05 08 10 12 14(01 04 05 08 0B) 16 18 1A 52 60(0F 11 12 1B) 62 8D(01 02) AC AE B6 C6 C8 C9 CA(01 02) CC(02 03 04 0A) D6(01 02 03 04 05) DF)mswhql(1)asset_eep(40)mccs_ver(2.2))";

pub fn synthetic_edid_256_bytes() -> Vec<u8> {
    let combined = format!("{SYNTHETIC_EDID_256_BLOCK0_HEX}{SYNTHETIC_EDID_256_BLOCK1_HEX}");
    parse_edid_hex_text(&combined).expect("synthetic 256-byte EDID fixture must parse")
}

pub fn sample_monitors() -> Vec<MonitorTopologyInfo> {
    vec![
        MonitorTopologyInfo {
            monitor_id: "GUP2701".to_string(),
            device_name: "\\\\.\\DISPLAY1".to_string(),
            friendly_name: "GUAP-U27Q".to_string(),
            device_interface_id: "\\\\?\\DISPLAY#GUP2701#5&2de1bb75&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
                .to_string(),
            primary: true,
            active: true,
            width: 1920,
            height: 1080,
            position_x: 0,
            position_y: 0,
            orientation: MonitorOrientation::Deg0,
            bpp: 32,
            refresh_hz: 60,
            scale_pct: 100,
        },
        MonitorTopologyInfo {
            monitor_id: "GUP2702".to_string(),
            device_name: "\\\\.\\DISPLAY2".to_string(),
            friendly_name: "GUAP-S27P".to_string(),
            device_interface_id: "\\\\?\\DISPLAY#GUP2702#5&2de1bb75&0&UID4357#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
                .to_string(),
            primary: false,
            active: true,
            width: 2560,
            height: 1440,
            position_x: 1920,
            position_y: 0,
            orientation: MonitorOrientation::Deg0,
            bpp: 32,
            refresh_hz: 144,
            scale_pct: 125,
        },
    ]
}

pub fn sample_edid_entries() -> Vec<RawEdidEntry> {
    vec![RawEdidEntry {
        registry_path:
            "HKLM\\SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\GUP2701\\5&2de1bb75&0&UID4352\\Device Parameters\\EDID"
                .to_string(),
        monitor_id: "GUP2701".to_string(),
        instance_id: "5&2de1bb75&0&UID4352".to_string(),
        device_interface_id: "\\\\?\\DISPLAY#GUP2701#5&2de1bb75&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
            .to_string(),
        active: true,
        raw_bytes: synthetic_edid_256_bytes(),
    }]
}

pub fn sample_ddc_snapshots() -> Vec<DdcMonitorSnapshot> {
    vec![DdcMonitorSnapshot {
        monitor_id: "GUP2701".to_string(),
        monitor_name: "GUAP-U27Q".to_string(),
        device_name: "\\\\.\\DISPLAY1".to_string(),
        capabilities_raw: Some(SYNTHETIC_DDC_CAPABILITIES.to_string()),
        vcp_readings: vec![
            DdcVcpReading {
                vcp_code: 0x10,
                vcp_name: "Luminance (Brightness)".to_string(),
                current_value: 70,
                max_value: 100,
                is_momentary: false,
                possible_values: vec![],
                read_only: false,
            },
            DdcVcpReading {
                vcp_code: 0x12,
                vcp_name: "Contrast".to_string(),
                current_value: 50,
                max_value: 100,
                is_momentary: false,
                possible_values: vec![],
                read_only: false,
            },
            DdcVcpReading {
                vcp_code: 0x60,
                vcp_name: "Input Source".to_string(),
                current_value: 0x0F,
                max_value: 0x1B,
                is_momentary: false,
                possible_values: vec![0x0F, 0x11, 0x12, 0x1B],
                read_only: false,
            },
            DdcVcpReading {
                vcp_code: 0xDF,
                vcp_name: "VCP Version".to_string(),
                current_value: 0x0202,
                max_value: 0xFFFF,
                is_momentary: false,
                possible_values: vec![],
                read_only: true,
            },
        ],
    }]
}
