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
const OTHER_CLAUDE_FIXTURE_SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
const CODEX_NESTED_FIXTURE_SESSION: &str = "codex-cli/sessions/2026/05/05/rollout-2026-05-05T09-00-00-00000000-0000-4000-8000-000000000021.jsonl";
const CODEX_NESTED_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000021";
const CODEX_ARCHIVED_FIXTURE_SESSION: &str = "codex-cli/archived_sessions/rollout-2026-03-28T10-42-29-00000000-0000-4000-8000-000000000021.jsonl";
const CODEX_ARCHIVED_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000121";
const CODEX_LEGACY_FIXTURE_SESSION_ID: &str = "22222222-2222-4222-8222-222222222222";
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
fn top_level_help_aliases_print_the_same_usage() {
    let long = Command::new(binary())
        .arg("--help")
        .output()
        .expect("run jottrace --help");
    let short = Command::new(binary())
        .arg("-h")
        .output()
        .expect("run jottrace -h");

    assert!(long.status.success());
    assert!(short.status.success());
    assert!(long.stderr.is_empty());
    assert!(short.stderr.is_empty());
    assert_eq!(long.stdout, short.stdout);

    let stdout = String::from_utf8_lossy(&long.stdout);
    assert!(stdout.contains("Usage:"));
    assert!(stdout.contains("jottrace doctor"));
    assert!(stdout.contains("jottrace web [--port <port>] [--once]"));
    assert!(stdout.contains("jottrace <command> --help"));
}

