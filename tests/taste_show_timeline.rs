mod common;

use common::taste_fixture;
use jottrace::taste::{
    TasteExtractOptions, TasteShowTimelineOptions, TimelineSourceKind, run_taste_extract,
    show_timeline_for_data_dir,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TASTE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000031";
const TASTE_SUBAGENT_ID: &str = "agent-taste000000000001";
const TASTE_TARGET: &str = "src/taste_target.rs";

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
fn taste_show_timeline_returns_fixture_rows_after_extract() {
    let root = common::temp_root("taste-show-timeline-lib");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract_with_home(&root, &data_dir);

    let report = show_timeline_for_data_dir(
        &data_dir,
        TasteShowTimelineOptions {
            source_session_id: TASTE_SESSION_ID.to_string(),
            file_path: TASTE_TARGET.to_string(),
        },
    )
    .expect("show timeline");

    assert_eq!(report.source_session_id, TASTE_SESSION_ID);
    assert_eq!(report.file_path, TASTE_TARGET);
    assert_eq!(report.rows.len(), 4);
    assert_eq!(
        report.rows[0].source_kind,
        TimelineSourceKind::InlineSnapshot
    );
    assert_eq!(
        report.rows[1].trigger_event_ref.as_deref(),
        Some("toolu_taste_edit_accept")
    );
    assert!(
        report.rows[3]
            .content
            .as_deref()
            .is_some_and(|content| content.contains("# appended by bash fixture"))
    );
}

#[test]
fn taste_show_timeline_cli_prints_fixture_snapshots() {
    let root = common::temp_root("taste-show-timeline-cli");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract_with_home(&root, &data_dir);

    let output = Command::new(binary())
        .args([
            "taste",
            "show",
            "timeline",
            "--session",
            TASTE_SESSION_ID,
            "--file",
            TASTE_TARGET,
        ])
        .env("HOME", root.as_ref())
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace taste show timeline");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rows: 4"));
    assert!(stdout.contains("trigger=toolu_taste_edit_accept"));
    assert!(stdout.contains("taste fixture baseline"));
    assert!(stdout.contains("# appended by bash fixture"));
}

#[test]
fn taste_show_timeline_errors_before_extract() {
    let root = common::temp_root("taste-show-timeline-missing");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let error = show_timeline_for_data_dir(
        &data_dir,
        TasteShowTimelineOptions {
            source_session_id: TASTE_SESSION_ID.to_string(),
            file_path: TASTE_TARGET.to_string(),
        },
    )
    .expect_err("timeline should be missing before extract");

    assert!(
        error.to_string().contains("timeline not found"),
        "unexpected error: {error}"
    );
}
