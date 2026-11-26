use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::{ClipHistEntry, DesktopBackend, validate_safe_path};
use crate::clipboard::{format_clipboard_preview, sha256_hex};

pub fn default_archive_path() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    base.join("winctl2rust").join("clipboard_history.db")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardJsonArchive {
    pub schema_version: u32,
    pub entries: Vec<ClipboardJsonEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardJsonEntry {
    pub entry_id: u64,
    pub timestamp: String,
    pub format_id: u32,
    pub format: String,
    pub size_bytes: u64,
    pub text_preview: String,
    pub sha256: String,
    pub payload_hex: String,
    #[serde(default)]
    pub source_app: String,
}

pub fn encode_hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub fn decode_hex(hex: &str) -> WinctlResult<Vec<u8>> {
    let trimmed = hex.trim();
    if !trimmed.len().is_multiple_of(2) {
        return Err(WinctlError::ParseError(format!(
            "hex payload has odd length {}",
            trimmed.len()
        )));
    }
    let mut out = Vec::with_capacity(trimmed.len() / 2);
    for pair in trimmed.as_bytes().as_chunks::<2>().0 {
        let s = std::str::from_utf8(pair)
            .map_err(|_| WinctlError::ParseError("non-UTF-8 byte in hex payload".to_string()))?;
        let byte = u8::from_str_radix(s, 16).map_err(|_| WinctlError::ParseError(format!("invalid hex byte '{s}'")))?;
        out.push(byte);
    }
    Ok(out)
}

pub struct ClipboardArchive {
    conn: Connection,
}

impl ClipboardArchive {
    pub fn open(path: &Path) -> WinctlResult<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite open '{}': {e}", path.display()))))?;
        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> WinctlResult<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite in-memory open: {e}"))))?;
        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> WinctlResult<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT OR IGNORE INTO schema_meta (key, value) VALUES ('schema_version', '1');

            CREATE TABLE IF NOT EXISTS clipboard_entries (
                entry_id     INTEGER PRIMARY KEY AUTOINCREMENT,
                captured_at  TEXT NOT NULL,
                format_id    INTEGER NOT NULL,
                format_name  TEXT NOT NULL,
                size_bytes   INTEGER NOT NULL,
                text_preview TEXT NOT NULL,
                sha256       TEXT NOT NULL,
                payload      BLOB NOT NULL,
                source_app   TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS idx_clipboard_captured_at ON clipboard_entries(captured_at DESC, entry_id DESC);
            CREATE INDEX IF NOT EXISTS idx_clipboard_sha256 ON clipboard_entries(sha256);",
        )
        .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite schema init: {e}"))))?;
        Ok(())
    }

    pub fn insert_entry(
        &self,
        format_id: u32,
        format_name: &str,
        payload: &[u8],
        source_app: &str,
    ) -> WinctlResult<ClipHistEntry> {
        let captured_at = Utc::now();
        let captured_str = captured_at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
        let sha256 = sha256_hex(payload);
        let (text_preview, _) = format_clipboard_preview(format_id, format_name, payload);
        let size_bytes = payload.len() as u64;

        self.conn
            .execute(
                "INSERT INTO clipboard_entries
                 (captured_at, format_id, format_name, size_bytes, text_preview, sha256, payload, source_app)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    captured_str,
                    i64::from(format_id),
                    format_name,
                    size_bytes as i64,
                    text_preview,
                    sha256,
                    payload,
                    source_app,
                ],
            )
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite insert: {e}"))))?;

        let entry_id = self.conn.last_insert_rowid() as u64;
        Ok(ClipHistEntry {
            entry_id,
            captured_at,
            format_id,
            format_name: format_name.to_string(),
            size_bytes,
            sha256,
            text_preview,
            payload: payload.to_vec(),
            source_app: source_app.to_string(),
        })
    }

    pub fn list_entries(&self) -> WinctlResult<Vec<ClipHistEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entry_id, captured_at, format_id, format_name, size_bytes, text_preview, sha256, payload, source_app
                 FROM clipboard_entries
                 ORDER BY entry_id DESC",
            )
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite prepare: {e}"))))?;

        let rows = stmt
            .query_map([], |row| {
                let entry_id_i: i64 = row.get(0)?;
                let captured_s: String = row.get(1)?;
                let format_id_i: i64 = row.get(2)?;
                let format_name: String = row.get(3)?;
                let size_i: i64 = row.get(4)?;
                let text_preview: String = row.get(5)?;
                let sha256: String = row.get(6)?;
                let payload: Vec<u8> = row.get(7)?;
                let source_app: String = row.get(8)?;
                Ok((
                    entry_id_i,
                    captured_s,
                    format_id_i,
                    format_name,
                    size_i,
                    text_preview,
                    sha256,
                    payload,
                    source_app,
                ))
            })
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite query: {e}"))))?;

        let mut out = Vec::new();
        for item in rows {
            let (entry_id_i, captured_s, format_id_i, format_name, size_i, text_preview, sha256, payload, source_app) =
                item.map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite row: {e}"))))?;
            let captured_at = DateTime::parse_from_rfc3339(&captured_s)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            out.push(ClipHistEntry {
                entry_id: entry_id_i.max(0) as u64,
                captured_at,
                format_id: format_id_i.max(0) as u32,
                format_name,
                size_bytes: size_i.max(0) as u64,
                sha256,
                text_preview,
                payload,
                source_app,
            });
        }
        Ok(out)
    }

    pub fn get_entry(&self, entry_id: u64) -> WinctlResult<Option<ClipHistEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entry_id, captured_at, format_id, format_name, size_bytes, text_preview, sha256, payload, source_app
                 FROM clipboard_entries
                 WHERE entry_id = ?1",
            )
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite prepare: {e}"))))?;
        let mut rows = stmt
            .query(params![entry_id as i64])
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite query: {e}"))))?;
        if let Some(row) = rows
            .next()
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite next: {e}"))))?
        {
            let entry_id_i: i64 = row
                .get(0)
                .map_err(|e| WinctlError::Io(std::io::Error::other(e.to_string())))?;
            let captured_s: String = row
                .get(1)
                .map_err(|e| WinctlError::Io(std::io::Error::other(e.to_string())))?;
            let format_id_i: i64 = row
                .get(2)
                .map_err(|e| WinctlError::Io(std::io::Error::other(e.to_string())))?;
            let format_name: String = row
                .get(3)
                .map_err(|e| WinctlError::Io(std::io::Error::other(e.to_string())))?;
            let size_i: i64 = row
                .get(4)
                .map_err(|e| WinctlError::Io(std::io::Error::other(e.to_string())))?;
            let text_preview: String = row
                .get(5)
                .map_err(|e| WinctlError::Io(std::io::Error::other(e.to_string())))?;
            let sha256: String = row
                .get(6)
                .map_err(|e| WinctlError::Io(std::io::Error::other(e.to_string())))?;
            let payload: Vec<u8> = row
                .get(7)
                .map_err(|e| WinctlError::Io(std::io::Error::other(e.to_string())))?;
            let source_app: String = row
                .get(8)
                .map_err(|e| WinctlError::Io(std::io::Error::other(e.to_string())))?;
            let captured_at = DateTime::parse_from_rfc3339(&captured_s)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            return Ok(Some(ClipHistEntry {
                entry_id: entry_id_i.max(0) as u64,
                captured_at,
                format_id: format_id_i.max(0) as u32,
                format_name,
                size_bytes: size_i.max(0) as u64,
                sha256,
                text_preview,
                payload,
                source_app,
            }));
        }
        Ok(None)
    }

    pub fn delete_entry(&self, entry_id: u64) -> WinctlResult<bool> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM clipboard_entries WHERE entry_id = ?1",
                params![entry_id as i64],
            )
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite delete: {e}"))))?;
        Ok(affected > 0)
    }

    pub fn clear(&self) -> WinctlResult<usize> {
        let affected = self
            .conn
            .execute("DELETE FROM clipboard_entries", [])
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite clear: {e}"))))?;
        Ok(affected)
    }

    pub fn export_json(&self) -> WinctlResult<ClipboardJsonArchive> {
        let entries = self.list_entries()?;
        let json_entries = entries
            .into_iter()
            .map(|e| ClipboardJsonEntry {
                entry_id: e.entry_id,
                timestamp: e.captured_at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
                format_id: e.format_id,
                format: e.format_name,
                size_bytes: e.size_bytes,
                text_preview: e.text_preview,
                sha256: e.sha256,
                payload_hex: encode_hex_lower(&e.payload),
                source_app: e.source_app,
            })
            .collect();
        Ok(ClipboardJsonArchive {
            schema_version: 1,
            entries: json_entries,
        })
    }

    pub fn import_json(&self, archive: &ClipboardJsonArchive) -> WinctlResult<usize> {
        if archive.schema_version != 1 {
            return Err(WinctlError::ParseError(format!(
                "unsupported clipboard JSON schema_version {} (expected 1)",
                archive.schema_version
            )));
        }

        struct ValidatedEntry<'a> {
            timestamp: &'a str,
            format_id: u32,
            format_name: &'a str,
            size_bytes: usize,
            text_preview: String,
            sha256: String,
            payload: Vec<u8>,
            source_app: &'a str,
        }

        let mut validated = Vec::with_capacity(archive.entries.len());
        for item in &archive.entries {
            if item.format_id == 0 {
                return Err(WinctlError::ParseError(format!(
                    "clipboard JSON entry {} has invalid format_id 0",
                    item.entry_id
                )));
            }
            if item.format.trim().is_empty() {
                return Err(WinctlError::ParseError(format!(
                    "clipboard JSON entry {} has empty format name",
                    item.entry_id
                )));
            }
            if item.timestamp.trim().is_empty() {
                return Err(WinctlError::ParseError(format!(
                    "clipboard JSON entry {} has empty timestamp",
                    item.entry_id
                )));
            }
            let expected_sha = item.sha256.trim();
            if expected_sha.is_empty() {
                return Err(WinctlError::ParseError(format!(
                    "clipboard JSON entry {} is missing required sha256",
                    item.entry_id
                )));
            }
            let payload = decode_hex(&item.payload_hex)?;
            if item.size_bytes != payload.len() as u64 {
                return Err(WinctlError::ParseError(format!(
                    "clipboard JSON entry {} size_bytes mismatch (declared {}, decoded {})",
                    item.entry_id,
                    item.size_bytes,
                    payload.len()
                )));
            }
            let actual_sha = sha256_hex(&payload);
            if !actual_sha.eq_ignore_ascii_case(expected_sha) {
                return Err(WinctlError::ParseError(format!(
                    "clipboard JSON entry {} SHA-256 mismatch (expected {expected_sha}, got {actual_sha})",
                    item.entry_id
                )));
            }
            let (preview, _) = format_clipboard_preview(item.format_id, &item.format, &payload);
            let text_preview = if item.text_preview.is_empty() {
                preview
            } else {
                item.text_preview.clone()
            };
            validated.push(ValidatedEntry {
                timestamp: item.timestamp.trim(),
                format_id: item.format_id,
                format_name: item.format.trim(),
                size_bytes: payload.len(),
                text_preview,
                sha256: actual_sha,
                payload,
                source_app: &item.source_app,
            });
        }

        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite begin tx: {e}"))))?;
        let mut imported = 0usize;
        for entry in validated {
            tx.execute(
                "INSERT INTO clipboard_entries
                 (captured_at, format_id, format_name, size_bytes, text_preview, sha256, payload, source_app)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    entry.timestamp,
                    i64::from(entry.format_id),
                    entry.format_name,
                    entry.size_bytes as i64,
                    entry.text_preview,
                    entry.sha256,
                    entry.payload,
                    entry.source_app,
                ],
            )
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite import: {e}"))))?;
            imported += 1;
        }
        tx.commit()
            .map_err(|e| WinctlError::Io(std::io::Error::other(format!("sqlite commit tx: {e}"))))?;
        Ok(imported)
    }
}

