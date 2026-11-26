use std::fs;

use guap_export::{ExportFormat, compute_merkle_root_sha256, compute_merkle_root_sha512};
use guap_mcp::ActionRiskLevel;
use guap_table::{CellValue, ColumnDef, ColumnType, Row, TableData, TableSchema};
use serde_json::json;
use tempfile::tempdir;
use winctl2rust_report::{
    WinctlCustodyReport, WinctlReportOptions, append_audit_jsonl, record_action_audit, render_table, write_report,
    write_report_bundle,
};

fn sample_table() -> TableData {
    let schema = TableSchema::new(
        "cmd_nircmd_preview",
        "NirCmd Script Preview",
        vec![
            ColumnDef::new("line", "Line", ColumnType::Uint),
            ColumnDef::new("verb", "Verb", ColumnType::Text),
            ColumnDef::new("target", "Target", ColumnType::Text),
        ],
    );
    let rows = vec![
        Row::new(
            1,
            vec![
                CellValue::Uint(1),
                CellValue::Text("speak".to_string()),
                CellValue::Text("=CMD|' /C calc'!A0".to_string()),
            ],
        ),
        Row::new(
            2,
            vec![
                CellValue::Uint(2),
                CellValue::Text("beep".to_string()),
                CellValue::Text("880 150".to_string()),
            ],
        ),
    ];
    TableData {
        schema,
        rows,
        total_rows: 2,
    }
}

#[test]
fn render_and_write_report_across_all_7_formats_with_csv_neutralization() {
    let table = sample_table();
    let dir = tempdir().unwrap();
    let opts = WinctlReportOptions {
        operator: Some("analyst".to_string()),
        case_id: Some("CASE-42".to_string()),
        host_name: Some("WIN-WORKSTATION".to_string()),
        dry_run: true,
        ..WinctlReportOptions::default()
    };

    let all_formats = [
        ExportFormat::Csv,
        ExportFormat::Tsv,
        ExportFormat::Json,
        ExportFormat::Jsonl,
        ExportFormat::Html,
        ExportFormat::HtmlVertical,
        ExportFormat::Markdown,
    ];

    for fmt in all_formats {
        let bytes = render_table(&table, fmt, &opts).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.is_empty(), "empty export for {fmt}");

        if fmt == ExportFormat::Csv {
            assert!(text.contains("'=CMD|"), "CSV formula neutralization missing in: {text}");
        }

        let filename = match fmt {
            ExportFormat::HtmlVertical => "report.vertical.html".to_string(),
            _ => format!("report.{}", fmt.extension()),
        };
        let out_path = dir.path().join(filename);
        let custody_report = write_report(&table, &out_path, fmt, &opts).unwrap();
        assert_eq!(custody_report.custody.artifact_count, 1);
        assert_eq!(custody_report.custody.leaves.len(), 1);
        assert_eq!(custody_report.operator.as_deref(), Some("analyst"));
        assert!(custody_report.dry_run);

        let sidecar = custody_report.sidecar_path.as_ref().unwrap();
        assert!(sidecar.exists());
        let parsed: WinctlCustodyReport = serde_json::from_slice(&fs::read(sidecar).unwrap()).unwrap();
        assert_eq!(
            parsed.custody.merkle_root_sha256,
            custody_report.custody.merkle_root_sha256
        );
        assert_eq!(
            parsed.custody.merkle_root_sha512,
            custody_report.custody.merkle_root_sha512
        );
    }
}

