mod common;

use common::reader_fixture as fixture;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};

const OPENCODE_PARENT_SESSION_ID: &str = "ses_fixture_parent_00000000000";
const OPENCODE_CHILD_SESSION_ID: &str = "ses_fixture_child_000000000000";

#[test]
fn reader_source_inventory_documents_fixture_gate() {
    let inventory = reader_source_inventory();

    for required in [
        "## Reader Fixture Gate",
        "Stable `source_session_id`",
        "deterministic `seq`",
        "Raw role/content/tool/reasoning/result payloads",
        "read-only change detection",
    ] {
        assert!(
            inventory.contains(required),
            "inventory note should document fixture gate requirement: {required}"
        );
    }
}

#[test]
fn architecture_links_reader_source_inventory_gate() {
    let design = fs::read_to_string("docs/design.md").expect("read architecture design doc");

    assert!(
        design.contains("docs/reader-source-inventory.md"),
        "architecture doc should link to the reader source inventory gate"
    );
}

#[test]
fn reader_source_inventory_documents_known_sources_in_inventory_table() {
    let inventory = reader_source_inventory();
    let table = inventory
        .split("## Known Source Inventory")
        .nth(1)
        .and_then(|tail| tail.split("## Deferred And Ignored Sources").next())
        .expect("reader source inventory note should contain a known source inventory section");

    for source in [
        "Claude CLI / Claude Code",
        "Claude Desktop / local agent mode",
        "Codex CLI",
        "Hermes native SessionDB",
        "Pi agent",
        "Factory / Droid-style agent sessions",
        "OpenCode",
        "Gemini CLI",
    ] {
        assert!(
            table.contains(source),
            "known source inventory table should document source: {source}"
        );
    }
}

#[test]
fn reader_source_inventory_documents_claude_local_agent_reader_shape() {
    let inventory = reader_source_inventory();
    let table = inventory
        .split("## Known Source Inventory")
        .nth(1)
        .and_then(|tail| tail.split("## Deferred And Ignored Sources").next())
        .expect("reader source inventory note should contain a known source inventory section");
    let row = table
        .lines()
        .find(|line| line.contains("Claude Desktop / local agent mode"))
        .expect("inventory table should document Claude Desktop local-agent mode");

    for required in [
        "source=`claude_local_agent`",
        "`local_*.json` sidecar metadata",
        "`audit.jsonl`",
        "`session_id`",
        "Browser session storage remains excluded",
    ] {
        assert!(
            row.contains(required),
            "local-agent inventory row should document {required}"
        );
    }
}

#[test]
fn reader_source_inventory_documents_deferred_and_privacy_boundaries() {
    let inventory = reader_source_inventory();

    for required in [
        "## Deferred And Ignored Sources",
        "Browser and Electron `Session Storage`",
        "app caches",
        "opaque state",
        "Thin command histories",
        "## Fixture Requirements And Privacy",
        "human review",
        "No reader issue should commit raw private transcripts",
    ] {
        assert!(
            inventory.contains(required),
            "inventory note should document source boundary or privacy requirement: {required}"
        );
    }
}

#[test]
fn reader_source_inventory_names_issue_69_ignored_and_deferred_sources() {
    let inventory = reader_source_inventory();

    for required in [
        "Aider home startup history",
        "Electron browser session storage",
        "skills-manager app state",
        "sidecar/cache outputs",
        "Windsurf",
        "VS Code/Copilot/ChatGPT extension state",
        "Antigravity app/protobuf/browser state",
        "fixture proof required to reconsider",
    ] {
        assert!(
            inventory.contains(required),
            "inventory note should explicitly name issue #69 source boundary: {required}"
        );
    }
}

#[test]
fn reader_source_inventory_documents_pi_agent_fixture_findings() {
    let inventory = reader_source_inventory();

    for required in [
        "Pi agent (#64 fixture-confirmed)",
        "`~/.pi/agent/sessions/<encoded-cwd>/<timestamp>_<session-id>.jsonl`",
        "`session.id`",
        "JSONL line number",
        "`parentId`",
        "file size, mtime, fingerprint, and next read offset",
    ] {
        assert!(
            inventory.contains(required),
            "inventory note should document Pi agent fixture finding: {required}"
        );
    }
}

