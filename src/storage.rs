use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, params};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{
    JottraceError, Result, data_dir_from_env, ensure_private_file, unsupported_schema_version,
};

pub const DB_FILE_NAME: &str = "db.sqlite";
pub const LATEST_SCHEMA_VERSION: i64 = 13;
pub(crate) const RAW_CODEC: &str = "raw";
pub(crate) const ZSTD_CODEC: &str = "zstd";
/// Minimum source payload size considered for zstd. Keeping this named makes
/// future corpus/fixture tuning deliberate instead of hidden in insert logic.
pub(crate) const ZSTD_MIN_PAYLOAD_BYTES: usize = 1024;

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("migrations/001_initial_schema.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("migrations/002_ingest_error_recency_index.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("migrations/003_claude_sidechain_source_ids.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("migrations/004_unsupported_event_codec_index.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("migrations/005_supported_zstd_codec_index.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("migrations/006_raw_event_compaction_index.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("migrations/007_session_source_file_path_index.sql"),
    },
    Migration {
        version: 8,
        sql: include_str!("migrations/008_session_source_metadata.sql"),
    },
    Migration {
        version: 9,
        sql: include_str!("migrations/009_session_prefix_fingerprint.sql"),
    },
    Migration {
        version: 10,
        sql: include_str!("migrations/010_taste_extraction.sql"),
    },
    Migration {
        version: 11,
        sql: include_str!("migrations/011_preference_examples.sql"),
    },
    Migration {
        version: 12,
        sql: include_str!("migrations/012_preference_examples_mcp_evidence.sql"),
    },
    Migration {
        version: 13,
        sql: include_str!("migrations/013_taste_extractions.sql"),
    },
];
const UNRESOLVED_INGEST_ERROR_COUNT_SQL: &str =
    "SELECT COUNT(*) FROM ingest_errors WHERE resolved_at IS NULL";

#[derive(Debug)]
struct Migration {
    version: i64,
    sql: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub db_path: PathBuf,
    pub schema_version: i64,
    pub session_count: u64,
    pub event_count: u64,
    pub unresolved_ingest_error_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestErrorSummary {
    pub source: String,
    pub source_session_id: Option<String>,
    pub file_path: PathBuf,
    pub generation: Option<i64>,
    pub byte_offset: Option<i64>,
    pub line_number: Option<i64>,
    pub error_kind: String,
    pub message: String,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub occurrence_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncodedEventPayload {
    pub payload: Vec<u8>,
    pub codec: &'static str,
    pub payload_size: usize,
}

pub fn db_path_from_env() -> Result<PathBuf> {
    Ok(data_dir_from_env()?.join(DB_FILE_NAME))
}

pub fn run_status() -> Result<StatusReport> {
    let db_path = db_path_from_env()?;
    status_for_path(&db_path)
}

pub fn status_for_path(path: &Path) -> Result<StatusReport> {
    let conn = open_database(path)?;
    status_from_connection(path, &conn)
}

pub fn unresolved_ingest_errors_for_path(
    path: &Path,
    limit: usize,
) -> Result<Vec<IngestErrorSummary>> {
    let conn = open_database(path)?;
    unresolved_ingest_errors_from_connection(path, &conn, limit)
}

pub fn decode_event_payload(payload: &[u8], codec: &str) -> Result<Vec<u8>> {
    if codec == RAW_CODEC {
        return Ok(payload.to_vec());
    }
    if codec == ZSTD_CODEC {
        return zstd::stream::decode_all(payload).map_err(zstd_codec_error);
    }

    Err(unsupported_event_payload_codec(codec))
}

pub(crate) fn decode_event_payload_prefix(
    payload: &[u8],
    codec: &str,
    byte_limit: usize,
) -> Result<Vec<u8>> {
    if codec == RAW_CODEC {
        return Ok(payload[..payload.len().min(byte_limit)].to_vec());
    }
    if codec == ZSTD_CODEC {
        let decoder = zstd::stream::read::Decoder::new(payload).map_err(zstd_codec_error)?;
        let mut decoder = decoder.take(byte_limit as u64);
        let mut decoded = Vec::with_capacity(byte_limit);
        decoder
            .read_to_end(&mut decoded)
            .map_err(zstd_codec_error)?;
        return Ok(decoded);
    }

    Err(unsupported_event_payload_codec(codec))
}

pub(crate) fn encode_event_payload(payload: &[u8]) -> Result<EncodedEventPayload> {
    if payload.len() < ZSTD_MIN_PAYLOAD_BYTES {
        return Ok(raw_event_payload(payload));
    }

    let compressed = zstd::stream::encode_all(payload, 0).map_err(zstd_codec_error)?;
    if compressed.len() < payload.len() {
        return Ok(EncodedEventPayload {
            payload: compressed,
            codec: ZSTD_CODEC,
            payload_size: payload.len(),
        });
    }

    Ok(raw_event_payload(payload))
}

fn raw_event_payload(payload: &[u8]) -> EncodedEventPayload {
    EncodedEventPayload {
        payload: payload.to_vec(),
        codec: RAW_CODEC,
        payload_size: payload.len(),
    }
}

pub fn open_database(path: &Path) -> Result<Connection> {
    ensure_private_file(path)?;

    let mut conn = Connection::open(path).map_err(|source| sqlite_error(path, source))?;
    configure_connection(path, &conn)?;
    run_migrations(path, &mut conn)?;
    Ok(conn)
}

/// Open a SQLite database read-only, reporting a failed open through
/// `make_error`.
///
/// Foreign session stores (and jottrace's own journal in the web server) are
/// only ever read, so they are opened with `SQLITE_OPEN_READ_ONLY` rather than
/// through [`open_database`], which migrates and writes. Callers pass a
/// source-specific error constructor because the same open failure is labelled
/// differently per store.
pub(crate) fn open_readonly_database(
    path: &Path,
    make_error: fn(&Path, rusqlite::Error) -> JottraceError,
) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| make_error(path, source))
}

fn configure_connection(path: &Path, conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|source| sqlite_error(path, source))?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;",
    )
    .map_err(|source| sqlite_error(path, source))
}

