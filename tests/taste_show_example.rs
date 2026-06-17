mod common;

use common::taste_fixture;
use jottrace::taste::{
    EvidenceKind, PreferenceOutcome, TasteExtractOptions, TasteShowExampleOptions,
    show_example_for_data_dir, taste_extract_for_data_dir,
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
fn taste_show_example_returns_fixture_row_after_extract() {
    let root = common::temp_root("taste-show-example-lib");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract(&root, &data_dir);

    let report = show_example_for_data_dir(
        &data_dir,
        TasteShowExampleOptions {
            tool_use_id: "toolu_taste_edit_reject".to_string(),
            source_session_id: None,
        },
    )
    .expect("show example");

    assert_eq!(report.example.tool_use_id, "toolu_taste_edit_reject");
    assert_eq!(report.example.source_session_id, TASTE_SESSION_ID);
    assert_eq!(report.example.file_path.as_deref(), Some(TASTE_TARGET));
    assert_eq!(report.example.outcome, PreferenceOutcome::Rejected);
    assert_eq!(report.example.evidence_kind, EvidenceKind::PermissionDenial);
    assert_eq!(report.example.confidence, 1.0);
    assert!(
        report
            .example
            .context
            .as_deref()
            .is_some_and(|content| content.contains("taste fixture baseline"))
    );
}

#[test]
fn taste_show_example_cli_prints_fixture_context() {
    let root = common::temp_root("taste-show-example-cli");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);
    run_extract(&root, &data_dir);

    let output = Command::new(binary())
        .args([
            "taste",
            "show",
            "example",
            "--session",
            TASTE_SESSION_ID,
            "toolu_taste_edit_reject",
        ])
        .env("HOME", root.as_ref())
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace taste show example");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tool_use_id: toolu_taste_edit_reject"));
    assert!(stdout.contains("outcome: rejected"));
    assert!(stdout.contains("evidence_kind: permission_denial"));
    assert!(stdout.contains("taste fixture baseline"));
}

#[test]
fn taste_show_example_errors_before_extract() {
    let root = common::temp_root("taste-show-example-missing");
    let data_dir = root.join(".jottrace");
    install_taste_claude_fixture(&root);
    run_ingest_with_home(&root, &data_dir);

    let error = show_example_for_data_dir(
        &data_dir,
        TasteShowExampleOptions {
            tool_use_id: "toolu_taste_edit_reject".to_string(),
            source_session_id: None,
        },
    )
    .expect_err("example should be missing before extract");

    assert!(
        error.to_string().contains("preference example not found"),
        "unexpected error: {error}"
    );
}
