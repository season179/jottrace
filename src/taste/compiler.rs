use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, params};

use crate::JottraceError;
use crate::storage::sqlite_error;

use super::parse::{ContentRef, ParseKind, ParsedEvent};
use super::timeline::{FileTimelineRow, normalize_file_path};

/// Version tag stored on compiled preference rows for idempotent re-extraction.
pub const EXTRACTOR_VERSION: &str = "0.1.0";

/// Minimum confidence for a proposal to count toward high-confidence coverage.
pub const HIGH_CONFIDENCE_THRESHOLD: f64 = 1.0;

/// Labeled outcome for a detected tool proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceOutcome {
    Accepted,
    Rejected,
    Edited,
}

impl PreferenceOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Edited => "edited",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            "edited" => Some(Self::Edited),
            _ => None,
        }
    }
}

/// How a proposal was linked to file state for outcome detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    DirectEdit,
    DirectWrite,
    BashCorrelation,
    PermissionDenial,
    MissingFinalState,
}

impl EvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectEdit => "direct_edit",
            Self::DirectWrite => "direct_write",
            Self::BashCorrelation => "bash_correlation",
            Self::PermissionDenial => "permission_denial",
            Self::MissingFinalState => "missing_final_state",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "direct_edit" => Some(Self::DirectEdit),
            "direct_write" => Some(Self::DirectWrite),
            "bash_correlation" => Some(Self::BashCorrelation),
            "permission_denial" => Some(Self::PermissionDenial),
            "missing_final_state" => Some(Self::MissingFinalState),
            _ => None,
        }
    }
}

/// One export-ready labeled preference row from the compiler.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferenceExample {
    pub source: String,
    pub source_session_id: String,
    pub generation: usize,
    pub proposal_event_seq: usize,
    pub tool_use_id: String,
    pub file_path: Option<String>,
    pub tool_name: String,
    pub proposal_content: Option<String>,
    pub context: Option<String>,
    pub outcome: PreferenceOutcome,
    pub confidence: f64,
    pub evidence_kind: EvidenceKind,
    pub extractor_version: String,
}

/// Joins parsed proposals with materialized timelines and labels outcomes.
#[derive(Debug, Default)]
pub struct PreferenceCompiler;

impl PreferenceCompiler {
    pub fn compile(
        source: &str,
        source_session_id: &str,
        cwd: Option<&str>,
        events: &[ParsedEvent],
        timeline_rows: &[FileTimelineRow],
    ) -> Vec<PreferenceExample> {
        let denied_tools = denied_tool_refs(events);
        let timelines = index_timelines(timeline_rows);
        let mut generation = 0usize;
        let mut examples = Vec::new();

        for event in events {
            if event.kind != ParseKind::ToolProposal {
                continue;
            }
            let Some(tool_name) = event.tool_name.as_deref() else {
                continue;
            };
            if !is_file_modifying_tool(tool_name) {
                continue;
            }

            let tool_use_id = match event.tool_ref.as_deref() {
                Some(tool_use_id) => tool_use_id.to_string(),
                None => continue,
            };

            let proposal_content = event.content_or_ref.as_ref().and_then(content_ref_text);

            let file_path = event
                .file_path
                .as_deref()
                .map(|path| normalize_file_path(path, cwd))
                .or_else(|| infer_bash_target_file(proposal_content.as_deref(), cwd));

            let (outcome, confidence, evidence_kind) = if denied_tools.contains(&tool_use_id) {
                (
                    PreferenceOutcome::Rejected,
                    1.0,
                    EvidenceKind::PermissionDenial,
                )
            } else {
                classify_present_at_session_end(
                    tool_name,
                    file_path.as_deref(),
                    &tool_use_id,
                    event.seq,
                    &timelines,
                    events,
                )
            };

            let context = file_path
                .as_deref()
                .and_then(|path| before_state_content(path, event.seq, &timelines));

            examples.push(PreferenceExample {
                source: source.to_string(),
                source_session_id: source_session_id.to_string(),
                generation,
                proposal_event_seq: event.seq,
                tool_use_id,
                file_path,
                tool_name: tool_name.to_string(),
                proposal_content,
                context,
                outcome,
                confidence,
                evidence_kind,
                extractor_version: EXTRACTOR_VERSION.to_string(),
            });
            generation += 1;
        }

        examples
    }
}

