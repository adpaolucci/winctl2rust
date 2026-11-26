use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use guap_export::sha256_hex;
use guap_mcp::ActionRiskLevel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use winctl2rust_core::{WinctlError, WinctlResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionAuditRecord {
    pub timestamp: DateTime<Utc>,
    pub command_id: String,
    pub risk_level: ActionRiskLevel,
    pub dry_run: bool,
    pub succeeded: bool,
    pub summary: String,
    pub payload_sha256: String,
}

pub fn record_action_audit(
    command_id: &str,
    risk_level: ActionRiskLevel,
    dry_run: bool,
    succeeded: bool,
    summary: impl Into<String>,
    payload: &Value,
) -> ActionAuditRecord {
    let canonical_bytes = serde_json::to_vec(payload).unwrap_or_default();
    ActionAuditRecord {
        timestamp: Utc::now(),
        command_id: command_id.to_string(),
        risk_level,
        dry_run,
        succeeded,
        summary: summary.into(),
        payload_sha256: sha256_hex(&canonical_bytes),
    }
}

pub fn append_audit_jsonl(path: &Path, record: &ActionAuditRecord) -> WinctlResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(record).map_err(|e| WinctlError::Export(e.into()))?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}
