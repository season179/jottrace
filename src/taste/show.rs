use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

use crate::JottraceError;
use crate::storage::{DB_FILE_NAME, open_database, sqlite_error};
use crate::{Result, acquire_data_lock, data_dir_from_env};

use super::timeline::{FileTimelineRow, TimelineSourceKind, normalize_file_path};

const CLAUDE_SOURCE: &str = "claude_cli";

/// Options for `jottrace taste show timeline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasteShowTimelineOptions {
    pub source_session_id: String,
    pub file_path: String,
}

/// Reconstructed per-file timeline rows for one Claude session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasteTimelineShowReport {
    pub db_path: PathBuf,
    pub source_session_id: String,
    pub file_path: String,
    pub rows: Vec<FileTimelineRow>,
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
    let db_path = data_dir.join(DB_FILE_NAME);
    let _lock = acquire_data_lock(data_dir)?;
    let conn = open_database(&db_path)?;
    load_timeline(&db_path, &conn, options)
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

fn query_timeline_rows(
    db_path: &Path,
    conn: &Connection,
    source: &str,
    source_session_id: &str,
    file_path: &str,
) -> Result<Vec<FileTimelineRow>> {
    let mut statement = conn
        .prepare(
            "SELECT seq, event_seq, content, trigger_event_ref, source_kind
             FROM file_timelines
             WHERE source = ?1 AND source_session_id = ?2 AND file_path = ?3
             ORDER BY seq ASC",
        )
        .map_err(|source| sqlite_error(db_path, source))?;

    let rows = statement
        .query_map(params![source, source_session_id, file_path], |row| {
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
        })
        .map_err(|source| sqlite_error(db_path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| sqlite_error(db_path, source))?;

    Ok(rows)
}
