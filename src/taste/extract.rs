use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::path::{Path, PathBuf};

use crate::JottraceError;
use crate::storage::{
    DB_FILE_NAME, for_each_decoded_event_payload_for_session, open_database, sqlite_error,
};
use crate::{Result, acquire_data_lock, data_dir_from_env};

use super::compiler::{EXTRACTOR_VERSION, PreferenceCompiler, replace_session_preference_examples};
use super::parse::{SourceStream, merge_streams, parse_jsonl};
use super::sidecar::SnapshotSidecarResolver;
use super::timeline::{FileTimelineMaterializer, replace_session_file_timelines};

const CLAUDE_SOURCE: &str = "claude_cli";

/// Options for `jottrace taste extract`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TasteExtractOptions {
    /// When set, extract only this Claude parent `source_session_id`.
    pub source_session_id: Option<String>,
    /// Re-extract even when rows already exist at the current extractor version.
    pub force: bool,
    /// Override the Claude file-history root (tests and future overrides).
    pub sidecar_history_root: Option<PathBuf>,
}

/// Summary returned after a taste extraction run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasteExtractReport {
    pub db_path: PathBuf,
    pub sessions_processed: u64,
    pub sessions_skipped: u64,
    pub timeline_rows_written: u64,
    pub preference_examples_written: u64,
}

struct SessionTarget {
    db_id: i64,
    source_session_id: String,
    cwd: Option<String>,
}

struct ChildSession {
    source_session_id: String,
}

/// Run the taste extraction pipeline for Claude sessions in the local journal.
pub fn run_taste_extract(options: TasteExtractOptions) -> Result<TasteExtractReport> {
    let data_dir = data_dir_from_env()?;
    taste_extract_for_data_dir(&data_dir, options)
}

/// Run taste extraction against a specific journal directory (tests).
pub fn taste_extract_for_data_dir(
    data_dir: &Path,
    options: TasteExtractOptions,
) -> Result<TasteExtractReport> {
    let db_path = data_dir.join(DB_FILE_NAME);
    let _lock = acquire_data_lock(data_dir)?;
    let mut conn = open_database(&db_path)?;

    let resolver = match &options.sidecar_history_root {
        Some(root) => SnapshotSidecarResolver::with_history_root(root),
        None => SnapshotSidecarResolver::claude_home()?,
    };

    let targets =
        list_parent_claude_sessions(&db_path, &conn, options.source_session_id.as_deref())?;

    let mut report = TasteExtractReport {
        db_path: db_path.clone(),
        sessions_processed: 0,
        sessions_skipped: 0,
        timeline_rows_written: 0,
        preference_examples_written: 0,
    };

    for target in targets {
        if !options.force
            && !session_needs_extract(&db_path, &conn, target.db_id, &target.source_session_id)?
        {
            report.sessions_skipped += 1;
            continue;
        }

        let children = list_child_sessions(&db_path, &conn, target.db_id)?;
        let merged_event_count = count_merged_session_events(&db_path, &conn, target.db_id)?;
        let events = load_merged_session_events(&db_path, &target, &children)?;
        let cwd = target.cwd.as_deref();

        let timeline_rows = FileTimelineMaterializer::materialize(
            CLAUDE_SOURCE,
            &target.source_session_id,
            cwd,
            &resolver,
            &events,
        )?;
        let examples = PreferenceCompiler::compile(
            CLAUDE_SOURCE,
            &target.source_session_id,
            cwd,
            &events,
            &timeline_rows,
        );

        let timeline_count;
        let example_count;
        {
            let tx = conn
                .transaction()
                .map_err(|source| sqlite_error(&db_path, source))?;
            timeline_count = replace_session_file_timelines(
                &db_path,
                &tx,
                CLAUDE_SOURCE,
                &target.source_session_id,
                &timeline_rows,
            )?;
            example_count = replace_session_preference_examples(
                &db_path,
                &tx,
                CLAUDE_SOURCE,
                &target.source_session_id,
                &examples,
            )?;
            replace_taste_extraction_meta(
                &db_path,
                &tx,
                CLAUDE_SOURCE,
                &target.source_session_id,
                merged_event_count,
            )?;
            commit_transaction(&db_path, tx)?;
        }

        report.sessions_processed += 1;
        report.timeline_rows_written += u64::try_from(timeline_count).expect("timeline count");
        report.preference_examples_written += u64::try_from(example_count).expect("example count");
    }

    Ok(report)
}

