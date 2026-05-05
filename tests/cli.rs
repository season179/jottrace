mod common;

use common::reader_fixture;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

const CLAUDE_FIXTURE_SESSION: &str = "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021.jsonl";
const CLAUDE_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000021";

#[test]
fn version_prints_package_version() {
    // Run the compiled binary instead of the library so this test covers the
    // real command-line dispatch a user or script will exercise.
    let output = Command::new(binary())
        .arg("--version")
        .output()
        .expect("run jottrace --version");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("jottrace {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn doctor_creates_private_data_dir() {
    let root = temp_root("doctor-ok");
    let data_dir = root.join(".jottrace");

    // JOTTRACE_HOME keeps the integration test hermetic; it must never create
    // or chmod state under the developer's actual HOME.
    let output = Command::new(binary())
        .arg("doctor")
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace doctor");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("jottrace doctor"));
    assert!(stdout.contains("permissions: private (ok)"));

    #[cfg(unix)]
    assert_eq!(mode(&data_dir), 0o700);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn status_reports_empty_fresh_database() {
    let root = temp_root("status-empty");
    let data_dir = root.join(".jottrace");

    let output = Command::new(binary())
        .arg("status")
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace status");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("jottrace status"));
    assert!(stdout.contains("sessions: 0"));
    assert!(stdout.contains("events: 0"));
    assert!(stdout.contains("unresolved_ingest_errors: 0"));

    let db_path = db_path(&data_dir);
    assert!(
        db_path.exists(),
        "status should initialize the local database"
    );

    #[cfg(unix)]
    assert_eq!(mode(&db_path), 0o600);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_is_idempotent_for_unchanged_claude_cli_fixture() {
    let root = temp_root("ingest-idempotent");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 12"));
    assert!(first.contains("inserted_events: 12"));

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 12"));
    assert!(second.contains("inserted_events: 0"));

    let status = Command::new(binary())
        .arg("status")
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace status");

    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("sessions: 1"));
    assert!(stdout.contains("events: 12"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .expect("event count");
    assert_eq!(event_count, 12);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_appends_only_new_complete_claude_cli_lines() {
    let root = temp_root("ingest-append");
    let data_dir = root.join(".jottrace");
    let session_file = install_primary_claude_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 12"));
    assert!(first.contains("inserted_events: 12"));

    let appended_line = first_fixture_line();
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&session_file)
        .expect("open session for append");
    file.write_all(&appended_line).expect("append event line");
    file.write_all(b"\n").expect("append newline");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 13"));
    assert!(second.contains("inserted_events: 1"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .expect("event count");
    let last_seq: i64 = conn
        .query_row("SELECT MAX(seq) FROM events", [], |row| row.get(0))
        .expect("last seq");
    assert_eq!(event_count, 13);
    assert_eq!(last_seq, 12);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_defers_unterminated_claude_cli_tail() {
    let root = temp_root("ingest-partial-tail");
    let data_dir = root.join(".jottrace");
    let session_file = install_primary_claude_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 12"));
    assert!(first.contains("inserted_events: 12"));
    let original_len = fs::metadata(&session_file).expect("session metadata").len() as i64;

    let appended_line = first_fixture_line();
    let partial_len = appended_line.len() / 2;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&session_file)
        .expect("open session for partial append");
    file.write_all(&appended_line[..partial_len])
        .expect("append partial event line");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 12"));
    assert!(second.contains("inserted_events: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (event_count, file_size, next_read_offset): (i64, i64, i64) = conn
        .query_row(
            "SELECT event_count, file_size, next_read_offset
             FROM sessions
             WHERE source = 'claude_cli'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("session offsets");
    assert_eq!(event_count, 12);
    assert_eq!(file_size, original_len + partial_len as i64);
    assert_eq!(next_read_offset, original_len);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_reports_lock_contention_as_clear_cli_failure() {
    let root = temp_root("ingest-lock-held");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let mut lock_file = jottrace::create_private_file(&data_dir.join(jottrace::LOCK_FILE_NAME))
        .expect("create held lock");
    lock_file
        .write_all(b"held by integration test\n")
        .expect("write held lock");

    let output = Command::new(binary())
        .arg("ingest")
        .env("HOME", &root)
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace ingest");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jottrace ingest failed"));
    assert!(stderr.contains("another jottrace DB-mutating command is already running"));
    assert!(stderr.contains(jottrace::LOCK_FILE_NAME));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_claude_cli_fixture_and_status_reports_counts() {
    let root = temp_root("ingest-claude");
    let data_dir = root.join(".jottrace");
    let session_file = install_primary_claude_fixture(&root);

    run_ingest(&root, &data_dir);

    let status = Command::new(binary())
        .arg("status")
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace status");

    assert!(
        status.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );

    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("sessions: 1"));
    assert!(stdout.contains("events: 12"));
    assert!(stdout.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (source_session_id, cwd, started_at, ended_at, event_count, file_size, file_mtime): (
        String,
        String,
        String,
        String,
        i64,
        i64,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT source_session_id, cwd, started_at, ended_at, event_count, file_size, file_mtime
             FROM sessions
             WHERE source = 'claude_cli'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .expect("session metadata");
    assert_eq!(source_session_id, CLAUDE_FIXTURE_SESSION_ID);
    assert_eq!(cwd, "/Users/fixture/Workspace/jottrace");
    assert_eq!(started_at, "2026-05-05T01:00:00.000Z");
    assert_eq!(ended_at, "2026-05-05T01:00:08.000Z");
    assert_eq!(event_count, 12);
    assert_eq!(
        file_size,
        fs::metadata(&session_file).expect("fixture metadata").len() as i64
    );
    assert!(file_mtime.is_some());

    let event_ts: String = conn
        .query_row("SELECT ts FROM events WHERE seq = 1", [], |row| row.get(0))
        .expect("snapshot event timestamp");
    assert_eq!(event_ts, "2026-05-05T01:00:00.000Z");

    let (payload, codec): (Vec<u8>, String) = conn
        .query_row(
            "SELECT payload, codec
             FROM events
             WHERE seq = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("first event payload");
    assert_eq!(codec, "raw");
    assert_eq!(payload, first_fixture_line());

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn doctor_rejects_insecure_existing_data_dir() {
    let root = temp_root("doctor-insecure");
    let data_dir = root.join(".jottrace");
    fs::create_dir_all(&data_dir).expect("create data dir");
    fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o755))
        .expect("set insecure dir mode");

    // This protects the privacy boundary across releases: an existing loose
    // directory must stay visible as a problem instead of being accepted.
    let output = Command::new(binary())
        .arg("doctor")
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace doctor");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jottrace doctor failed"));
    assert!(stderr.contains("expected 700"));

    let _ = fs::remove_dir_all(root);
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_jottrace")
}

fn first_fixture_line() -> Vec<u8> {
    fs::read(reader_fixture(CLAUDE_FIXTURE_SESSION))
        .expect("read fixture")
        .split(|byte| *byte == b'\n')
        .next()
        .expect("first line")
        .to_vec()
}

fn run_ingest(home: &Path, data_dir: &Path) -> String {
    let output = Command::new(binary())
        .arg("ingest")
        .env("HOME", home)
        .env("JOTTRACE_HOME", data_dir)
        .output()
        .expect("run jottrace ingest");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf-8")
}

fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join(jottrace::storage::DB_FILE_NAME)
}

fn install_primary_claude_fixture(root: &Path) -> PathBuf {
    let session_file = root
        .join(".claude/projects/-Users-fixture-Workspace-jottrace")
        .join(format!("{CLAUDE_FIXTURE_SESSION_ID}.jsonl"));
    if let Some(parent) = session_file.parent() {
        fs::create_dir_all(parent).expect("create fixture destination parent");
    }
    fs::copy(reader_fixture(CLAUDE_FIXTURE_SESSION), &session_file).expect("copy fixture");
    session_file
}

fn temp_root(name: &str) -> PathBuf {
    // Cargo can run tests concurrently, so the temp path needs more entropy
    // than the test name alone.
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id()))
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    // Metadata includes file-type bits; masking leaves only chmod-style perms.
    fs::metadata(path).expect("metadata").mode() & 0o777
}
