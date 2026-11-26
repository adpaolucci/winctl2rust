#![forbid(unsafe_code)]

pub mod audit;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use guap_export::{
    CustodyLeaf, CustodyManifest, ExportFormat, ExportOptions, build_custody_manifest, export_table, sha256_hex,
    sha512_hex, write_table_export,
};
use guap_table::TableData;
use serde::{Deserialize, Serialize};
use winctl2rust_core::WinctlResult;

pub use audit::{ActionAuditRecord, append_audit_jsonl, record_action_audit};
pub use guap_export::{ExportFormat as ReportFormat, neutralize_spreadsheet_cell};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinctlReportOptions {
    pub export: ExportOptions,
    pub write_custody_sidecar: bool,
    pub operator: Option<String>,
    pub case_id: Option<String>,
    pub host_name: Option<String>,
    pub dry_run: bool,
}

impl Default for WinctlReportOptions {
    fn default() -> Self {
        Self {
            export: ExportOptions {
                tool_version: env!("CARGO_PKG_VERSION").to_string(),
                ..ExportOptions::default()
            },
            write_custody_sidecar: true,
            operator: None,
            case_id: None,
            host_name: None,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinctlCustodyReport {
    pub suite_name: String,
    pub suite_version: String,
    pub operator: Option<String>,
    pub case_id: Option<String>,
    pub host_name: Option<String>,
    pub dry_run: bool,
    pub custody: CustodyManifest,
    pub artifact_paths: Vec<PathBuf>,
    pub sidecar_path: Option<PathBuf>,
}

pub fn render_table(data: &TableData, format: ExportFormat, opts: &WinctlReportOptions) -> WinctlResult<Vec<u8>> {
    Ok(export_table(data, format, &opts.export)?)
}

pub fn write_report(
    data: &TableData,
    path: &Path,
    format: ExportFormat,
    opts: &WinctlReportOptions,
) -> WinctlResult<WinctlCustodyReport> {
    let custody = write_table_export(data, path, format, &opts.export)?;
    let sidecar_path = if opts.write_custody_sidecar {
        let sidecar = sidecar_path_for(path);
        let mut report = build_report_envelope(custody, vec![path.to_path_buf()], Some(sidecar.clone()), opts);
        write_report_sidecar(&report, &sidecar)?;
        report.sidecar_path = Some(sidecar);
        return Ok(report);
    } else {
        None
    };

    Ok(build_report_envelope(
        custody,
        vec![path.to_path_buf()],
        sidecar_path,
        opts,
    ))
}

pub fn write_report_bundle(
    tables: &[&TableData],
    dir: &Path,
    formats: &[ExportFormat],
    opts: &WinctlReportOptions,
) -> WinctlResult<WinctlCustodyReport> {
    fs::create_dir_all(dir)?;

    let mut leaves = Vec::new();
    let mut artifact_paths = Vec::new();
    let mut used_filenames: HashSet<String> = HashSet::new();
    if opts.write_custody_sidecar {
        used_filenames.insert("custody_manifest.json".to_string());
    }

    for table in tables {
        let safe_tool_id = sanitize_filename_component(&table.schema.tool_id);
        for &format in formats {
            let mut counter = 1usize;
            let filename = loop {
                let stem = if counter == 1 {
                    safe_tool_id.clone()
                } else {
                    format!("{safe_tool_id}_{counter}")
                };
                let candidate = match format {
                    ExportFormat::HtmlVertical => format!("{stem}.vertical.html"),
                    _ => format!("{stem}.{}", format.extension()),
                };
                if used_filenames.insert(candidate.to_ascii_lowercase()) {
                    break candidate;
                }
                counter += 1;
            };
            let target = dir.join(filename);
            let bytes = export_table(table, format, &opts.export)?;
            fs::write(&target, &bytes)?;

            let leaf = CustodyLeaf {
                index: leaves.len(),
                path: target.display().to_string(),
                size_bytes: bytes.len() as u64,
                sha256: sha256_hex(&bytes),
                sha512: sha512_hex(&bytes),
            };
            leaves.push(leaf);
            artifact_paths.push(target);
        }
    }

    let custody = build_custody_manifest("winctl2rust", &opts.export.tool_version, leaves);
    let sidecar_path = if opts.write_custody_sidecar {
        let sidecar = dir.join("custody_manifest.json");
        let report = build_report_envelope(custody, artifact_paths, Some(sidecar.clone()), opts);
        write_report_sidecar(&report, &sidecar)?;
        return Ok(report);
    } else {
        None
    };

    Ok(build_report_envelope(custody, artifact_paths, sidecar_path, opts))
}

fn build_report_envelope(
    custody: CustodyManifest,
    artifact_paths: Vec<PathBuf>,
    sidecar_path: Option<PathBuf>,
    opts: &WinctlReportOptions,
) -> WinctlCustodyReport {
    WinctlCustodyReport {
        suite_name: "winctl2rust".to_string(),
        suite_version: opts.export.tool_version.clone(),
        operator: opts.operator.clone(),
        case_id: opts.case_id.clone(),
        host_name: opts.host_name.clone(),
        dry_run: opts.dry_run,
        custody,
        artifact_paths,
        sidecar_path,
    }
}

fn sidecar_path_for(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".custody.json");
    PathBuf::from(os)
}

fn write_report_sidecar(report: &WinctlCustodyReport, path: &Path) -> WinctlResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(report).map_err(|e| winctl2rust_core::WinctlError::Export(e.into()))?;
    fs::write(path, json)?;
    Ok(())
}

fn sanitize_filename_component(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = if cleaned.is_empty() {
        "table".to_string()
    } else {
        cleaned
    };
    if is_windows_reserved_stem(&base) {
        format!("table_{base}")
    } else {
        base
    }
}

fn is_windows_reserved_stem(stem: &str) -> bool {
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}
