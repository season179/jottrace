use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn reader_fixture_corpus_has_issue_21_required_shapes() {
    for path in [
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021.jsonl",
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021/subagents/agent-a000000000000021.jsonl",
        "codex-cli/sessions/2026/05/05/rollout-2026-05-05T09-00-00-00000000-0000-4000-8000-000000000021.jsonl",
        "codex-cli/archived_sessions/rollout-2026-03-28T10-42-29-00000000-0000-4000-8000-000000000021.jsonl",
        "edge-cases/partial-tail.jsonl",
        "edge-cases/corrupt-line.jsonl",
        "edge-cases/truncation-before.jsonl",
        "edge-cases/truncation-after.jsonl",
        "edge-cases/same-size-rewrite-before.jsonl",
        "edge-cases/same-size-rewrite-after.jsonl",
    ] {
        assert!(fixture(path).exists(), "missing fixture {path}");
    }
}

#[test]
fn edge_case_fixtures_encode_jsonl_reader_state() {
    let partial = fs::read(fixture("edge-cases/partial-tail.jsonl")).expect("read partial tail");
    assert!(
        !partial.ends_with(b"\n"),
        "partial-tail fixture must end with an unterminated line"
    );

    let corrupt =
        fs::read_to_string(fixture("edge-cases/corrupt-line.jsonl")).expect("read corrupt fixture");
    assert!(
        corrupt
            .lines()
            .any(|line| line.contains("not valid json")),
        "corrupt-line fixture must contain an intentionally invalid JSONL line"
    );

    let truncated_before = fs::read(fixture("edge-cases/truncation-before.jsonl"))
        .expect("read truncation before fixture");
    let truncated_after =
        fs::read(fixture("edge-cases/truncation-after.jsonl")).expect("read truncation after fixture");
    assert!(
        truncated_after.len() < truncated_before.len(),
        "truncation-after must be smaller than truncation-before"
    );

    let rewrite_before = fs::read(fixture("edge-cases/same-size-rewrite-before.jsonl"))
        .expect("read rewrite before fixture");
    let rewrite_after = fs::read(fixture("edge-cases/same-size-rewrite-after.jsonl"))
        .expect("read rewrite after fixture");
    assert_eq!(
        rewrite_before.len(),
        rewrite_after.len(),
        "same-size rewrite fixtures must have equal byte lengths"
    );
    assert_ne!(
        rewrite_before, rewrite_after,
        "same-size rewrite fixtures must differ in content"
    );
}

#[test]
fn committed_reader_fixtures_do_not_contain_known_local_sensitive_markers() {
    let markers = [
        "/Users/season",
        "season179",
        ".codex/worktrees",
        ".claude/projects",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "ghp_",
        "sk-",
        "xoxb-",
    ];

    for path in fixture_files(&fixture("")) {
        let content = fs::read_to_string(&path).expect("fixture should be utf8");
        for marker in markers {
            assert!(
                !content.contains(marker),
                "{} contains sensitive marker {marker}",
                path.display()
            );
        }
    }
}

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/readers")
        .join(relative)
}

fn fixture_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).expect("read fixture directory") {
        let entry = entry.expect("read fixture entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}