fn run_migrations(path: &Path, conn: &mut Connection) -> Result<()> {
    let current = user_version(path, conn)?;

    if current > LATEST_SCHEMA_VERSION {
        return Err(unsupported_schema_version(
            path,
            current,
            LATEST_SCHEMA_VERSION,
        ));
    }

    if current == LATEST_SCHEMA_VERSION {
        return Ok(());
    }

    let tx = conn
        .transaction()
        .map_err(|source| sqlite_error(path, source))?;

    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version > current)
    {
        tx.execute_batch(migration.sql)
            .map_err(|source| sqlite_error(path, source))?;
        tx.pragma_update(None, "user_version", migration.version)
            .map_err(|source| sqlite_error(path, source))?;
    }

    tx.commit().map_err(|source| sqlite_error(path, source))
}

pub(crate) fn status_from_connection(path: &Path, conn: &Connection) -> Result<StatusReport> {
    Ok(StatusReport {
        db_path: path.to_path_buf(),
        schema_version: user_version(path, conn)?,
        session_count: count(path, conn, "SELECT COUNT(*) FROM sessions", [])?,
        event_count: count(path, conn, "SELECT COUNT(*) FROM events", [])?,
        unresolved_ingest_error_count: unresolved_ingest_error_count_from_connection(path, conn)?,
    })
}

pub(crate) fn unresolved_ingest_error_count_from_connection(
    path: &Path,
    conn: &Connection,
) -> Result<u64> {
    count(path, conn, UNRESOLVED_INGEST_ERROR_COUNT_SQL, [])
}

