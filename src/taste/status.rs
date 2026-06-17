use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

use crate::storage::{count, sqlite_error};
use crate::{Result, data_dir_from_env, open_locked_database};

use super::compiler::{EXTRACTOR_VERSION, HIGH_CONFIDENCE_THRESHOLD};
use super::extract::session_extract_is_up_to_date;

const CLAUDE_SOURCE: &str = "claude_cli";

/// Outcome class counts from materialized preference rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TasteOutcomeCounts {
    pub accepted: u64,
    pub rejected: u64,
    pub edited: u64,
}

/// Evidence-kind counts from materialized preference rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TasteEvidenceCounts {
    pub direct_edit: u64,
    pub direct_write: u64,
    pub bash_correlation: u64,
    pub mcp_correlation: u64,
    pub permission_denial: u64,
    pub missing_final_state: u64,
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
    pub evidence: TasteEvidenceCounts,
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
    let (db_path, _lock, conn) = open_locked_database(data_dir)?;
    taste_status_from_connection(&db_path, &conn)
}

fn taste_status_from_connection(db_path: &Path, conn: &Connection) -> Result<TasteStatusReport> {
    let claude_parent_sessions = count_claude_parent_sessions(db_path, conn)?;
    let sessions_processed = count_sessions_up_to_date(db_path, conn)?;
    let sessions_pending = claude_parent_sessions.saturating_sub(sessions_processed);
    let proposals = count_proposals(db_path, conn)?;
    let outcomes = count_outcomes(db_path, conn)?;
    let evidence = count_evidence(db_path, conn)?;
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
        evidence,
        high_confidence_proposals,
        coverage_percent,
    })
}

fn count_claude_parent_sessions(db_path: &Path, conn: &Connection) -> Result<u64> {
    count(
        db_path,
        conn,
        "SELECT COUNT(*)
             FROM sessions
             WHERE source = ?1 AND parent_session_id IS NULL",
        params![CLAUDE_SOURCE],
    )
}

fn count_sessions_up_to_date(db_path: &Path, conn: &Connection) -> Result<u64> {
    let mut statement = conn
        .prepare(
            "SELECT id, source_session_id
             FROM sessions
             WHERE source = ?1 AND parent_session_id IS NULL
             ORDER BY id",
        )
        .map_err(|source| sqlite_error(db_path, source))?;

    let rows = statement
        .query_map(params![CLAUDE_SOURCE], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|source| sqlite_error(db_path, source))?;

    let mut count = 0u64;
    for row in rows {
        let (parent_db_id, source_session_id) =
            row.map_err(|source| sqlite_error(db_path, source))?;
        if session_extract_is_up_to_date(db_path, conn, parent_db_id, &source_session_id)? {
            count += 1;
        }
    }

    Ok(count)
}

fn count_proposals(db_path: &Path, conn: &Connection) -> Result<u64> {
    count(
        db_path,
        conn,
        "SELECT COUNT(*) FROM preference_examples WHERE source = ?1",
        params![CLAUDE_SOURCE],
    )
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

fn count_evidence(db_path: &Path, conn: &Connection) -> Result<TasteEvidenceCounts> {
    let mut statement = conn
        .prepare(
            "SELECT evidence_kind, COUNT(*)
             FROM preference_examples
             WHERE source = ?1
             GROUP BY evidence_kind",
        )
        .map_err(|source| sqlite_error(db_path, source))?;

    let mut counts = TasteEvidenceCounts::default();
    let rows = statement
        .query_map(params![CLAUDE_SOURCE], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|source| sqlite_error(db_path, source))?;

    for row in rows {
        let (evidence_kind, count) = row.map_err(|source| sqlite_error(db_path, source))?;
        let count = u64::try_from(count).expect("evidence count fits in u64");
        match evidence_kind.as_str() {
            "direct_edit" => counts.direct_edit = count,
            "direct_write" => counts.direct_write = count,
            "bash_correlation" => counts.bash_correlation = count,
            "mcp_correlation" => counts.mcp_correlation = count,
            "permission_denial" => counts.permission_denial = count,
            "missing_final_state" => counts.missing_final_state = count,
            _ => {}
        }
    }

    Ok(counts)
}

fn count_high_confidence_proposals(db_path: &Path, conn: &Connection) -> Result<u64> {
    count(
        db_path,
        conn,
        "SELECT COUNT(*)
             FROM preference_examples
             WHERE source = ?1 AND confidence >= ?2",
        params![CLAUDE_SOURCE, HIGH_CONFIDENCE_THRESHOLD],
    )
}
