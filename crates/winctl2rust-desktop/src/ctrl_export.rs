use std::sync::Arc;

use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde_json::Value;

use crate::backend::{CtrlExportItem, DesktopBackend, format_hwnd};

#[used]
pub static WPM_CTRL_EXPORT_MARKER: &[u8] = b"winctl2rust_desktop::ctrl_export::LVITEMW::TVITEMW::HDITEMW\0";

/// 32-bit WOW64 binary layout for `LVITEMW` (used when a 64-bit host reads a 32-bit target process).
/// Strictly a data layout for `LVM_GETITEMW` — never executable code or DLL injection.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LvItemW32 {
    pub mask: u32,
    pub i_item: i32,
    pub i_sub_item: i32,
    pub state: u32,
    pub state_mask: u32,
    pub psz_text: u32,
    pub cch_text_max: i32,
    pub i_image: i32,
    pub l_param: i32,
    pub i_indent: i32,
    pub i_group_id: i32,
    pub c_columns: u32,
    pub pu_columns: u32,
    pub pi_col_fmt: u32,
    pub i_group: i32,
}

/// 32-bit WOW64 binary layout for `TVITEMW`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TvItemW32 {
    pub mask: u32,
    pub h_item: u32,
    pub state: u32,
    pub state_mask: u32,
    pub psz_text: u32,
    pub cch_text_max: i32,
    pub i_image: i32,
    pub i_selected_image: i32,
    pub c_children: i32,
    pub l_param: i32,
}

/// 32-bit WOW64 binary layout for `HDITEMW`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct HdItemW32 {
    pub mask: u32,
    pub cxy: i32,
    pub psz_text: u32,
    pub hbm: u32,
    pub cch_text_max: i32,
    pub fmt: i32,
    pub l_param: i32,
    pub i_image: i32,
    pub i_order: i32,
    pub r#type: u32,
    pub pv_filter: u32,
    pub state: u32,
}

pub fn decode_utf16_nul_terminated(units: &[u16]) -> String {
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

pub fn is_supported_ctrl_export_class(class_name: &str) -> bool {
    let lower = class_name.to_ascii_lowercase();
    lower.contains("syslistview32")
        || lower.contains("systreeview32")
        || lower == "listbox"
        || lower == "combobox"
        || lower == "edit"
}

pub fn ctrl_export_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_ctrl_export",
        "Control Data Export",
        vec![
            ColumnDef::new("source_hwnd", "Source HWND", ColumnType::Hex).with_width_hint(110),
            ColumnDef::new("control_class", "Control Class", ColumnType::Text).with_width_hint(140),
            ColumnDef::new("process_name", "Process", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("row_index", "Row", ColumnType::Uint).with_width_hint(65),
            ColumnDef::new("col_index", "Col", ColumnType::Uint).with_width_hint(65),
            ColumnDef::new("column_title", "Column Title", ColumnType::Text).with_width_hint(150),
            ColumnDef::new("item_text", "Item Text", ColumnType::Text).with_width_hint(260),
        ],
    )
}

pub fn ctrl_export_item_to_row(id: u64, item: &CtrlExportItem) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Text(format_hwnd(item.source_hwnd)),
            CellValue::Text(item.control_class.clone()),
            CellValue::Text(item.process_name.clone()),
            CellValue::Uint(u64::from(item.row_index)),
            CellValue::Uint(u64::from(item.col_index)),
            CellValue::Text(item.column_title.clone()),
            CellValue::Text(item.item_text.clone()),
        ],
    )
    .with_detail_json(serde_json::to_value(item).unwrap_or(Value::Null))
}

pub struct CtrlExportTableProvider {
    backend: Arc<dyn DesktopBackend>,
}

impl CtrlExportTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for CtrlExportTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_ctrl_export"
    }

    fn description(&self) -> &'static str {
        "Read SysListView32/SysTreeView32/ListBox/ComboBox text via non-injecting cross-process messages"
    }

    fn schema(&self) -> TableSchema {
        ctrl_export_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let items = self
            .backend
            .export_sys_controls()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = items
            .iter()
            .enumerate()
            .map(|(idx, item)| ctrl_export_item_to_row(idx as u64 + 1, item))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}