#[test]
fn doctor_help_aliases_print_command_help_without_initializing_database() {
    let root = temp_root("doctor-help");
    let data_dir = root.join(".jottrace");

    for help_arg in ["-h", "--help"] {
        let output = Command::new(binary())
            .args(["doctor", help_arg])
            .env("JOTTRACE_HOME", &data_dir)
            .output()
            .expect("run jottrace doctor help");

        assert!(
            output.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("jottrace doctor"));
        assert!(stdout.contains("Usage:"));
        assert!(stdout.contains("Check the journal directory"));
        assert!(
            !db_path(&data_dir).exists(),
            "help should not initialize the local database"
        );
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn subcommand_help_aliases_print_command_specific_usage() {
    let cases = [
        ("doctor", "Check the journal directory"),
        ("ingest", "Preserve Claude and Codex JSONL sessions"),
        ("status", "Print journal schema"),
        ("compact", "--batch-size <n>"),
        ("events", "--source <source>"),
        ("web", "--port <port>"),
    ];

    for (command, expected_detail) in cases {
        let long = Command::new(binary())
            .args([command, "--help"])
            .output()
            .unwrap_or_else(|error| panic!("run jottrace {command} --help: {error}"));
        let short = Command::new(binary())
            .args([command, "-h"])
            .output()
            .unwrap_or_else(|error| panic!("run jottrace {command} -h: {error}"));

        assert!(
            long.status.success(),
            "{command} --help stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&long.stdout),
            String::from_utf8_lossy(&long.stderr)
        );
        assert!(
            short.status.success(),
            "{command} -h stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&short.stdout),
            String::from_utf8_lossy(&short.stderr)
        );
        assert_eq!(
            long.stdout, short.stdout,
            "{command} help aliases should match"
        );
        assert!(long.stderr.is_empty(), "{command} help should not warn");

        let stdout = String::from_utf8_lossy(&long.stdout);
        assert!(stdout.contains(&format!("jottrace {command}")));
        assert!(stdout.contains("Usage:"));
        assert!(
            stdout.contains(expected_detail),
            "{command} help:\n{stdout}"
        );
    }
}

#[test]
fn unknown_subcommand_options_exit_with_targeted_help_hint() {
    for command in ["doctor", "ingest", "status", "compact", "events", "web"] {
        let root = temp_root(&format!("{command}-unknown-option"));
        let data_dir = root.join(".jottrace");
        let output = Command::new(binary())
            .args([command, "--definitely-not-an-option"])
            .env("JOTTRACE_HOME", &data_dir)
            .output()
            .unwrap_or_else(|error| {
                panic!("run jottrace {command} --definitely-not-an-option: {error}")
            });

        assert_eq!(
            output.status.code(),
            Some(2),
            "{command} unknown option stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty(),
            "{command} should not print stdout"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!(
                "unknown {command} option: --definitely-not-an-option"
            )),
            "{command} stderr:\n{stderr}"
        );
        assert!(
            stderr.contains(&format!("jottrace {command} --help")),
            "{command} stderr:\n{stderr}"
        );
        assert!(
            !db_path(&data_dir).exists(),
            "usage errors should not initialize the database"
        );

        let _ = fs::remove_dir_all(root);
    }
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
fn ingest_preserves_nested_codex_cli_session() {
    let root = temp_root("ingest-codex-nested");
    let data_dir = root.join(".jottrace");
    let session_file = install_nested_codex_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 13"));
    assert!(ingest.contains("inserted_events: 13"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (source_session_id, cwd, started_at, ended_at, event_count, file_path): (
        String,
        String,
        String,
        String,
        i64,
        String,
    ) = conn
        .query_row(
            "SELECT source_session_id, cwd, started_at, ended_at, event_count, file_path
             FROM sessions
             WHERE source = 'codex_cli'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .expect("codex session metadata");
    assert_eq!(source_session_id, CODEX_NESTED_FIXTURE_SESSION_ID);
    assert_eq!(cwd, "/Users/fixture/Workspace/jottrace");
    assert_eq!(started_at, "2026-05-05T09:00:00.000Z");
    assert_eq!(ended_at, "2026-05-05T09:00:12.000Z");
    assert_eq!(event_count, 13);
    assert_eq!(PathBuf::from(file_path), session_file);

    let max_seq: i64 = conn
        .query_row(
            "SELECT MAX(seq)
             FROM events
             JOIN sessions ON sessions.id = events.session_id
             WHERE sessions.source = 'codex_cli'",
            [],
            |row| row.get(0),
        )
        .expect("codex max seq");
    assert_eq!(max_seq, 12);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_archived_codex_cli_session() {
    let root = temp_root("ingest-codex-archived");
    let data_dir = root.join(".jottrace");
    let session_file = install_archived_codex_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 6"));
    assert!(ingest.contains("inserted_events: 6"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (source_session_id, event_count, file_path): (String, i64, String) = conn
        .query_row(
            "SELECT source_session_id, event_count, file_path
             FROM sessions
             WHERE source = 'codex_cli'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("archived codex session metadata");
    assert_eq!(source_session_id, CODEX_ARCHIVED_FIXTURE_SESSION_ID);
    assert_eq!(event_count, 6);
    assert_eq!(PathBuf::from(file_path), session_file);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_uses_codex_session_meta_id_not_rollout_filename() {
    let root = temp_root("ingest-codex-meta-id");
    let data_dir = root.join(".jottrace");
    let session_file = install_nested_codex_fixture_at(
        &root,
        "rollout-2026-05-05T09-00-00-99999999-9999-4999-8999-999999999999.jsonl",
    );

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 13"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (source_session_id, file_path): (String, String) = conn
        .query_row(
            "SELECT source_session_id, file_path
             FROM sessions
             WHERE source = 'codex_cli'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("codex session identity");
    assert_eq!(source_session_id, CODEX_NESTED_FIXTURE_SESSION_ID);
    assert_eq!(PathBuf::from(file_path), session_file);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_updates_codex_file_path_when_rollout_moves_to_archive() {
    let root = temp_root("ingest-codex-archive-move");
    let data_dir = root.join(".jottrace");
    let live_file = install_nested_codex_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("sessions: 1"));
    assert!(first.contains("events: 13"));
    assert!(first.contains("inserted_events: 13"));

    let archived_file = root
        .join(".codex/archived_sessions")
        .join("rollout-2026-05-05T09-00-00-00000000-0000-4000-8000-000000000021.jsonl");
    fs::create_dir_all(archived_file.parent().expect("archived parent"))
        .expect("create archived parent");
    fs::rename(&live_file, &archived_file).expect("move rollout to archive");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 1"));
    assert!(second.contains("events: 13"));
    assert!(second.contains("inserted_events: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (session_count, event_count): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(DISTINCT sessions.id), COUNT(events.seq)
             FROM sessions
             JOIN events ON events.session_id = sessions.id
             WHERE sessions.source = 'codex_cli'
               AND sessions.source_session_id = ?1",
            [CODEX_NESTED_FIXTURE_SESSION_ID],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("codex moved session counts");
    assert_eq!(session_count, 1);
    assert_eq!(event_count, 13);

    let file_path: String = conn
        .query_row(
            "SELECT file_path
             FROM sessions
             WHERE source = 'codex_cli'
               AND source_session_id = ?1",
            [CODEX_NESTED_FIXTURE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("codex moved session file path");
    assert_eq!(PathBuf::from(file_path), archived_file);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_legacy_codex_session_with_top_level_id_header() {
    let root = temp_root("ingest-codex-legacy-header");
    let data_dir = root.join(".jottrace");
    let session_file = legacy_codex_session_file(&root);
    write_text_file(&session_file, &legacy_codex_session_contents());

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 3"));
    assert!(ingest.contains("inserted_events: 3"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (source_session_id, event_count, started_at, file_path): (String, i64, String, String) =
        conn.query_row(
            "SELECT source_session_id, event_count, started_at, file_path
             FROM sessions
             WHERE source = 'codex_cli'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("legacy codex session metadata");
    assert_eq!(source_session_id, CODEX_LEGACY_FIXTURE_SESSION_ID);
    assert_eq!(event_count, 3);
    assert_eq!(started_at, "2025-09-12T09:54:22.802Z");
    assert_eq!(PathBuf::from(file_path), session_file);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_ignores_empty_codex_rollout_files_without_recording_errors() {
    let root = temp_root("ingest-codex-empty");
    let data_dir = root.join(".jottrace");
    let session_file = empty_codex_rollout_file(&root);
    write_text_file(&session_file, "");

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 0"));
    assert!(ingest.contains("events: 0"));
    assert!(ingest.contains("inserted_events: 0"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));
    assert_eq!(ingest_error_count(&data_dir), 0);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_drops_prior_empty_codex_placeholder_session() {
    let root = temp_root("ingest-codex-empty-drops-placeholder");
    let data_dir = root.join(".jottrace");
    let session_file = empty_codex_rollout_file(&root);
    write_text_file(
        &session_file,
        &invalid_codex_meta_line("2026-03-28T10:29:07.000Z"),
    );

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("sessions: 1"));
    assert!(first.contains("unresolved_ingest_errors: 1"));

    write_text_file(&session_file, "");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 0"));
    assert!(second.contains("events: 0"));
    assert!(second.contains("unresolved_ingest_errors: 0"));

    let session_count: i64 = Connection::open(db_path(&data_dir))
        .expect("open preserved db")
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .expect("session count");
    assert_eq!(session_count, 0);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_existing_codex_session_when_file_truncates_to_empty() {
    let root = temp_root("ingest-codex-empty-truncation");
    let data_dir = root.join(".jottrace");
    let session_file = install_nested_codex_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("sessions: 1"));
    assert!(first.contains("events: 13"));
    assert!(first.contains("inserted_events: 13"));

    write_text_file(&session_file, "");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 1"));
    assert!(second.contains("events: 13"));
    assert!(second.contains("inserted_events: 0"));
    assert!(second.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (current_generation, event_count, file_size, next_read_offset): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT current_generation, event_count, file_size, next_read_offset
             FROM sessions
             WHERE source = 'codex_cli'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("empty codex truncation state");
    assert_eq!(current_generation, 1);
    assert_eq!(event_count, 0);
    assert_eq!(file_size, 0);
    assert_eq!(next_read_offset, 0);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_resolves_prior_invalid_codex_meta_when_legacy_header_is_recognized() {
    let root = temp_root("ingest-codex-legacy-resolves-error");
    let data_dir = root.join(".jottrace");
    let session_file = legacy_codex_session_file(&root);
    write_text_file(
        &session_file,
        &invalid_codex_meta_line("2025-09-12T09:54:22.802Z"),
    );

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("sessions: 1"));
    assert!(first.contains("unresolved_ingest_errors: 1"));

    write_text_file(&session_file, &legacy_codex_session_contents());

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 1"));
    assert!(second.contains("events: 3"));
    assert!(second.contains("inserted_events: 3"));
    assert!(second.contains("unresolved_ingest_errors: 0"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert!(errors.is_empty());

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let source_session_ids: Vec<String> = conn
        .prepare(
            "SELECT source_session_id
             FROM sessions
             WHERE source = 'codex_cli'
             ORDER BY source_session_id",
        )
        .expect("prepare codex session ids")
        .query_map([], |row| row.get(0))
        .expect("query codex session ids")
        .map(|row| row.expect("codex session id"))
        .collect();
    assert_eq!(source_session_ids, vec![CODEX_LEGACY_FIXTURE_SESSION_ID]);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_invalid_codex_session_meta_without_blocking_other_files() {
    let root = temp_root("ingest-codex-invalid-meta");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let bad_file = root
        .join(".codex/sessions/2026/05/05")
        .join("rollout-bad-meta.jsonl");
    write_text_file(
        &bad_file,
        &invalid_codex_meta_line("2026-05-05T09:00:00.000Z"),
    );

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 12"));
    assert!(ingest.contains("inserted_events: 12"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "codex_cli");
    assert_eq!(error.source_session_id.as_deref(), Some("rollout-bad-meta"));
    assert_eq!(error.file_path, bad_file);
    assert_eq!(error.error_kind, "invalid_session_meta");
    assert!(error.message.contains("session_meta"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_compresses_large_payloads_and_events_decodes_original_jsonl() {
    let root = temp_root("ingest-zstd-large-payload");
    let data_dir = root.join(".jottrace");
    let large_line = large_compressible_event_line();
    let session_file = claude_project_dir(&root).join(format!("{CLAUDE_FIXTURE_SESSION_ID}.jsonl"));
    write_text_file(&session_file, &format!("{large_line}\n"));

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("events: 1"));
    assert!(ingest.contains("inserted_events: 1"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (stored_payload, codec, payload_size): (Vec<u8>, String, i64) = conn
        .query_row(
            "SELECT payload, codec, payload_size
             FROM events
             WHERE seq = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("stored zstd event payload");
    assert_eq!(codec, "zstd");
    assert_eq!(payload_size, large_line.len() as i64);
    assert!(
        stored_payload.len() < large_line.len(),
        "compressed payload should be smaller than source line"
    );

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--limit",
            "1",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("events stdout should be utf-8"),
        format!("{large_line}\n")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_uses_zstd_threshold_and_events_decodes_mixed_raw_and_zstd_rows() {
    let root = temp_root("ingest-zstd-threshold");
    let data_dir = root.join(".jottrace");
    let below_threshold = compressible_event_line_with_len(1023);
    let at_threshold = compressible_event_line_with_len(1024);
    let session_file = claude_project_dir(&root).join(format!("{CLAUDE_FIXTURE_SESSION_ID}.jsonl"));
    write_text_file(
        &session_file,
        &format!("{below_threshold}\n{at_threshold}\n"),
    );

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("events: 2"));
    assert!(ingest.contains("inserted_events: 2"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let codecs = event_codecs(&conn);
    assert_eq!(
        codecs,
        vec![(0, "raw".to_string()), (1, "zstd".to_string())]
    );

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--all",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events --all");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("events stdout should be utf-8"),
        format!("{below_threshold}\n{at_threshold}\n")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_leaves_existing_raw_rows_unchanged_on_idempotent_rerun() {
    let root = temp_root("ingest-existing-raw-idempotent");
    let data_dir = root.join(".jottrace");
    let line = large_compressible_event_line();
    let session_file = claude_project_dir(&root).join(format!("{CLAUDE_FIXTURE_SESSION_ID}.jsonl"));
    write_text_file(&session_file, &format!("{line}\n"));
    set_modified(&session_file, 1_700_000_000);
    insert_legacy_raw_session(&data_dir, &session_file, &line);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("events: 1"));
    assert!(ingest.contains("inserted_events: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (stored_payload, codec): (Vec<u8>, String) = conn
        .query_row(
            "SELECT payload, codec FROM events WHERE seq = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("legacy raw event payload");
    assert_eq!(codec, "raw");
    assert_eq!(stored_payload, line.as_bytes());

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--limit",
            "1",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("events stdout should be utf-8"),
        format!("{line}\n")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_dry_run_reports_savings_without_mutating_payloads() {
    let root = temp_root("compact-dry-run");
    let data_dir = root.join(".jottrace");
    let line = large_compressible_event_line();
    let session_file = claude_project_dir(&root).join(format!("{CLAUDE_FIXTURE_SESSION_ID}.jsonl"));
    write_text_file(&session_file, &format!("{line}\n"));
    set_modified(&session_file, 1_700_000_000);
    insert_legacy_raw_session(&data_dir, &session_file, &line);

    let output = Command::new(binary())
        .arg("compact")
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace compact");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("compact stdout should be utf-8");
    assert!(stdout.contains("jottrace compact"));
    assert!(stdout.contains("mode: dry-run"));
    assert!(stdout.contains("raw_events_before: 1"));
    assert!(stdout.contains("zstd_events_before: 0"));
    assert!(stdout.contains("eligible_raw_events: 1"));
    assert!(stdout.contains("converted_events: 0"));
    assert!(stdout.contains("unresolved_ingest_errors: 0"));
    assert!(
        compact_metric(&stdout, "estimated_bytes_saved") > 0,
        "{stdout}"
    );
    assert!(
        compact_metric(&stdout, "stored_bytes_after")
            < compact_metric(&stdout, "stored_bytes_before"),
        "{stdout}"
    );

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (stored_payload, codec): (Vec<u8>, String) = conn
        .query_row(
            "SELECT payload, codec FROM events WHERE seq = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("legacy raw event payload");
    assert_eq!(codec, "raw");
    assert_eq!(stored_payload, line.as_bytes());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_apply_rewrites_only_eligible_raw_rows_and_is_idempotent() {
    let root = temp_root("compact-apply");
    let data_dir = root.join(".jottrace");
    let fixture = insert_compaction_fixture(&data_dir);
    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let before_identity = event_identity_at_seq(&conn, 0);
    drop(conn);

    let output = Command::new(binary())
        .args(["compact", "--apply", "--batch-size", "2"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace compact --apply");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("compact stdout should be utf-8");
    assert!(stdout.contains("mode: apply"));
    assert!(stdout.contains("batch_size: 2"));
    assert!(stdout.contains("raw_events_before: 3"));
    assert!(stdout.contains("zstd_events_before: 1"));
    assert!(stdout.contains("unsupported_codec_events: 1"));
    assert!(stdout.contains("eligible_raw_events: 1"));
    assert!(stdout.contains("converted_events: 1"));
    assert!(stdout.contains("skipped_small_events: 1"));
    assert!(stdout.contains("skipped_not_smaller_events: 1"));
    assert!(stdout.contains("raw_events_after: 2"));
    assert!(stdout.contains("zstd_events_after: 2"));
    assert!(
        compact_metric(&stdout, "stored_bytes_after")
            < compact_metric(&stdout, "stored_bytes_before"),
        "{stdout}"
    );

    let conn = Connection::open(db_path(&data_dir)).expect("open compacted db");
    let (stored_payload, codec, payload_size) = event_payload_record(&conn, 0);
    assert_eq!(codec, "zstd");
    assert_eq!(payload_size, fixture.eligible_payload.len() as i64);
    assert_eq!(
        jottrace::storage::decode_event_payload(&stored_payload, &codec)
            .expect("decode compacted event"),
        fixture.eligible_payload
    );
    assert_eq!(event_identity_at_seq(&conn, 0), before_identity);
    assert_eq!(event_payload_record(&conn, 1).1, "raw");
    assert_eq!(event_payload_record(&conn, 2).1, "raw");
    assert_eq!(event_payload_record(&conn, 3).1, "zstd");
    assert_eq!(event_payload_record(&conn, 4).1, "snappy");
    drop(conn);

    let second = Command::new(binary())
        .args(["compact", "--apply"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("rerun jottrace compact --apply");
    assert!(
        second.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout = String::from_utf8(second.stdout).expect("compact stdout should be utf-8");
    assert!(second_stdout.contains("converted_events: 0"));
    assert!(second_stdout.contains("eligible_raw_events: 0"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_vacuum_reclaims_free_sqlite_pages_after_apply() {
    let root = temp_root("compact-vacuum");
    let data_dir = root.join(".jottrace");
    let payload = compressible_event_line_with_len(256 * 1024).into_bytes();
    insert_single_raw_compaction_session(&data_dir, &payload);

    let apply = Command::new(binary())
        .args(["compact", "--apply"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace compact --apply");
    assert!(
        apply.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply_stdout = String::from_utf8(apply.stdout).expect("compact stdout should be utf-8");
    assert!(
        compact_metric(&apply_stdout, "sqlite_reclaimable_bytes") > 0,
        "{apply_stdout}"
    );

    let vacuum = Command::new(binary())
        .args(["compact", "--vacuum"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace compact --vacuum");
    assert!(
        vacuum.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&vacuum.stdout),
        String::from_utf8_lossy(&vacuum.stderr)
    );
    let vacuum_stdout = String::from_utf8(vacuum.stdout).expect("compact stdout should be utf-8");
    assert!(vacuum_stdout.contains("mode: vacuum"));
    assert!(
        compact_metric(&vacuum_stdout, "sqlite_reclaimable_bytes_before") > 0,
        "{vacuum_stdout}"
    );
    assert_eq!(
        compact_metric(&vacuum_stdout, "sqlite_reclaimable_bytes"),
        0
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_rejects_invalid_batch_options_before_opening_database() {
    let root = temp_root("compact-invalid-batch");
    let data_dir = root.join(".jottrace");

    let oversized = Command::new(binary())
        .args(["compact", "--batch-size", "10001"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace compact with oversized batch");
    assert!(!oversized.status.success());
    let stderr = String::from_utf8_lossy(&oversized.stderr);
    assert!(stderr.contains("invalid batch size: 10001; expected at most 10000"));
    assert!(stderr.contains("jottrace compact --help"));
    assert!(
        !db_path(&data_dir).exists(),
        "usage errors should not initialize the database"
    );

    let vacuum_batch = Command::new(binary())
        .args(["compact", "--vacuum", "--batch-size", "2"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace compact --vacuum with batch size");
    assert!(!vacuum_batch.status.success());
    let stderr = String::from_utf8_lossy(&vacuum_batch.stderr);
    assert!(stderr.contains("compact --vacuum does not accept --batch-size"));
    assert!(stderr.contains("jottrace compact --help"));
    assert!(
        !db_path(&data_dir).exists(),
        "usage errors should not initialize the database"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_prints_decoded_payload_jsonl_for_bounded_session() {
    let root = temp_root("events-session-limit");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);

    run_ingest(&root, &data_dir);

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--limit",
            "2",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("events stdout should be utf-8"),
        fixture_lines(CLAUDE_FIXTURE_SESSION, 2)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_prints_all_decoded_payload_jsonl_for_session() {
    let root = temp_root("events-session-all");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);

    run_ingest(&root, &data_dir);

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--all",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events --all");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("events stdout should be utf-8"),
        fixture_lines(CLAUDE_FIXTURE_SESSION, 12)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_filters_by_source_and_session_identity() {
    let root = temp_root("events-source-session-identity");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);

    run_ingest(&root, &data_dir);
    insert_same_session_id_other_source(&data_dir);

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--limit",
            "2",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("events stdout should be utf-8"),
        fixture_lines(CLAUDE_FIXTURE_SESSION, 2)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_reports_unsupported_payload_codec() {
    let root = temp_root("events-unsupported-codec");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);

    run_ingest(&root, &data_dir);
    set_event_codec(&data_dir, 1, "snappy");

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--limit",
            "2",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jottrace events failed"));
    assert!(stderr.contains("unsupported event payload codec: snappy"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_limit_ignores_unsupported_codec_outside_selected_range() {
    let root = temp_root("events-limit-codec-range");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);

    run_ingest(&root, &data_dir);
    set_event_codec(&data_dir, 2, "snappy");

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--limit",
            "2",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("events stdout should be utf-8"),
        fixture_lines(CLAUDE_FIXTURE_SESSION, 2)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_reports_missing_session_identity() {
    let root = temp_root("events-missing-session");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);

    run_ingest(&root, &data_dir);

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            "missing-session",
            "--limit",
            "2",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jottrace events failed"));
    assert!(
        stderr.contains("session not found: source=claude_cli source_session_id=missing-session")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_requires_an_explicit_source() {
    let root = temp_root("events-requires-source");
    let data_dir = root.join(".jottrace");

    let output = Command::new(binary())
        .args([
            "events",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--limit",
            "1",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events without source");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("events requires --source <source>"));
    assert!(stderr.contains("jottrace events --help"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_requires_an_explicit_limit() {
    let root = temp_root("events-requires-limit");
    let data_dir = root.join(".jottrace");

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events without limit");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("events requires --limit <n> or --all"));
    assert!(stderr.contains("jottrace events --help"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_rejects_non_positive_limit() {
    let root = temp_root("events-rejects-non-positive-limit");
    let data_dir = root.join(".jottrace");

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--limit",
            "0",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events with zero limit");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid limit: 0; expected at least 1"));
    assert!(stderr.contains("jottrace events --help"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn events_rejects_limit_and_all_together() {
    let root = temp_root("events-rejects-limit-and-all");
    let data_dir = root.join(".jottrace");

    let output = Command::new(binary())
        .args([
            "events",
            "--source",
            "claude_cli",
            "--session",
            CLAUDE_FIXTURE_SESSION_ID,
            "--limit",
            "1",
            "--all",
        ])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace events with limit and all");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("events accepts either --limit <n> or --all, not both"));
    assert!(stderr.contains("jottrace events --help"));

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
        sidechain_source_session_id(CLAUDE_FIXTURE_SESSION_ID)
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

#[test]
fn ingest_keeps_same_named_claude_sidechains_under_different_parents_separate() {
    let root = temp_root("ingest-claude-sidechain-collision");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    install_claude_fixture(
        &root,
        OTHER_CLAUDE_FIXTURE_SESSION_ID,
        CLAUDE_FIXTURE_SESSION,
    );
    install_claude_sidechain_fixture_for_parent(&root, CLAUDE_FIXTURE_SESSION_ID);
    install_claude_sidechain_fixture_for_parent(&root, OTHER_CLAUDE_FIXTURE_SESSION_ID);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 4"));
    assert!(ingest.contains("events: 38"));
    assert!(ingest.contains("inserted_events: 38"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let child_links = sidechain_parent_links(&conn);
    assert_eq!(
        child_links,
        vec![
            (
                sidechain_source_session_id(CLAUDE_FIXTURE_SESSION_ID),
                CLAUDE_FIXTURE_SESSION_ID.to_string(),
                7,
            ),
            (
                sidechain_source_session_id(OTHER_CLAUDE_FIXTURE_SESSION_ID),
                OTHER_CLAUDE_FIXTURE_SESSION_ID.to_string(),
                7,
            ),
        ]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_ignores_claude_flat_root_non_session_jsonl() {
    let root = temp_root("ingest-ignore-history-jsonl");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let history_file = root.join(".claude/history.jsonl");
    write_text_file(&history_file, "{\"timestamp\":1759140367754}\n");

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 12"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let history_sessions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE source_session_id = 'history'",
            [],
            |row| row.get(0),
        )
        .expect("history session count");
    assert_eq!(history_sessions, 0);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_ignores_issue_69_ignored_and_deferred_source_paths_by_default() {
    let root = temp_root("ingest-ignore-non-scope-sources");
    let data_dir = root.join(".jottrace");

    for (path, content) in [
        (
            root.join(".aider.chat.history.md"),
            "Model: test\nCommand: /clear\n",
        ),
        (
            root.join("Library/Application Support/Claude/Session Storage/000003.log"),
            "{\"role\":\"assistant\",\"content\":\"cached browser state\"}\n",
        ),
        (
            root.join("Library/Application Support/Codex/Session Storage/000003.log"),
            "{\"role\":\"assistant\",\"content\":\"cached browser state\"}\n",
        ),
        (
            root.join(".agents/skills-manager/state.db"),
            "skills-manager app state",
        ),
        (
            root.join("Library/Caches/claude-cli-nodejs/mcp-output.jsonl"),
            "{\"role\":\"tool\",\"content\":\"sidecar cache output\"}\n",
        ),
        (
            root.join("Library/Application Support/Windsurf/User/globalStorage/state.vscdb"),
            "vscode-derived app state",
        ),
        (
            root.join("Library/Application Support/Code/User/globalStorage/state.vscdb"),
            "copilot/chatgpt extension app state",
        ),
        (
            root.join(".gemini/antigravity/brain/state.pbtxt"),
            "protobuf/app/browser state",
        ),
    ] {
        write_text_file(&path, content);
    }

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("files: 0"));
    assert!(ingest.contains("sessions: 0"));
    assert!(ingest.contains("events: 0"));
    assert!(ingest.contains("inserted_events: 0"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .expect("session count");
    assert_eq!(session_count, 0);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_uuid_named_claude_flat_root_session() {
    let root = temp_root("ingest-flat-root-uuid");
    let data_dir = root.join(".jottrace");
    let flat_session_file = root
        .join(".claude")
        .join(format!("{CLAUDE_FIXTURE_SESSION_ID}.jsonl"));
    copy_reader_fixture(CLAUDE_FIXTURE_SESSION, &flat_session_file);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 12"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let stored_file_path: String = conn
        .query_row(
            "SELECT file_path
             FROM sessions
             WHERE source_session_id = ?1",
            [CLAUDE_FIXTURE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("flat root session path");
    assert_eq!(PathBuf::from(stored_file_path), flat_session_file);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_does_not_treat_subagents_project_directory_as_sidechain_parent() {
    let root = temp_root("ingest-subagents-project-dir");
    let data_dir = root.join(".jottrace");
    let session_file = root
        .join(".claude/projects/subagents")
        .join(format!("{CLAUDE_FIXTURE_SESSION_ID}.jsonl"));
    copy_reader_fixture(CLAUDE_FIXTURE_SESSION, &session_file);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 12"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (source_session_id, parent_session_id): (String, Option<i64>) = conn
        .query_row(
            "SELECT source_session_id, parent_session_id
             FROM sessions
             WHERE file_path = ?1",
            [session_file.to_string_lossy().as_ref()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("subagents project session metadata");
    assert_eq!(source_session_id, CLAUDE_FIXTURE_SESSION_ID);
    assert_eq!(parent_session_id, None);

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

fn fixture_lines(fixture_relative: &str, count: usize) -> String {
    let contents = fs::read_to_string(reader_fixture(fixture_relative)).expect("read fixture");
    let mut selected = contents.lines().take(count).collect::<Vec<_>>().join("\n");
    selected.push('\n');
    selected
}

fn large_compressible_event_line() -> String {
    compressible_event_line_with_len(1600)
}

fn compressible_event_line_with_len(len: usize) -> String {
    let prefix = r#"{"type":"assistant","timestamp":"2026-05-05T01:00:00.000Z","cwd":"/Users/fixture/Workspace/jottrace","message":""#;
    let suffix = r#""}"#;
    assert!(len >= prefix.len() + suffix.len());
    let line = format!(
        "{prefix}{}{suffix}",
        "x".repeat(len - prefix.len() - suffix.len())
    );
    assert_eq!(line.len(), len);
    line
}

fn legacy_codex_session_contents() -> String {
    format!(
        "{{\"id\":\"{CODEX_LEGACY_FIXTURE_SESSION_ID}\",\"timestamp\":\"2025-09-12T09:54:22.802Z\",\"instructions\":\"fixture legacy instructions redacted\"}}\n\
         {{\"record_type\":\"state\"}}\n\
         {{\"type\":\"message\",\"role\":\"user\",\"content\":\"legacy Codex message\"}}\n"
    )
}

fn legacy_codex_session_file(root: &Path) -> PathBuf {
    root.join(".codex/sessions/2025/09/12")
        .join("rollout-2025-09-12T09-54-22-22222222-2222-4222-8222-222222222222.jsonl")
}

fn empty_codex_rollout_file(root: &Path) -> PathBuf {
    root.join(".codex/sessions/2026/03/28")
        .join("rollout-2026-03-28T10-29-07-019d3246-1417-77a0-8b5c-70f9bc4045c0.jsonl")
}

fn invalid_codex_meta_line(timestamp: &str) -> String {
    format!(
        "{{\"timestamp\":\"{timestamp}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\"}}}}\n"
    )
}

fn ingest_error_count(data_dir: &Path) -> i64 {
    Connection::open(db_path(data_dir))
        .expect("open preserved db")
        .query_row("SELECT COUNT(*) FROM ingest_errors", [], |row| row.get(0))
        .expect("ingest error count")
}

fn event_codecs(conn: &Connection) -> Vec<(i64, String)> {
    let mut statement = conn
        .prepare("SELECT seq, codec FROM events ORDER BY seq")
        .expect("prepare event codecs");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query event codecs")
        .map(|row| row.expect("event codec"))
        .collect()
}

fn compact_metric(stdout: &str, name: &str) -> u64 {
    let prefix = format!("{name}: ");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("missing compact metric {name} in:\n{stdout}"))
        .parse()
        .unwrap_or_else(|error| panic!("invalid compact metric {name} in:\n{stdout}\n{error}"))
}

#[derive(Debug)]
struct CompactionFixture {
    eligible_payload: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
struct EventIdentity {
    session_id: i64,
    generation: i64,
    seq: i64,
    ts: Option<String>,
    created_at: String,
}

fn insert_compaction_fixture(data_dir: &Path) -> CompactionFixture {
    let conn = jottrace::storage::open_database(&db_path(data_dir)).expect("open database");
    conn.execute(
        "INSERT INTO sessions (source, source_session_id, event_count)
         VALUES ('claude_cli', 'compact-session', 5)",
        [],
    )
    .expect("insert compact session");
    let session_id = conn.last_insert_rowid();

    let eligible_payload = large_compressible_event_line().into_bytes();
    let small_payload = compressible_event_line_with_len(512).into_bytes();
    let incompressible_payload = deterministic_bytes(2048);
    let existing_zstd_source = compressible_event_line_with_len(1500).into_bytes();
    let existing_zstd_payload = zstd::stream::encode_all(&existing_zstd_source[..], 0)
        .expect("compress existing zstd payload");
    insert_event_payload(
        &conn,
        session_id,
        0,
        &eligible_payload,
        "raw",
        eligible_payload.len(),
    );
    insert_event_payload(
        &conn,
        session_id,
        1,
        &small_payload,
        "raw",
        small_payload.len(),
    );
    insert_event_payload(
        &conn,
        session_id,
        2,
        &incompressible_payload,
        "raw",
        incompressible_payload.len(),
    );
    insert_event_payload(
        &conn,
        session_id,
        3,
        &existing_zstd_payload,
        "zstd",
        existing_zstd_source.len(),
    );
    conn.execute("PRAGMA ignore_check_constraints = ON", [])
        .expect("allow unsupported codec fixture");
    let unsupported_payload = br#"{"codec":"snappy"}"#;
    insert_event_payload(
        &conn,
        session_id,
        4,
        unsupported_payload,
        "snappy",
        unsupported_payload.len(),
    );

    CompactionFixture { eligible_payload }
}

fn insert_single_raw_compaction_session(data_dir: &Path, payload: &[u8]) {
    let conn = jottrace::storage::open_database(&db_path(data_dir)).expect("open database");
    conn.execute(
        "INSERT INTO sessions (source, source_session_id, event_count)
         VALUES ('claude_cli', 'compact-vacuum-session', 1)",
        [],
    )
    .expect("insert compact vacuum session");
    let session_id = conn.last_insert_rowid();
    insert_event_payload(&conn, session_id, 0, payload, "raw", payload.len());
}

fn insert_event_payload(
    conn: &Connection,
    session_id: i64,
    seq: i64,
    payload: &[u8],
    codec: &str,
    payload_size: usize,
) {
    conn.execute(
        "INSERT INTO events (session_id, generation, seq, ts, payload, codec, payload_size)
         VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            session_id,
            seq,
            format!("2026-05-05T01:00:0{seq}.000Z"),
            payload,
            codec,
            payload_size as i64
        ],
    )
    .expect("insert compact event");
}

fn event_payload_record(conn: &Connection, seq: i64) -> (Vec<u8>, String, i64) {
    conn.query_row(
        "SELECT payload, codec, payload_size
         FROM events
         WHERE seq = ?1",
        [seq],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .expect("event payload record")
}

fn event_identity_at_seq(conn: &Connection, seq: i64) -> EventIdentity {
    conn.query_row(
        "SELECT session_id, generation, seq, ts, created_at
         FROM events
         WHERE seq = ?1",
        [seq],
        |row| {
            Ok(EventIdentity {
                session_id: row.get(0)?,
                generation: row.get(1)?,
                seq: row.get(2)?,
                ts: row.get(3)?,
                created_at: row.get(4)?,
            })
        },
    )
    .expect("event identity")
}

fn deterministic_bytes(len: usize) -> Vec<u8> {
    let mut state = 0x1234_5678_9abc_def0_u64;
    let mut bytes = Vec::with_capacity(len);
    while bytes.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

fn insert_legacy_raw_session(data_dir: &Path, session_file: &Path, line: &str) {
    let conn = jottrace::storage::open_database(&db_path(data_dir)).expect("open database");
    let file_size = fs::metadata(session_file)
        .expect("legacy session metadata")
        .len() as i64;
    conn.execute(
        "INSERT INTO sessions
            (source, source_session_id, file_path, current_generation, file_mtime,
             file_size, content_fingerprint, next_read_offset, event_count)
         VALUES ('claude_cli', ?1, ?2, 0, ?3, ?4, 'legacy-raw', ?4, 1)",
        rusqlite::params![
            CLAUDE_FIXTURE_SESSION_ID,
            session_file.to_string_lossy(),
            1_700_000_000_i64,
            file_size,
        ],
    )
    .expect("insert legacy raw session");
    conn.execute(
        "INSERT INTO events (session_id, generation, seq, ts, payload, codec, payload_size)
         VALUES (last_insert_rowid(), 0, 0, '2026-05-05T01:00:00.000Z', ?1, 'raw', ?2)",
        rusqlite::params![line.as_bytes(), line.len() as i64],
    )
    .expect("insert legacy raw event");
}

fn insert_same_session_id_other_source(data_dir: &Path) {
    let conn = Connection::open(db_path(data_dir)).expect("open preserved db");
    let payload = br#"{"source":"codex_cli"}"#;
    conn.execute(
        "INSERT INTO sessions (source, source_session_id, event_count)
         VALUES ('codex_cli', ?1, 1)",
        [CLAUDE_FIXTURE_SESSION_ID],
    )
    .expect("insert same source_session_id for another source");
    conn.execute(
        "INSERT INTO events (session_id, generation, seq, payload, codec, payload_size)
         VALUES (last_insert_rowid(), 0, 0, ?1, 'raw', ?2)",
        rusqlite::params![payload, payload.len() as i64],
    )
    .expect("insert other source event");
}

fn set_event_codec(data_dir: &Path, seq: i64, codec: &str) {
    let conn = Connection::open(db_path(data_dir)).expect("open preserved db");
    conn.execute("PRAGMA ignore_check_constraints = ON", [])
        .expect("allow unsupported codec fixture");
    let updated = conn
        .execute(
            "UPDATE events
         SET codec = ?1
         WHERE session_id = (
             SELECT id FROM sessions
             WHERE source = 'claude_cli' AND source_session_id = ?2
         )
           AND generation = 0
           AND seq = ?3",
            rusqlite::params![codec, CLAUDE_FIXTURE_SESSION_ID, seq],
        )
        .expect("set event codec");
    assert_eq!(
        updated, 1,
        "expected to update one fixture event at seq {seq}"
    )
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
        [sidechain_source_session_id(CLAUDE_FIXTURE_SESSION_ID)],
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
    install_claude_sidechain_fixture_for_parent(root, CLAUDE_FIXTURE_SESSION_ID)
}

fn install_claude_sidechain_fixture_for_parent(root: &Path, parent_session_id: &str) -> PathBuf {
    let sidechain_file = claude_project_dir(root)
        .join(parent_session_id)
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

fn install_nested_codex_fixture(root: &Path) -> PathBuf {
    install_nested_codex_fixture_at(
        root,
        "rollout-2026-05-05T09-00-00-00000000-0000-4000-8000-000000000021.jsonl",
    )
}

fn install_nested_codex_fixture_at(root: &Path, file_name: &str) -> PathBuf {
    let session_file = root.join(".codex/sessions/2026/05/05").join(file_name);
    copy_reader_fixture(CODEX_NESTED_FIXTURE_SESSION, &session_file);
    session_file
}

fn install_archived_codex_fixture(root: &Path) -> PathBuf {
    let session_file = root
        .join(".codex/archived_sessions")
        .join("rollout-2026-03-28T10-42-29-00000000-0000-4000-8000-000000000021.jsonl");
    copy_reader_fixture(CODEX_ARCHIVED_FIXTURE_SESSION, &session_file);
    session_file
}

fn replace_with_fixture(path: &Path, fixture_relative: &str) {
    copy_reader_fixture(fixture_relative, path);
}

fn claude_project_dir(root: &Path) -> PathBuf {
    root.join(".claude/projects/-Users-fixture-Workspace-jottrace")
}

fn sidechain_source_session_id(parent_session_id: &str) -> String {
    format!("{parent_session_id}/subagents/{CLAUDE_SIDECHAIN_FIXTURE_SESSION_ID}")
}

fn sidechain_parent_links(conn: &Connection) -> Vec<(String, String, i64)> {
    let mut statement = conn
        .prepare(
            "SELECT child.source_session_id, parent.source_session_id, child.event_count
             FROM sessions child
             JOIN sessions parent ON parent.id = child.parent_session_id
             WHERE child.source = 'claude_cli'
               AND child.source_session_id LIKE '%/subagents/%'
             ORDER BY child.source_session_id",
        )
        .expect("prepare sidechain parent links");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query sidechain parent links")
        .map(|row| row.expect("sidechain parent link"))
        .collect()
}

fn copy_reader_fixture(fixture_relative: &str, destination: &Path) {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("create fixture destination parent");
    }
    fs::copy(reader_fixture(fixture_relative), destination).expect("copy fixture");
}

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create text fixture parent");
    }
    fs::write(path, content).expect("write text fixture");
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