pub fn clip_hist_table_schema() -> TableSchema {
    TableSchema::new(
        "desktop_clip_hist",
        "Local Clipboard Archive",
        vec![
            ColumnDef::new("entry_id", "Entry ID", ColumnType::Uint).with_width_hint(80),
            ColumnDef::new("captured_at", "Captured At", ColumnType::Timestamp).with_width_hint(180),
            ColumnDef::new("format_id", "Format ID", ColumnType::Uint).with_width_hint(85),
            ColumnDef::new("format_name", "Format", ColumnType::Text).with_width_hint(140),
            ColumnDef::new("size_bytes", "Size", ColumnType::BytesSize).with_width_hint(100),
            ColumnDef::new("sha256", "SHA-256", ColumnType::Text).with_width_hint(220),
            ColumnDef::new("text_preview", "Preview", ColumnType::Text).with_width_hint(260),
            ColumnDef::new("source_app", "Source App", ColumnType::Text).with_width_hint(140),
        ],
    )
}

pub fn clip_hist_entry_to_row(id: u64, e: &ClipHistEntry) -> Row {
    Row::new(
        id,
        vec![
            CellValue::Uint(e.entry_id),
            CellValue::Timestamp(e.captured_at),
            CellValue::Uint(u64::from(e.format_id)),
            CellValue::Text(e.format_name.clone()),
            CellValue::Bytes(e.size_bytes),
            CellValue::Text(e.sha256.clone()),
            CellValue::Text(e.text_preview.clone()),
            CellValue::Text(e.source_app.clone()),
        ],
    )
    .with_raw_bytes(e.payload.clone())
    .with_detail_json(json!({
        "entry_id": e.entry_id,
        "captured_at": e.captured_at,
        "format_id": e.format_id,
        "format_name": e.format_name,
        "size_bytes": e.size_bytes,
        "sha256": e.sha256,
        "text_preview": e.text_preview,
        "source_app": e.source_app,
    }))
}

