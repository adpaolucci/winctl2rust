use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::{AppPropertyStorePatch, AudioBackend, RawAppPropertyStoreRecord};

pub const POLICY_STORE_KEY_PREFIX: &str =
    r"Software\Microsoft\Internet Explorer\LowRegistry\Audio\PolicyConfig\PropertyStore";
pub const AUDIO_APP_PROPERTY_BAG_GUID: &str = "{219ED5A0-9CBF-4F3A-B927-37C9E5C5F14F}";
pub const SERIALIZED_PROPSTORAGE_MAGIC: u32 = 0x5350_5331; // "1SPS"

pub const VT_EMPTY: u16 = 0x0000;
pub const VT_I4: u16 = 0x0003;
pub const VT_R4: u16 = 0x0004;
pub const VT_BOOL: u16 = 0x000B;
pub const VT_UI4: u16 = 0x0013;
pub const VT_LPWSTR: u16 = 0x001F;
pub const VT_BLOB: u16 = 0x0041;
pub const VT_VECTOR_R4: u16 = 0x1004;
pub const VT_ARRAY_R4: u16 = 0x2004;

/// Mixed-endian Windows GUID for `FMTID_UserDefinedProperties` `{D5CDD505-2E9C-101B-9397-08002B2CF9AE}`
pub const FMTID_USER_DEFINED_PROPERTIES: [u8; 16] = [
    0x05, 0xD5, 0xCD, 0xD5, 0x9C, 0x2E, 0x1B, 0x10, 0x93, 0x97, 0x08, 0x00, 0x2B, 0x2C, 0xF9, 0xAE,
];

