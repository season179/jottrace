mod common;

use common::taste_fixture;
use jottrace::storage::{LATEST_SCHEMA_VERSION, open_database};
use jottrace::taste::{
    FileTimelineMaterializer, SnapshotSidecarResolver, SourceStream, TimelineSourceKind,
    merge_streams, parse_jsonl, replace_session_file_timelines,
};
use rusqlite::params;
use std::fs;

const TASTE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000031";
const TASTE_TARGET: &str = "src/taste_target.rs";
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

#[test]
fn timeline_materializer_builds_per_file_snapshot_sequence_from_fixture() {
    let rows = FileTimelineMaterializer::materialize(
        "claude_cli",
        TASTE_SESSION_ID,
        Some(TASTE_CWD),
        &fixture_resolver(),
        &merged_fixture_events(),
    )
    .expect("materialize");

    let target_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.file_path == TASTE_TARGET)
        .collect();
    assert_eq!(target_rows.len(), 4, "expected four taste_target snapshots");

    assert_eq!(target_rows[0].seq, 0);
    assert_eq!(
        target_rows[0].source_kind,
        TimelineSourceKind::InlineSnapshot
    );
    assert!(
        target_rows[0]
            .content
            .as_deref()
            .is_some_and(|content| content.contains("taste fixture baseline"))
    );
    assert_eq!(target_rows[0].trigger_event_ref, None);

    assert_eq!(
        target_rows[1].source_kind,
        TimelineSourceKind::SidecarSnapshot
    );
    assert_eq!(
        target_rows[1].trigger_event_ref.as_deref(),
        Some("toolu_taste_edit_accept")
    );
    assert!(
        target_rows[1]
            .content
            .as_deref()
            .is_some_and(|content| content.contains("accepted_fn"))
    );

    assert_eq!(
        target_rows[2].trigger_event_ref.as_deref(),
        Some("toolu_taste_edit_reject"),
        "Bash proposals lack file_path, so trigger falls back to the prior Edit on this file"
    );

    assert_eq!(
        target_rows[3].trigger_event_ref.as_deref(),
        Some("toolu_taste_edit_revert")
    );
    assert!(
        target_rows[3]
            .content
            .as_deref()
            .is_some_and(|content| !content.contains("accepted_fn"))
    );
}

#[test]
fn timeline_materializer_flags_missing_sidecar_without_content() {
    let resolver = fixture_resolver();
    let mut events = merged_fixture_events();
    events.push(jottrace::taste::ParsedEvent {
        seq: events.len(),
        timestamp: Some("2026-06-17T10:00:12.000Z".to_string()),
        kind: jottrace::taste::ParseKind::FileSnapshot,
        file_path: Some("src/missing.rs".to_string()),
        content_or_ref: Some(jottrace::taste::ContentRef::Sidecar {
            backup_file_name: "fixture-missing@v9".to_string(),
            version: Some(9),
        }),
        tool_ref: None,
        tool_name: None,
        source_stream: SourceStream::Parent,
    });

    let rows = FileTimelineMaterializer::materialize(
        "claude_cli",
        TASTE_SESSION_ID,
        Some(TASTE_CWD),
        &resolver,
        &events,
    )
    .expect("materialize");

    let missing = rows
        .iter()
        .find(|row| row.file_path == "src/missing.rs")
        .expect("missing sidecar row");
    assert_eq!(missing.source_kind, TimelineSourceKind::MissingSidecar);
    assert_eq!(missing.content, None);
}

#[test]
fn replace_session_file_timelines_persists_rows_in_database() {
    let root = common::temp_root("taste-timeline-db");
    let db_path = root.join(jottrace::storage::DB_FILE_NAME);
    let conn = open_database(&db_path).expect("open database");
    assert_eq!(
        jottrace::storage::LATEST_SCHEMA_VERSION,
        LATEST_SCHEMA_VERSION
    );

    let rows = FileTimelineMaterializer::materialize(
        "claude_cli",
        TASTE_SESSION_ID,
        Some(TASTE_CWD),
        &fixture_resolver(),
        &merged_fixture_events(),
    )
    .expect("materialize");

    let inserted =
        replace_session_file_timelines(&db_path, &conn, "claude_cli", TASTE_SESSION_ID, &rows)
            .expect("persist timelines");
    assert_eq!(inserted, rows.len());

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_timelines WHERE source = ?1 AND source_session_id = ?2",
            params!["claude_cli", TASTE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("count rows");
    assert_eq!(count as usize, rows.len());

    let final_content: String = conn
        .query_row(
            "SELECT content FROM file_timelines
             WHERE source = ?1 AND source_session_id = ?2 AND file_path = ?3
             ORDER BY seq DESC LIMIT 1",
            params!["claude_cli", TASTE_SESSION_ID, TASTE_TARGET],
            |row| row.get(0),
        )
        .expect("final snapshot");
    assert!(
        final_content.contains("# appended by bash fixture"),
        "final row should reflect v3 sidecar content"
    );
    assert!(!final_content.contains("accepted_fn"));
}