pub struct ClipHistTableProvider {
    backend: Arc<dyn DesktopBackend>,
    archive_path_override: Option<PathBuf>,
}

impl ClipHistTableProvider {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self {
            backend,
            archive_path_override: None,
        }
    }

    pub fn with_archive_path(backend: Arc<dyn DesktopBackend>, path: PathBuf) -> Self {
        Self {
            backend,
            archive_path_override: Some(path),
        }
    }

    fn resolved_db_path(&self) -> PathBuf {
        self.archive_path_override
            .clone()
            .or_else(|| self.backend.default_clip_hist_db_path())
            .unwrap_or_else(default_archive_path)
    }
}

impl TableProvider for ClipHistTableProvider {
    fn tool_id(&self) -> &'static str {
        "desktop_clip_hist"
    }

    fn description(&self) -> &'static str {
        "Browse local SQLite clipboard history archive"
    }

    fn schema(&self) -> TableSchema {
        clip_hist_table_schema()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let db_path = self.resolved_db_path();
        if !db_path.exists() {
            return apply_query(&self.schema(), &[], req);
        }
        let archive = ClipboardArchive::open(&db_path).map_err(|e| TableError::Provider(e.to_string()))?;
        let entries = archive
            .list_entries()
            .map_err(|e| TableError::Provider(e.to_string()))?;
        let rows: Vec<Row> = entries
            .iter()
            .enumerate()
            .map(|(idx, e)| clip_hist_entry_to_row(idx as u64 + 1, e))
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ClipHistControlInput {
    operation: String,
    #[serde(default)]
    entry_id: Option<u64>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    db_path: Option<String>,
    #[serde(default)]
    wait_for_update: Option<bool>,
    #[serde(default)]
    timeout_ms: Option<u32>,
}

pub struct ClipHistControlAction {
    backend: Arc<dyn DesktopBackend>,
    archive_path_override: Option<PathBuf>,
}

impl ClipHistControlAction {
    pub fn new(backend: Arc<dyn DesktopBackend>) -> Self {
        Self {
            backend,
            archive_path_override: None,
        }
    }

    pub fn with_archive_path(backend: Arc<dyn DesktopBackend>, path: PathBuf) -> Self {
        Self {
            backend,
            archive_path_override: Some(path),
        }
    }

    fn resolved_db_path(&self, req_db_path: Option<&str>) -> PathBuf {
        req_db_path
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| self.archive_path_override.clone())
            .or_else(|| self.backend.default_clip_hist_db_path())
            .unwrap_or_else(default_archive_path)
    }
}

