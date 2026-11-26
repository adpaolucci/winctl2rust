use std::sync::Arc;

use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde_json::Value;

use crate::backend::{DesktopBackend, KeyStateEntry};

pub fn vk_canonical_name(vk: u8) -> String {
    match vk {
        0x01 => "VK_LBUTTON".to_string(),
        0x02 => "VK_RBUTTON".to_string(),
        0x03 => "VK_CANCEL".to_string(),
        0x04 => "VK_MBUTTON".to_string(),
        0x05 => "VK_XBUTTON1".to_string(),
        0x06 => "VK_XBUTTON2".to_string(),
        0x08 => "VK_BACK".to_string(),
        0x09 => "VK_TAB".to_string(),
        0x0C => "VK_CLEAR".to_string(),
        0x0D => "VK_RETURN".to_string(),
        0x10 => "VK_SHIFT".to_string(),
        0x11 => "VK_CONTROL".to_string(),
        0x12 => "VK_MENU".to_string(),
        0x13 => "VK_PAUSE".to_string(),
        0x14 => "VK_CAPITAL".to_string(),
        0x1B => "VK_ESCAPE".to_string(),
        0x20 => "VK_SPACE".to_string(),
        0x21 => "VK_PRIOR".to_string(),
        0x22 => "VK_NEXT".to_string(),
        0x23 => "VK_END".to_string(),
        0x24 => "VK_HOME".to_string(),
        0x25 => "VK_LEFT".to_string(),
        0x26 => "VK_UP".to_string(),
        0x27 => "VK_RIGHT".to_string(),
        0x28 => "VK_DOWN".to_string(),
        0x2C => "VK_SNAPSHOT".to_string(),
        0x2D => "VK_INSERT".to_string(),
        0x2E => "VK_DELETE".to_string(),
        b'0'..=b'9' => format!("VK_{}", vk as char),
        b'A'..=b'Z' => format!("VK_{}", vk as char),
        0x5B => "VK_LWIN".to_string(),
        0x5C => "VK_RWIN".to_string(),
        0x5D => "VK_APPS".to_string(),
        0x60..=0x69 => format!("VK_NUMPAD{}", vk - 0x60),
        0x6A => "VK_MULTIPLY".to_string(),
        0x6B => "VK_ADD".to_string(),
        0x6C => "VK_SEPARATOR".to_string(),
        0x6D => "VK_SUBTRACT".to_string(),
        0x6E => "VK_DECIMAL".to_string(),
        0x6F => "VK_DIVIDE".to_string(),
        0x70..=0x87 => format!("VK_F{}", vk - 0x70 + 1),
        0x90 => "VK_NUMLOCK".to_string(),
        0x91 => "VK_SCROLL".to_string(),
        0xA0 => "VK_LSHIFT".to_string(),
        0xA1 => "VK_RSHIFT".to_string(),
        0xA2 => "VK_LCONTROL".to_string(),
        0xA3 => "VK_RCONTROL".to_string(),
        0xA4 => "VK_LMENU".to_string(),
        0xA5 => "VK_RMENU".to_string(),
        0xBA => "VK_OEM_1".to_string(),
        0xBB => "VK_OEM_PLUS".to_string(),
        0xBC => "VK_OEM_COMMA".to_string(),
        0xBD => "VK_OEM_MINUS".to_string(),
        0xBE => "VK_OEM_PERIOD".to_string(),
        0xBF => "VK_OEM_2".to_string(),
        0xC0 => "VK_OEM_3".to_string(),
        0xDB => "VK_OEM_4".to_string(),
        0xDC => "VK_OEM_5".to_string(),
        0xDD => "VK_OEM_6".to_string(),
        0xDE => "VK_OEM_7".to_string(),
        other => format!("VK_{other:02X}"),
    }
}

pub fn is_extended_vk(vk: u8) -> bool {
    matches!(
        vk,
        0x21..=0x28 | 0x2C..=0x2E | 0x5B..=0x5D | 0x6F | 0x90 | 0xA3 | 0xA5
    )
}

pub fn keys_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_keys",
        "KeyboardStateView Virtual Key States",
        vec![
            ColumnDef::new("vk_code", "VK Code", ColumnType::Hex).with_width_hint(85),
            ColumnDef::new("vk_name", "VK Name", ColumnType::Text).with_width_hint(140),
            ColumnDef::new("scan_code", "Scan Code", ColumnType::Hex).with_width_hint(90),
            ColumnDef::new("key_label", "Key Label", ColumnType::Text).with_width_hint(160),
            ColumnDef::new("pressed", "Pressed", ColumnType::Bool).with_width_hint(75),
            ColumnDef::new("toggled", "Toggled", ColumnType::Bool).with_width_hint(75),
            ColumnDef::new("extended", "Extended", ColumnType::Bool).with_width_hint(75),
        ],
    )
}

pub fn key_state_to_row(id: u64, k: &KeyStateEntry) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Text(format!("0x{:02X}", k.vk_code)),
            CellValue::Text(k.vk_name.clone()),
            CellValue::Text(format!("0x{:02X}", k.scan_code)),
            CellValue::Text(k.key_label.clone()),
            CellValue::Bool(k.pressed),
            CellValue::Bool(k.toggled),
            CellValue::Bool(k.extended),
        ],
    )
    .with_detail_json(serde_json::to_value(k).unwrap_or(Value::Null))
}

pub struct KeysTableProvider {
    backend: Arc<dyn DesktopBackend>,
}

impl KeysTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for KeysTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_keys"
    }

    fn description(&self) -> &'static str {
        "Inspect virtual key pressed/toggled states, scan codes, and localized key names (KeyboardStateView)"
    }

    fn schema(&self) -> TableSchema {
        keys_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let items = self
            .backend
            .query_keyboard_state()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = items
            .iter()
            .enumerate()
            .map(|(idx, k)| key_state_to_row(idx as u64 + 1, k))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}
