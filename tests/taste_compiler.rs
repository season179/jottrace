mod common;

use common::taste_fixture;
use jottrace::taste::{
    EvidenceKind, FileTimelineMaterializer, PreferenceCompiler, PreferenceOutcome,
    SnapshotSidecarResolver, SourceStream, merge_streams, parse_jsonl,
};
use std::collections::HashMap;
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
    PreferenceCompiler::compile(
        "claude_cli",
        TASTE_SESSION_ID,
        Some(TASTE_CWD),
        &events,
        &rows,
    )
}

fn example_by_tool<'a>(
    examples: &'a [jottrace::taste::PreferenceExample],
    tool_use_id: &str,
) -> &'a jottrace::taste::PreferenceExample {
    examples
        .iter()
        .find(|example| example.tool_use_id == tool_use_id)
        .unwrap_or_else(|| panic!("missing example for {tool_use_id}"))
}

#[test]
fn compiler_labels_fixture_proposals_with_present_at_session_end_outcomes() {
    let examples = compile_fixture_examples();
    let by_tool: HashMap<_, _> = examples
        .iter()
        .map(|example| (example.tool_use_id.as_str(), example))
        .collect();

    for required in [
        "toolu_taste_edit_accept",
        "toolu_taste_edit_reject",
        "toolu_taste_write",
        "toolu_taste_bash",
        "toolu_taste_edit_revert",
        "toolu_taste_sub_edit",
        "toolu_taste_notebook_edit",
    ] {
        assert!(
            by_tool.contains_key(required),
            "compiler should emit example for {required}"
        );
    }

    let accept = example_by_tool(&examples, "toolu_taste_edit_accept");
    assert_eq!(accept.file_path.as_deref(), Some(TASTE_TARGET));
    assert_eq!(accept.outcome, PreferenceOutcome::Rejected);
    assert_eq!(accept.evidence_kind, EvidenceKind::DirectEdit);
    assert!(
        accept
            .context
            .as_deref()
            .is_some_and(|content| content.contains("taste fixture baseline"))
    );

    let reject = example_by_tool(&examples, "toolu_taste_edit_reject");
    assert_eq!(reject.outcome, PreferenceOutcome::Rejected);
    assert_eq!(reject.evidence_kind, EvidenceKind::PermissionDenial);

    let revert = example_by_tool(&examples, "toolu_taste_edit_revert");
    assert_eq!(revert.outcome, PreferenceOutcome::Accepted);
    assert_eq!(revert.evidence_kind, EvidenceKind::DirectEdit);

    let bash = example_by_tool(&examples, "toolu_taste_bash");
    assert_eq!(bash.file_path.as_deref(), Some(TASTE_TARGET));
    assert_eq!(bash.outcome, PreferenceOutcome::Accepted);
    assert_eq!(bash.evidence_kind, EvidenceKind::BashCorrelation);
    assert!(bash.confidence < 1.0);

    let write = example_by_tool(&examples, "toolu_taste_write");
    assert_eq!(write.file_path.as_deref(), Some("src/taste_new.rs"));
    assert_eq!(write.outcome, PreferenceOutcome::Accepted);
    assert_eq!(write.evidence_kind, EvidenceKind::MissingFinalState);
    assert!(write.confidence < 1.0);

    let sub = example_by_tool(&examples, "toolu_taste_sub_edit");
    assert_eq!(sub.file_path.as_deref(), Some("src/taste_subagent.rs"));
    assert_eq!(sub.outcome, PreferenceOutcome::Rejected);
    assert_eq!(sub.evidence_kind, EvidenceKind::MissingFinalState);

    let notebook = example_by_tool(&examples, "toolu_taste_notebook_edit");
    assert_eq!(
        notebook.file_path.as_deref(),
        Some("notebooks/taste_fixture.ipynb")
    );
    assert_eq!(notebook.outcome, PreferenceOutcome::Accepted);
    assert_eq!(notebook.evidence_kind, EvidenceKind::DirectEdit);
    assert_eq!(notebook.confidence, 1.0);
    assert!(
        notebook
            .proposal_content
            .as_deref()
            .is_some_and(|content| content.contains("notebook_marker"))
    );
}

#[test]
fn compiler_assigns_monotonic_generation_order() {
    let examples = compile_fixture_examples();
    for (expected, example) in examples.iter().enumerate() {
        assert_eq!(example.generation, expected);
    }
}
