mod common;

use common::taste_fixture;
use jottrace::taste::{ContentRef, ParseKind, SourceStream, merge_streams, parse_jsonl};
use std::fs;

const TASTE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000031";

fn read_jsonl_lines(path: &std::path::Path) -> Vec<Vec<u8>> {
    fs::read_to_string(path)
        .expect("read fixture")
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.as_bytes().to_vec())
        .collect()
}

#[test]
fn claude_parser_emits_snapshots_proposals_and_denials_from_fixture() {
    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let events = parse_jsonl(&SourceStream::Parent, read_jsonl_lines(&session)).expect("parse");

    let snapshots: Vec<_> = events
        .iter()
        .filter(|event| event.kind == ParseKind::FileSnapshot)
        .collect();
    assert!(
        snapshots.len() >= 5,
        "expected inline + 4 sidecar/inline snapshots, got {}",
        snapshots.len()
    );
    assert!(
        snapshots
            .iter()
            .any(|event| matches!(event.content_or_ref, Some(ContentRef::Inline(_))))
    );
    assert!(snapshots.iter().any(|event| matches!(
        event.content_or_ref,
        Some(ContentRef::Sidecar {
            backup_file_name: _,
            ..
        })
    )));

    let proposals: Vec<_> = events
        .iter()
        .filter(|event| event.kind == ParseKind::ToolProposal)
        .collect();
    let tool_names: Vec<_> = proposals
        .iter()
        .filter_map(|event| event.tool_name.as_deref())
        .collect();
    for required in ["Edit", "Write", "Bash", "NotebookEdit"] {
        assert!(
            tool_names.contains(&required),
            "missing tool proposal for {required}"
        );
    }

    let denials: Vec<_> = events
        .iter()
        .filter(|event| event.kind == ParseKind::PermissionDenial)
        .collect();
    assert_eq!(denials.len(), 1);
    assert_eq!(
        denials[0].tool_ref.as_deref(),
        Some("toolu_taste_edit_reject")
    );
}

#[test]
fn claude_parser_merges_subagent_edits_into_parent_timeline() {
    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let subagent = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}/subagents/agent-taste000000000001.jsonl"
    ));

    let parent_events =
        parse_jsonl(&SourceStream::Parent, read_jsonl_lines(&session)).expect("parent");
    let subagent_events = parse_jsonl(
        &SourceStream::Subagent {
            agent_id: "agent-taste000000000001".to_string(),
        },
        read_jsonl_lines(&subagent),
    )
    .expect("subagent");

    let merged = merge_streams(vec![
        (SourceStream::Parent, parent_events),
        (
            SourceStream::Subagent {
                agent_id: "agent-taste000000000001".to_string(),
            },
            subagent_events,
        ),
    ]);

    let subagent_edit = merged
        .iter()
        .find(|event| event.tool_ref.as_deref() == Some("toolu_taste_sub_edit"));
    assert!(
        subagent_edit.is_some(),
        "subagent Edit should appear in merged timeline"
    );
    assert!(matches!(
        subagent_edit.unwrap().source_stream,
        SourceStream::Subagent { .. }
    ));

    let accept_idx = merged
        .iter()
        .position(|event| event.tool_ref.as_deref() == Some("toolu_taste_edit_accept"))
        .expect("accepted edit");
    let sub_idx = merged
        .iter()
        .position(|event| event.tool_ref.as_deref() == Some("toolu_taste_sub_edit"))
        .expect("subagent edit");
    assert!(
        accept_idx < sub_idx,
        "subagent edit at {} should follow parent accept at {}",
        sub_idx,
        accept_idx
    );
}

#[test]
fn claude_parser_assigns_monotonic_seq_after_merge() {
    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let subagent = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}/subagents/agent-taste000000000001.jsonl"
    ));

    let merged = merge_streams(vec![
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
    ]);

    for (expected, event) in merged.iter().enumerate() {
        assert_eq!(event.seq, expected);
    }
}
