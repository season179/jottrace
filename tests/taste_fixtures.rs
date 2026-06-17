mod common;

use common::taste_fixture;
use std::fs;

const TASTE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000031";

#[test]
fn taste_fixture_corpus_has_required_session_shapes() {
    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let content = fs::read_to_string(&session).expect("read taste session fixture");

    for required in [
        "\"type\":\"file-history-snapshot\"",
        "\"trackedFileBackups\":[{\"filePath\"",
        "\"content\":",
        "\"backupFileName\":\"fixture-a1b2c3d4@v1\"",
        "\"name\":\"Edit\"",
        "\"name\":\"Write\"",
        "\"name\":\"Bash\"",
        "\"name\":\"NotebookEdit\"",
        "new_string was NOT written",
        "toolu_taste_edit_revert",
        "toolu_taste_notebook_edit",
        "notebook_marker",
    ] {
        assert!(
            content.contains(required),
            "taste session fixture should contain {required}"
        );
    }
}

#[test]
fn taste_fixture_corpus_has_snapshot_sidecar_blobs() {
    for version in ["v1", "v2", "v3"] {
        let sidecar = taste_fixture(&format!(
            "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-a1b2c3d4@{version}"
        ));
        assert!(
            sidecar.exists(),
            "missing sidecar fixture-a1b2c3d4@{version}"
        );
        let body = fs::read_to_string(sidecar).expect("read sidecar");
        assert!(
            body.contains("taste fixture baseline"),
            "sidecar @{version} should contain baseline marker"
        );
    }
}

#[test]
fn taste_fixture_corpus_has_subagent_edit_sidechain() {
    let subagent = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}/subagents/agent-taste000000000001.jsonl"
    ));
    let content = fs::read_to_string(subagent).expect("read taste subagent fixture");

    for required in [
        "\"isSidechain\":true",
        "\"name\":\"Edit\"",
        "taste_subagent.rs",
        "subagent_marker",
    ] {
        assert!(
            content.contains(required),
            "taste subagent fixture should contain {required}"
        );
    }
}

#[test]
fn taste_fixture_readme_documents_coverage() {
    let readme = fs::read_to_string(taste_fixture("README.md")).expect("read taste README");

    for required in [
        TASTE_SESSION_ID,
        "backupFileName",
        "permission denial",
        "subagent",
        "file-history",
    ] {
        assert!(
            readme.contains(required),
            "taste README should document {required}"
        );
    }
}

#[test]
fn committed_taste_fixtures_do_not_contain_known_local_sensitive_markers() {
    let markers = [
        "/Users/season",
        "season179",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "ghp_",
        "sk-",
    ];

    for entry in fs::read_dir(taste_fixture("")).expect("read taste fixture root") {
        let path = entry.expect("fixture entry").path();
        if path.is_dir() {
            check_taste_tree(&path, &markers);
        } else {
            check_taste_file(&path, &markers);
        }
    }
}

fn check_taste_tree(path: &std::path::Path, markers: &[&str]) {
    for entry in fs::read_dir(path).expect("read fixture directory") {
        let path = entry.expect("fixture entry").path();
        if path.is_dir() {
            check_taste_tree(&path, markers);
        } else {
            check_taste_file(&path, markers);
        }
    }
}

fn check_taste_file(path: &std::path::Path, markers: &[&str]) {
    let content = fs::read_to_string(path).expect("fixture should be utf8");
    for marker in markers {
        assert!(
            !content.contains(marker),
            "{} contains sensitive marker {marker}",
            path.display()
        );
    }
}