fn commit_transaction(db_path: &Path, tx: Transaction<'_>) -> Result<()> {
    tx.commit().map_err(|source| sqlite_error(db_path, source))
}

fn list_parent_claude_sessions(
    db_path: &Path,
    conn: &Connection,
    source_session_id: Option<&str>,
) -> Result<Vec<SessionTarget>> {
    let mut statement = conn
        .prepare(
            "SELECT id, source_session_id, cwd
             FROM sessions
             WHERE source = ?1
               AND parent_session_id IS NULL
               AND (?2 IS NULL OR source_session_id = ?2)
             ORDER BY id",
        )
        .map_err(|source| sqlite_error(db_path, source))?;

    let rows = statement
        .query_map(params![CLAUDE_SOURCE, source_session_id], |row| {
            Ok(SessionTarget {
                db_id: row.get(0)?,
                source_session_id: row.get(1)?,
                cwd: row.get(2)?,
            })
        })
        .map_err(|source| sqlite_error(db_path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| sqlite_error(db_path, source))?;

    if let Some(requested) = source_session_id
        && rows.is_empty()
    {
        return Err(JottraceError::SessionNotFound {
            source: CLAUDE_SOURCE.to_string(),
            source_session_id: requested.to_string(),
        });
    }

    Ok(rows)
}

fn list_child_sessions(
    db_path: &Path,
    conn: &Connection,
    parent_db_id: i64,
) -> Result<Vec<ChildSession>> {
    let mut statement = conn
        .prepare(
            "SELECT source_session_id
             FROM sessions
             WHERE source = ?1
               AND parent_session_id = ?2
             ORDER BY id",
        )
        .map_err(|source| sqlite_error(db_path, source))?;

    statement
        .query_map(params![CLAUDE_SOURCE, parent_db_id], |row| {
            Ok(ChildSession {
                source_session_id: row.get(0)?,
            })
        })
        .map_err(|source| sqlite_error(db_path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| sqlite_error(db_path, source))
}

/// Whether a parent session's stored extraction matches the current extractor and event stream.
pub(crate) fn session_extract_is_up_to_date(
    db_path: &Path,
    conn: &Connection,
    parent_db_id: i64,
    source_session_id: &str,
) -> Result<bool> {
    Ok(!session_needs_extract(
        db_path,
        conn,
        parent_db_id,
        source_session_id,
    )?)
}

fn session_needs_extract(
    db_path: &Path,
    conn: &Connection,
    parent_db_id: i64,
    source_session_id: &str,
) -> Result<bool> {
    let current_event_count = count_merged_session_events(db_path, conn, parent_db_id)?;
    let stored = conn
        .query_row(
            "SELECT extractor_version, event_count
             FROM taste_extractions
             WHERE source = ?1 AND source_session_id = ?2",
            params![CLAUDE_SOURCE, source_session_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|source| sqlite_error(db_path, source))?;

    if let Some((version, event_count)) = stored {
        return Ok(version != EXTRACTOR_VERSION
            || u64::try_from(event_count).expect("event_count fits in u64")
                != current_event_count);
    }

    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM preference_examples
             WHERE source = ?1 AND source_session_id = ?2",
            params![CLAUDE_SOURCE, source_session_id],
            |row| row.get(0),
        )
        .map_err(|source| sqlite_error(db_path, source))?;
    if total == 0 {
        return Ok(true);
    }

    let stale: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM preference_examples
             WHERE source = ?1
               AND source_session_id = ?2
               AND extractor_version != ?3",
            params![CLAUDE_SOURCE, source_session_id, EXTRACTOR_VERSION],
            |row| row.get(0),
        )
        .map_err(|source| sqlite_error(db_path, source))?;
    Ok(stale > 0)
}

fn count_merged_session_events(
    db_path: &Path,
    conn: &Connection,
    parent_db_id: i64,
) -> Result<u64> {
    let parent_count: i64 = conn
        .query_row(
            "SELECT event_count FROM sessions WHERE id = ?1",
            params![parent_db_id],
            |row| row.get(0),
        )
        .map_err(|source| sqlite_error(db_path, source))?;

    let child_count: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(event_count), 0)
             FROM sessions
             WHERE source = ?1 AND parent_session_id = ?2",
            params![CLAUDE_SOURCE, parent_db_id],
            |row| row.get(0),
        )
        .map_err(|source| sqlite_error(db_path, source))?;

    let total = parent_count
        .checked_add(child_count)
        .expect("merged session event count fits in i64");
    Ok(u64::try_from(total).expect("merged session event count fits in u64"))
}

