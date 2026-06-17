mod common;

use common::taste_fixture;
use jottrace::storage::{open_database, DB_FILE_NAME};
use jottrace::taste::{PreferenceOutcome, TasteExtractOptions, run_taste_extract};
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

    let history_dir = root
        .join(".claude/file-history")
        .join(TASTE_SESSION_ID);
    for version in ["v1", "v2", "v3"] {
        copy_fixture_file(
            &format!("claude-cli/file-history/{TASTE_SESSION_ID}/fixture-a1b2c3d4@{version}"),
            &history_dir.join(format!("fixture-a1b2c3d4@{version}")),
        );
    }
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

fn run_extract_with_home(home: &Path, data_dir: &Path, options: TasteExtractOptions) -> String {
    // SAFETY: each test owns an isolated temp root and runs single-threaded.
    unsafe {
        std::env::set_var("HOME", home);
        std::env::set_var("JOTTRACE_HOME", data_dir);
    }
    let report = run_taste_extract(options).expect("run taste extract");
    format!(
        "sessions_processed={} sessions_skipped={} timeline_rows={} preference_examples={}",
        report.sessions_processed,
        report.sessions_skipped,
        report.timeline_rows_written,
        report.preference_examples_written
    )
}

#[test]
fn taste_extract_materializes_fixture_session_end_to_end() {
    let root = common::temp_root("taste-extract-e2e");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let summary = run_extract_with_home(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            ..TasteExtractOptions::default()
        },
    );
    assert!(summary.contains("sessions_processed=1"));
    assert!(summary.contains("preference_examples=6"));

    let conn = open_database(&data_dir.join(DB_FILE_NAME)).expect("open db");
    let timeline_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_timelines WHERE source = 'claude_cli' AND source_session_id = ?1",
            params![TASTE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("count timelines");
    assert_eq!(timeline_count, 4);

    let example_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM preference_examples WHERE source = 'claude_cli' AND source_session_id = ?1",
            params![TASTE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("count examples");
    assert_eq!(example_count, 6);

    let reverted_outcome: String = conn
        .query_row(
            "SELECT outcome FROM preference_examples
             WHERE source = 'claude_cli' AND source_session_id = ?1 AND tool_use_id = ?2",
            params![TASTE_SESSION_ID, "toolu_taste_edit_accept"],
            |row| row.get(0),
        )
        .expect("reverted accept row");
    assert_eq!(reverted_outcome, PreferenceOutcome::Rejected.as_str());
}

#[test]
fn taste_extract_skips_sessions_already_at_current_extractor_version() {
    let root = common::temp_root("taste-extract-skip");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let first = run_extract_with_home(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            ..TasteExtractOptions::default()
        },
    );
    assert!(first.contains("sessions_processed=1"));
    assert!(first.contains("sessions_skipped=0"));

    let second = run_extract_with_home(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            ..TasteExtractOptions::default()
        },
    );
    assert!(second.contains("sessions_processed=0"));
    assert!(second.contains("sessions_skipped=1"));
}

#[test]
fn taste_extract_cli_reports_counts_for_fixture_session() {
    let root = common::temp_root("taste-extract-cli");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let output = Command::new(binary())
        .args([
            "taste",
            "extract",
            "--session",
            TASTE_SESSION_ID,
        ])
        .env("HOME", root.as_ref())
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace taste extract");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sessions_processed: 1"));
    assert!(stdout.contains("preference_examples: 6"));
}
