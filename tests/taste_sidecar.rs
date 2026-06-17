mod common;

use common::taste_fixture;
use jottrace::taste::{
    ContentRef, ParseKind, ResolvedContent, SnapshotSidecarResolver, SourceStream, parse_jsonl,
};
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

fn fixture_resolver() -> SnapshotSidecarResolver {
    SnapshotSidecarResolver::with_history_root(taste_fixture("claude-cli/file-history"))
}

#[test]
fn sidecar_resolver_reads_fixture_blobs_for_all_versions() {
    let resolver = fixture_resolver();
    let expected: Vec<_> = ["v1", "v2", "v3"]
        .into_iter()
        .map(|version| {
            fs::read_to_string(taste_fixture(&format!(
                "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-a1b2c3d4@{version}"
            )))
            .expect("read sidecar fixture")
        })
        .collect();

    for (version, expected_content) in ["v1", "v2", "v3"].into_iter().zip(expected) {
        let resolved = resolver
            .resolve(
                TASTE_SESSION_ID,
                &ContentRef::Sidecar {
                    backup_file_name: format!("fixture-a1b2c3d4@{version}"),
                    version: Some(version[1..].parse().expect("version number")),
                },
            )
            .expect("resolve sidecar");
        assert_eq!(
            resolved,
            ResolvedContent::Sidecar {
                content: expected_content,
                backup_file_name: format!("fixture-a1b2c3d4@{version}"),
                version: Some(version[1..].parse().expect("version number")),
            }
        );
    }
}

#[test]
fn sidecar_resolver_resolves_parsed_session_snapshots() {
    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let events = parse_jsonl(&SourceStream::Parent, read_jsonl_lines(&session)).expect("parse");
    let resolver = fixture_resolver();

    let resolved = resolver
        .resolve_snapshot_events(TASTE_SESSION_ID, &events)
        .expect("resolve snapshots");

    assert_eq!(
        resolved.len(),
        11,
        "expected inline + 9 sidecar/inline snapshots"
    );

    let inline = resolved
        .iter()
        .find(|(_, content)| matches!(content, ResolvedContent::Inline(_)))
        .expect("inline snapshot");
    assert!(
        matches!(&inline.1, ResolvedContent::Inline(content) if content.contains("taste fixture baseline"))
    );

    let sidecars: Vec<_> = resolved
        .iter()
        .filter_map(|(seq, content)| match content {
            ResolvedContent::Sidecar {
                backup_file_name, ..
            } => Some((seq, backup_file_name.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(sidecars.len(), 8);
    for (seq, name) in sidecars {
        assert!(
            name.starts_with("fixture-a1b2c3d4@v")
                || name == "fixture-mcpb5e6f7a@v1"
                || name == "fixture-writenew1@v1"
                || name == "fixture-subagent1@v1"
                || name.starts_with("fixture-partial1@v"),
            "seq {seq} should reference fixture sidecar, got {name}"
        );
    }

    let final_snapshot = resolved
        .iter()
        .find(|(_, content)| {
            matches!(
                content,
                ResolvedContent::Sidecar {
                    backup_file_name,
                    ..
                } if backup_file_name == "fixture-a1b2c3d4@v3"
            )
        })
        .expect("v3 sidecar");
    assert!(matches!(
        &final_snapshot.1,
        ResolvedContent::Sidecar { content, .. } if !content.contains("accepted_fn")
    ));
}

#[test]
fn sidecar_resolver_reports_missing_blob_without_error() {
    let resolver = fixture_resolver();
    let resolved = resolver
        .resolve(
            TASTE_SESSION_ID,
            &ContentRef::Sidecar {
                backup_file_name: "fixture-missing@v9".to_string(),
                version: Some(9),
            },
        )
        .expect("missing sidecar should not error");

    assert!(matches!(
        resolved,
        ResolvedContent::MissingSidecar {
            backup_file_name,
            version: Some(9),
            ..
        } if backup_file_name == "fixture-missing@v9"
    ));
}

#[test]
fn sidecar_resolver_leaves_non_snapshot_events_unresolved() {
    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let events = parse_jsonl(&SourceStream::Parent, read_jsonl_lines(&session)).expect("parse");
    let resolver = fixture_resolver();

    let proposals: Vec<_> = events
        .iter()
        .filter(|event| event.kind == ParseKind::ToolProposal)
        .collect();
    assert!(!proposals.is_empty());

    for event in proposals {
        let resolved = resolver
            .resolve_event(TASTE_SESSION_ID, event)
            .expect("resolve proposal");
        assert!(
            matches!(resolved, Some(ResolvedContent::Inline(_))),
            "tool proposals stay inline"
        );
    }
}