/// Replace all preference rows for one session with freshly compiled output.
pub fn replace_session_preference_examples(
    db_path: &Path,
    conn: &Connection,
    source: &str,
    source_session_id: &str,
    examples: &[PreferenceExample],
) -> Result<usize, JottraceError> {
    conn.execute(
        "DELETE FROM preference_examples WHERE source = ?1 AND source_session_id = ?2",
        params![source, source_session_id],
    )
    .map_err(|source| sqlite_error(db_path, source))?;

    let mut inserted = 0usize;
    for example in examples {
        conn.execute(
            "INSERT INTO preference_examples (
                source,
                source_session_id,
                generation,
                proposal_event_seq,
                tool_use_id,
                file_path,
                tool_name,
                proposal_content,
                context,
                outcome,
                confidence,
                evidence_kind,
                extractor_version
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                example.source,
                example.source_session_id,
                i64::try_from(example.generation).expect("generation fits in i64"),
                i64::try_from(example.proposal_event_seq).expect("proposal_event_seq fits in i64"),
                example.tool_use_id,
                example.file_path,
                example.tool_name,
                example.proposal_content,
                example.context,
                example.outcome.as_str(),
                example.confidence,
                example.evidence_kind.as_str(),
                example.extractor_version,
            ],
        )
        .map_err(|source| sqlite_error(db_path, source))?;
        inserted += 1;
    }

    Ok(inserted)
}

fn is_file_modifying_tool(tool_name: &str) -> bool {
    matches!(tool_name, "Edit" | "Write" | "NotebookEdit" | "Bash")
}

fn content_ref_text(content_ref: &ContentRef) -> Option<String> {
    match content_ref {
        ContentRef::Inline(content) => Some(content.clone()),
        ContentRef::Sidecar { .. } => None,
    }
}

fn denied_tool_refs(events: &[ParsedEvent]) -> HashSet<String> {
    events
        .iter()
        .filter(|event| event.kind == ParseKind::PermissionDenial)
        .filter_map(|event| event.tool_ref.clone())
        .collect()
}

struct FileTimelineIndex {
    rows: Vec<FileTimelineRow>,
    by_trigger: HashMap<String, usize>,
}

fn index_timelines(rows: &[FileTimelineRow]) -> HashMap<String, FileTimelineIndex> {
    let mut grouped: HashMap<String, Vec<FileTimelineRow>> = HashMap::new();
    for row in rows {
        grouped
            .entry(row.file_path.clone())
            .or_default()
            .push(row.clone());
    }

    grouped
        .into_iter()
        .map(|(file_path, mut rows)| {
            rows.sort_by_key(|row| row.seq);
            let by_trigger = rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    row.trigger_event_ref
                        .as_ref()
                        .map(|trigger| (trigger.clone(), index))
                })
                .collect();
            (file_path, FileTimelineIndex { rows, by_trigger })
        })
        .collect()
}

fn before_state_content(
    file_path: &str,
    proposal_seq: usize,
    timelines: &HashMap<String, FileTimelineIndex>,
) -> Option<String> {
    let index = timelines.get(file_path)?;
    index
        .rows
        .iter()
        .filter(|row| row.event_seq < proposal_seq)
        .max_by_key(|row| row.seq)
        .and_then(|row| row.content.clone())
}

