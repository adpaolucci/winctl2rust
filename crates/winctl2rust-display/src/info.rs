use std::sync::Arc;

use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::{DisplayBackend, MonitorTopologyInfo, RawEdidEntry};
use crate::edid::parse_edid_bytes;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfoEntry {
    pub monitor_name: String,
    pub vendor_id: String,
    pub product_id: String,
    pub serial_number: String,
    pub manufacture_date: String,
    pub max_resolution: String,
    pub aspect_ratio: String,
    pub gamma: Option<f32>,
    pub color_primaries: String,
    pub active: bool,
    pub registry_path: String,
    pub device_interface_id: String,
}

pub fn correlate_monitor_info(
    edids: &[RawEdidEntry],
    monitors: &[MonitorTopologyInfo],
) -> Vec<(MonitorInfoEntry, Vec<u8>, Value)> {
    let mut matched_indices: Vec<Option<usize>> = vec![None; edids.len()];
    let mut claimed_monitors = vec![false; monitors.len()];

    // Pass 1: exact device_interface_id match
    for (ei, rec) in edids.iter().enumerate() {
        if rec.device_interface_id.is_empty() {
            continue;
        }
        if let Some(mi) = monitors.iter().enumerate().position(|(idx, m)| {
            !claimed_monitors[idx] && m.device_interface_id.eq_ignore_ascii_case(&rec.device_interface_id)
        }) {
            matched_indices[ei] = Some(mi);
            claimed_monitors[mi] = true;
        }
    }

    // Pass 2: instance_id match (scoped by monitor_id if non-empty)
    for (ei, rec) in edids.iter().enumerate() {
        if matched_indices[ei].is_some() || rec.instance_id.is_empty() {
            continue;
        }
        let inst_lower = rec.instance_id.to_ascii_lowercase();
        let mon_lower = rec.monitor_id.to_ascii_lowercase();
        if let Some(mi) = monitors.iter().enumerate().position(|(idx, m)| {
            if claimed_monitors[idx] {
                return false;
            }
            let dev_lower = m.device_interface_id.to_ascii_lowercase();
            dev_lower.contains(&inst_lower) && (mon_lower.is_empty() || dev_lower.contains(&mon_lower))
        }) {
            matched_indices[ei] = Some(mi);
            claimed_monitors[mi] = true;
        }
    }

    // Pass 3: monitor_id fallback only for entries lacking interface/instance identifiers
    for (ei, rec) in edids.iter().enumerate() {
        if matched_indices[ei].is_some()
            || rec.monitor_id.is_empty()
            || !rec.device_interface_id.is_empty()
            || !rec.instance_id.is_empty()
        {
            continue;
        }
        let mon_lower = rec.monitor_id.to_ascii_lowercase();
        if let Some(mi) = monitors.iter().enumerate().position(|(idx, m)| {
            !claimed_monitors[idx] && m.device_interface_id.to_ascii_lowercase().contains(&mon_lower)
        }) {
            matched_indices[ei] = Some(mi);
            claimed_monitors[mi] = true;
        }
    }

    let mut out = Vec::new();
    for (ei, rec) in edids.iter().enumerate() {
        let parsed = match parse_edid_bytes(&rec.raw_bytes) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let matched_mon = matched_indices[ei].map(|mi| &monitors[mi]);
        let active = rec.active || matched_mon.is_some_and(|m| m.active);
        let monitor_name = parsed
            .monitor_name
            .clone()
            .or_else(|| matched_mon.map(|m| m.friendly_name.clone()))
            .unwrap_or_else(|| format!("{} {}", parsed.manufacturer_id, parsed.product_code_hex));
        let serial_number = parsed
            .ascii_serial
            .clone()
            .unwrap_or_else(|| format!("{:#010X}", parsed.serial_number_u32));
        let manufacture_date = match (parsed.manufacture_week, parsed.is_model_year) {
            (Some(w), false) => format!("{} (Week {})", parsed.manufacture_year, w),
            (_, true) => format!("{} (Model Year)", parsed.manufacture_year),
            _ => parsed.manufacture_year.to_string(),
        };
        let (max_w, max_h, max_hz) = parsed.max_resolution();
        let max_resolution = if max_w > 0 && max_h > 0 {
            format!("{max_w}x{max_h} @ {max_hz:.0}Hz")
        } else {
            "Unknown".to_string()
        };
        let aspect_ratio = if let Some(ar) = parsed.aspect_ratio_hint {
            format!("{ar:.2}:1")
        } else if max_w > 0 && max_h > 0 {
            format!("{:.2}:1", f64::from(max_w) / f64::from(max_h))
        } else {
            "Unknown".to_string()
        };
        let color_primaries = format!(
            "R({:.3},{:.3}) G({:.3},{:.3}) B({:.3},{:.3}) W({:.3},{:.3})",
            parsed.chromaticity.red_x,
            parsed.chromaticity.red_y,
            parsed.chromaticity.green_x,
            parsed.chromaticity.green_y,
            parsed.chromaticity.blue_x,
            parsed.chromaticity.blue_y,
            parsed.chromaticity.white_x,
            parsed.chromaticity.white_y
        );

        let entry = MonitorInfoEntry {
            monitor_name,
            vendor_id: parsed.manufacturer_id.clone(),
            product_id: parsed.product_code_hex.clone(),
            serial_number,
            manufacture_date,
            max_resolution,
            aspect_ratio,
            gamma: parsed.gamma,
            color_primaries,
            active,
            registry_path: rec.registry_path.clone(),
            device_interface_id: rec.device_interface_id.clone(),
        };
        let detail = serde_json::json!({
            "info": entry,
            "topology": matched_mon,
            "edid": parsed,
        });
        out.push((entry, rec.raw_bytes.clone(), detail));
    }
    out
}