impl ActionCommand for ClipHistControlAction {
    fn command_id(&self) -> &'static str {
        "desktop_clip_hist_control"
    }

    fn description(&self) -> &'static str {
        "Capture, restore, delete, clear, or export/import local SQLite/JSON clipboard history"
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
                    "enum": ["capture_current", "restore", "delete", "clear", "export_json", "import_json"]
                },
                "entry_id": { "type": "integer", "minimum": 1 },
                "path": { "type": "string", "description": "JSON file path for export_json/import_json" },
                "db_path": { "type": "string", "description": "Optional override SQLite archive path" },
                "wait_for_update": { "type": "boolean", "description": "When true during capture_current, listen for WM_CLIPBOARDUPDATE via AddClipboardFormatListener" },
                "timeout_ms": { "type": "integer", "minimum": 0 }
            },
            "required": ["operation"]
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let input: ClipHistControlInput = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid desktop_clip_hist_control request: {e}")))?;

        let mut r4_args = Vec::new();
        if let Some(ref p) = input.path {
            r4_args.push(p.clone());
        }
        if let Some(ref db) = input.db_path {
            r4_args.push(db.clone());
        }
        validate_r4_compliance(&input.operation, &r4_args).map_err(McpToolError::from)?;

        let validated_path = match input.path.as_deref() {
            Some(raw_path) => Some(validate_safe_path(raw_path, "path").map_err(McpToolError::from)?),
            None => None,
        };
        let validated_db_path = match input.db_path.as_deref() {
            Some(raw_db) => Some(validate_safe_path(raw_db, "db_path").map_err(McpToolError::from)?),
            None => None,
        };

        let op_norm = input.operation.trim().to_ascii_lowercase();
        match op_norm.as_str() {
            "capture_current" | "clear" => {}
            "restore" | "delete" => {
                let id = input.entry_id.filter(|&v| v > 0).ok_or_else(|| {
                    McpToolError::InvalidParams(format!("operation '{op_norm}' requires positive 'entry_id'"))
                })?;
                let _ = id;
            }
            "export_json" | "import_json" => {
                if validated_path.is_none() {
                    return Err(McpToolError::InvalidParams(format!(
                        "operation '{op_norm}' requires 'path'"
                    )));
                }
            }
            other => {
                return Err(McpToolError::InvalidParams(format!(
                    "unsupported desktop_clip_hist_control operation '{other}'"
                )));
            }
        }

        let db_path = self.resolved_db_path(validated_db_path.as_deref());
        if dry_run {
            return Ok(dry_run_preview(
                self.command_id(),
                "Preview SQLite/JSON archive or clipboard restore operation",
                json!({
                    "operation": op_norm,
                    "db_path": db_path,
                    "entry_id": input.entry_id,
                    "path": validated_path,
                }),
            ));
        }

        let archive = ClipboardArchive::open(&db_path).map_err(McpToolError::from)?;
        match op_norm.as_str() {
            "capture_current" => {
                let captured = if input.wait_for_update.unwrap_or(false) {
                    let timeout = input.timeout_ms.unwrap_or(2000);
                    match self
                        .backend
                        .monitor_clipboard_update(timeout)
                        .map_err(McpToolError::from)?
                    {
                        Some(entry) => vec![entry],
                        None => Vec::new(),
                    }
                } else {
                    self.backend.inspect_clipboard().map_err(McpToolError::from)?
                };
                let mut stored = Vec::with_capacity(captured.len());
                for item in captured {
                    let row = archive
                        .insert_entry(item.format_id, &item.format_name, &item.raw_bytes, "")
                        .map_err(McpToolError::from)?;
                    stored.push(json!({
                        "entry_id": row.entry_id,
                        "format_id": row.format_id,
                        "format_name": row.format_name,
                        "size_bytes": row.size_bytes,
                        "sha256": row.sha256,
                    }));
                }
                Ok(json!({
                    "dry_run": false,
                    "applied": true,
                    "operation": "capture_current",
                    "db_path": db_path,
                    "captured_count": stored.len(),
                    "entries": stored,
                }))
            }
            "restore" => {
                let entry_id = input.entry_id.unwrap();
                let entry = archive
                    .get_entry(entry_id)
                    .map_err(McpToolError::from)?
                    .ok_or_else(|| {
                        McpToolError::from(WinctlError::InvalidInput(format!(
                            "clipboard history entry {entry_id} not found"
                        )))
                    })?;
                let res = self
                    .backend
                    .set_clipboard_payload(entry.format_id, &entry.payload)
                    .map_err(McpToolError::from)?;
                Ok(json!({
                    "dry_run": false,
                    "applied": true,
                    "operation": "restore",
                    "entry_id": entry_id,
                    "result": res,
                }))
            }
            "delete" => {
                let entry_id = input.entry_id.unwrap();
                let deleted = archive.delete_entry(entry_id).map_err(McpToolError::from)?;
                if !deleted {
                    return Err(McpToolError::from(WinctlError::InvalidInput(format!(
                        "clipboard history entry {entry_id} not found"
                    ))));
                }
                Ok(json!({
                    "dry_run": false,
                    "applied": true,
                    "operation": "delete",
                    "entry_id": entry_id,
                    "deleted": true,
                }))
            }
            "clear" => {
                let removed = archive.clear().map_err(McpToolError::from)?;
                Ok(json!({
                    "dry_run": false,
                    "applied": true,
                    "operation": "clear",
                    "removed_count": removed,
                }))
            }
            "export_json" => {
                let path_str = validated_path.as_deref().unwrap();
                let json_archive = archive.export_json().map_err(McpToolError::from)?;
                let body = serde_json::to_string_pretty(&json_archive)
                    .map_err(|e| McpToolError::ExecutionFailed(e.to_string()))?;
                fs::write(Path::new(path_str), body)
                    .map_err(|e| McpToolError::ExecutionFailed(format!("failed to write '{path_str}': {e}")))?;
                Ok(json!({
                    "dry_run": false,
                    "applied": true,
                    "operation": "export_json",
                    "path": path_str,
                    "entry_count": json_archive.entries.len(),
                }))
            }
            "import_json" => {
                let path_str = validated_path.as_deref().unwrap();
                let raw = fs::read_to_string(Path::new(path_str))
                    .map_err(|e| McpToolError::ExecutionFailed(format!("failed to read '{path_str}': {e}")))?;
                let parsed: ClipboardJsonArchive = serde_json::from_str(&raw)
                    .map_err(|e| McpToolError::from(WinctlError::ParseError(format!("invalid clipboard JSON: {e}"))))?;
                let imported = archive.import_json(&parsed).map_err(McpToolError::from)?;
                Ok(json!({
                    "dry_run": false,
                    "applied": true,
                    "operation": "import_json",
                    "path": path_str,
                    "imported_count": imported,
                }))
            }
            _ => unreachable!(),
        }
    }
}
