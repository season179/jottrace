mod common;

use common::taste_fixture;
use jottrace::storage::{open_database, LATEST_SCHEMA_VERSION};
use jottrace::taste::{
    FileTimelineMaterializer, PreferenceCompiler, PreferenceOutcome, SourceStream,
    SnapshotSidecarResolver, merge_streams, parse_jsonl, replace_session_preference_examples,
};
use rusqlite::params;
use std::fs;

const TASTE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000031";
const TASTE_CWD: &str = "/Users/fixture/Workspace/jottrace";

fn read_jsonl_lines(path: &std::path::Path) -> Vec<Vec<u8>> {
    fs::read_to_string(path)
        .expect("read fixture")
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.as_bytes().to_vec())
        .collect()
}

fn fixture_resolver() -> SnapshotSidecarResolver {
    SnapshotSidecarResolver::with_history_root(taste_fixture("claude-cli/file-history"))
}

fn merged_fixture_events() -> Vec<jottrace::taste::ParsedEvent> {
    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let subagent = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}/subagents/agent-taste000000000001.jsonl"
    ));

    merge_streams(vec![
        (
            SourceStream::Parent,
            parse_jsonl(&SourceStream::Parent, read_jsonl_lines(&session)).expect("parent"),
        ),
        (
            SourceStream::Subagent {
                agent_id: "agent-taste000000000001".to_string(),
            },
            parse_jsonl(
                &SourceStream::Subagent {
                    agent_id: "agent-taste000000000001".to_string(),
                },
                read_jsonl_lines(&subagent),
            )
            .expect("subagent"),
        ),
    ])
}

fn compile_fixture_examples() -> Vec<jottrace::taste::PreferenceExample> {
    let events = merged_fixture_events();
    let rows = FileTimelineMaterializer::materialize(
        "claude_cli",
        TASTE_SESSION_ID,
        Some(TASTE_CWD),
        &fixture_resolver(),
        &events,
    )
    .expect("materialize");
    PreferenceCompiler::compile("claude_cli", TASTE_SESSION_ID, Some(TASTE_CWD), &events, &rows)
}

#[test]
fn replace_session_preference_examples_persists_compiled_rows() {
    let root = common::temp_root("taste-preference-examples-db");
    let db_path = root.join(jottrace::storage::DB_FILE_NAME);
    let conn = open_database(&db_path).expect("open database");
    assert_eq!(LATEST_SCHEMA_VERSION, 11);

    let examples = compile_fixture_examples();
    assert_eq!(examples.len(), 6);

    let inserted = replace_session_preference_examples(
        &db_path,
        &conn,
        "claude_cli",
        TASTE_SESSION_ID,
        &examples,
    )
    .expect("persist examples");
    assert_eq!(inserted, examples.len());

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM preference_examples WHERE source = ?1 AND source_session_id = ?2",
            params!["claude_cli", TASTE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("count rows");
    assert_eq!(count as usize, examples.len());

    let (outcome, evidence_kind, confidence): (String, String, f64) = conn
        .query_row(
            "SELECT outcome, evidence_kind, confidence FROM preference_examples
             WHERE source = ?1 AND source_session_id = ?2 AND tool_use_id = ?3",
            params!["claude_cli", TASTE_SESSION_ID, "toolu_taste_edit_reject"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("rejected edit row");
    assert_eq!(outcome, PreferenceOutcome::Rejected.as_str());
    assert_eq!(evidence_kind, "permission_denial");
    assert_eq!(confidence, 1.0);

    let reverted_outcome: String = conn
        .query_row(
            "SELECT outcome FROM preference_examples
             WHERE source = ?1 AND source_session_id = ?2 AND tool_use_id = ?3",
            params!["claude_cli", TASTE_SESSION_ID, "toolu_taste_edit_accept"],
            |row| row.get(0),
        )
        .expect("reverted accept row");
    assert_eq!(reverted_outcome, PreferenceOutcome::Rejected.as_str());
}
