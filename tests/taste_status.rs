mod common;

use common::taste_fixture;
use jottrace::storage::{DB_FILE_NAME, open_database};
use jottrace::taste::{
    TasteEvidenceCounts, TasteExtractOptions, TasteOutcomeCounts, taste_extract_for_data_dir,
    taste_status_for_data_dir,
};
use rusqlite::params;
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

#[test]
fn taste_status_reports_fixture_coverage_after_extract() {
    let root = common::temp_root("taste-status-lib");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract(&root, &data_dir);

    let report = taste_status_for_data_dir(&data_dir).expect("taste status");
    assert_eq!(report.claude_parent_sessions, 1);
    assert_eq!(report.sessions_processed, 1);
    assert_eq!(report.sessions_pending, 0);
    assert_eq!(report.proposals, 10);
    assert_eq!(
        report.outcomes,
        TasteOutcomeCounts {
            accepted: 7,
            rejected: 2,
            edited: 1,
        }
    );
    assert_eq!(report.high_confidence_proposals, 7);
    assert!((report.coverage_percent - (7.0 / 10.0 * 100.0)).abs() < 1e-9);
    assert_eq!(
        report.evidence,
        TasteEvidenceCounts {
            direct_edit: 6,
            direct_write: 1,
            bash_correlation: 1,
            mcp_correlation: 1,
            permission_denial: 1,
            missing_final_state: 0,
        }
    );
}

#[test]
fn taste_status_cli_reports_fixture_counts() {
    let root = common::temp_root("taste-status-cli");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract(&root, &data_dir);

    let output = Command::new(binary())
        .args(["taste", "status"])
        .env("HOME", root.as_ref())
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace taste status");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sessions_processed: 1"));
    assert!(stdout.contains("proposals: 10"));
    assert!(stdout.contains("accepted: 7"));
    assert!(stdout.contains("rejected: 2"));
    assert!(stdout.contains("edited: 1"));
    assert!(stdout.contains("high_confidence_coverage: 70.0%"));
}

#[test]
fn taste_status_details_reports_evidence_breakdown() {
    let root = common::temp_root("taste-status-details");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract(&root, &data_dir);

    let output = Command::new(binary())
        .args(["taste", "status", "--details"])
        .env("HOME", root.as_ref())
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace taste status --details");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("evidence:"));
    assert!(stdout.contains("bash_correlation: 1"));
    assert!(stdout.contains("mcp_correlation: 1"));
    assert!(stdout.contains("low_confidence_proposals: 3"));
}

#[test]
fn taste_status_before_extract_shows_pending_session() {
    let root = common::temp_root("taste-status-pending");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let report = taste_status_for_data_dir(&data_dir).expect("taste status");
    assert_eq!(report.claude_parent_sessions, 1);
    assert_eq!(report.sessions_processed, 0);
    assert_eq!(report.sessions_pending, 1);
    assert_eq!(report.proposals, 0);
    assert_eq!(report.coverage_percent, 0.0);
}

#[test]
fn taste_status_counts_extracted_session_processed_when_preference_examples_cleared() {
    let root = common::temp_root("taste-status-zero-proposals");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract(&root, &data_dir);

    let conn = open_database(&data_dir.join(DB_FILE_NAME)).expect("open db");
    conn.execute(
        "DELETE FROM preference_examples WHERE source = 'claude_cli' AND source_session_id = ?1",
        params![TASTE_SESSION_ID],
    )
    .expect("clear preference examples");

    let report = taste_status_for_data_dir(&data_dir).expect("taste status");
    assert_eq!(report.sessions_processed, 1);
    assert_eq!(report.sessions_pending, 0);
    assert_eq!(report.proposals, 0);
}

#[test]
fn taste_status_marks_session_pending_when_merged_event_count_changes() {
    let root = common::temp_root("taste-status-stale-events");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract(&root, &data_dir);

    let conn = open_database(&data_dir.join(DB_FILE_NAME)).expect("open db");
    let session_db_id: i64 = conn
        .query_row(
            "SELECT id FROM sessions WHERE source = 'claude_cli' AND source_session_id = ?1",
            params![TASTE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("parent session id");
    conn.execute(
        "UPDATE sessions SET event_count = event_count + 1 WHERE id = ?1",
        params![session_db_id],
    )
    .expect("bump event_count");

    let report = taste_status_for_data_dir(&data_dir).expect("taste status");
    assert_eq!(report.sessions_processed, 0);
    assert_eq!(report.sessions_pending, 1);
}