pub fn monitor_info_schema() -> TableSchema {
    TableSchema::new(
        "display_info",
        "MonitorInfoView Display & EDID Summary",
        vec![
            ColumnDef::new("monitor_name", "Monitor Name", ColumnType::Text).with_width_hint(180),
            ColumnDef::new("vendor_id", "Vendor ID", ColumnType::Text).with_width_hint(90),
            ColumnDef::new("product_id", "Product ID", ColumnType::Text).with_width_hint(90),
            ColumnDef::new("serial_number", "Serial Number", ColumnType::Text).with_width_hint(140),
            ColumnDef::new("manufacture_date", "Manufacture Date", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("max_resolution", "Max Resolution", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("aspect_ratio", "Aspect Ratio", ColumnType::Text).with_width_hint(100),
            ColumnDef::new("gamma", "Gamma", ColumnType::Float).with_width_hint(70),
            ColumnDef::new("color_primaries", "Color Primaries", ColumnType::Text).with_width_hint(280),
            ColumnDef::new("active", "Active", ColumnType::Bool).with_width_hint(70),
        ],
    )
}

pub struct MonitorInfoTableProvider {
    backend: Arc<dyn DisplayBackend>,
}

impl MonitorInfoTableProvider {
    pub fn new(backend: Arc<dyn DisplayBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for MonitorInfoTableProvider {
    fn tool_id(&self) -> &'static str {
        "display_info"
    }

    fn description(&self) -> &'static str {
        "Correlate live DisplayConfig monitor targets with EDID serials, gamma, primaries, and max timings (MonitorInfoView)"
    }

    fn schema(&self) -> TableSchema {
        monitor_info_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let edids = self
            .backend
            .list_edid_records()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let monitors = self.backend.list_monitors().unwrap_or_default();
        let correlated = correlate_monitor_info(&edids, &monitors);
        let rows: Vec<Row> = correlated
            .into_iter()
            .enumerate()
            .map(|(idx, (entry, raw_bytes, detail))| {
                Row::new(
                    idx as u64 + 1,
                    vec![
                        CellValue::Text(entry.monitor_name),
                        CellValue::Text(entry.vendor_id),
                        CellValue::Text(entry.product_id),
                        CellValue::Text(entry.serial_number),
                        CellValue::Text(entry.manufacture_date),
                        CellValue::Text(entry.max_resolution),
                        CellValue::Text(entry.aspect_ratio),
                        entry
                            .gamma
                            .map(|g| CellValue::Float(f64::from(g)))
                            .unwrap_or(CellValue::Null),
                        CellValue::Text(entry.color_primaries),
                        CellValue::Bool(entry.active),
                    ],
                )
                .with_raw_bytes(raw_bytes)
                .with_detail_json(detail)
            })
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}
