use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{JottraceError, Result, data_dir_from_env, ensure_private_file};

pub const DB_FILE_NAME: &str = "db.sqlite";
pub const LATEST_SCHEMA_VERSION: i64 = 1;

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("migrations/001_initial_schema.sql"),
}];

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

fn status_from_connection(path: &Path, conn: &Connection) -> Result<StatusReport> {
    Ok(StatusReport {
        db_path: path.to_path_buf(),
        schema_version: user_version(path, conn)?,
        session_count: count(path, conn, "SELECT COUNT(*) FROM sessions")?,
        event_count: count(path, conn, "SELECT COUNT(*) FROM events")?,
        unresolved_ingest_error_count: count(
            path,
            conn,
            "SELECT COUNT(*) FROM ingest_errors WHERE resolved_at IS NULL",
        )?,
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

fn sqlite_error(path: &Path, source: rusqlite::Error) -> JottraceError {
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