pub(crate) fn unresolved_ingest_errors_from_connection(
    path: &Path,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<IngestErrorSummary>> {
    query_collect(
        path,
        conn,
        "SELECT source, source_session_id, file_path, generation, byte_offset,
                line_number, error_kind, message, first_seen_at, last_seen_at,
                occurrence_count
         FROM ingest_errors
         WHERE resolved_at IS NULL
         ORDER BY last_seen_at DESC, id DESC
         LIMIT ?1",
        params![limit as i64],
        ingest_error_summary_from_row,
    )
}

pub fn for_each_decoded_event_payload_for_session(
    path: &Path,
    source: &str,
    source_session_id: &str,
    limit: Option<i64>,
    visit: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    let conn = open_database(path)?;
    let session_id =
        event_session_id(path, &conn, source, source_session_id)?.ok_or_else(|| {
            JottraceError::SessionNotFound {
                source: source.to_string(),
                source_session_id: source_session_id.to_string(),
            }
        })?;
    for_each_decoded_event_payload_from_connection(path, &conn, session_id, limit, visit)
}

fn event_session_id(
    path: &Path,
    conn: &Connection,
    source: &str,
    source_session_id: &str,
) -> Result<Option<i64>> {
    query_optional(
        path,
        conn,
        "SELECT id
         FROM sessions
         WHERE source = ?1
           AND source_session_id = ?2",
        params![source, source_session_id],
        |row| row.get(0),
    )
}

fn for_each_decoded_event_payload_from_connection(
    path: &Path,
    conn: &Connection,
    session_id: i64,
    limit: Option<i64>,
    mut visit: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    if let Some(limit) = limit {
        validate_decoded_event_limit(limit)?;
    }
    reject_unsupported_event_codecs(
        path,
        conn,
        session_id,
        selected_event_upper_bound(path, conn, session_id, limit)?,
    )?;

    let sql = match limit {
        Some(_) => {
            "SELECT events.payload, events.codec
             FROM events
             WHERE events.session_id = ?1
             ORDER BY events.generation, events.seq
             LIMIT ?2"
        }
        None => {
            "SELECT events.payload, events.codec
             FROM events
             WHERE events.session_id = ?1
             ORDER BY events.generation, events.seq"
        }
    };
    let mut statement = conn
        .prepare(sql)
        .map_err(|source| sqlite_error(path, source))?;
    let mut rows = match limit {
        Some(limit) => statement
            .query(params![session_id, limit])
            .map_err(|source| sqlite_error(path, source))?,
        None => statement
            .query(params![session_id])
            .map_err(|source| sqlite_error(path, source))?,
    };

    while let Some(row) = rows.next().map_err(|source| sqlite_error(path, source))? {
        let payload: Vec<u8> = row_value(path, row, 0)?;
        let codec: String = row_value(path, row, 1)?;
        if codec == RAW_CODEC {
            visit(&payload)?;
        } else {
            let decoded = decode_event_payload(&payload, &codec)?;
            visit(&decoded)?;
        }
    }

    Ok(())
}

fn reject_unsupported_event_codecs(
    path: &Path,
    conn: &Connection,
    session_id: i64,
    upper_bound: Option<(i64, i64)>,
) -> Result<()> {
    let (bound_generation, bound_seq) = match upper_bound {
        Some((generation, seq)) => (Some(generation), Some(seq)),
        None => (None, None),
    };
    let codec: Option<String> = query_optional(
        path,
        conn,
        "SELECT events.codec
             FROM events
             WHERE events.session_id = ?1
               AND events.codec NOT IN (?2, ?3)
               AND (
                   ?4 IS NULL
                   OR events.generation < ?4
                   OR (events.generation = ?4 AND events.seq <= ?5)
               )
             ORDER BY events.generation, events.seq
             LIMIT 1",
        params![
            session_id,
            RAW_CODEC,
            ZSTD_CODEC,
            bound_generation,
            bound_seq
        ],
        |row| row.get(0),
    )?;

    match codec {
        Some(codec) => Err(unsupported_event_payload_codec(&codec)),
        None => Ok(()),
    }
}

fn selected_event_upper_bound(
    path: &Path,
    conn: &Connection,
    session_id: i64,
    limit: Option<i64>,
) -> Result<Option<(i64, i64)>> {
    let Some(limit) = limit else {
        return Ok(None);
    };

    let offset = limit - 1;
    query_optional(
        path,
        conn,
        "SELECT generation, seq
         FROM events
         WHERE session_id = ?1
         ORDER BY generation, seq
         LIMIT 1 OFFSET ?2",
        params![session_id, offset],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

fn unsupported_event_payload_codec(codec: &str) -> JottraceError {
    JottraceError::UnsupportedEventPayloadCodec {
        codec: codec.to_string(),
    }
}

fn zstd_codec_error(source: std::io::Error) -> JottraceError {
    JottraceError::EventPayloadCodec {
        codec: ZSTD_CODEC.to_string(),
        source,
    }
}

fn validate_decoded_event_limit(limit: i64) -> Result<()> {
    if limit >= 1 {
        return Ok(());
    }

    Err(JottraceError::InvalidEventLimit { limit })
}

fn ingest_error_summary_from_row(row: &Row<'_>) -> rusqlite::Result<IngestErrorSummary> {
    let file_path: String = row.get("file_path")?;
    let occurrence_count: i64 = row.get("occurrence_count")?;
    Ok(IngestErrorSummary {
        source: row.get("source")?,
        source_session_id: row.get("source_session_id")?,
        file_path: PathBuf::from(file_path),
        generation: row.get("generation")?,
        byte_offset: row.get("byte_offset")?,
        line_number: row.get("line_number")?,
        error_kind: row.get("error_kind")?,
        message: row.get("message")?,
        first_seen_at: row.get("first_seen_at")?,
        last_seen_at: row.get("last_seen_at")?,
        occurrence_count: occurrence_count as u64,
    })
}

pub(crate) fn count(
    path: &Path,
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<u64> {
    let value: i64 = query_one(path, conn, sql, params, |row| row.get(0))?;
    Ok(value as u64)
}

fn user_version(path: &Path, conn: &Connection) -> Result<i64> {
    query_one(path, conn, "PRAGMA user_version", [], |row| row.get(0))
}

pub(crate) fn sqlite_error(path: &Path, source: rusqlite::Error) -> JottraceError {
    JottraceError::Sqlite {
        path: path.to_path_buf(),
        source,
    }
}

/// Prepare `sql`, run it with `params`, and collect every mapped row into a `Vec`,
/// routing each fallible step's failure through [`sqlite_error`] for `path`.
pub(crate) fn query_collect<T, P, F>(
    path: &Path,
    conn: &Connection,
    sql: &str,
    params: P,
    mapper: F,
) -> Result<Vec<T>>
where
    P: rusqlite::Params,
    F: FnMut(&Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = conn
        .prepare(sql)
        .map_err(|source| sqlite_error(path, source))?;
    statement
        .query_map(params, mapper)
        .map_err(|source| sqlite_error(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| sqlite_error(path, source))
}

/// Run `sql` with `params`, returning the single mapped row if present (`Ok(None)` when
/// no row matches), routing query failures through [`sqlite_error`] for `path`.
pub(crate) fn query_optional<T, P, F>(
    path: &Path,
    conn: &Connection,
    sql: &str,
    params: P,
    mapper: F,
) -> Result<Option<T>>
where
    P: rusqlite::Params,
    F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
{
    conn.query_row(sql, params, mapper)
        .optional()
        .map_err(|source| sqlite_error(path, source))
}

/// Run `sql` with `params`, returning the single mapped row and routing query
/// failures (including no matching row) through [`sqlite_error`] for `path`.
pub(crate) fn query_one<T, P, F>(
    path: &Path,
    conn: &Connection,
    sql: &str,
    params: P,
    mapper: F,
) -> Result<T>
where
    P: rusqlite::Params,
    F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
{
    conn.query_row(sql, params, mapper)
        .map_err(|source| sqlite_error(path, source))
}

/// Execute `sql` with `params`, returning the number of affected rows and
/// routing failures through [`sqlite_error`] for `path`.
pub(crate) fn execute_sql<P>(path: &Path, conn: &Connection, sql: &str, params: P) -> Result<usize>
where
    P: rusqlite::Params,
{
    conn.execute(sql, params)
        .map_err(|source| sqlite_error(path, source))
}

/// Read column `idx` from `row`, routing an access/decode failure through
/// [`sqlite_error`] for `path`. The manual row-streaming loops (whose bodies run
/// in the crate `Result` rather than a `rusqlite::Result` mapper closure) use
/// this to map each column read without repeating the `sqlite_error` closure.
pub(crate) fn row_value<T>(path: &Path, row: &Row<'_>, idx: usize) -> Result<T>
where
    T: rusqlite::types::FromSql,
{
    row.get(idx).map_err(|source| sqlite_error(path, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn fresh_database_runs_initial_migration() {
        let root = temp_root("storage-fresh");
        let db_path = root.join(DB_FILE_NAME);

        let conn = open_database(&db_path).expect("open database");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert_sql_object(&conn, "table", "sessions");
        assert_sql_object(&conn, "table", "events");
        assert_sql_object(&conn, "table", "ingest_errors");
        assert_sql_object(&conn, "table", "file_timelines");
        assert_sql_object(&conn, "table", "preference_examples");
        assert_sql_object(&conn, "index", "idx_sessions_source_session_id");
        assert_sql_object(&conn, "index", "idx_sessions_source_started_at");
        assert_sql_object(&conn, "index", "idx_events_session_ts");
        assert_sql_object(&conn, "index", "idx_ingest_errors_unresolved");
        assert_sql_object(&conn, "index", "idx_ingest_errors_unresolved_last_seen");
        assert_sql_object(&conn, "index", "idx_events_unsupported_codec");
        assert_sql_object(&conn, "index", "idx_events_raw_compaction");
        assert_sql_object(&conn, "index", "idx_sessions_source_file_path");
        assert!(
            index_sql(&conn, "idx_events_unsupported_codec")
                .contains("codec NOT IN ('raw', 'zstd')")
        );
        assert!(index_sql(&conn, "idx_events_raw_compaction").contains("codec = 'raw'"));
        assert!(
            index_sql(&conn, "idx_sessions_source_file_path").contains("file_path IS NOT NULL")
        );

        assert!(columns(&conn, "sessions").contains(&"id".to_string()));
        assert!(columns(&conn, "sessions").contains(&"source_session_id".to_string()));
        assert!(columns(&conn, "sessions").contains(&"source_metadata".to_string()));
        assert!(columns(&conn, "events").contains(&"generation".to_string()));
        assert!(columns(&conn, "events").contains(&"payload".to_string()));
        assert!(columns(&conn, "events").contains(&"codec".to_string()));
        assert!(columns(&conn, "ingest_errors").contains(&"resolved_at".to_string()));

        let report = status_from_connection(&db_path, &conn).expect("status");
        assert_eq!(report.session_count, 0);
        assert_eq!(report.event_count, 0);
        assert_eq!(report.unresolved_ingest_error_count, 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_existing_v0_database_to_latest_version() {
        let root = temp_root("storage-upgrade");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v0 db");
            conn.execute("CREATE TABLE v0_marker (id INTEGER PRIMARY KEY)", [])
                .expect("create marker");
            conn.execute("INSERT INTO v0_marker DEFAULT VALUES", [])
                .expect("insert marker");
            conn.pragma_update(None, "user_version", 0)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert_sql_object(&conn, "table", "sessions");
        let marker_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM v0_marker", [], |row| row.get(0))
            .expect("marker count");
        assert_eq!(marker_count, 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_existing_v1_database_to_latest_version() {
        let root = temp_root("storage-upgrade-v1");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v1 db");
            conn.execute_batch(include_str!("migrations/001_initial_schema.sql"))
                .expect("create v1 schema");
            conn.execute(
                "INSERT INTO sessions (source, source_session_id)
                 VALUES ('claude_cli', 's1')",
                [],
            )
            .expect("insert session");
            conn.execute(
                "INSERT INTO ingest_errors
                    (source, source_session_id, session_id, file_path, line_number, error_kind, message)
                 VALUES ('claude_cli', 's1', 1, '/tmp/s1.jsonl', 1, 'invalid_json', 'bad line')",
                [],
            )
            .expect("insert v1 ingest error");
            conn.pragma_update(None, "user_version", 1)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert_sql_object(&conn, "index", "idx_ingest_errors_unresolved_last_seen");
        let line_number: i64 = conn
            .query_row("SELECT line_number FROM ingest_errors", [], |row| {
                row.get(0)
            })
            .expect("migrated line number");
        assert_eq!(line_number, 2);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_v2_claude_sidechain_source_session_ids_to_parent_qualified() {
        let root = temp_root("storage-upgrade-v2-sidechain-ids");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");
        let parent_source_session_id = "00000000-0000-4000-8000-000000000021";
        let old_sidechain_source_session_id = "agent-a000000000000021";
        let new_sidechain_source_session_id =
            "00000000-0000-4000-8000-000000000021/subagents/agent-a000000000000021";
        let sidechain_file_path = "/Users/fixture/subagents/archive/.claude/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021/subagents/agent-a000000000000021.jsonl";
        let unlinked_old_sidechain_source_session_id = "agent-a000000000000022";
        let unlinked_new_sidechain_source_session_id =
            "11111111-1111-4111-8111-111111111111/subagents/agent-a000000000000022";
        let unlinked_sidechain_file_path = "/Users/fixture/subagents/archive/.claude/projects/-Users-fixture-Workspace-jottrace/11111111-1111-4111-8111-111111111111/subagents/agent-a000000000000022.jsonl";
        let non_uuid_parent_source_session_id = "agent-a000000000000023";
        let non_uuid_parent_file_path =
            "/Users/fixture/.claude/projects/subagents/agent-a000000000000023.jsonl";

        {
            let conn = Connection::open(&db_path).expect("open v2 db");
            conn.execute_batch(include_str!("migrations/001_initial_schema.sql"))
                .expect("create v1 schema");
            conn.execute_batch(include_str!(
                "migrations/002_ingest_error_recency_index.sql"
            ))
            .expect("create v2 schema");
            conn.execute(
                "INSERT INTO sessions (source, source_session_id, file_path)
                 VALUES ('claude_cli', ?1, '/Users/fixture/.claude/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021.jsonl')",
                [parent_source_session_id],
            )
            .expect("insert parent session");
            conn.execute(
                "INSERT INTO sessions (source, source_session_id, file_path, parent_session_id)
                 VALUES ('claude_cli', ?1, ?2, 1)",
                [old_sidechain_source_session_id, sidechain_file_path],
            )
            .expect("insert old sidechain session");
            conn.execute(
                "INSERT INTO sessions (source, source_session_id, file_path)
                 VALUES ('claude_cli', ?1, ?2)",
                [
                    unlinked_old_sidechain_source_session_id,
                    unlinked_sidechain_file_path,
                ],
            )
            .expect("insert unlinked old sidechain session");
            conn.execute(
                "INSERT INTO sessions (source, source_session_id, file_path)
                 VALUES ('claude_cli', ?1, ?2)",
                [non_uuid_parent_source_session_id, non_uuid_parent_file_path],
            )
            .expect("insert non-uuid parent session");
            conn.execute(
                "INSERT INTO ingest_errors
                    (source, source_session_id, session_id, file_path, line_number, error_kind, message)
                 VALUES ('claude_cli', ?1, 2, ?2, 1, 'invalid_json', 'bad line')",
                [old_sidechain_source_session_id, sidechain_file_path],
            )
            .expect("insert old sidechain ingest error");
            conn.pragma_update(None, "user_version", 2)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        let migrated_source_session_id: String = conn
            .query_row(
                "SELECT source_session_id FROM sessions WHERE id = 2",
                [],
                |row| row.get(0),
            )
            .expect("migrated sidechain source id");
        assert_eq!(migrated_source_session_id, new_sidechain_source_session_id);
        let migrated_error_source_session_id: String = conn
            .query_row(
                "SELECT source_session_id FROM ingest_errors WHERE session_id = 2",
                [],
                |row| row.get(0),
            )
            .expect("migrated ingest error source id");
        assert_eq!(
            migrated_error_source_session_id,
            new_sidechain_source_session_id
        );
        let unlinked_migrated_source_session_id: String = conn
            .query_row(
                "SELECT source_session_id FROM sessions WHERE id = 3",
                [],
                |row| row.get(0),
            )
            .expect("migrated unlinked sidechain source id");
        assert_eq!(
            unlinked_migrated_source_session_id,
            unlinked_new_sidechain_source_session_id
        );
        let non_uuid_parent_migrated_source_session_id: String = conn
            .query_row(
                "SELECT source_session_id FROM sessions WHERE id = 4",
                [],
                |row| row.get(0),
            )
            .expect("non-uuid parent source id");
        assert_eq!(
            non_uuid_parent_migrated_source_session_id,
            non_uuid_parent_source_session_id
        );
        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_v4_unsupported_codec_index_to_exclude_supported_zstd_rows() {
        let root = temp_root("storage-upgrade-v4-codec-index");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v4 db");
            apply_migrations_through(&conn, 4);
            assert!(index_sql(&conn, "idx_events_unsupported_codec").contains("codec != 'raw'"));
            conn.pragma_update(None, "user_version", 4)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert!(
            index_sql(&conn, "idx_events_unsupported_codec")
                .contains("codec NOT IN ('raw', 'zstd')")
        );
        assert!(index_sql(&conn, "idx_events_raw_compaction").contains("codec = 'raw'"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_v5_database_to_add_raw_compaction_index() {
        let root = temp_root("storage-upgrade-v5-raw-compaction-index");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v5 db");
            apply_migrations_through(&conn, 5);
            assert!(!sql_object_exists(
                &conn,
                "index",
                "idx_events_raw_compaction"
            ));
            conn.pragma_update(None, "user_version", 5)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert!(index_sql(&conn, "idx_events_raw_compaction").contains("codec = 'raw'"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_v6_database_to_add_session_source_file_path_index() {
        let root = temp_root("storage-upgrade-v6-session-file-path-index");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v6 db");
            apply_migrations_through(&conn, 6);
            assert!(!sql_object_exists(
                &conn,
                "index",
                "idx_sessions_source_file_path"
            ));
            conn.pragma_update(None, "user_version", 6)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert!(
            index_sql(&conn, "idx_sessions_source_file_path").contains("file_path IS NOT NULL")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_v7_database_to_add_session_source_metadata() {
        let root = temp_root("storage-upgrade-v7-source-metadata");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v7 db");
            apply_migrations_through(&conn, 7);
            assert!(
                index_sql(&conn, "idx_sessions_source_file_path").contains("file_path IS NOT NULL")
            );
            assert!(!columns(&conn, "sessions").contains(&"source_metadata".to_string()));
            conn.pragma_update(None, "user_version", 7)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert!(columns(&conn, "sessions").contains(&"source_metadata".to_string()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_v9_database_to_add_file_timelines_table() {
        let root = temp_root("storage-upgrade-v9-file-timelines");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v9 db");
            apply_migrations_through(&conn, 9);
            assert!(!sql_object_exists(&conn, "table", "file_timelines"));
            conn.pragma_update(None, "user_version", 9)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert_sql_object(&conn, "table", "file_timelines");
        assert_sql_object(&conn, "index", "idx_file_timelines_session_file");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_v10_database_to_add_preference_examples_table() {
        let root = temp_root("storage-upgrade-v10-preference-examples");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v10 db");
            apply_migrations_through(&conn, 10);
            assert!(!sql_object_exists(&conn, "table", "preference_examples"));
            conn.pragma_update(None, "user_version", 10)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert_sql_object(&conn, "table", "preference_examples");
        assert_sql_object(&conn, "index", "idx_preference_examples_session");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_v11_database_to_allow_mcp_correlation_evidence_kind() {
        let root = temp_root("storage-upgrade-v11-mcp-evidence");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v11 db");
            apply_migrations_through(&conn, 11);
            conn.execute(
                "INSERT INTO preference_examples (
                    source, source_session_id, generation, proposal_event_seq,
                    tool_use_id, file_path, tool_name, proposal_content, context,
                    outcome, confidence, evidence_kind, extractor_version
                 ) VALUES (
                    'claude_cli', 'sess', 0, 1, 'tool-mcp', 'src/a.rs',
                    'mcp_fixture_edit', 'content', NULL, 'accepted', 0.6,
                    'bash_correlation', '0.1.0'
                 )",
                [],
            )
            .expect("seed preference row");
            conn.pragma_update(None, "user_version", 11)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );

        conn.execute(
            "INSERT INTO preference_examples (
                source, source_session_id, generation, proposal_event_seq,
                tool_use_id, file_path, tool_name, proposal_content, context,
                outcome, confidence, evidence_kind, extractor_version
             ) VALUES (
                'claude_cli', 'sess', 1, 2, 'tool-mcp-2', 'src/b.rs',
                'mcp_fixture_edit', 'content', NULL, 'accepted', 0.6,
                'mcp_correlation', '0.1.1'
             )",
            [],
        )
        .expect("insert mcp_correlation row");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_v12_database_to_add_taste_extractions_table() {
        let root = temp_root("storage-upgrade-v12-taste-extractions");
        let db_path = root.join(DB_FILE_NAME);
        ensure_private_file(&db_path).expect("create db file");

        {
            let conn = Connection::open(&db_path).expect("open v12 db");
            apply_migrations_through(&conn, 12);
            assert!(!sql_object_exists(&conn, "table", "taste_extractions"));
            conn.pragma_update(None, "user_version", 12)
                .expect("set user_version");
        }

        let conn = open_database(&db_path).expect("migrate db");

        assert_eq!(
            user_version(&db_path, &conn).expect("user_version"),
            LATEST_SCHEMA_VERSION
        );
        assert_sql_object(&conn, "table", "taste_extractions");

        conn.execute(
            "INSERT INTO taste_extractions (source, source_session_id, extractor_version, event_count)
             VALUES ('claude_cli', 'sess', '0.1.6', 42)",
            [],
        )
        .expect("insert taste extraction row");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn status_counts_unresolved_ingest_errors_only() {
        let root = temp_root("storage-status");
        let db_path = root.join(DB_FILE_NAME);
        let conn = open_database(&db_path).expect("open database");

        conn.execute(
            "INSERT INTO sessions (source, source_session_id) VALUES ('claude_cli', 's1')",
            [],
        )
        .expect("insert session");
        conn.execute(
            "INSERT INTO events (session_id, generation, seq, payload, codec, payload_size)
             VALUES (1, 0, 0, x'7B7D', 'raw', 2)",
            [],
        )
        .expect("insert event");
        conn.execute(
            "INSERT INTO ingest_errors
                (source, source_session_id, session_id, file_path, error_kind, message)
             VALUES ('claude_cli', 's1', 1, '/tmp/s1.jsonl', 'invalid_json', 'bad line')",
            [],
        )
        .expect("insert unresolved error");
        conn.execute(
            "INSERT INTO ingest_errors
                (source, source_session_id, session_id, file_path, error_kind, message, resolved_at)
             VALUES ('claude_cli', 's1', 1, '/tmp/s1.jsonl', 'invalid_json', 'fixed', '2026-05-05T00:00:00Z')",
            [],
        )
        .expect("insert resolved error");

        let report = status_from_connection(&db_path, &conn).expect("status");
        assert_eq!(report.session_count, 1);
        assert_eq!(report.event_count, 1);
        assert_eq!(report.unresolved_ingest_error_count, 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn decode_event_payload_returns_raw_payload_bytes() {
        let payload = br#"{"type":"event_msg"}"#;

        let decoded = decode_event_payload(payload, RAW_CODEC).expect("decode raw payload");

        assert_eq!(decoded, payload);
    }

    #[test]
    fn decode_event_payload_returns_zstd_payload_bytes() {
        let payload = br#"{"type":"event_msg","message":"compressible payload"}"#;
        let encoded = zstd::stream::encode_all(&payload[..], 0).expect("encode zstd payload");

        let decoded = decode_event_payload(&encoded, ZSTD_CODEC).expect("decode zstd payload");

        assert_eq!(decoded, payload);
    }

    #[test]
    fn decode_event_payload_prefix_returns_bounded_zstd_payload_bytes() {
        let payload = vec![b'x'; 4096];
        let encoded = zstd::stream::encode_all(&payload[..], 0).expect("encode zstd payload");

        let decoded =
            decode_event_payload_prefix(&encoded, ZSTD_CODEC, 1024).expect("decode zstd prefix");

        assert_eq!(decoded, payload[..1024]);
    }

    #[test]
    fn decode_event_payload_rejects_unknown_codec() {
        let error =
            decode_event_payload(br#"{"type":"event_msg"}"#, "snappy").expect_err("codec error");

        assert_eq!(error.to_string(), "unsupported event payload codec: snappy");
    }

    #[test]
    fn encode_event_payload_keeps_subthreshold_payload_raw() {
        let payload = vec![b'x'; ZSTD_MIN_PAYLOAD_BYTES - 1];

        let encoded = encode_event_payload(&payload).expect("encode event payload");

        assert_eq!(encoded.codec, RAW_CODEC);
        assert_eq!(encoded.payload, payload);
        assert_eq!(encoded.payload_size, ZSTD_MIN_PAYLOAD_BYTES - 1);
    }

    #[test]
    fn encode_event_payload_uses_zstd_when_compressed_payload_is_smaller() {
        let payload = vec![b'x'; ZSTD_MIN_PAYLOAD_BYTES];

        let encoded = encode_event_payload(&payload).expect("encode event payload");

        assert_eq!(encoded.codec, ZSTD_CODEC);
        assert!(encoded.payload.len() < payload.len());
        assert_eq!(encoded.payload_size, payload.len());
        assert_eq!(
            decode_event_payload(&encoded.payload, encoded.codec).expect("decode event payload"),
            payload
        );
    }

    #[test]
    fn encode_event_payload_keeps_incompressible_payload_raw() {
        let payload = pseudo_random_bytes(4096);

        let encoded = encode_event_payload(&payload).expect("encode event payload");

        assert_eq!(encoded.codec, RAW_CODEC);
        assert_eq!(encoded.payload, payload);
        assert_eq!(encoded.payload_size, 4096);
    }

    fn pseudo_random_bytes(len: usize) -> Vec<u8> {
        let mut state = 0x4d59_5df4_d0f3_3173_u64;
        let mut output = Vec::with_capacity(len);
        for _ in 0..len {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            output.push(state as u8);
        }
        output
    }

    fn assert_sql_object(conn: &Connection, kind: &str, name: &str) {
        assert!(
            sql_object_exists(conn, kind, name),
            "{kind} {name} should exist"
        );
    }

    fn apply_migrations_through(conn: &Connection, version: i64) {
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= version)
        {
            conn.execute_batch(migration.sql)
                .unwrap_or_else(|error| panic!("apply migration {}: {error}", migration.version));
        }
    }

    fn sql_object_exists(conn: &Connection, kind: &str, name: &str) -> bool {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
                [kind, name],
                |row| row.get(0),
            )
            .expect("query sqlite_master");
        found == 1
    }

    fn columns(conn: &Connection, table: &str) -> Vec<String> {
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare table_info");
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query columns")
            .map(|row| row.expect("column name"))
            .collect()
    }

    fn index_sql(conn: &Connection, name: &str) -> String {
        conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .expect("index sql")
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id()))
    }
}