/// Mixed-endian Windows GUID for `{219ED5A0-9CBF-4F3A-B927-37C9E5C5F14F}`
pub const FMTID_AUDIO_POLICY_BAG: [u8; 16] = [
    0xA0, 0xD5, 0x9E, 0x21, 0xBF, 0x9C, 0x3A, 0x4F, 0xB9, 0x27, 0x37, 0xC9, 0xE5, 0xC5, 0xF1, 0x4F,
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedAppConfigEntry {
    pub subkey: String,
    pub registry_key: String,
    pub device_id: String,
    pub app_path: String,
    pub app_name: String,
    pub grouping_guid: String,
    pub volume_scalar: Option<f32>,
    pub volume_percent: Option<f32>,
    pub muted: Option<bool>,
    pub channel_balances: Vec<f32>,
    pub raw_format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SerializedPropertyValue {
    Empty,
    I4(i32),
    U4(u32),
    R4(f32),
    Bool(bool),
    Lpwstr(String),
    Blob(Vec<u8>),
    R4List(Vec<f32>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyKeyId {
    Pid(u32),
    Name(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SerializedPropertyEntry {
    pub key: PropertyKeyId,
    pub vt: u16,
    pub value: SerializedPropertyValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SerializedPropertyStorage {
    pub fmtid: [u8; 16],
    pub fmtid_guid: String,
    pub entries: Vec<SerializedPropertyEntry>,
}

pub fn format_mixed_endian_guid(bytes: &[u8; 16]) -> String {
    let d1 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let d2 = u16::from_le_bytes([bytes[4], bytes[5]]);
    let d3 = u16::from_le_bytes([bytes[6], bytes[7]]);
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        d1, d2, d3, bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

pub fn encode_propvar_r4(scalar: f32) -> WinctlResult<Vec<u8>> {
    if !scalar.is_finite() {
        return Err(WinctlError::InvalidInput(
            "VT_R4 volume scalar must be finite".to_string(),
        ));
    }
    let mut out = vec![0u8; 24];
    out[0..2].copy_from_slice(&VT_R4.to_le_bytes());
    out[8..12].copy_from_slice(&scalar.clamp(0.0, 1.0).to_le_bytes());
    Ok(out)
}

pub fn decode_propvar_r4(bytes: &[u8]) -> WinctlResult<f32> {
    if bytes.len() < 12 {
        return Err(WinctlError::ParseError(format!(
            "VT_R4 PROPVARIANT blob too short ({} bytes, need >= 12)",
            bytes.len()
        )));
    }
    let vt = u16::from_le_bytes([bytes[0], bytes[1]]);
    if vt != VT_R4 {
        return Err(WinctlError::ParseError(format!(
            "expected VT_R4 (0x0004), found vt={vt:#06x}"
        )));
    }
    let val = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if !val.is_finite() {
        return Err(WinctlError::ParseError("non-finite VT_R4 volume scalar".to_string()));
    }
    Ok(val)
}

pub fn encode_propvar_bool(muted: bool) -> Vec<u8> {
    let mut out = vec![0u8; 24];
    out[0..2].copy_from_slice(&VT_BOOL.to_le_bytes());
    let raw: i16 = if muted { -1 } else { 0 };
    out[8..10].copy_from_slice(&raw.to_le_bytes());
    out
}

pub fn decode_propvar_bool(bytes: &[u8]) -> WinctlResult<bool> {
    if bytes.len() < 10 {
        return Err(WinctlError::ParseError(format!(
            "VT_BOOL PROPVARIANT blob too short ({} bytes, need >= 10)",
            bytes.len()
        )));
    }
    let vt = u16::from_le_bytes([bytes[0], bytes[1]]);
    if vt != VT_BOOL {
        return Err(WinctlError::ParseError(format!(
            "expected VT_BOOL (0x000B), found vt={vt:#06x}"
        )));
    }
    let raw = i16::from_le_bytes([bytes[8], bytes[9]]);
    Ok(raw != 0)
}

pub fn encode_propvar_r4_array(channels: &[f32]) -> WinctlResult<Vec<u8>> {
    if channels.iter().any(|c| !c.is_finite()) {
        return Err(WinctlError::InvalidInput(
            "VT_ARRAY|VT_R4 channel scalars must all be finite".to_string(),
        ));
    }
    let mut out = Vec::with_capacity(24 + channels.len() * 4);
    out.extend_from_slice(&VT_ARRAY_R4.to_le_bytes());
    out.extend_from_slice(&[0u8; 6]);
    out.extend_from_slice(&24u64.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    for &ch in channels {
        out.extend_from_slice(&ch.clamp(0.0, 1.0).to_le_bytes());
    }
    Ok(out)
}

pub fn decode_propvar_r4_array(bytes: &[u8]) -> WinctlResult<Vec<f32>> {
    if bytes.len() < 16 {
        return Err(WinctlError::ParseError(format!(
            "VT_ARRAY|VT_R4 blob too short ({} bytes)",
            bytes.len()
        )));
    }
    let vt = u16::from_le_bytes([bytes[0], bytes[1]]);
    if vt != VT_ARRAY_R4 && vt != VT_VECTOR_R4 {
        return Err(WinctlError::ParseError(format!(
            "expected VT_ARRAY|VT_R4 (0x2004), found vt={vt:#06x}"
        )));
    }
    let offset_64 = if bytes.len() >= 24 {
        u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize
    } else {
        16usize
    };
    let data_start = if (offset_64 == 24 || offset_64 == 16) && bytes.len() >= offset_64 {
        offset_64
    } else if bytes.len() >= 24 {
        24
    } else {
        16
    };
    let payload = &bytes[data_start..];
    if !payload.len().is_multiple_of(4) {
        return Err(WinctlError::ParseError(format!(
            "VT_ARRAY|VT_R4 channel payload length {} is not a multiple of 4",
            payload.len()
        )));
    }
    let mut out = Vec::with_capacity(payload.len() / 4);
    for chunk in payload.as_chunks::<4>().0 {
        let v = f32::from_le_bytes(*chunk);
        if !v.is_finite() {
            return Err(WinctlError::ParseError(
                "non-finite channel scalar in array".to_string(),
            ));
        }
        out.push(v);
    }
    Ok(out)
}

pub fn parse_serialized_prop_storage(bytes: &[u8]) -> WinctlResult<Vec<SerializedPropertyStorage>> {
    let mut storages = Vec::new();
    let mut offset = 0usize;

    while offset + 4 <= bytes.len() {
        let storage_size = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if storage_size == 0 {
            break;
        }
        if storage_size < 24 || offset + storage_size > bytes.len() {
            return Err(WinctlError::ParseError(format!(
                "invalid 1SPS storage_size {storage_size} at offset {offset} (total {})",
                bytes.len()
            )));
        }
        let block = &bytes[offset..offset + storage_size];
        let version = u32::from_le_bytes(block[4..8].try_into().unwrap());
        if version != SERIALIZED_PROPSTORAGE_MAGIC {
            return Err(WinctlError::ParseError(format!(
                "invalid SERIALIZEDPROPSTORAGE version {version:#010x} (expected {SERIALIZED_PROPSTORAGE_MAGIC:#010x})"
            )));
        }
        let mut fmtid = [0u8; 16];
        fmtid.copy_from_slice(&block[8..24]);
        let is_named = fmtid == FMTID_USER_DEFINED_PROPERTIES;

        let mut pos = 24usize;
        let mut entries = Vec::new();
        while pos + 4 <= block.len() {
            let value_size = u32::from_le_bytes(block[pos..pos + 4].try_into().unwrap()) as usize;
            if value_size == 0 {
                break;
            }
            if value_size < 13 || pos + value_size > block.len() {
                return Err(WinctlError::ParseError(format!(
                    "invalid 1SPS value_size {value_size} at block offset {pos}"
                )));
            }
            let val_slice = &block[pos..pos + value_size];
            let id_or_name_size = u32::from_le_bytes(val_slice[4..8].try_into().unwrap());
            let _reserved = val_slice[8];

            let (key, typed_offset) = if is_named {
                let name_bytes_len = id_or_name_size as usize;
                if 9 + name_bytes_len > val_slice.len() || !name_bytes_len.is_multiple_of(2) {
                    return Err(WinctlError::ParseError(
                        "invalid UTF-16 property name length in 1SPS".to_string(),
                    ));
                }
                let raw_name = &val_slice[9..9 + name_bytes_len];
                let units: Vec<u16> = raw_name
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                let name = String::from_utf16_lossy(&units).trim_end_matches('\0').to_string();
                (PropertyKeyId::Name(name), 9 + name_bytes_len)
            } else {
                (PropertyKeyId::Pid(id_or_name_size), 9)
            };

            if typed_offset + 4 > val_slice.len() {
                return Err(WinctlError::ParseError(
                    "truncated TypedPropertyValue header in 1SPS".to_string(),
                ));
            }
            let vt = u16::from_le_bytes([val_slice[typed_offset], val_slice[typed_offset + 1]]);
            let payload = &val_slice[typed_offset + 4..];
            let value = decode_1sps_typed_value(vt, payload)?;
            entries.push(SerializedPropertyEntry { key, vt, value });
            pos += value_size;
        }

        storages.push(SerializedPropertyStorage {
            fmtid,
            fmtid_guid: format_mixed_endian_guid(&fmtid),
            entries,
        });
        offset += storage_size;
    }

    if storages.is_empty() {
        return Err(WinctlError::ParseError(
            "1SPS container had no SERIALIZEDPROPSTORAGE blocks".to_string(),
        ));
    }
    Ok(storages)
}

fn decode_1sps_typed_value(vt: u16, payload: &[u8]) -> WinctlResult<SerializedPropertyValue> {
    match vt {
        VT_EMPTY => Ok(SerializedPropertyValue::Empty),
        VT_I4 => {
            if payload.len() < 4 {
                return Err(WinctlError::ParseError("truncated VT_I4 in 1SPS".to_string()));
            }
            Ok(SerializedPropertyValue::I4(i32::from_le_bytes(
                payload[..4].try_into().unwrap(),
            )))
        }
        VT_UI4 => {
            if payload.len() < 4 {
                return Err(WinctlError::ParseError("truncated VT_UI4 in 1SPS".to_string()));
            }
            Ok(SerializedPropertyValue::U4(u32::from_le_bytes(
                payload[..4].try_into().unwrap(),
            )))
        }
        VT_R4 => {
            if payload.len() < 4 {
                return Err(WinctlError::ParseError("truncated VT_R4 in 1SPS".to_string()));
            }
            let v = f32::from_le_bytes(payload[..4].try_into().unwrap());
            if !v.is_finite() {
                return Err(WinctlError::ParseError("non-finite VT_R4 in 1SPS".to_string()));
            }
            Ok(SerializedPropertyValue::R4(v))
        }
        VT_BOOL => {
            if payload.len() < 2 {
                return Err(WinctlError::ParseError("truncated VT_BOOL in 1SPS".to_string()));
            }
            let raw = i16::from_le_bytes([payload[0], payload[1]]);
            Ok(SerializedPropertyValue::Bool(raw != 0))
        }
        VT_LPWSTR => {
            if payload.len() < 4 {
                return Err(WinctlError::ParseError(
                    "truncated VT_LPWSTR length in 1SPS".to_string(),
                ));
            }
            let chars = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
            let byte_len = chars * 2;
            if 4 + byte_len > payload.len() {
                return Err(WinctlError::ParseError(
                    "truncated VT_LPWSTR string in 1SPS".to_string(),
                ));
            }
            let units: Vec<u16> = payload[4..4 + byte_len]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            Ok(SerializedPropertyValue::Lpwstr(
                String::from_utf16_lossy(&units).trim_end_matches('\0').to_string(),
            ))
        }
        VT_BLOB => {
            if payload.len() < 4 {
                return Err(WinctlError::ParseError("truncated VT_BLOB length in 1SPS".to_string()));
            }
            let byte_len = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
            if 4 + byte_len > payload.len() {
                return Err(WinctlError::ParseError("truncated VT_BLOB bytes in 1SPS".to_string()));
            }
            Ok(SerializedPropertyValue::Blob(payload[4..4 + byte_len].to_vec()))
        }
        VT_VECTOR_R4 | VT_ARRAY_R4 => {
            if payload.len() < 4 {
                return Err(WinctlError::ParseError(
                    "truncated VT_VECTOR|VT_R4 count in 1SPS".to_string(),
                ));
            }
            let count = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
            if 4 + count * 4 > payload.len() {
                return Err(WinctlError::ParseError(
                    "truncated VT_VECTOR|VT_R4 elements in 1SPS".to_string(),
                ));
            }
            let mut list = Vec::with_capacity(count);
            for i in 0..count {
                let off = 4 + i * 4;
                let v = f32::from_le_bytes(payload[off..off + 4].try_into().unwrap());
                if !v.is_finite() {
                    return Err(WinctlError::ParseError("non-finite f32 in 1SPS R4 list".to_string()));
                }
                list.push(v);
            }
            Ok(SerializedPropertyValue::R4List(list))
        }
        other => Err(WinctlError::ParseError(format!(
            "unsupported 1SPS variant type {other:#06x}"
        ))),
    }
}

pub fn encode_serialized_prop_storage(storages: &[SerializedPropertyStorage]) -> Vec<u8> {
    let mut out = Vec::new();
    for st in storages {
        let mut block = Vec::new();
        block.extend_from_slice(&0u32.to_le_bytes()); // placeholder storage_size
        block.extend_from_slice(&SERIALIZED_PROPSTORAGE_MAGIC.to_le_bytes());
        block.extend_from_slice(&st.fmtid);

        for entry in &st.entries {
            let mut val_buf = Vec::new();
            val_buf.extend_from_slice(&0u32.to_le_bytes()); // placeholder value_size
            match &entry.key {
                PropertyKeyId::Pid(pid) => {
                    val_buf.extend_from_slice(&pid.to_le_bytes());
                    val_buf.push(0); // reserved
                }
                PropertyKeyId::Name(name) => {
                    let units: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
                    let name_bytes = (units.len() * 2) as u32;
                    val_buf.extend_from_slice(&name_bytes.to_le_bytes());
                    val_buf.push(0); // reserved
                    for u in units {
                        val_buf.extend_from_slice(&u.to_le_bytes());
                    }
                }
            }
            val_buf.extend_from_slice(&entry.vt.to_le_bytes());
            val_buf.extend_from_slice(&0u16.to_le_bytes()); // reserved

            match &entry.value {
                SerializedPropertyValue::Empty => {}
                SerializedPropertyValue::I4(v) => val_buf.extend_from_slice(&v.to_le_bytes()),
                SerializedPropertyValue::U4(v) => val_buf.extend_from_slice(&v.to_le_bytes()),
                SerializedPropertyValue::R4(v) => val_buf.extend_from_slice(&v.to_le_bytes()),
                SerializedPropertyValue::Bool(b) => {
                    let raw: i16 = if *b { -1 } else { 0 };
                    val_buf.extend_from_slice(&raw.to_le_bytes());
                    val_buf.extend_from_slice(&0u16.to_le_bytes());
                }
                SerializedPropertyValue::Lpwstr(s) => {
                    let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
                    val_buf.extend_from_slice(&(units.len() as u32).to_le_bytes());
                    for u in units {
                        val_buf.extend_from_slice(&u.to_le_bytes());
                    }
                }
                SerializedPropertyValue::Blob(b) => {
                    val_buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
                    val_buf.extend_from_slice(b);
                }
                SerializedPropertyValue::R4List(list) => {
                    val_buf.extend_from_slice(&(list.len() as u32).to_le_bytes());
                    for f in list {
                        val_buf.extend_from_slice(&f.to_le_bytes());
                    }
                }
            }

            while val_buf.len() % 4 != 0 {
                val_buf.push(0);
            }
            let val_size = val_buf.len() as u32;
            val_buf[0..4].copy_from_slice(&val_size.to_le_bytes());
            block.extend_from_slice(&val_buf);
        }

        block.extend_from_slice(&0u32.to_le_bytes()); // value terminator
        while block.len() % 4 != 0 {
            block.push(0);
        }
        let storage_size = block.len() as u32;
        block[0..4].copy_from_slice(&storage_size.to_le_bytes());
        out.extend_from_slice(&block);
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // storage list terminator
    out
}

pub fn parse_policy_default_string(raw: &str) -> (String, String, String, String) {
    let trimmed = raw.trim();
    let (dev_part, rest) = trimmed.split_once('|').unwrap_or((trimmed, ""));
    let (app_path, grouping_guid) = rest.split_once("%b").unwrap_or((rest, ""));
    let app_name = app_path
        .rsplit(['\\', '/'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(app_path)
        .to_string();
    (
        dev_part.to_string(),
        app_path.to_string(),
        app_name,
        grouping_guid.to_string(),
    )
}

pub fn decode_raw_record(rec: &RawAppPropertyStoreRecord) -> DecodedAppConfigEntry {
    let (device_id, app_path, app_name, grouping_guid) = parse_policy_default_string(&rec.default_value);

    let mut volume_scalar = rec.blob_3_volume.as_deref().and_then(|b| decode_propvar_r4(b).ok());
    let mut channel_balances = rec
        .blob_4_channels
        .as_deref()
        .and_then(|b| decode_propvar_r4_array(b).ok())
        .unwrap_or_default();
    let mut muted = rec.blob_5_mute.as_deref().and_then(|b| decode_propvar_bool(b).ok());
    let mut raw_format = "PolicyConfig_PROPVARIANT".to_string();

    if let Some(ref sps) = rec.raw_1sps_container
        && let Ok(blocks) = parse_serialized_prop_storage(sps)
    {
        raw_format = "1SPS_SERIALIZEDPROPSTORAGE".to_string();
        for block in blocks {
            for entry in block.entries {
                match (&entry.key, &entry.value) {
                    (PropertyKeyId::Pid(3), SerializedPropertyValue::R4(v)) => volume_scalar = Some(*v),
                    (PropertyKeyId::Pid(4), SerializedPropertyValue::R4List(vs)) => channel_balances = vs.clone(),
                    (PropertyKeyId::Pid(5), SerializedPropertyValue::Bool(m)) => muted = Some(*m),
                    _ => {}
                }
            }
        }
    }

    DecodedAppConfigEntry {
        subkey: rec.subkey.clone(),
        registry_key: rec.registry_key.clone(),
        device_id,
        app_path,
        app_name,
        grouping_guid,
        volume_scalar,
        volume_percent: volume_scalar.map(|s| (s * 100.0 * 100.0).round() / 100.0),
        muted,
        channel_balances,
        raw_format,
    }
}

pub struct AppAudioConfigTableProvider {
    backend: Arc<dyn AudioBackend>,
}

impl AppAudioConfigTableProvider {
    pub fn new(backend: Arc<dyn AudioBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for AppAudioConfigTableProvider {
    fn tool_id(&self) -> &'static str {
        "audio_app_config"
    }

    fn description(&self) -> &'static str {
        "Inspect per-application Windows PolicyConfig\\PropertyStore audio settings"
    }

    fn schema(&self) -> TableSchema {
        TableSchema::new(
            "audio_app_config",
            "Per-App PropertyStore Audio Entries",
            vec![
                ColumnDef::new("subkey", "Subkey", ColumnType::Text).with_width_hint(140),
                ColumnDef::new("app_path", "Application Path", ColumnType::Text).with_width_hint(280),
                ColumnDef::new("app_name", "Application", ColumnType::Text).with_width_hint(150),
                ColumnDef::new("device_id", "Endpoint Device", ColumnType::Text).with_width_hint(260),
                ColumnDef::new("grouping_guid", "Grouping GUID", ColumnType::Text).with_width_hint(220),
                ColumnDef::new("volume_percent", "Volume %", ColumnType::Float).with_width_hint(90),
                ColumnDef::new("muted", "Muted", ColumnType::Bool).with_width_hint(70),
                ColumnDef::new("channel_balances", "Channel Balances", ColumnType::Text).with_width_hint(160),
                ColumnDef::new("raw_format", "Binary Format", ColumnType::Text).with_width_hint(180),
                ColumnDef::new("registry_key", "Registry Key", ColumnType::Text).with_width_hint(320),
            ],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let records = self
            .backend
            .read_app_audio_configs()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = records
            .into_iter()
            .enumerate()
            .map(|(idx, rec)| {
                let decoded = decode_raw_record(&rec);
                let ch_str = decoded
                    .channel_balances
                    .iter()
                    .map(|c| format!("{c:.3}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut raw_bytes = Vec::new();
                if let Some(ref b3) = rec.blob_3_volume {
                    raw_bytes.extend_from_slice(b3);
                }
                if let Some(ref b4) = rec.blob_4_channels {
                    raw_bytes.extend_from_slice(b4);
                }
                if let Some(ref b5) = rec.blob_5_mute {
                    raw_bytes.extend_from_slice(b5);
                }
                if let Some(ref sps) = rec.raw_1sps_container {
                    raw_bytes.extend_from_slice(sps);
                }

                let mut row = Row::new(
                    idx as u64 + 1,
                    vec![
                        CellValue::Text(decoded.subkey.clone()),
                        CellValue::Text(decoded.app_path.clone()),
                        CellValue::Text(decoded.app_name.clone()),
                        CellValue::Text(decoded.device_id.clone()),
                        CellValue::Text(decoded.grouping_guid.clone()),
                        decoded
                            .volume_percent
                            .map(|v| CellValue::Float(f64::from(v)))
                            .unwrap_or(CellValue::Null),
                        decoded.muted.map(CellValue::Bool).unwrap_or(CellValue::Null),
                        CellValue::Text(ch_str),
                        CellValue::Text(decoded.raw_format.clone()),
                        CellValue::Text(decoded.registry_key.clone()),
                    ],
                )
                .with_detail_json(serde_json::to_value(&decoded).unwrap_or(Value::Null));
                if !raw_bytes.is_empty() {
                    row = row.with_raw_bytes(raw_bytes);
                }
                row
            })
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

pub fn validate_property_store_subkey(subkey: &str) -> WinctlResult<&str> {
    let trimmed = subkey.trim();
    if trimmed.is_empty() || trimmed.contains(['\\', '/', '\0']) || trimmed.contains("..") {
        return Err(WinctlError::InvalidInput(
            "subkey must be a non-empty single PropertyStore subkey name".to_string(),
        ));
    }
    validate_r4_compliance("app_config_update", &[trimmed.to_string()])?;
    Ok(trimmed)
}

#[derive(Debug, Clone, Deserialize)]
struct AppAudioConfigUpdateRequest {
    subkey: String,
    #[serde(default)]
    volume_percent: Option<f32>,
    #[serde(default)]
    channel_balances: Option<Vec<f32>>,
    #[serde(default)]
    muted: Option<bool>,
    #[serde(default)]
    delete_subkey: bool,
}

pub struct AppAudioConfigUpdateAction {
    backend: Arc<dyn AudioBackend>,
}

impl AppAudioConfigUpdateAction {
    pub fn new(backend: Arc<dyn AudioBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for AppAudioConfigUpdateAction {
    fn command_id(&self) -> &'static str {
        "audio_app_config_update"
    }

    fn description(&self) -> &'static str {
        "Update or remove per-application PolicyConfig\\PropertyStore binary volume/mute/channel blobs"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "subkey": { "type": "string", "description": "PropertyStore subkey name (e.g. 186430e1_0)" },
                "volume_percent": { "type": "number", "minimum": 0.0, "maximum": 100.0 },
                "channel_balances": { "type": "array", "items": { "type": "number", "minimum": 0.0, "maximum": 1.0 } },
                "muted": { "type": "boolean" },
                "delete_subkey": { "type": "boolean", "default": false }
            },
            "required": ["subkey"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let req: AppAudioConfigUpdateRequest = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid audio_app_config_update request: {e}")))?;
        let trimmed_subkey = validate_property_store_subkey(&req.subkey).map_err(McpToolError::from)?;

        if let Some(pct) = req.volume_percent
            && (!pct.is_finite() || !(0.0..=100.0).contains(&pct))
        {
            return Err(McpToolError::from(WinctlError::InvalidInput(format!(
                "volume_percent {pct} out of range 0.0..=100.0"
            ))));
        }
        if let Some(ref chs) = req.channel_balances
            && (chs.is_empty() || chs.iter().any(|c| !c.is_finite() || !(0.0..=1.0).contains(c)))
        {
            return Err(McpToolError::from(WinctlError::InvalidInput(
                "channel_balances must be non-empty scalars in 0.0..=1.0".to_string(),
            )));
        }

        if !req.delete_subkey && req.volume_percent.is_none() && req.channel_balances.is_none() && req.muted.is_none() {
            return Err(McpToolError::InvalidParams(
                "must specify at least one of volume_percent, channel_balances, muted, or delete_subkey=true"
                    .to_string(),
            ));
        }

        let patch = AppPropertyStorePatch {
            volume_scalar: req.volume_percent.map(|v| v / 100.0),
            channel_volumes: req.channel_balances.clone(),
            muted: req.muted,
            delete_subkey: req.delete_subkey,
        };

        if dry_run {
            let blob_3_hex = patch
                .volume_scalar
                .map(|s| encode_propvar_r4(s).map(|b| to_hex(&b)))
                .transpose()
                .map_err(McpToolError::from)?;
            let blob_4_hex = patch
                .channel_volumes
                .as_ref()
                .map(|c| encode_propvar_r4_array(c).map(|b| to_hex(&b)))
                .transpose()
                .map_err(McpToolError::from)?;
            let blob_5_hex = patch.muted.map(|m| to_hex(&encode_propvar_bool(m)));
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview PolicyConfig\\PropertyStore registry binary blob update without writing HKCU",
                json!({
                    "subkey": trimmed_subkey,
                    "delete_subkey": req.delete_subkey,
                    "encoded_blob_3_hex": blob_3_hex,
                    "encoded_blob_4_hex": blob_4_hex,
                    "encoded_blob_5_hex": blob_5_hex,
                }),
            ));
        }

        self.backend
            .write_app_audio_config(trimmed_subkey, &patch)
            .map_err(McpToolError::from)?;
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "subkey": trimmed_subkey,
            "patch": patch,
        }))
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}
