use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

use crate::JottraceError;
use crate::storage::{query_collect, sqlite_error};
use crate::{Result, data_dir_from_env, open_locked_database};

use super::compiler::PreferenceExample;
use super::timeline::{FileTimelineRow, TimelineSourceKind, normalize_file_path};

const CLAUDE_SOURCE: &str = "claude_cli";

/// Options for `jottrace taste show timeline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasteShowTimelineOptions {
    pub source_session_id: String,
    pub file_path: String,
}

/// Options for `jottrace taste show example`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasteShowExampleOptions {
    pub tool_use_id: String,
    pub source_session_id: Option<String>,
}

/// One labeled preference example loaded from the journal.
#[derive(Debug, Clone, PartialEq)]
pub struct TasteExampleShowReport {
    pub db_path: PathBuf,
    pub example: PreferenceExample,
}

/// Reconstructed per-file timeline rows for one Claude session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasteTimelineShowReport {
    pub db_path: PathBuf,
    pub source_session_id: String,
    pub file_path: String,
    pub rows: Vec<FileTimelineRow>,
}

/// Load and return one preference example for the local journal.
pub fn run_taste_show_example(options: TasteShowExampleOptions) -> Result<TasteExampleShowReport> {
    let data_dir = data_dir_from_env()?;
    show_example_for_data_dir(&data_dir, options)
}

/// Load a preference example from a specific journal directory (tests).
pub fn show_example_for_data_dir(
    data_dir: &Path,
    options: TasteShowExampleOptions,
) -> Result<TasteExampleShowReport> {
    let (db_path, _lock, conn) = open_locked_database(data_dir)?;
    load_example(&db_path, &conn, options)
}

/// Load and return the materialized file timeline for the local journal.
pub fn run_taste_show_timeline(
    options: TasteShowTimelineOptions,
) -> Result<TasteTimelineShowReport> {
    let data_dir = data_dir_from_env()?;
    show_timeline_for_data_dir(&data_dir, options)
}

/// Load a file timeline from a specific journal directory (tests).
pub fn show_timeline_for_data_dir(
    data_dir: &Path,
    options: TasteShowTimelineOptions,
) -> Result<TasteTimelineShowReport> {
    let (db_path, _lock, conn) = open_locked_database(data_dir)?;
    load_timeline(&db_path, &conn, options)
}

fn load_example(
    db_path: &Path,
    conn: &Connection,
    options: TasteShowExampleOptions,
) -> Result<TasteExampleShowReport> {
    let example = query_preference_example(
        db_path,
        conn,
        CLAUDE_SOURCE,
        &options.tool_use_id,
        options.source_session_id.as_deref(),
    )?;
    Ok(TasteExampleShowReport {
        db_path: db_path.to_path_buf(),
        example,
    })
}

fn load_timeline(
    db_path: &Path,
    conn: &Connection,
    options: TasteShowTimelineOptions,
) -> Result<TasteTimelineShowReport> {
    let cwd = lookup_session_cwd(db_path, conn, &options.source_session_id)?;
    let file_path = normalize_file_path(&options.file_path, cwd.as_deref());
    let rows = query_timeline_rows(
        db_path,
        conn,
        CLAUDE_SOURCE,
        &options.source_session_id,
        &file_path,
    )?;
    if rows.is_empty() {
        return Err(JottraceError::TimelineNotFound {
            source_session_id: options.source_session_id,
            file_path,
        });
    }

    Ok(TasteTimelineShowReport {
        db_path: db_path.to_path_buf(),
        source_session_id: options.source_session_id,
        file_path,
        rows,
    })
}

fn lookup_session_cwd(
    db_path: &Path,
    conn: &Connection,
    source_session_id: &str,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT cwd
         FROM sessions
         WHERE source = ?1
           AND source_session_id = ?2
           AND parent_session_id IS NULL",
        params![CLAUDE_SOURCE, source_session_id],
        |row| row.get(0),
    )
    .map_err(|source| match source {
        rusqlite::Error::QueryReturnedNoRows => JottraceError::SessionNotFound {
            source: CLAUDE_SOURCE.to_string(),
            source_session_id: source_session_id.to_string(),
        },
        source => sqlite_error(db_path, source),
    })
}

fn query_preference_example(
    db_path: &Path,
    conn: &Connection,
    source: &str,
    tool_use_id: &str,
    source_session_id: Option<&str>,
) -> Result<PreferenceExample> {
    let rows = match source_session_id {
        Some(source_session_id) => query_collect(
            db_path,
            conn,
            "SELECT source, source_session_id, generation, proposal_event_seq, tool_use_id,
                    file_path, tool_name, proposal_content, context, outcome, confidence,
                    evidence_kind, extractor_version
             FROM preference_examples
             WHERE source = ?1 AND source_session_id = ?2 AND tool_use_id = ?3",
            params![source, source_session_id, tool_use_id],
            PreferenceExample::from_row,
        )?,
        None => query_collect(
            db_path,
            conn,
            "SELECT source, source_session_id, generation, proposal_event_seq, tool_use_id,
                    file_path, tool_name, proposal_content, context, outcome, confidence,
                    evidence_kind, extractor_version
             FROM preference_examples
             WHERE source = ?1 AND tool_use_id = ?2
             ORDER BY source_session_id ASC",
            params![source, tool_use_id],
            PreferenceExample::from_row,
        )?,
    };

    match rows.len() {
        0 => Err(JottraceError::ExampleNotFound {
            tool_use_id: tool_use_id.to_string(),
        }),
        1 => Ok(rows.into_iter().next().expect("one row")),
        _ => Err(JottraceError::AmbiguousExample {
            tool_use_id: tool_use_id.to_string(),
            session_count: rows.len(),
        }),
    }
}

fn query_timeline_rows(
    db_path: &Path,
    conn: &Connection,
    source: &str,
    source_session_id: &str,
    file_path: &str,
) -> Result<Vec<FileTimelineRow>> {
    query_collect(
        db_path,
        conn,
        "SELECT seq, event_seq, content, trigger_event_ref, source_kind
         FROM file_timelines
         WHERE source = ?1 AND source_session_id = ?2 AND file_path = ?3
         ORDER BY seq ASC",
        params![source, source_session_id, file_path],
        |row| {
            let seq: i64 = row.get(0)?;
            let event_seq: i64 = row.get(1)?;
            let source_kind: String = row.get(4)?;
            Ok(FileTimelineRow {
                source: source.to_string(),
                source_session_id: source_session_id.to_string(),
                file_path: file_path.to_string(),
                seq: usize::try_from(seq).expect("seq fits in usize"),
                event_seq: usize::try_from(event_seq).expect("event_seq fits in usize"),
                content: row.get(2)?,
                trigger_event_ref: row.get(3)?,
                source_kind: TimelineSourceKind::from_db_str(&source_kind)
                    .expect("valid source_kind"),
            })
        },
    )
}
