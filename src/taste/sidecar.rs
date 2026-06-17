use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::JottraceError;

use super::parse::{ContentRef, ParseKind, ParsedEvent};

const CLAUDE_FILE_HISTORY_DIR: &str = ".claude/file-history";

/// Resolved snapshot content, including graceful degradation when sidecars are missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedContent {
    Inline(String),
    Sidecar {
        content: String,
        backup_file_name: String,
        version: Option<u64>,
    },
    MissingSidecar {
        backup_file_name: String,
        version: Option<u64>,
        path: PathBuf,
    },
}

/// Reads Claude `backupFileName` snapshot blobs from `~/.claude/file-history/<session>/`.
#[derive(Debug, Clone)]
pub struct SnapshotSidecarResolver {
    history_root: PathBuf,
}

impl SnapshotSidecarResolver {
    /// Resolve sidecars from the real Claude CLI file-history directory under `$HOME`.
    pub fn claude_home() -> Result<Self, JottraceError> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(JottraceError::MissingHome)?;
        Ok(Self::with_history_root(home.join(CLAUDE_FILE_HISTORY_DIR)))
    }

    /// Resolve sidecars from a custom file-history root (used by tests and future overrides).
    pub fn with_history_root(history_root: impl Into<PathBuf>) -> Self {
        Self {
            history_root: history_root.into(),
        }
    }

    pub fn history_root(&self) -> &Path {
        &self.history_root
    }

    pub fn sidecar_path(&self, session_id: &str, backup_file_name: &str) -> PathBuf {
        self.history_root
            .join(session_id)
            .join(backup_file_name)
    }

    pub fn resolve(&self, session_id: &str, content_ref: &ContentRef) -> Result<ResolvedContent, JottraceError> {
        match content_ref {
            ContentRef::Inline(content) => Ok(ResolvedContent::Inline(content.clone())),
            ContentRef::Sidecar {
                backup_file_name,
                version,
            } => self.resolve_sidecar(session_id, backup_file_name, *version),
        }
    }

    pub fn resolve_event(
        &self,
        session_id: &str,
        event: &ParsedEvent,
    ) -> Result<Option<ResolvedContent>, JottraceError> {
        let Some(content_ref) = event.content_or_ref.as_ref() else {
            return Ok(None);
        };
        self.resolve(session_id, content_ref).map(Some)
    }

    pub fn resolve_snapshot_events<'a, I>(
        &self,
        session_id: &str,
        events: I,
    ) -> Result<Vec<(usize, ResolvedContent)>, JottraceError>
    where
        I: IntoIterator<Item = &'a ParsedEvent>,
    {
        let mut resolved = Vec::new();
        for event in events {
            if event.kind != ParseKind::FileSnapshot {
                continue;
            }
            if let Some(content) = self.resolve_event(session_id, event)? {
                resolved.push((event.seq, content));
            }
        }
        Ok(resolved)
    }

    fn resolve_sidecar(
        &self,
        session_id: &str,
        backup_file_name: &str,
        version: Option<u64>,
    ) -> Result<ResolvedContent, JottraceError> {
        let path = self.sidecar_path(session_id, backup_file_name);
        match fs::read_to_string(&path) {
            Ok(content) => Ok(ResolvedContent::Sidecar {
                content,
                backup_file_name: backup_file_name.to_string(),
                version,
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(ResolvedContent::MissingSidecar {
                    backup_file_name: backup_file_name.to_string(),
                    version,
                    path,
                })
            }
            Err(source) => Err(JottraceError::Io { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taste::parse::{ParseKind, SourceStream};

    #[test]
    fn sidecar_path_joins_history_root_session_and_blob_name() {
        let resolver = SnapshotSidecarResolver::with_history_root("/tmp/history");
        assert_eq!(
            resolver.sidecar_path("sess-1", "blob@v2"),
            PathBuf::from("/tmp/history/sess-1/blob@v2")
        );
    }

    #[test]
    fn inline_content_ref_resolves_without_disk_access() {
        let resolver = SnapshotSidecarResolver::with_history_root("/does/not/exist");
        let resolved = resolver
            .resolve("sess", &ContentRef::Inline("hello".to_string()))
            .expect("inline resolve");
        assert_eq!(resolved, ResolvedContent::Inline("hello".to_string()));
    }

    #[test]
    fn missing_sidecar_returns_missing_variant_not_error() {
        let resolver = SnapshotSidecarResolver::with_history_root("/does/not/exist");
        let resolved = resolver
            .resolve(
                "sess",
                &ContentRef::Sidecar {
                    backup_file_name: "missing@v1".to_string(),
                    version: Some(1),
                },
            )
            .expect("missing sidecar");
        assert_eq!(
            resolved,
            ResolvedContent::MissingSidecar {
                backup_file_name: "missing@v1".to_string(),
                version: Some(1),
                path: PathBuf::from("/does/not/exist/sess/missing@v1"),
            }
        );
    }

    #[test]
    fn resolve_snapshot_events_skips_non_snapshot_rows() {
        let resolver = SnapshotSidecarResolver::with_history_root("/does/not/exist");
        let events = vec![
            ParsedEvent {
                seq: 0,
                timestamp: None,
                kind: ParseKind::ToolProposal,
                file_path: None,
                content_or_ref: Some(ContentRef::Inline("ignored".to_string())),
                tool_ref: None,
                tool_name: None,
                source_stream: SourceStream::Parent,
            },
            ParsedEvent {
                seq: 1,
                timestamp: None,
                kind: ParseKind::FileSnapshot,
                file_path: Some("src/a.rs".to_string()),
                content_or_ref: Some(ContentRef::Inline("snap".to_string())),
                tool_ref: None,
                tool_name: None,
                source_stream: SourceStream::Parent,
            },
        ];

        let resolved = resolver
            .resolve_snapshot_events("sess", &events)
            .expect("resolve snapshots");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].0, 1);
        assert_eq!(resolved[0].1, ResolvedContent::Inline("snap".to_string()));
    }
}
