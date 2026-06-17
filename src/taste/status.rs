use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

use crate::storage::{DB_FILE_NAME, open_database, sqlite_error};
use crate::{Result, acquire_data_lock, data_dir_from_env};

use super::compiler::{EXTRACTOR_VERSION, HIGH_CONFIDENCE_THRESHOLD};

const CLAUDE_SOURCE: &str = "claude_cli";

/// Outcome class counts from materialized preference rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TasteOutcomeCounts {
    pub accepted: u64,
    pub rejected: u64,
    pub edited: u64,
}

/// Summary of taste extraction coverage in the local journal.
#[derive(Debug, Clone, PartialEq)]
pub struct TasteStatusReport {
    pub db_path: PathBuf,
    pub extractor_version: String,
    pub claude_parent_sessions: u64,
    pub sessions_processed: u64,
    pub sessions_pending: u64,
    pub proposals: u64,
    pub outcomes: TasteOutcomeCounts,
    pub high_confidence_proposals: u64,
    /// Percentage of proposals resolved with high confidence (0–100).
    pub coverage_percent: f64,
}

/// Report taste extraction counts and high-confidence coverage for the local journal.
pub fn run_taste_status() -> Result<TasteStatusReport> {
    let data_dir = data_dir_from_env()?;
    taste_status_for_data_dir(&data_dir)
}

/// Report taste extraction counts for a specific journal directory (tests).
pub fn taste_status_for_data_dir(data_dir: &Path) -> Result<TasteStatusReport> {
    let db_path = data_dir.join(DB_FILE_NAME);
    let _lock = acquire_data_lock(data_dir)?;
    let conn = open_database(&db_path)?;
    taste_status_from_connection(&db_path, &conn)
}

fn taste_status_from_connection(db_path: &Path, conn: &Connection) -> Result<TasteStatusReport> {
    let claude_parent_sessions = count_claude_parent_sessions(db_path, conn)?;
    let sessions_processed = count_sessions_at_extractor_version(db_path, conn)?;
    let sessions_pending = claude_parent_sessions.saturating_sub(sessions_processed);
    let proposals = count_proposals(db_path, conn)?;
    let outcomes = count_outcomes(db_path, conn)?;
    let high_confidence_proposals = count_high_confidence_proposals(db_path, conn)?;
    let coverage_percent = if proposals == 0 {
        0.0
    } else {
        (high_confidence_proposals as f64 / proposals as f64) * 100.0
    };

    Ok(TasteStatusReport {
        db_path: db_path.to_path_buf(),
        extractor_version: EXTRACTOR_VERSION.to_string(),
        claude_parent_sessions,
        sessions_processed,
        sessions_pending,
        proposals,
        outcomes,
        high_confidence_proposals,
        coverage_percent,
    })
}

fn count_claude_parent_sessions(db_path: &Path, conn: &Connection) -> Result<u64> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM sessions
             WHERE source = ?1 AND parent_session_id IS NULL",
            params![CLAUDE_SOURCE],
            |row| row.get(0),
        )
        .map_err(|source| sqlite_error(db_path, source))?;
    Ok(u64::try_from(count).expect("session count fits in u64"))
}

fn count_sessions_at_extractor_version(db_path: &Path, conn: &Connection) -> Result<u64> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT source_session_id)
             FROM preference_examples
             WHERE source = ?1 AND extractor_version = ?2",
            params![CLAUDE_SOURCE, EXTRACTOR_VERSION],
            |row| row.get(0),
        )
        .map_err(|source| sqlite_error(db_path, source))?;
    Ok(u64::try_from(count).expect("processed session count fits in u64"))
}

fn count_proposals(db_path: &Path, conn: &Connection) -> Result<u64> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM preference_examples WHERE source = ?1",
            params![CLAUDE_SOURCE],
            |row| row.get(0),
        )
        .map_err(|source| sqlite_error(db_path, source))?;
    Ok(u64::try_from(count).expect("proposal count fits in u64"))
}

fn count_outcomes(db_path: &Path, conn: &Connection) -> Result<TasteOutcomeCounts> {
    let mut statement = conn
        .prepare(
            "SELECT outcome, COUNT(*)
             FROM preference_examples
             WHERE source = ?1
             GROUP BY outcome",
        )
        .map_err(|source| sqlite_error(db_path, source))?;

    let mut counts = TasteOutcomeCounts::default();
    let rows = statement
        .query_map(params![CLAUDE_SOURCE], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|source| sqlite_error(db_path, source))?;

    for row in rows {
        let (outcome, count) = row.map_err(|source| sqlite_error(db_path, source))?;
        let count = u64::try_from(count).expect("outcome count fits in u64");
        match outcome.as_str() {
            "accepted" => counts.accepted = count,
            "rejected" => counts.rejected = count,
            "edited" => counts.edited = count,
            _ => {}
        }
    }

    Ok(counts)
}

fn count_high_confidence_proposals(db_path: &Path, conn: &Connection) -> Result<u64> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM preference_examples
             WHERE source = ?1 AND confidence >= ?2",
            params![CLAUDE_SOURCE, HIGH_CONFIDENCE_THRESHOLD],
            |row| row.get(0),
        )
        .map_err(|source| sqlite_error(db_path, source))?;
    Ok(u64::try_from(count).expect("high-confidence count fits in u64"))
}
