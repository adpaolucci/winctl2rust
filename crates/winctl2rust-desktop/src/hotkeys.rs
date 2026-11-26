use std::fs;
use std::path::Path;
use std::sync::Arc;

use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde_json::Value;

use crate::backend::{DesktopBackend, HotkeyEntry};
use crate::lnk::{format_win32_modifiers, parse_lnk_file, vk_short_label};

pub fn scan_lnk_hotkeys_in_dir(dir: &Path, max_depth: usize) -> Vec<HotkeyEntry> {
    let mut out = Vec::new();
    scan_dir_recursive(dir, 0, max_depth, &mut out);
    out
}

fn scan_dir_recursive(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<HotkeyEntry>) {
    if depth > max_depth {
        return;
    }
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir_recursive(&path, depth + 1, max_depth, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
            && let Ok(parsed) = parse_lnk_file(&path)
            && parsed.vk_code != 0
        {
            out.push(HotkeyEntry {
                source: "LnkShortcut".to_string(),
                hotkey: parsed.hotkey_label,
                modifiers: parsed.modifiers_label,
                vk_code: u32::from(parsed.vk_code),
                status: "ShortcutBound".to_string(),
                owner_hint: parsed.owner_hint,
                lnk_path: path.display().to_string(),
            });
        }
    }
}

pub fn common_hotkey_probe_candidates() -> Vec<(u32, u32)> {
    use crate::lnk::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
    let mut out = Vec::new();
    // Probe PrintScreen, Pause, F1..F12 with common modifier combos
    for mods in [
        MOD_CONTROL | MOD_ALT,
        MOD_CONTROL | MOD_SHIFT,
        MOD_ALT | MOD_SHIFT,
        MOD_WIN,
    ] {
        for vk in (0x41u32..=0x5A).chain(0x70u32..=0x7B) {
            out.push((mods, vk));
        }
    }
    out.push((0, 0x2C)); // PrintScreen
    out.push((MOD_ALT, 0x2C)); // Alt+PrintScreen
    out
}

pub fn format_probe_hotkey(mods: u32, vk: u32) -> (String, String) {
    let mods_label = format_win32_modifiers(mods);
    let key_label = u8::try_from(vk)
        .map(vk_short_label)
        .unwrap_or_else(|_| format!("VK_{vk:02X}"));
    let combo = if mods_label == "None" {
        key_label
    } else {
        format!("{mods_label}+{key_label}")
    };
    (combo, mods_label)
}

pub fn hotkeys_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_hotkeys",
        "Registered Hotkeys & .lnk Bindings",
        vec![
            ColumnDef::new("source", "Source", ColumnType::Text).with_width_hint(120),
            ColumnDef::new("hotkey", "Hotkey", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("modifiers", "Modifiers", ColumnType::Text).with_width_hint(120),
            ColumnDef::new("vk_code", "VK Code", ColumnType::Hex).with_width_hint(85),
            ColumnDef::new("status", "Status", ColumnType::Text).with_width_hint(120),
            ColumnDef::new("owner_hint", "Owner / Target", ColumnType::Text).with_width_hint(240),
            ColumnDef::new("lnk_path", "Shortcut Path", ColumnType::Text).with_width_hint(240),
        ],
    )
}

pub fn hotkey_to_row(id: u64, h: &HotkeyEntry) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Text(h.source.clone()),
            CellValue::Text(h.hotkey.clone()),
            CellValue::Text(h.modifiers.clone()),
            CellValue::Text(format!("0x{:02X}", h.vk_code)),
            CellValue::Text(h.status.clone()),
            CellValue::Text(h.owner_hint.clone()),
            CellValue::Text(h.lnk_path.clone()),
        ],
    )
    .with_detail_json(serde_json::to_value(h).unwrap_or(Value::Null))
}

pub struct HotkeysTableProvider {
    backend: Arc<dyn DesktopBackend>,
}

impl HotkeysTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for HotkeysTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_hotkeys"
    }

    fn description(&self) -> &'static str {
        "Probe global hotkeys via RegisterHotKey and decode [MS-SHLLINK] .lnk wHotKey bindings"
    }

    fn schema(&self) -> TableSchema {
        hotkeys_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let items = self
            .backend
            .probe_hotkeys()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = items
            .iter()
            .enumerate()
            .map(|(idx, h)| hotkey_to_row(idx as u64 + 1, h))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}