#[test]
fn write_report_bundle_computes_merkle_roots_for_all_artifacts() {
    let table = sample_table();
    let dir = tempdir().unwrap();
    let opts = WinctlReportOptions::default();

    let all_formats = [
        ExportFormat::Csv,
        ExportFormat::Tsv,
        ExportFormat::Json,
        ExportFormat::Jsonl,
        ExportFormat::Html,
        ExportFormat::HtmlVertical,
        ExportFormat::Markdown,
    ];

    let report = write_report_bundle(&[&table], dir.path(), &all_formats, &opts).unwrap();
    assert_eq!(report.custody.artifact_count, 7);
    assert_eq!(report.artifact_paths.len(), 7);
    for path in &report.artifact_paths {
        assert!(path.exists(), "missing bundle artifact {}", path.display());
    }

    let sha256_leaves: Vec<String> = report.custody.leaves.iter().map(|l| l.sha256.clone()).collect();
    let sha512_leaves: Vec<String> = report.custody.leaves.iter().map(|l| l.sha512.clone()).collect();
    assert_eq!(
        report.custody.merkle_root_sha256,
        compute_merkle_root_sha256(&sha256_leaves)
    );
    assert_eq!(
        report.custody.merkle_root_sha512,
        compute_merkle_root_sha512(&sha512_leaves)
    );
    assert!(report.sidecar_path.as_ref().unwrap().exists());
}

#[test]
fn audit_jsonl_appends_multiple_records() {
    let dir = tempdir().unwrap();
    let audit_path = dir.path().join("logs").join("audit.jsonl");

    let rec1 = record_action_audit(
        "cmd_nircmd",
        ActionRiskLevel::Mutating,
        true,
        true,
        "dry-run preview",
        &json!({ "script": "beep 440 100" }),
    );
    let rec2 = record_action_audit(
        "cmd_from_proc",
        ActionRiskLevel::PrivilegedMutating,
        false,
        false,
        "policy denied",
        &json!({ "parent_pid": 4 }),
    );

    append_audit_jsonl(&audit_path, &rec1).unwrap();
    append_audit_jsonl(&audit_path, &rec2).unwrap();

    let content = fs::read_to_string(&audit_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);

    let decoded_1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let decoded_2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(decoded_1["command_id"], "cmd_nircmd");
    assert_eq!(decoded_1["dry_run"], true);
    assert_eq!(decoded_1["payload_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(decoded_2["command_id"], "cmd_from_proc");
    assert_eq!(decoded_2["risk_level"], "PrivilegedMutating");
}

#[test]
fn write_report_bundle_avoids_filename_collisions_and_reserved_stems() {
    let make_table = |tool_id: &str, val: &str| TableData {
        schema: TableSchema::new(
            tool_id,
            "Collision Test",
            vec![ColumnDef::new("col", "Col", ColumnType::Text)],
        ),
        rows: vec![Row::new(1, vec![CellValue::Text(val.to_string())])],
        total_rows: 1,
    };

    let t1 = make_table("a/b", "first");
    let t2 = make_table("a:b", "second");
    let t3 = make_table("custody_manifest", "third");
    let t4 = make_table("CON", "reserved_con");
    let t5 = make_table("nul", "reserved_nul");

    let dir = tempdir().unwrap();
    let opts = WinctlReportOptions::default();
    let report = write_report_bundle(&[&t1, &t2, &t3, &t4, &t5], dir.path(), &[ExportFormat::Json], &opts).unwrap();

    assert_eq!(report.artifact_paths.len(), 5);
    assert_eq!(report.custody.artifact_count, 5);
    let sidecar = report.sidecar_path.as_ref().unwrap();
    assert_eq!(sidecar.file_name().unwrap().to_str().unwrap(), "custody_manifest.json");
    assert!(sidecar.exists());

    let names: Vec<String> = report
        .artifact_paths
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec![
            "a_b.json",
            "a_b_2.json",
            "custody_manifest_2.json",
            "table_CON.json",
            "table_nul.json",
        ]
    );

    let first_body = fs::read_to_string(&report.artifact_paths[0]).unwrap();
    let second_body = fs::read_to_string(&report.artifact_paths[1]).unwrap();
    let third_body = fs::read_to_string(&report.artifact_paths[2]).unwrap();
    assert!(first_body.contains("first"));
    assert!(second_body.contains("second"));
    assert!(third_body.contains("third"));

    let sidecar_parsed: WinctlCustodyReport = serde_json::from_slice(&fs::read(sidecar).unwrap()).unwrap();
    assert_eq!(sidecar_parsed.custody.artifact_count, 5);
}
