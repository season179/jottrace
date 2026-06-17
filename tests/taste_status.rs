mod common;

use common::taste_fixture;
use jottrace::taste::{
    TasteExtractOptions, TasteOutcomeCounts, run_taste_extract, taste_status_for_data_dir,
};
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

fn run_extract_with_home(home: &Path, data_dir: &Path) {
    // SAFETY: each test owns an isolated temp root and runs single-threaded.
    unsafe {
        std::env::set_var("HOME", home);
        std::env::set_var("JOTTRACE_HOME", data_dir);
    }
    run_taste_extract(TasteExtractOptions {
        source_session_id: Some(TASTE_SESSION_ID.to_string()),
        ..TasteExtractOptions::default()
    })
    .expect("run taste extract");
}

#[test]
fn taste_status_reports_fixture_coverage_after_extract() {
    let root = common::temp_root("taste-status-lib");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract_with_home(&root, &data_dir);

    let report = taste_status_for_data_dir(&data_dir).expect("taste status");
    assert_eq!(report.claude_parent_sessions, 1);
    assert_eq!(report.sessions_processed, 1);
    assert_eq!(report.sessions_pending, 0);
    assert_eq!(report.proposals, 8);
    assert_eq!(
        report.outcomes,
        TasteOutcomeCounts {
            accepted: 5,
            rejected: 3,
            edited: 0,
        }
    );
    assert_eq!(report.high_confidence_proposals, 4);
    assert!((report.coverage_percent - (4.0 / 8.0 * 100.0)).abs() < 1e-9);
}

#[test]
fn taste_status_cli_reports_fixture_counts() {
    let root = common::temp_root("taste-status-cli");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract_with_home(&root, &data_dir);

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
    assert!(stdout.contains("proposals: 8"));
    assert!(stdout.contains("accepted: 5"));
    assert!(stdout.contains("rejected: 3"));
    assert!(stdout.contains("high_confidence_coverage: 50.0%"));
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
