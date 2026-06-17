use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, params};

use crate::JottraceError;
use crate::storage::sqlite_error;

use super::parse::{ParseKind, ParsedEvent};
use super::sidecar::{ResolvedContent, SnapshotSidecarResolver};

/// Where a timeline row's content was resolved from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineSourceKind {
    InlineSnapshot,
    SidecarSnapshot,
    MissingSidecar,
}

impl TimelineSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InlineSnapshot => "inline_snapshot",
            Self::SidecarSnapshot => "sidecar_snapshot",
            Self::MissingSidecar => "missing_sidecar",
        }
    }
}

/// One reconstructed per-file content state in a session timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTimelineRow {
    pub source: String,
    pub source_session_id: String,
    pub file_path: String,
    pub seq: usize,
    pub event_seq: usize,
    pub content: Option<String>,
    pub trigger_event_ref: Option<String>,
    pub source_kind: TimelineSourceKind,
}

/// Builds per-file content timelines from parsed and resolved snapshot events.
#[derive(Debug, Default)]
pub struct FileTimelineMaterializer;

impl FileTimelineMaterializer {
    pub fn materialize(
        source: &str,
        source_session_id: &str,
        cwd: Option<&str>,
        resolver: &SnapshotSidecarResolver,
        events: &[ParsedEvent],
    ) -> Result<Vec<FileTimelineRow>, JottraceError> {
        let mut per_file_seq: HashMap<String, usize> = HashMap::new();
        let mut last_trigger: HashMap<String, String> = HashMap::new();
        let mut rows = Vec::new();

        for event in events {
            if event.kind == ParseKind::ToolProposal {
                if let (Some(file_path), Some(tool_ref)) = (&event.file_path, &event.tool_ref) {
                    let file_path = normalize_file_path(file_path, cwd);
                    last_trigger.insert(file_path, tool_ref.clone());
                }
                continue;
            }

            if event.kind != ParseKind::FileSnapshot {
                continue;
            }

            let Some(file_path) = event.file_path.as_ref() else {
                continue;
            };
            let file_path = normalize_file_path(file_path, cwd);

            let Some(content_ref) = event.content_or_ref.as_ref() else {
                continue;
            };

            let resolved = resolver.resolve(source_session_id, content_ref)?;
            let (content, source_kind) = match resolved {
                ResolvedContent::Inline(content) => {
                    (Some(content), TimelineSourceKind::InlineSnapshot)
                }
                ResolvedContent::Sidecar { content, .. } => {
                    (Some(content), TimelineSourceKind::SidecarSnapshot)
                }
                ResolvedContent::MissingSidecar { .. } => {
                    (None, TimelineSourceKind::MissingSidecar)
                }
            };

            let seq = *per_file_seq.entry(file_path.clone()).or_insert(0);
            per_file_seq.insert(file_path.clone(), seq + 1);
            let trigger_event_ref = last_trigger.get(&file_path).cloned();

            rows.push(FileTimelineRow {
                source: source.to_string(),
                source_session_id: source_session_id.to_string(),
                file_path,
                seq,
                event_seq: event.seq,
                content,
                trigger_event_ref,
                source_kind,
            });
        }

        Ok(rows)
    }
}

pub(crate) fn normalize_file_path(path: &str, cwd: Option<&str>) -> String {
    let path = path.replace('\\', "/");
    if let Some(cwd) = cwd {
        let cwd = cwd.trim_end_matches('/');
        let prefix = format!("{cwd}/");
        if let Some(relative) = path.strip_prefix(&prefix) {
            return relative.to_string();
        }
    }
    path
}

/// Replace all timeline rows for one session with freshly materialized output.
pub fn replace_session_file_timelines(
    db_path: &Path,
    conn: &Connection,
    source: &str,
    source_session_id: &str,
    rows: &[FileTimelineRow],
) -> Result<usize, JottraceError> {
    conn.execute(
        "DELETE FROM file_timelines WHERE source = ?1 AND source_session_id = ?2",
        params![source, source_session_id],
    )
    .map_err(|source| sqlite_error(db_path, source))?;

    let mut inserted = 0usize;
    for row in rows {
        conn.execute(
            "INSERT INTO file_timelines (
                source,
                source_session_id,
                file_path,
                seq,
                event_seq,
                content,
                trigger_event_ref,
                source_kind
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.source,
                row.source_session_id,
                row.file_path,
                i64::try_from(row.seq).expect("seq fits in i64"),
                i64::try_from(row.event_seq).expect("event_seq fits in i64"),
                row.content,
                row.trigger_event_ref,
                row.source_kind.as_str(),
            ],
        )
        .map_err(|source| sqlite_error(db_path, source))?;
        inserted += 1;
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taste::parse::{ContentRef, ParseKind, SourceStream};

    fn snapshot_event(
        seq: usize,
        file_path: &str,
        content_ref: ContentRef,
        timestamp: &str,
    ) -> ParsedEvent {
        ParsedEvent {
            seq,
            timestamp: Some(timestamp.to_string()),
            kind: ParseKind::FileSnapshot,
            file_path: Some(file_path.to_string()),
            content_or_ref: Some(content_ref),
            tool_ref: None,
            tool_name: None,
            source_stream: SourceStream::Parent,
        }
    }

    #[test]
    fn normalize_file_path_strips_session_cwd_prefix() {
        assert_eq!(
            normalize_file_path(
                "/Users/fixture/Workspace/jottrace/src/taste_target.rs",
                Some("/Users/fixture/Workspace/jottrace"),
            ),
            "src/taste_target.rs"
        );
    }

    #[test]
    fn materialize_assigns_per_file_seq_and_trigger_refs() {
        let resolver = SnapshotSidecarResolver::with_history_root("/does/not/exist");
        let events = vec![
            snapshot_event(
                0,
                "src/a.rs",
                ContentRef::Inline("v0".to_string()),
                "2026-06-17T10:00:00.000Z",
            ),
            ParsedEvent {
                seq: 1,
                timestamp: Some("2026-06-17T10:00:01.000Z".to_string()),
                kind: ParseKind::ToolProposal,
                file_path: Some("src/a.rs".to_string()),
                content_or_ref: Some(ContentRef::Inline("proposal".to_string())),
                tool_ref: Some("toolu_edit".to_string()),
                tool_name: Some("Edit".to_string()),
                source_stream: SourceStream::Parent,
            },
            snapshot_event(
                2,
                "src/a.rs",
                ContentRef::Inline("v1".to_string()),
                "2026-06-17T10:00:02.000Z",
            ),
        ];

        let rows = FileTimelineMaterializer::materialize(
            "claude_cli",
            "sess",
            None,
            &resolver,
            &events,
        )
            .expect("materialize");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq, 0);
        assert_eq!(rows[0].content.as_deref(), Some("v0"));
        assert_eq!(rows[0].trigger_event_ref, None);
        assert_eq!(rows[0].source_kind, TimelineSourceKind::InlineSnapshot);

        assert_eq!(rows[1].seq, 1);
        assert_eq!(rows[1].content.as_deref(), Some("v1"));
        assert_eq!(rows[1].trigger_event_ref.as_deref(), Some("toolu_edit"));
    }
}