fn classify_present_at_session_end(
    tool_name: &str,
    file_path: Option<&str>,
    tool_use_id: &str,
    proposal_seq: usize,
    timelines: &HashMap<String, FileTimelineIndex>,
    events: &[ParsedEvent],
) -> (PreferenceOutcome, f64, EvidenceKind) {
    let evidence_kind = evidence_kind_for_tool(tool_name);
    let base_confidence = base_confidence_for_tool(tool_name);

    let Some(file_path) = file_path else {
        return (
            PreferenceOutcome::Rejected,
            0.2,
            EvidenceKind::MissingFinalState,
        );
    };

    let Some(index) = timelines.get(file_path) else {
        if tool_name == "Write" && tool_executed(events, tool_use_id) {
            return (
                PreferenceOutcome::Accepted,
                0.4,
                EvidenceKind::MissingFinalState,
            );
        }
        return (
            PreferenceOutcome::Rejected,
            0.3,
            EvidenceKind::MissingFinalState,
        );
    };

    let before = before_state_content(file_path, proposal_seq, timelines);
    let post_apply = post_apply_content(tool_name, tool_use_id, proposal_seq, index, events);
    let final_content = index.rows.iter().rev().find_map(|row| row.content.as_ref());

    match (post_apply.as_deref(), final_content) {
        (None, _) => (PreferenceOutcome::Rejected, base_confidence, evidence_kind),
        (Some(post), Some(final_content)) => {
            let before_text = before.as_deref().unwrap_or("");
            if effect_present(before_text, post, final_content) {
                (PreferenceOutcome::Accepted, base_confidence, evidence_kind)
            } else if partial_effect_present(before_text, post, final_content) {
                (
                    PreferenceOutcome::Edited,
                    (base_confidence * 0.5).max(0.2),
                    evidence_kind,
                )
            } else {
                (PreferenceOutcome::Rejected, base_confidence, evidence_kind)
            }
        }
        (Some(_), None) => (
            PreferenceOutcome::Rejected,
            0.3,
            EvidenceKind::MissingFinalState,
        ),
    }
}

fn evidence_kind_for_tool(tool_name: &str) -> EvidenceKind {
    match tool_name {
        "Edit" | "NotebookEdit" => EvidenceKind::DirectEdit,
        "Write" => EvidenceKind::DirectWrite,
        "Bash" => EvidenceKind::BashCorrelation,
        _ => EvidenceKind::MissingFinalState,
    }
}

fn base_confidence_for_tool(tool_name: &str) -> f64 {
    match tool_name {
        "Edit" | "Write" | "NotebookEdit" => 1.0,
        "Bash" => 0.6,
        _ => 0.3,
    }
}

fn tool_executed(events: &[ParsedEvent], tool_use_id: &str) -> bool {
    events.iter().any(|event| {
        event.kind == ParseKind::ToolResult && event.tool_ref.as_deref() == Some(tool_use_id)
    })
}

fn post_apply_content(
    tool_name: &str,
    tool_use_id: &str,
    proposal_seq: usize,
    index: &FileTimelineIndex,
    _events: &[ParsedEvent],
) -> Option<String> {
    if let Some(row_index) = index.by_trigger.get(tool_use_id) {
        return index.rows[*row_index].content.clone();
    }

    if tool_name != "Bash" {
        return None;
    }

    index
        .rows
        .iter()
        .filter(|row| row.event_seq > proposal_seq)
        .min_by_key(|row| row.event_seq)
        .and_then(|row| row.content.clone())
}

fn effect_present(before: &str, after: &str, final_content: &str) -> bool {
    let added = line_delta(before, after);
    let removed = line_delta(after, before);

    added.iter().all(|line| {
        final_content
            .lines()
            .any(|final_line| final_line == line.as_str())
    }) && removed.iter().all(|line| {
        !final_content
            .lines()
            .any(|final_line| final_line == line.as_str())
    })
}

fn partial_effect_present(before: &str, after: &str, final_content: &str) -> bool {
    let added = line_delta(before, after);
    if added.is_empty() {
        return false;
    }
    let preserved = added
        .iter()
        .filter(|line| {
            final_content
                .lines()
                .any(|final_line| final_line == line.as_str())
        })
        .count();
    preserved > 0 && preserved < added.len()
}

