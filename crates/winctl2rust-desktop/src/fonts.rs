use std::path::Path;
use std::sync::Arc;

use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde_json::Value;
use winctl2rust_core::WinctlResult;

use crate::backend::{DesktopBackend, InstalledFontEntry};
use crate::opentype::parse_font_bytes;

pub fn font_entries_from_bytes(bytes: &[u8], source_path: &str) -> WinctlResult<Vec<InstalledFontEntry>> {
    let faces = parse_font_bytes(bytes)?;
    let mut out = Vec::with_capacity(faces.len());
    for face in faces {
        let os2 = face.os2.as_ref();
        out.push(InstalledFontEntry {
            family: face.family(),
            subfamily: face.subfamily(),
            full_name: face.full_name(),
            postscript_name: face.postscript_name(),
            font_format: face.format.label().to_string(),
            weight_class: os2.map_or(400, |o| o.us_weight_class),
            width_class: os2.map_or(5, |o| o.us_width_class),
            fs_type: os2.map_or(0, |o| o.fs_type),
            panose: os2.map_or_else(String::new, |o| o.panose_formatted()),
            version: face.version(),
            source_path: source_path.to_string(),
            face_index: face.face_index,
        });
    }
    Ok(out)
}

pub fn font_entries_from_file(path: &Path) -> WinctlResult<Vec<InstalledFontEntry>> {
    let bytes = std::fs::read(path)?;
    font_entries_from_bytes(&bytes, &path.display().to_string())
}

pub fn fonts_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_fonts",
        "WinFontsView Installed Fonts & OpenType Metadata",
        vec![
            ColumnDef::new("family", "Family", ColumnType::Text).with_width_hint(180),
            ColumnDef::new("subfamily", "Subfamily", ColumnType::Text).with_width_hint(100),
            ColumnDef::new("full_name", "Full Name", ColumnType::Text).with_width_hint(200),
            ColumnDef::new("postscript_name", "PostScript Name", ColumnType::Text).with_width_hint(180),
            ColumnDef::new("font_format", "Format", ColumnType::Text).with_width_hint(120),
            ColumnDef::new("weight_class", "Weight", ColumnType::Uint).with_width_hint(75),
            ColumnDef::new("width_class", "Width", ColumnType::Uint).with_width_hint(70),
            ColumnDef::new("fs_type", "fsType", ColumnType::Hex).with_width_hint(80),
            ColumnDef::new("panose", "PANOSE", ColumnType::Text).with_width_hint(180),
            ColumnDef::new("version", "Version", ColumnType::Text).with_width_hint(130),
            ColumnDef::new("source_path", "Source Path", ColumnType::Text).with_width_hint(240),
            ColumnDef::new("face_index", "Face", ColumnType::Uint).with_width_hint(60),
        ],
    )
}

pub fn font_entry_to_row(id: u64, f: &InstalledFontEntry) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Text(f.family.clone()),
            CellValue::Text(f.subfamily.clone()),
            CellValue::Text(f.full_name.clone()),
            CellValue::Text(f.postscript_name.clone()),
            CellValue::Text(f.font_format.clone()),
            CellValue::Uint(u64::from(f.weight_class)),
            CellValue::Uint(u64::from(f.width_class)),
            CellValue::Text(format!("0x{:04X}", f.fs_type)),
            CellValue::Text(f.panose.clone()),
            CellValue::Text(f.version.clone()),
            CellValue::Text(f.source_path.clone()),
            CellValue::Uint(u64::from(f.face_index)),
        ],
    )
    .with_detail_json(serde_json::to_value(f).unwrap_or(Value::Null))
}

pub struct FontsTableProvider {
    backend: Arc<dyn DesktopBackend>,
}

impl FontsTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for FontsTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_fonts"
    }

    fn description(&self) -> &'static str {
        "Enumerate installed Windows fonts and decode OpenType name/OS2 tables (WinFontsView)"
    }

    fn schema(&self) -> TableSchema {
        fonts_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let fonts = self
            .backend
            .list_installed_fonts()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = fonts
            .iter()
            .enumerate()
            .map(|(idx, f)| font_entry_to_row(idx as u64 + 1, f))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}
