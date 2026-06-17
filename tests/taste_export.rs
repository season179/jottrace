mod common;

use common::taste_fixture;
use jottrace::taste::{
    PreferenceOutcome, TasteExportFormat, TasteExportOptions, TasteExtractOptions,
    taste_export_for_data_dir, taste_extract_for_data_dir,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TASTE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000031";
const TASTE_SUBAGENT_ID: &str = "agent-taste000000000001";

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_jottrace")
}

fn claude_project_dir(root: &Path) -> PathBuf {
    root.join(".claude/projects/-Users-fixture-Workspace-jottrace")
}

fn copy_fixture_file(relative: &str, dest: &Path) {
    let source = taste_fixture(relative);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::copy(&source, dest).expect("copy taste fixture");
}

fn install_taste_claude_fixture(root: &Path) {
    let project = claude_project_dir(root);
    copy_fixture_file(
        &format!("claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"),
        &project.join(format!("{TASTE_SESSION_ID}.jsonl")),
    );

    let subagents = project.join(TASTE_SESSION_ID).join("subagents");
    copy_fixture_file(
        &format!(
            "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}/subagents/{TASTE_SUBAGENT_ID}.jsonl"
        ),
        &subagents.join(format!("{TASTE_SUBAGENT_ID}.jsonl")),
    );
    copy_fixture_file(
        &format!(
            "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}/subagents/{TASTE_SUBAGENT_ID}.meta.json"
        ),
        &subagents.join(format!("{TASTE_SUBAGENT_ID}.meta.json")),
    );

    let history_dir = root.join(".claude/file-history").join(TASTE_SESSION_ID);
    for version in ["v1", "v2", "v3"] {
        copy_fixture_file(
            &format!("claude-cli/file-history/{TASTE_SESSION_ID}/fixture-a1b2c3d4@{version}"),
            &history_dir.join(format!("fixture-a1b2c3d4@{version}")),
        );
    }
    copy_fixture_file(
        &format!("claude-cli/file-history/{TASTE_SESSION_ID}/fixture-mcpb5e6f7a@v1"),
        &history_dir.join("fixture-mcpb5e6f7a@v1"),
    );
    copy_fixture_file(
        &format!("claude-cli/file-history/{TASTE_SESSION_ID}/fixture-writenew1@v1"),
        &history_dir.join("fixture-writenew1@v1"),
    );
    for version in ["v1", "v2"] {
        copy_fixture_file(
            &format!("claude-cli/file-history/{TASTE_SESSION_ID}/fixture-partial1@{version}"),
            &history_dir.join(format!("fixture-partial1@{version}")),
        );
    }
    copy_fixture_file(
        &format!("claude-cli/file-history/{TASTE_SESSION_ID}/fixture-subagent1@v1"),
        &history_dir.join("fixture-subagent1@v1"),
    );
}

fn run_ingest_with_home(home: &Path, data_dir: &Path) {
    let output = Command::new(binary())
        .arg("ingest")
        .env("HOME", home)
        .env("JOTTRACE_HOME", data_dir)
        .output()
        .expect("run jottrace ingest");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_extract(root: &Path, data_dir: &Path) {
    taste_extract_for_data_dir(
        data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            sidecar_history_root: Some(root.join(".claude/file-history")),
            ..TasteExtractOptions::default()
        },
    )
    .expect("run taste extract");
}

fn parse_jsonl(content: &str) -> Vec<Value> {
    content
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("valid jsonl row"))
        .collect()
}

#[test]
fn taste_export_writes_fixture_rows_to_file() {
    let root = common::temp_root("taste-export-lib");
    let data_dir = root.join(".jottrace");
    let out_path = root.join("preferences.jsonl");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract(&root, &data_dir);

    let report = taste_export_for_data_dir(
        &data_dir,
        TasteExportOptions {
            format: TasteExportFormat::Jsonl,
            output_path: Some(out_path.clone()),
        },
    )
    .expect("export preferences");

    assert_eq!(report.rows_exported, 10);
    let content = fs::read_to_string(&out_path).expect("read export file");
    let rows = parse_jsonl(&content);
    assert_eq!(rows.len(), 10);

    let rejected = rows
        .iter()
        .find(|row| row["tool_use_id"] == "toolu_taste_edit_reject")
        .expect("rejected row");
    assert_eq!(rejected["outcome"], "rejected");
    assert!(rejected["rejected"].is_string());
    assert!(rejected["chosen"].is_null());
    assert!(
        rejected["context"]
            .as_str()
            .is_some_and(|content| content.contains("taste fixture baseline"))
    );
    assert!(
        rejected["context"]
            .as_str()
            .is_some_and(|content| content.contains("--- prior events ---"))
    );

    let accepted = rows
        .iter()
        .find(|row| row["tool_use_id"] == "toolu_taste_edit_revert")
        .expect("accepted row");
    assert_eq!(accepted["outcome"], "accepted");
    assert!(accepted["chosen"].is_string());
    assert!(accepted["rejected"].is_null());
}

#[test]
fn taste_export_cli_writes_jsonl_to_stdout() {
    let root = common::temp_root("taste-export-cli");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract(&root, &data_dir);

    let output = Command::new(binary())
        .args(["taste", "export", "--format", "jsonl"])
        .env("HOME", root.as_ref())
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace taste export");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let rows = parse_jsonl(&stdout);
    assert_eq!(rows.len(), 10);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("rows_exported: 10"));

    let bash = rows
        .iter()
        .find(|row| row["tool_use_id"] == "toolu_taste_bash")
        .expect("bash row");
    assert_eq!(bash["outcome"], PreferenceOutcome::Accepted.as_str());
    assert_eq!(bash["evidence_kind"], "bash_correlation");
}

#[test]
fn taste_export_returns_empty_file_before_extract() {
    let root = common::temp_root("taste-export-empty");
    let data_dir = root.join(".jottrace");
    let out_path = root.join("preferences.jsonl");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let report = taste_export_for_data_dir(
        &data_dir,
        TasteExportOptions {
            format: TasteExportFormat::Jsonl,
            output_path: Some(out_path.clone()),
        },
    )
    .expect("export empty preferences");

    assert_eq!(report.rows_exported, 0);
    assert_eq!(fs::read_to_string(&out_path).expect("read export file"), "");
}