fn line_delta(left: &str, right: &str) -> HashSet<String> {
    let left_lines: HashSet<_> = left.lines().map(str::to_string).collect();
    let right_lines: HashSet<_> = right.lines().map(str::to_string).collect();
    right_lines.difference(&left_lines).cloned().collect()
}

fn infer_bash_target_file(command: Option<&str>, cwd: Option<&str>) -> Option<String> {
    let command = command?;
    let token = command
        .split_whitespace()
        .rev()
        .find(|token| token.contains('/') || token.ends_with(".rs") || token.ends_with(".ts"))?;
    let token = token.trim_matches(|ch: char| matches!(ch, '>' | '"' | '\''));
    if token.is_empty() {
        return None;
    }
    Some(normalize_file_path(token, cwd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taste::parse::SourceStream;
    use crate::taste::timeline::TimelineSourceKind;

    fn timeline_row(
        file_path: &str,
        seq: usize,
        event_seq: usize,
        content: &str,
        trigger: Option<&str>,
    ) -> FileTimelineRow {
        FileTimelineRow {
            source: "claude_cli".to_string(),
            source_session_id: "sess".to_string(),
            file_path: file_path.to_string(),
            seq,
            event_seq,
            content: Some(content.to_string()),
            trigger_event_ref: trigger.map(str::to_string),
            source_kind: TimelineSourceKind::InlineSnapshot,
        }
    }

    fn proposal(
        seq: usize,
        tool_use_id: &str,
        tool_name: &str,
        file_path: Option<&str>,
        proposal_content: Option<&str>,
    ) -> ParsedEvent {
        ParsedEvent {
            seq,
            timestamp: None,
            kind: ParseKind::ToolProposal,
            file_path: file_path.map(str::to_string),
            content_or_ref: proposal_content.map(|content| ContentRef::Inline(content.to_string())),
            tool_ref: Some(tool_use_id.to_string()),
            tool_name: Some(tool_name.to_string()),
            source_stream: SourceStream::Parent,
        }
    }

    #[test]
    fn effect_present_detects_persisted_and_reverted_deltas() {
        let before = "a\nb\n";
        let after = "a\nb\nc\n";
        assert!(effect_present(before, after, "a\nb\nc\n"));
        assert!(!effect_present(before, after, "a\nb\n"));
    }

    #[test]
    fn compiler_labels_permission_denial_as_rejected() {
        let events = vec![
            proposal(1, "toolu_denied", "Edit", Some("src/a.rs"), Some("new")),
            ParsedEvent {
                seq: 2,
                timestamp: None,
                kind: ParseKind::PermissionDenial,
                file_path: None,
                content_or_ref: None,
                tool_ref: Some("toolu_denied".to_string()),
                tool_name: None,
                source_stream: SourceStream::Parent,
            },
        ];
        let rows = vec![timeline_row("src/a.rs", 0, 0, "base\n", None)];

        let examples = PreferenceCompiler::compile("claude_cli", "sess", None, &events, &rows);
        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].outcome, PreferenceOutcome::Rejected);
        assert_eq!(examples[0].evidence_kind, EvidenceKind::PermissionDenial);
        assert_eq!(examples[0].confidence, 1.0);
    }

    #[test]
    fn compiler_detects_reverted_edit_as_rejected_at_session_end() {
        let events = vec![proposal(
            1,
            "toolu_edit",
            "Edit",
            Some("src/a.rs"),
            Some("added\n"),
        )];
        let rows = vec![
            timeline_row("src/a.rs", 0, 0, "base\n", None),
            timeline_row("src/a.rs", 1, 2, "base\nadded\n", Some("toolu_edit")),
            timeline_row("src/a.rs", 2, 4, "base\n", Some("toolu_revert")),
        ];

        let examples = PreferenceCompiler::compile("claude_cli", "sess", None, &events, &rows);
        assert_eq!(examples[0].outcome, PreferenceOutcome::Rejected);
    }
}
