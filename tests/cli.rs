mod common;

use common::reader_fixture;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

const CLAUDE_FIXTURE_SESSION: &str = "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021.jsonl";
const CLAUDE_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000021";
const CLAUDE_SIDECHAIN_FIXTURE_SESSION: &str = "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021/subagents/agent-a000000000000021.jsonl";
const CLAUDE_SIDECHAIN_FIXTURE_META: &str = "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021/subagents/agent-a000000000000021.meta.json";
const CLAUDE_SIDECHAIN_FIXTURE_SESSION_ID: &str = "agent-a000000000000021";
const CORRUPT_FIXTURE_SESSION: &str = "edge-cases/corrupt-line.jsonl";
const CORRUPT_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000000";
const UNREADABLE_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000001";

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
fn doctor_reports_unresolved_ingest_errors() {
    let root = temp_root("doctor-ingest-errors");
    let data_dir = root.join(".jottrace");
    install_claude_fixture(&root, CORRUPT_FIXTURE_SESSION_ID, CORRUPT_FIXTURE_SESSION);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

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
    assert!(stdout.contains("unresolved_ingest_errors: 1"));
    assert!(stdout.contains(CORRUPT_FIXTURE_SESSION_ID));
    assert!(stdout.contains("line: 2"));
    assert!(stdout.contains("kind: invalid_json"));
    assert!(stdout.contains("message: "));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_corrupt_jsonl_without_blocking_unrelated_files() {
    let root = temp_root("ingest-corrupt-nonblocking");
    let data_dir = root.join(".jottrace");
    let corrupt_file =
        install_claude_fixture(&root, CORRUPT_FIXTURE_SESSION_ID, CORRUPT_FIXTURE_SESSION);
    install_primary_claude_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 13"));
    assert!(ingest.contains("inserted_events: 13"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

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
    assert!(stdout.contains("sessions: 2"));
    assert!(stdout.contains("events: 13"));
    assert!(stdout.contains("unresolved_ingest_errors: 1"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let valid_event_count: i64 = conn
        .query_row(
            "SELECT event_count
             FROM sessions
             WHERE source_session_id = ?1",
            [CLAUDE_FIXTURE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("valid session event count");
    assert_eq!(valid_event_count, 12);

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    let byte_offset = error.byte_offset.expect("error byte offset");
    assert_eq!(error.source, "claude_cli");
    assert_eq!(
        error.source_session_id.as_deref(),
        Some(CORRUPT_FIXTURE_SESSION_ID)
    );
    assert_eq!(error.file_path, corrupt_file);
    assert!(byte_offset > 0);
    assert_eq!(error.line_number, Some(2));
    assert_eq!(error.error_kind, "invalid_json");
    assert!(!error.message.is_empty());
    assert!(!error.first_seen_at.is_empty());
    assert!(!error.last_seen_at.is_empty());
    assert_eq!(error.occurrence_count, 1);

    let next_read_offset: i64 = conn
        .query_row(
            "SELECT next_read_offset
             FROM sessions
             WHERE source_session_id = ?1",
            [CORRUPT_FIXTURE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("corrupt session offset");
    assert_eq!(next_read_offset, byte_offset);

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn ingest_records_unreadable_file_without_blocking_unrelated_files() {
    let root = temp_root("ingest-unreadable-nonblocking");
    let data_dir = root.join(".jottrace");
    let unreadable_file =
        install_claude_fixture(&root, UNREADABLE_FIXTURE_SESSION_ID, CLAUDE_FIXTURE_SESSION);
    install_primary_claude_fixture(&root);
    fs::set_permissions(&unreadable_file, fs::Permissions::from_mode(0o000))
        .expect("make fixture unreadable");

    if fs::File::open(&unreadable_file).is_ok() {
        let _ = fs::remove_dir_all(root);
        return;
    }

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 12"));
    assert!(ingest.contains("inserted_events: 12"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "claude_cli");
    assert_eq!(
        error.source_session_id.as_deref(),
        Some(UNREADABLE_FIXTURE_SESSION_ID)
    );
    assert_eq!(error.file_path, unreadable_file);
    assert_eq!(error.byte_offset, None);
    assert_eq!(error.line_number, None);
    assert_eq!(error.error_kind, "read_error");
    assert!(!error.message.is_empty());

    let valid_event_count: i64 = Connection::open(db_path(&data_dir))
        .expect("open preserved db")
        .query_row(
            "SELECT event_count
             FROM sessions
             WHERE source_session_id = ?1",
            [CLAUDE_FIXTURE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("valid session event count");
    assert_eq!(valid_event_count, 12);

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
fn ingest_preserves_truncated_claude_cli_file_as_next_generation() {
    let root = temp_root("ingest-truncation-generation");
    let data_dir = root.join(".jottrace");
    let session_file = install_claude_fixture(
        &root,
        "issue-25-truncation",
        "edge-cases/truncation-before.jsonl",
    );

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 4"));
    assert!(first.contains("inserted_events: 4"));

    replace_with_fixture(&session_file, "edge-cases/truncation-after.jsonl");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 6"));
    assert!(second.contains("inserted_events: 2"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (current_generation, event_count, file_size, next_read_offset): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT current_generation, event_count, file_size, next_read_offset
             FROM sessions
             WHERE source = 'claude_cli'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("session rewrite state");
    let rewritten_len = fs::metadata(&session_file)
        .expect("rewritten metadata")
        .len() as i64;
    assert_eq!(current_generation, 1);
    assert_eq!(event_count, 2);
    assert_eq!(file_size, rewritten_len);
    assert_eq!(next_read_offset, rewritten_len);
    assert_eq!(generation_counts(&conn), vec![(0, 4), (1, 2)]);
    assert!(event_payload(&conn, 0, 3).contains("Complete before truncation."));
    assert!(event_payload(&conn, 1, 1).contains("Line one after truncation."));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_same_size_claude_cli_rewrite_as_next_generation() {
    let root = temp_root("ingest-same-size-rewrite");
    let data_dir = root.join(".jottrace");
    let session_file = install_claude_fixture(
        &root,
        "issue-25-same-size",
        "edge-cases/same-size-rewrite-before.jsonl",
    );
    set_modified(&session_file, 1_700_000_000);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 2"));
    assert!(first.contains("inserted_events: 2"));
    let original_len = fs::metadata(&session_file)
        .expect("original metadata")
        .len();

    replace_with_fixture(&session_file, "edge-cases/same-size-rewrite-after.jsonl");
    set_modified(&session_file, 1_700_000_100);

    assert_eq!(
        fs::metadata(&session_file)
            .expect("rewritten metadata")
            .len(),
        original_len
    );
    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 4"));
    assert!(second.contains("inserted_events: 2"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let current_generation: i64 = conn
        .query_row(
            "SELECT current_generation FROM sessions WHERE source = 'claude_cli'",
            [],
            |row| row.get(0),
        )
        .expect("current generation");
    assert_eq!(current_generation, 1);
    assert_eq!(generation_counts(&conn), vec![(0, 2), (1, 2)]);
    assert!(event_payload(&conn, 0, 1).contains("alpha"));
    assert!(event_payload(&conn, 1, 1).contains("bravo"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_skips_unchanged_same_size_claude_cli_file_after_fingerprint_check() {
    let root = temp_root("ingest-same-size-unchanged");
    let data_dir = root.join(".jottrace");
    let session_file = install_claude_fixture(
        &root,
        "issue-25-same-size-unchanged",
        "edge-cases/same-size-rewrite-before.jsonl",
    );
    set_modified(&session_file, 1_700_000_000);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 2"));
    assert!(first.contains("inserted_events: 2"));

    set_modified(&session_file, 1_700_000_100);

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 2"));
    assert!(second.contains("inserted_events: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (current_generation, file_mtime): (i64, i64) = conn
        .query_row(
            "SELECT current_generation, file_mtime
             FROM sessions
             WHERE source = 'claude_cli'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("same-size skip state");
    assert_eq!(current_generation, 0);
    assert_eq!(file_mtime, 1_700_000_100);
    assert_eq!(generation_counts(&conn), vec![(0, 2)]);

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

#[test]
fn ingest_preserves_claude_cli_sidechain_as_child_session() {
    let root = temp_root("ingest-claude-sidechain");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let sidechain_file = install_claude_sidechain_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 19"));
    assert!(ingest.contains("inserted_events: 19"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let sidechain = sidechain_session(&conn);
    assert_eq!(
        sidechain.source_session_id,
        CLAUDE_SIDECHAIN_FIXTURE_SESSION_ID
    );
    assert!(sidechain.parent_session_id.is_some());
    assert_eq!(
        sidechain.parent_source_session_id.as_deref(),
        Some(CLAUDE_FIXTURE_SESSION_ID)
    );
    assert_eq!(sidechain.event_count, 7);
    assert_eq!(
        sidechain.cwd.as_deref(),
        Some("/Users/fixture/Workspace/jottrace")
    );
    assert_eq!(
        sidechain.started_at.as_deref(),
        Some("2026-05-05T01:01:00.000Z")
    );
    assert_eq!(
        sidechain.ended_at.as_deref(),
        Some("2026-05-05T01:01:06.000Z")
    );
    assert_eq!(sidechain.file_path, sidechain_file);

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 2"));
    assert!(second.contains("events: 19"));
    assert!(second.contains("inserted_events: 0"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_claude_cli_sidechain_without_parent_until_parent_is_available() {
    let root = temp_root("ingest-claude-sidechain-late-parent");
    let data_dir = root.join(".jottrace");
    let sidechain_file = install_claude_sidechain_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("sessions: 1"));
    assert!(first.contains("events: 7"));
    assert!(first.contains("inserted_events: 7"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let sidechain = sidechain_session(&conn);
    assert_eq!(sidechain.parent_session_id, None);
    assert_eq!(sidechain.parent_source_session_id, None);
    assert_eq!(sidechain.event_count, 7);
    assert_eq!(sidechain.file_path, sidechain_file);
    drop(conn);

    install_primary_claude_fixture(&root);

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 2"));
    assert!(second.contains("events: 19"));
    assert!(second.contains("inserted_events: 12"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let sidechain = sidechain_session(&conn);
    assert_eq!(
        sidechain.parent_source_session_id.as_deref(),
        Some(CLAUDE_FIXTURE_SESSION_ID)
    );

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

struct SidechainSession {
    source_session_id: String,
    parent_session_id: Option<i64>,
    parent_source_session_id: Option<String>,
    event_count: i64,
    cwd: Option<String>,
    started_at: Option<String>,
    ended_at: Option<String>,
    file_path: PathBuf,
}

fn sidechain_session(conn: &Connection) -> SidechainSession {
    conn.query_row(
        "SELECT child.source_session_id, child.parent_session_id, parent.source_session_id,
                child.event_count, child.cwd, child.started_at, child.ended_at, child.file_path
         FROM sessions child
         LEFT JOIN sessions parent ON parent.id = child.parent_session_id
         WHERE child.source = 'claude_cli'
           AND child.source_session_id = ?1",
        [CLAUDE_SIDECHAIN_FIXTURE_SESSION_ID],
        |row| {
            let file_path: String = row.get(7)?;
            Ok(SidechainSession {
                source_session_id: row.get(0)?,
                parent_session_id: row.get(1)?,
                parent_source_session_id: row.get(2)?,
                event_count: row.get(3)?,
                cwd: row.get(4)?,
                started_at: row.get(5)?,
                ended_at: row.get(6)?,
                file_path: PathBuf::from(file_path),
            })
        },
    )
    .expect("sidechain session")
}

fn install_primary_claude_fixture(root: &Path) -> PathBuf {
    install_claude_fixture(root, CLAUDE_FIXTURE_SESSION_ID, CLAUDE_FIXTURE_SESSION)
}

fn install_claude_sidechain_fixture(root: &Path) -> PathBuf {
    let sidechain_file = claude_project_dir(root)
        .join(CLAUDE_FIXTURE_SESSION_ID)
        .join("subagents")
        .join(format!("{CLAUDE_SIDECHAIN_FIXTURE_SESSION_ID}.jsonl"));
    let sidechain_meta = sidechain_file.with_extension("meta.json");
    copy_reader_fixture(CLAUDE_SIDECHAIN_FIXTURE_SESSION, &sidechain_file);
    copy_reader_fixture(CLAUDE_SIDECHAIN_FIXTURE_META, &sidechain_meta);
    sidechain_file
}

fn install_claude_fixture(root: &Path, session_id: &str, fixture_relative: &str) -> PathBuf {
    let session_file = claude_project_dir(root).join(format!("{session_id}.jsonl"));
    copy_reader_fixture(fixture_relative, &session_file);
    session_file
}

fn replace_with_fixture(path: &Path, fixture_relative: &str) {
    copy_reader_fixture(fixture_relative, path);
}

fn claude_project_dir(root: &Path) -> PathBuf {
    root.join(".claude/projects/-Users-fixture-Workspace-jottrace")
}

fn copy_reader_fixture(fixture_relative: &str, destination: &Path) {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("create fixture destination parent");
    }
    fs::copy(reader_fixture(fixture_relative), destination).expect("copy fixture");
}

fn set_modified(path: &Path, unix_seconds: u64) {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open fixture for mtime update");
    let times = fs::FileTimes::new()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(unix_seconds));
    file.set_times(times).expect("set fixture mtime");
}

fn generation_counts(conn: &Connection) -> Vec<(i64, i64)> {
    let mut statement = conn
        .prepare(
            "SELECT generation, COUNT(*)
             FROM events
             GROUP BY generation
             ORDER BY generation",
        )
        .expect("prepare generation counts");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query generation counts")
        .map(|row| row.expect("generation count"))
        .collect()
}

fn event_payload(conn: &Connection, generation: i64, seq: i64) -> String {
    let payload: Vec<u8> = conn
        .query_row(
            "SELECT payload
             FROM events
             WHERE generation = ?1 AND seq = ?2",
            [generation, seq],
            |row| row.get(0),
        )
        .expect("event payload");
    String::from_utf8(payload).expect("fixture payload should be utf-8")
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