fn replace_taste_extraction_meta(
    db_path: &Path,
    conn: &Connection,
    source: &str,
    source_session_id: &str,
    event_count: u64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO taste_extractions (source, source_session_id, extractor_version, event_count)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (source, source_session_id) DO UPDATE SET
             extractor_version = excluded.extractor_version,
             event_count = excluded.event_count",
        params![
            source,
            source_session_id,
            EXTRACTOR_VERSION,
            i64::try_from(event_count).expect("event_count fits in i64"),
        ],
    )
    .map_err(|source| sqlite_error(db_path, source))?;
    Ok(())
}

fn load_merged_session_events(
    db_path: &Path,
    target: &SessionTarget,
    children: &[ChildSession],
) -> Result<Vec<super::parse::ParsedEvent>> {
    let parent_lines = load_session_event_lines(db_path, &target.source_session_id)?;
    let parent_stream = (
        SourceStream::Parent,
        parse_jsonl(&SourceStream::Parent, parent_lines)
            .map_err(|source| invalid_session_json(db_path, &target.source_session_id, source))?,
    );

    let mut streams = vec![parent_stream];
    for child in children {
        let agent_id = subagent_agent_id(&child.source_session_id);
        let source_stream = SourceStream::Subagent {
            agent_id: agent_id.clone(),
        };
        let lines = load_session_event_lines(db_path, &child.source_session_id)?;
        streams.push((
            source_stream.clone(),
            parse_jsonl(&source_stream, lines).map_err(|source| {
                invalid_session_json(db_path, &child.source_session_id, source)
            })?,
        ));
    }

    Ok(merge_streams(streams))
}

fn load_session_event_lines(db_path: &Path, source_session_id: &str) -> Result<Vec<Vec<u8>>> {
    let mut lines = Vec::new();
    for_each_decoded_event_payload_for_session(
        db_path,
        CLAUDE_SOURCE,
        source_session_id,
        None,
        |payload| {
            lines.push(payload.to_vec());
            Ok(())
        },
    )?;
    Ok(lines)
}

fn subagent_agent_id(source_session_id: &str) -> String {
    source_session_id
        .rsplit_once("/subagents/")
        .map(|(_, agent_id)| agent_id.to_string())
        .unwrap_or_else(|| source_session_id.to_string())
}

fn invalid_session_json(
    db_path: &Path,
    source_session_id: &str,
    source: serde_json::Error,
) -> JottraceError {
    JottraceError::Sqlite {
        path: db_path.to_path_buf(),
        source: rusqlite::Error::InvalidParameterName(format!(
            "invalid JSON in claude_cli session {source_session_id}: {source}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_agent_id_extracts_suffix_after_subagents_marker() {
        assert_eq!(
            subagent_agent_id(
                "00000000-0000-4000-8000-000000000031/subagents/agent-taste000000000001"
            ),
            "agent-taste000000000001"
        );
    }

    #[test]
    fn subagent_agent_id_falls_back_to_full_id_without_marker() {
        assert_eq!(
            subagent_agent_id("agent-a000000000000021"),
            "agent-a000000000000021"
        );
    }
}
