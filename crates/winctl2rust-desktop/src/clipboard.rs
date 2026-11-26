use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use winctl2rust_core::{dry_run_preview, validate_r4_compliance};

use crate::backend::{ClipboardFormatEntry, DesktopBackend};
use crate::lnk::decode_cp1252;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub fn standard_clipboard_format_name(format_id: u32) -> &'static str {
    match format_id {
        1 => "CF_TEXT",
        2 => "CF_BITMAP",
        3 => "CF_METAFILEPICT",
        4 => "CF_SYLK",
        5 => "CF_DIF",
        6 => "CF_TIFF",
        7 => "CF_OEMTEXT",
        8 => "CF_DIB",
        9 => "CF_PALETTE",
        10 => "CF_PENDATA",
        11 => "CF_RIFF",
        12 => "CF_WAVE",
        13 => "CF_UNICODETEXT",
        14 => "CF_ENHMETAFILE",
        15 => "CF_HDROP",
        16 => "CF_LOCALE",
        17 => "CF_DIBV5",
        _ => "CF_REGISTERED",
    }
}

pub fn is_text_clipboard_format(format_id: u32, format_name: &str) -> bool {
    matches!(format_id, 1 | 7 | 13)
        || format_name.eq_ignore_ascii_case("HTML Format")
        || format_name.eq_ignore_ascii_case("Rich Text Format")
}

pub fn format_clipboard_preview(format_id: u32, format_name: &str, raw_bytes: &[u8]) -> (String, bool) {
    if format_id == 13 {
        let mut units = Vec::with_capacity(raw_bytes.len() / 2);
        for chunk in raw_bytes.as_chunks::<2>().0 {
            let u = u16::from_le_bytes(*chunk);
            if u == 0 {
                break;
            }
            units.push(u);
        }
        let text = String::from_utf16_lossy(&units);
        return (truncate_chars(&text, 256), true);
    }
    if matches!(format_id, 1 | 7)
        || format_name.eq_ignore_ascii_case("HTML Format")
        || format_name.eq_ignore_ascii_case("Rich Text Format")
    {
        let end = raw_bytes.iter().position(|&b| b == 0).unwrap_or(raw_bytes.len());
        let slice = &raw_bytes[..end];
        let text = match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(_) => decode_cp1252(slice),
        };
        return (truncate_chars(&text, 256), true);
    }
    (format!("[binary {} bytes]", raw_bytes.len()), false)
}

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut count = 0usize;
    let mut end_byte = s.len();
    for (idx, _) in s.char_indices() {
        if count == max_chars {
            end_byte = idx;
            break;
        }
        count += 1;
    }
    if count >= max_chars && end_byte < s.len() {
        format!("{}...", &s[..end_byte])
    } else {
        s.to_string()
    }
}

pub fn build_clipboard_entry(
    format_id: u32,
    registered_name: Option<String>,
    raw_bytes: Vec<u8>,
) -> ClipboardFormatEntry {
    let format_name = registered_name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| standard_clipboard_format_name(format_id).to_string());
    let sha256 = sha256_hex(&raw_bytes);
    let size_bytes = raw_bytes.len() as u64;
    let (text_preview, is_text) = format_clipboard_preview(format_id, &format_name, &raw_bytes);
    ClipboardFormatEntry {
        format_id,
        format_name,
        size_bytes,
        sha256,
        text_preview,
        is_text,
        raw_bytes,
    }
}

pub fn clipboard_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_clipboard",
        "InsideClipboard Active Formats",
        vec![
            ColumnDef::new("format_id", "Format ID", ColumnType::Uint).with_width_hint(90),
            ColumnDef::new("format_name", "Format Name", ColumnType::Text).with_width_hint(180),
            ColumnDef::new("size_bytes", "Size", ColumnType::BytesSize).with_width_hint(100),
            ColumnDef::new("sha256", "SHA-256", ColumnType::Text).with_width_hint(220),
            ColumnDef::new("text_preview", "Preview", ColumnType::Text).with_width_hint(260),
            ColumnDef::new("is_text", "Text", ColumnType::Bool).with_width_hint(65),
        ],
    )
}

pub fn clipboard_entry_to_row(id: u64, c: &ClipboardFormatEntry) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Uint(u64::from(c.format_id)),
            CellValue::Text(c.format_name.clone()),
            CellValue::Bytes(c.size_bytes),
            CellValue::Text(c.sha256.clone()),
            CellValue::Text(c.text_preview.clone()),
            CellValue::Bool(c.is_text),
        ],
    )
    .with_raw_bytes(c.raw_bytes.clone())
    .with_detail_json(json!({
        "format_id": c.format_id,
        "format_name": c.format_name,
        "size_bytes": c.size_bytes,
        "sha256": c.sha256,
        "text_preview": c.text_preview,
        "is_text": c.is_text,
    }))
}

pub struct ClipboardTableProvider {
    backend: Arc<dyn DesktopBackend>,
}

impl ClipboardTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl TableProvider for ClipboardTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_clipboard"
    }

    fn description(&self) -> &'static str {
        "Inspect active Windows clipboard formats, byte sizes, SHA-256 hashes, and previews (InsideClipboard)"
    }

    fn schema(&self) -> TableSchema {
        clipboard_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let entries = self
            .backend
            .inspect_clipboard()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = entries
            .iter()
            .enumerate()
            .map(|(idx, e)| clipboard_entry_to_row(idx as u64 + 1, e))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ClipboardControlInput {
    operation: String,
    #[serde(default)]
    text: Option<String>,
}

pub struct ClipboardControlAction {
    backend: Arc<dyn DesktopBackend>,
}

impl ClipboardControlAction {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for ClipboardControlAction {
    fn command_id(&self) -> &'static str {
        "desktop_clipboard_control"
    }

    fn description(&self) -> &'static str {
        "Clear the Windows clipboard or set CF_UNICODETEXT content (InsideClipboard)"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["clear", "set_text"]
                },
                "text": {
                    "type": "string",
                    "description": "UTF-8 text to place onto clipboard as CF_UNICODETEXT when operation='set_text'"
                }
            },
            "required": ["operation"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let input: ClipboardControlInput = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid desktop_clipboard_control request: {e}")))?;

        let mut r4_args = Vec::new();
        if let Some(ref t) = input.text {
            r4_args.push(t.clone());
        }
        validate_r4_compliance(&input.operation, &r4_args).map_err(McpToolError::from)?;

        let op_norm = input.operation.trim().to_ascii_lowercase();
        let maybe_text = match op_norm.as_str() {
            "clear" => None,
            "set_text" => {
                let text = input
                    .text
                    .ok_or_else(|| McpToolError::InvalidParams("set_text requires 'text'".to_string()))?;
                Some(text)
            }
            other => {
                return Err(McpToolError::InvalidParams(format!(
                    "unsupported desktop_clipboard_control operation '{other}'"
                )));
            }
        };

        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview InsideClipboard mutation without calling EmptyClipboard/SetClipboardData",
                json!({
                    "operation": op_norm,
                    "text_len": maybe_text.as_ref().map(String::len),
                }),
            ));
        }

        let res = match maybe_text {
            None => self.backend.clear_clipboard().map_err(McpToolError::from)?,
            Some(ref text) => self.backend.set_clipboard_text(text).map_err(McpToolError::from)?,
        };
        Ok(json!({
            "dry_run": false,
            "applied": true,
            "result": res,
        }))
    }
}
