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
        "\"backupFileName\":\"fixture-writenew1@v1\"",
        "\"backupFileName\":\"fixture-subagent1@v1\"",
        "\"name\":\"Edit\"",
        "\"name\":\"Write\"",
        "\"name\":\"Bash\"",
        "\"name\":\"NotebookEdit\"",
        "\"name\":\"mcp_fixture_codedb_edit\"",
        "mcp_marker",
        "new_string was NOT written",
        "toolu_taste_edit_revert",
        "toolu_taste_notebook_edit",
        "toolu_taste_edit_partial",
        "partial_drop",
        "human_edited_marker",
        "toolu_taste_edit_manual",
        "toolu_taste_edit_missing_final",
        "missing_final_marker",
        "toolu_taste_edit_untracked",
        "untracked_marker",
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
fn taste_fixture_corpus_has_write_sidecar_blob() {
    let sidecar = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-writenew1@v1"
    ));
    assert!(sidecar.exists(), "missing sidecar fixture-writenew1@v1");
    let body = fs::read_to_string(sidecar).expect("read write sidecar");
    assert!(
        body.contains("written by Write tool fixture"),
        "write sidecar should contain Write fixture marker"
    );
}

#[test]
fn taste_fixture_corpus_has_subagent_sidecar_blob() {
    let sidecar = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-subagent1@v1"
    ));
    assert!(sidecar.exists(), "missing sidecar fixture-subagent1@v1");
    let body = fs::read_to_string(sidecar).expect("read subagent sidecar");
    assert!(
        body.contains("subagent_marker"),
        "subagent sidecar should contain subagent edit marker"
    );
}

#[test]
fn taste_fixture_corpus_has_manual_edit_sidecar_blob() {
    let sidecar = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-manual1@v1"
    ));
    assert!(sidecar.exists(), "missing sidecar fixture-manual1@v1");
    let body = fs::read_to_string(sidecar).expect("read manual sidecar");
    assert!(
        body.contains("human_edited_marker"),
        "manual sidecar should contain human IDE edit marker"
    );
}

#[test]
fn taste_fixture_corpus_has_missing_final_sidecar_blob_only_for_intermediate_snapshot() {
    let intermediate = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-missingfinal1@v1"
    ));
    assert!(
        intermediate.exists(),
        "missing sidecar fixture-missingfinal1@v1"
    );
    let body = fs::read_to_string(intermediate).expect("read missing-final sidecar");
    assert!(
        body.contains("missing_final_marker"),
        "intermediate sidecar should contain missing-final marker"
    );

    let final_sidecar = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-missingfinal1@v2"
    ));
    assert!(
        !final_sidecar.exists(),
        "final sidecar fixture-missingfinal1@v2 should be absent to simulate R1 degradation"
    );
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
