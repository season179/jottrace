use rusqlite::{Connection, Row, params};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{JottraceError, Result, data_dir_from_env, ensure_private_file};

pub const DB_FILE_NAME: &str = "db.sqlite";
pub const LATEST_SCHEMA_VERSION: i64 = 3;
pub(crate) const RAW_CODEC: &str = "raw";

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

pub fn open_database(path: &Path) -> Result<Connection> {
    ensure_private_file(path)?;

    let mut conn = Connection::open(path).map_err(|source| sqlite_error(path, source))?;
    configure_connection(path, &conn)?;
    run_migrations(path, &mut conn)?;
    Ok(conn)
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
        return Err(JottraceError::UnsupportedSchemaVersion {
            path: path.to_path_buf(),
            actual: current,
            supported: LATEST_SCHEMA_VERSION,
        });
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
        session_count: count(path, conn, "SELECT COUNT(*) FROM sessions")?,
        event_count: count(path, conn, "SELECT COUNT(*) FROM events")?,
        unresolved_ingest_error_count: unresolved_ingest_error_count_from_connection(path, conn)?,
    })
}

pub(crate) fn unresolved_ingest_error_count_from_connection(
    path: &Path,
    conn: &Connection,
) -> Result<u64> {
    count(path, conn, UNRESOLVED_INGEST_ERROR_COUNT_SQL)
}

pub(crate) fn unresolved_ingest_errors_from_connection(
    path: &Path,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<IngestErrorSummary>> {
    let mut statement = conn
        .prepare(
            "SELECT source, source_session_id, file_path, generation, byte_offset,
                    line_number, error_kind, message, first_seen_at, last_seen_at,
                    occurrence_count
             FROM ingest_errors
             WHERE resolved_at IS NULL
             ORDER BY last_seen_at DESC, id DESC
             LIMIT ?1",
        )
        .map_err(|source| sqlite_error(path, source))?;

    statement
        .query_map(params![limit as i64], ingest_error_summary_from_row)
        .map_err(|source| sqlite_error(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| sqlite_error(path, source))
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

fn count(path: &Path, conn: &Connection, sql: &str) -> Result<u64> {
    let value: i64 = conn
        .query_row(sql, [], |row| row.get(0))
        .map_err(|source| sqlite_error(path, source))?;
    Ok(value as u64)
}

fn user_version(path: &Path, conn: &Connection) -> Result<i64> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| sqlite_error(path, source))
}

pub(crate) fn sqlite_error(path: &Path, source: rusqlite::Error) -> JottraceError {
    JottraceError::Sqlite {
        path: path.to_path_buf(),
        source,
    }
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
        assert_sql_object(&conn, "index", "idx_sessions_source_session_id");
        assert_sql_object(&conn, "index", "idx_sessions_source_started_at");
        assert_sql_object(&conn, "index", "idx_events_session_ts");
        assert_sql_object(&conn, "index", "idx_ingest_errors_unresolved");
        assert_sql_object(&conn, "index", "idx_ingest_errors_unresolved_last_seen");

        assert!(columns(&conn, "sessions").contains(&"id".to_string()));
        assert!(columns(&conn, "sessions").contains(&"source_session_id".to_string()));
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

    fn assert_sql_object(conn: &Connection, kind: &str, name: &str) {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
                [kind, name],
                |row| row.get(0),
            )
            .expect("query sqlite_master");
        assert_eq!(found, 1, "{kind} {name} should exist");
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

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id()))
    }
}
