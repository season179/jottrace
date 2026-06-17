mod common;

use common::taste_fixture;
use jottrace::storage::{DB_FILE_NAME, open_database};
use jottrace::taste::{PreferenceOutcome, TasteExtractOptions, taste_extract_for_data_dir};
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
    for version in ["v1", "v2"] {
        copy_fixture_file(
            &format!("claude-cli/file-history/{TASTE_SESSION_ID}/fixture-manual1@{version}"),
            &history_dir.join(format!("fixture-manual1@{version}")),
        );
    }
    copy_fixture_file(
        &format!("claude-cli/file-history/{TASTE_SESSION_ID}/fixture-missingfinal1@v1"),
        &history_dir.join("fixture-missingfinal1@v1"),
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

fn run_extract(root: &Path, data_dir: &Path, options: TasteExtractOptions) -> String {
    let report = taste_extract_for_data_dir(
        data_dir,
        TasteExtractOptions {
            sidecar_history_root: Some(root.join(".claude/file-history")),
            ..options
        },
    )
    .expect("run taste extract");
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

    let summary = run_extract(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            ..TasteExtractOptions::default()
        },
    );
    assert!(summary.contains("sessions_processed=1"));
    assert!(summary.contains("preference_examples=13"));

    let conn = open_database(&data_dir.join(DB_FILE_NAME)).expect("open db");
    let timeline_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_timelines WHERE source = 'claude_cli' AND source_session_id = ?1",
            params![TASTE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("count timelines");
    assert_eq!(timeline_count, 16);

    let example_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM preference_examples WHERE source = 'claude_cli' AND source_session_id = ?1",
            params![TASTE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("count examples");
    assert_eq!(example_count, 13);

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

    let first = run_extract(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            ..TasteExtractOptions::default()
        },
    );
    assert!(first.contains("sessions_processed=1"));
    assert!(first.contains("sessions_skipped=0"));

    let second = run_extract(
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
fn taste_extract_reprocesses_session_when_merged_event_count_changes() {
    let root = common::temp_root("taste-extract-event-change");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let first = run_extract(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            ..TasteExtractOptions::default()
        },
    );
    assert!(first.contains("sessions_processed=1"));

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

    let second = run_extract(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            ..TasteExtractOptions::default()
        },
    );
    assert!(
        second.contains("sessions_processed=1"),
        "expected re-extract after event count change, got: {second}"
    );
    assert!(second.contains("sessions_skipped=0"));
}

#[test]
fn taste_extract_skips_session_with_taste_extractions_row_and_no_preference_examples() {
    let root = common::temp_root("taste-extract-zero-proposals");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    run_extract(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            sidecar_history_root: Some(root.join(".claude/file-history")),
            ..TasteExtractOptions::default()
        },
    );

    let conn = open_database(&data_dir.join(DB_FILE_NAME)).expect("open db");
    conn.execute(
        "DELETE FROM preference_examples WHERE source = 'claude_cli' AND source_session_id = ?1",
        params![TASTE_SESSION_ID],
    )
    .expect("clear preference examples");

    let second = run_extract(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            sidecar_history_root: Some(root.join(".claude/file-history")),
            ..TasteExtractOptions::default()
        },
    );
    assert!(
        second.contains("sessions_skipped=1"),
        "expected skip when taste_extractions exists without preference rows, got: {second}"
    );
    assert!(second.contains("sessions_processed=0"));
}

#[test]
fn taste_extract_force_reprocesses_up_to_date_session() {
    let root = common::temp_root("taste-extract-force");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let first = run_extract(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            ..TasteExtractOptions::default()
        },
    );
    assert!(first.contains("sessions_processed=1"));

    let skipped = run_extract(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            ..TasteExtractOptions::default()
        },
    );
    assert!(skipped.contains("sessions_skipped=1"));

    let forced = run_extract(
        &root,
        &data_dir,
        TasteExtractOptions {
            source_session_id: Some(TASTE_SESSION_ID.to_string()),
            force: true,
            ..TasteExtractOptions::default()
        },
    );
    assert!(
        forced.contains("sessions_processed=1"),
        "expected --force to reprocess up-to-date session, got: {forced}"
    );
    assert!(forced.contains("sessions_skipped=0"));
}

#[test]
fn taste_extract_cli_reports_counts_for_fixture_session() {
    let root = common::temp_root("taste-extract-cli");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let output = Command::new(binary())
        .args(["taste", "extract", "--session", TASTE_SESSION_ID])
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
    assert!(stdout.contains("preference_examples: 13"));
}

#[test]
fn taste_extract_cli_force_reprocesses_up_to_date_session() {
    let root = common::temp_root("taste-extract-cli-force");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let first = Command::new(binary())
        .args(["taste", "extract", "--session", TASTE_SESSION_ID])
        .env("HOME", root.as_ref())
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace taste extract");
    assert!(first.status.success());

    let skipped = Command::new(binary())
        .args(["taste", "extract", "--session", TASTE_SESSION_ID])
        .env("HOME", root.as_ref())
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace taste extract again");
    assert!(skipped.status.success());
    let skipped_stdout = String::from_utf8_lossy(&skipped.stdout);
    assert!(skipped_stdout.contains("sessions_skipped: 1"));

    let forced = Command::new(binary())
        .args(["taste", "extract", "--session", TASTE_SESSION_ID, "--force"])
        .env("HOME", root.as_ref())
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace taste extract --force");
    assert!(
        forced.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&forced.stdout),
        String::from_utf8_lossy(&forced.stderr)
    );
    let forced_stdout = String::from_utf8_lossy(&forced.stdout);
    assert!(
        forced_stdout.contains("sessions_processed: 1"),
        "expected --force to reprocess, got:\n{forced_stdout}"
    );
    assert!(forced_stdout.contains("sessions_skipped: 0"));
}