fn reader_source_inventory() -> String {
    fs::read_to_string("docs/reader-source-inventory.md")
        .expect("reader source inventory design note should exist")
}

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
fn reader_fixture_corpus_has_sanitized_opencode_sqlite_shape() {
    let fixture = fs::read_to_string(fixture("opencode/sqlite/opencode.sql"))
        .expect("read OpenCode SQLite fixture");
    let conn = Connection::open_in_memory().expect("open fixture db");

    conn.execute_batch(&fixture)
        .expect("OpenCode fixture SQL should load");

    let parent_id: String = conn
        .query_row(
            "SELECT parent_id FROM session WHERE id = ?1",
            [OPENCODE_CHILD_SESSION_ID],
            |row| row.get(0),
        )
        .expect("child session should have parent link");
    assert_eq!(parent_id, OPENCODE_PARENT_SESSION_ID);

    for (label, sql, expected) in [
        (
            "child session messages",
            "SELECT count(*) FROM message WHERE session_id = ?1",
            2,
        ),
        (
            "child session parts",
            "SELECT count(*) FROM part WHERE session_id = ?1",
            2,
        ),
    ] {
        let count: i64 = conn
            .query_row(sql, [OPENCODE_CHILD_SESSION_ID], |row| row.get(0))
            .unwrap_or_else(|_| panic!("count {label}"));
        assert_eq!(count, expected, "unexpected count for {label}");
    }
}

#[test]
fn reader_fixture_readme_documents_opencode_fixture_status() {
    let readme = fs::read_to_string(fixture("README.md")).expect("read reader fixture README");

    for required in [
        "opencode/sqlite/opencode.sql",
        "OpenCode SQLite",
        "session, message, part, and parent-child",
        "Cursor fixture capture is still pending",
    ] {
        assert!(
            readme.contains(required),
            "fixture README should document OpenCode/Cursor status: {required}"
        );
    }
}

#[test]
fn reader_fixture_corpus_has_issue_64_pi_agent_required_shapes() {
    let fixture_path = "pi-agent/sessions/--Users-fixture-Workspace-jottrace--/2026-05-06T02-00-00-000Z_00000000-0000-4000-8000-000000000064.jsonl";
    let content = fs::read_to_string(fixture(fixture_path)).expect("read Pi agent fixture");
    let readme = fs::read_to_string(fixture("README.md")).expect("read fixture README");

    for required in [
        "\"type\":\"session\"",
        "\"type\":\"message\"",
        "\"type\":\"model_change\"",
        "\"type\":\"thinking_level_change\"",
        "\"parentId\"",
        "\"timestamp\"",
    ] {
        assert!(
            content.contains(required),
            "Pi agent fixture should contain required source shape: {required}"
        );
    }
    assert!(
        readme.contains("pi-agent/sessions"),
        "fixture README should document Pi agent fixture coverage"
    );
}

#[test]
fn reader_fixture_corpus_has_issue_68_claude_local_agent_shapes() {
    let metadata = fixture(
        "claude-local-agent/local-agent-mode-sessions/desktop-fixture/workspace-fixture/local_00000000-0000-4000-8000-000000000068.json",
    );
    let audit = fixture(
        "claude-local-agent/local-agent-mode-sessions/desktop-fixture/workspace-fixture/local_00000000-0000-4000-8000-000000000068/audit.jsonl",
    );

    assert!(metadata.exists(), "missing local-agent metadata fixture");
    assert!(audit.exists(), "missing local-agent audit fixture");

    let audit = fs::read_to_string(audit).expect("read local-agent audit fixture");
    for event_type in [
        r#""type":"user""#,
        r#""type":"assistant""#,
        r#""type":"system""#,
        r#""type":"result""#,
        r#""type":"tool-summary""#,
        r#""type":"rate-limit""#,
    ] {
        assert!(
            audit.contains(event_type),
            "local-agent audit fixture should include {event_type}"
        );
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
        corrupt.lines().any(|line| line.contains("not valid json")),
        "corrupt-line fixture must contain an intentionally invalid JSONL line"
    );

    let truncated_before = fs::read(fixture("edge-cases/truncation-before.jsonl"))
        .expect("read truncation before fixture");
    let truncated_after = fs::read(fixture("edge-cases/truncation-after.jsonl"))
        .expect("read truncation after fixture");
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
