mod common;

use common::reader_fixture;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

#[cfg(unix)]
use std::os::unix::{
    fs::{MetadataExt, PermissionsExt},
    io::AsRawFd,
};

const CLAUDE_FIXTURE_SESSION: &str = "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021.jsonl";
const CLAUDE_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000021";
const CLAUDE_SIDECHAIN_FIXTURE_SESSION: &str = "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021/subagents/agent-a000000000000021.jsonl";
const CLAUDE_SIDECHAIN_FIXTURE_META: &str = "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021/subagents/agent-a000000000000021.meta.json";
const CLAUDE_SIDECHAIN_FIXTURE_SESSION_ID: &str = "agent-a000000000000021";
const CLAUDE_LOCAL_AGENT_FIXTURE_METADATA: &str = "claude-local-agent/local-agent-mode-sessions/desktop-fixture/workspace-fixture/local_00000000-0000-4000-8000-000000000068.json";
const CLAUDE_LOCAL_AGENT_FIXTURE_AUDIT: &str = "claude-local-agent/local-agent-mode-sessions/desktop-fixture/workspace-fixture/local_00000000-0000-4000-8000-000000000068/audit.jsonl";
const CLAUDE_LOCAL_AGENT_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000068";
const OTHER_CLAUDE_FIXTURE_SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
const CODEX_NESTED_FIXTURE_SESSION: &str = "codex-cli/sessions/2026/05/05/rollout-2026-05-05T09-00-00-00000000-0000-4000-8000-000000000021.jsonl";
const CODEX_NESTED_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000021";
const CODEX_ARCHIVED_FIXTURE_SESSION: &str = "codex-cli/archived_sessions/rollout-2026-03-28T10-42-29-00000000-0000-4000-8000-000000000021.jsonl";
const CODEX_ARCHIVED_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000121";
const CODEX_LEGACY_FIXTURE_SESSION_ID: &str = "22222222-2222-4222-8222-222222222222";
const PI_AGENT_FIXTURE_SESSION: &str = "pi-agent/sessions/--Users-fixture-Workspace-jottrace--/2026-05-06T02-00-00-000Z_00000000-0000-4000-8000-000000000064.jsonl";
const PI_AGENT_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000064";
const PI_AGENT_NESTED_RUN_FIXTURE_SESSION: &str = "pi-agent/sessions/--Users-fixture-Workspace-jottrace--/2026-05-06T02-00-00-000Z_00000000-0000-4000-8000-000000000064/abc12345/run-0/session.jsonl";
const PI_AGENT_NESTED_RUN_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000164";
const GEMINI_FIXTURE_SESSION: &str =
    "gemini-cli/tmp/fixture-project/chats/session-2026-05-06T09-00-gemini000.json";
const GEMINI_FIXTURE_SESSION_ID: &str = "33333333-3333-4333-8333-333333333333";
const FACTORY_FIXTURE_SESSION: &str =
    "factory/sessions/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000065.jsonl";
const FACTORY_FIXTURE_SETTINGS: &str = "factory/sessions/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000065.settings.json";
const FACTORY_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000065";
const OPENCODE_FIXTURE_SQL: &str = "opencode/sqlite/opencode.sql";
const OPENCODE_PARENT_SESSION_ID: &str = "ses_fixture_parent_00000000000";
const OPENCODE_CHILD_SESSION_ID: &str = "ses_fixture_child_000000000000";
const HERMES_FIXTURE_SQL: &str = "hermes/sqlite/state.sql";
const HERMES_PARENT_SESSION_ID: &str = "hermes_fixture_parent_0000000000";
const HERMES_CHILD_SESSION_ID: &str = "hermes_fixture_child_00000000000";
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
    assert!(stdout.contains("jottrace taste extract"));
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
        ("taste", "Extract labeled coding preference examples"),
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
fn taste_subcommand_help_aliases_print_command_specific_usage() {
    let cases = [
        ("taste extract", "Materialize file timelines"),
        ("taste status", "high-confidence coverage"),
        ("taste export", "JSONL for external trainer"),
        ("taste show", "Inspect materialized taste extraction artifacts"),
        ("taste show timeline", "--session <source_session_id>"),
        ("taste show example", "preference example"),
    ];

    for (command_parts, expected_detail) in cases {
        let mut args: Vec<&str> = command_parts.split_whitespace().collect();
        args.push("--help");
        let long = Command::new(binary())
            .args(&args)
            .output()
            .unwrap_or_else(|error| panic!("run jottrace {command_parts} --help: {error}"));
        let mut short_args = args.clone();
        short_args.pop();
        short_args.push("-h");
        let short = Command::new(binary())
            .args(&short_args)
            .output()
            .unwrap_or_else(|error| panic!("run jottrace {command_parts} -h: {error}"));

        assert!(
            long.status.success(),
            "{command_parts} --help stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&long.stdout),
            String::from_utf8_lossy(&long.stderr)
        );
        assert!(
            short.status.success(),
            "{command_parts} -h stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&short.stdout),
            String::from_utf8_lossy(&short.stderr)
        );
        assert_eq!(
            long.stdout, short.stdout,
            "{command_parts} help aliases should match"
        );
        assert!(
            long.stderr.is_empty(),
            "{command_parts} help should not warn"
        );

        let stdout = String::from_utf8_lossy(&long.stdout);
        assert!(stdout.contains(command_parts));
        assert!(stdout.contains("Usage:"));
        assert!(
            stdout.contains(expected_detail),
            "{command_parts} help:\n{stdout}"
        );
    }
}

#[test]
fn taste_subcommand_unknown_options_exit_with_targeted_help_hint() {
    for command in [
        "taste extract",
        "taste status",
        "taste export",
        "taste show timeline",
        "taste show example",
    ] {
        let root = temp_root(&format!("{}-unknown-option", command.replace(' ', "-")));
        let data_dir = root.join(".jottrace");
        let mut args: Vec<&str> = command.split_whitespace().collect();
        args.push("--definitely-not-an-option");
        let output = Command::new(binary())
            .args(&args)
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
    assert!(stdout.contains("unresolved_ingest_errors: 0"));
    assert!(!stdout.contains("data_dir: "));
    assert!(!stdout.contains("next: "));

    let detailed = Command::new(binary())
        .args(["doctor", "--details"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace doctor --details");

    assert!(
        detailed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&detailed.stdout),
        String::from_utf8_lossy(&detailed.stderr)
    );

    let detailed_stdout = String::from_utf8_lossy(&detailed.stdout);
    assert!(detailed_stdout.contains("jottrace doctor"));
    assert!(detailed_stdout.contains(&format!("data_dir: {} (ok)", data_dir.display())));
    assert!(detailed_stdout.contains("permissions: private (ok)"));
    assert!(detailed_stdout.contains("unresolved_ingest_errors: 0"));

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
    assert!(!stdout.contains("db: "));
    assert!(!stdout.contains("schema_version: "));

    let detailed = Command::new(binary())
        .args(["status", "--details"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace status --details");

    assert!(
        detailed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&detailed.stdout),
        String::from_utf8_lossy(&detailed.stderr)
    );
    let detailed_stdout = String::from_utf8_lossy(&detailed.stdout);
    assert!(detailed_stdout.contains(&format!("db: {}", db_path(&data_dir).display())));
    assert!(detailed_stdout.contains("schema_version: "));

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
    assert!(stdout.contains("next: run `jottrace doctor --details`"));
    assert!(!stdout.contains("data_dir: "));
    assert!(!stdout.contains(CORRUPT_FIXTURE_SESSION_ID));
    assert!(!stdout.contains("kind: invalid_json"));
    assert!(!stdout.contains("message: "));

    let detailed = Command::new(binary())
        .args(["doctor", "--details"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace doctor --details");

    assert!(
        detailed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&detailed.stdout),
        String::from_utf8_lossy(&detailed.stderr)
    );

    let detailed_stdout = String::from_utf8_lossy(&detailed.stdout);
    assert!(detailed_stdout.contains("data_dir: "));
    assert!(detailed_stdout.contains(CORRUPT_FIXTURE_SESSION_ID));
    assert!(detailed_stdout.contains("line: 2"));
    assert!(detailed_stdout.contains("kind: invalid_json"));
    assert!(detailed_stdout.contains("message: "));

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

#[test]
fn ingest_resolves_prior_invalid_json_when_line_becomes_parseable() {
    let root = temp_root("ingest-corrupt-line-resolves");
    let data_dir = root.join(".jottrace");
    let session_file =
        install_claude_fixture(&root, CORRUPT_FIXTURE_SESSION_ID, CORRUPT_FIXTURE_SESSION);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("unresolved_ingest_errors: 1"));

    write_text_file(
        &session_file,
        "{\"timestamp\":\"2026-05-05T09:11:00.000Z\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"fixed first line\"}}\n\
         {\"timestamp\":\"2026-05-05T09:11:02.000Z\",\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"fixed second line\"}]}}\n",
    );

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("unresolved_ingest_errors: 0"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert!(errors.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_resolves_prior_invalid_json_when_fixed_before_unterminated_tail() {
    let root = temp_root("ingest-corrupt-line-resolves-before-tail");
    let data_dir = root.join(".jottrace");
    let session_file =
        install_claude_fixture(&root, CORRUPT_FIXTURE_SESSION_ID, CORRUPT_FIXTURE_SESSION);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("unresolved_ingest_errors: 1"));

    write_text_file(
        &session_file,
        "{\"timestamp\":\"2026-05-05T09:11:00.000Z\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"fixed first line\"}}\n\
         {\"timestamp\":\"2026-05-05T09:11:02.000Z\",\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"fixed second line\"}]}}\n\
         {\"timestamp\":\"2026-05-05T09:11:03.000Z\",\"type\":\"user\"",
    );

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 2"));
    assert!(second.contains("unresolved_ingest_errors: 0"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert!(errors.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_keeps_prior_invalid_json_without_retry_when_file_is_unchanged() {
    let root = temp_root("ingest-corrupt-line-unchanged");
    let data_dir = root.join(".jottrace");
    install_claude_fixture(&root, CORRUPT_FIXTURE_SESSION_ID, CORRUPT_FIXTURE_SESSION);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("unresolved_ingest_errors: 1"));

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].error_kind, "invalid_json");
    assert_eq!(errors[0].occurrence_count, 1);

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
fn ingest_rereads_grown_claude_cli_rewrite_when_prefix_changes() {
    // Regression for the stuck workflow-journal case: a JSONL file that grows
    // *and* has its earlier bytes rewritten must be re-read from the start, not
    // resumed from the stored offset (which now lands mid-record and would log a
    // permanent invalid_json error).
    let root = temp_root("ingest-grow-rewrite");
    let data_dir = root.join(".jottrace");
    let session_file = install_claude_fixture(
        &root,
        "grow-rewrite",
        "edge-cases/grow-rewrite-before.jsonl",
    );

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 3"));
    assert!(first.contains("inserted_events: 3"));
    let before_len = fs::metadata(&session_file).expect("before metadata").len() as i64;

    replace_with_fixture(&session_file, "edge-cases/grow-rewrite-after.jsonl");
    let after_len = fs::metadata(&session_file).expect("after metadata").len() as i64;
    // The rewrite must grow the file so the old append path (pass_size >
    // file_size) would have been chosen and resumed from a stale offset.
    assert!(after_len > before_len);

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 8"));
    assert!(second.contains("inserted_events: 5"));

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
    assert_eq!(current_generation, 1);
    assert_eq!(event_count, 5);
    assert_eq!(file_size, after_len);
    assert_eq!(next_read_offset, after_len);
    assert_eq!(generation_counts(&conn), vec![(0, 3), (1, 5)]);

    let unresolved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ingest_errors WHERE resolved_at IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("unresolved error count");
    assert_eq!(unresolved, 0);

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

#[cfg(unix)]
#[test]
fn ingest_reports_lock_contention_as_clear_cli_failure() {
    let root = temp_root("ingest-lock-held");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let lock_file = jottrace::create_private_file(&data_dir.join(jottrace::LOCK_FILE_NAME))
        .expect("create held lock");
    // Simulate a live DB-mutating process holding the OS-level data lock.
    let lock_result = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(lock_result, 0);

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

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 12"));
    assert!(ingest.contains("inserted_events: 12"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));
    assert!(!ingest.contains("db: "));

    let detailed_ingest = Command::new(binary())
        .args(["ingest", "--details"])
        .env("HOME", &root)
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace ingest --details");
    assert!(
        detailed_ingest.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&detailed_ingest.stdout),
        String::from_utf8_lossy(&detailed_ingest.stderr)
    );
    let detailed_stdout = String::from_utf8_lossy(&detailed_ingest.stdout);
    assert!(detailed_stdout.contains(&format!("db: {}", db_path(&data_dir).display())));

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
fn ingest_preserves_claude_local_agent_audit_fixture_with_metadata_linkage() {
    let root = temp_root("ingest-claude-local-agent");
    let data_dir = root.join(".jottrace");
    let audit_file = install_claude_local_agent_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 6"));
    assert!(ingest.contains("inserted_events: 6"));
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
             WHERE source = 'claude_local_agent'",
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
        .expect("claude local-agent session metadata");
    assert_eq!(source_session_id, CLAUDE_LOCAL_AGENT_FIXTURE_SESSION_ID);
    assert_eq!(cwd, "/Users/fixture/Workspace/jottrace");
    assert_eq!(started_at, "2026-05-06T02:00:01.000Z");
    assert_eq!(ended_at, "2026-05-06T02:00:06.000Z");
    assert_eq!(event_count, 6);
    assert_eq!(PathBuf::from(file_path), audit_file);

    let max_seq: i64 = conn
        .query_row(
            "SELECT MAX(seq)
             FROM events
             JOIN sessions ON sessions.id = events.session_id
             WHERE sessions.source = 'claude_local_agent'",
            [],
            |row| row.get(0),
        )
        .expect("local-agent max seq");
    assert_eq!(max_seq, 5);
    assert!(event_payload(&conn, 0, 0).contains(r#""type":"user""#));

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 1"));
    assert!(second.contains("events: 6"));
    assert!(second.contains("inserted_events: 0"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_invalid_claude_local_agent_audit_without_blocking_other_files() {
    let root = temp_root("ingest-claude-local-agent-invalid");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let bad_audit = root
        .join("Library/Application Support/Claude/local-agent-mode-sessions/desktop-fixture/workspace-fixture/local_bad-audit-session")
        .join("audit.jsonl");
    write_text_file(
        &bad_audit,
        "{\"timestamp\":\"2026-05-06T02:10:00.000Z\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"missing session id\"}}\n",
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
    assert_eq!(error.source, "claude_local_agent");
    assert_eq!(
        error.source_session_id.as_deref(),
        Some("bad-audit-session")
    );
    assert_eq!(error.file_path, bad_audit);
    assert_eq!(error.error_kind, "invalid_session_meta");
    assert!(error.message.contains("session_id"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_then_resolves_uncommitted_claude_local_agent_audit_header() {
    let root = temp_root("ingest-claude-local-agent-uncommitted-header");
    let data_dir = root.join(".jottrace");
    let audit_file = install_claude_local_agent_fixture(&root);
    let audit_line = format!(
        "{{\"_audit_timestamp\":\"2026-05-06T02:35:00.000Z\",\"session_id\":\"{CLAUDE_LOCAL_AGENT_FIXTURE_SESSION_ID}\",\"type\":\"user\"}}"
    );
    write_text_file(&audit_file, &audit_line);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 0"));
    assert!(ingest.contains("inserted_events: 0"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].source, "claude_local_agent");
    assert_eq!(errors[0].file_path, audit_file);
    assert_eq!(errors[0].error_kind, "invalid_session_meta");
    assert!(errors[0].message.contains("no committed header line"));

    write_text_file(&audit_file, &format!("{audit_line}\n"));

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 1"));
    assert!(second.contains("events: 1"));
    assert!(second.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let stored_source_session_id: String = conn
        .query_row(
            "SELECT source_session_id
             FROM sessions
             WHERE source = 'claude_local_agent'",
            [],
            |row| row.get(0),
        )
        .expect("claude local-agent session id");
    assert_eq!(
        stored_source_session_id,
        CLAUDE_LOCAL_AGENT_FIXTURE_SESSION_ID
    );
    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("resolved ingest errors");
    assert!(errors.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_uses_claude_local_agent_path_identity_when_first_audit_line_exceeds_header_limit() {
    let root = temp_root("ingest-claude-local-agent-large-header");
    let data_dir = root.join(".jottrace");
    let audit_file = install_claude_local_agent_fixture(&root);
    let large_content = "x".repeat(70_000);
    write_text_file(
        &audit_file,
        &format!(
            "{{\"_audit_timestamp\":\"2026-05-06T02:40:00.000Z\",\"session_id\":\"{CLAUDE_LOCAL_AGENT_FIXTURE_SESSION_ID}\",\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{large_content}\"}}}}\n"
        ),
    );

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 1"));
    assert!(ingest.contains("inserted_events: 1"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let stored_source_session_id: String = conn
        .query_row(
            "SELECT source_session_id
             FROM sessions
             WHERE source = 'claude_local_agent'",
            [],
            |row| row.get(0),
        )
        .expect("claude local-agent session id");
    assert_eq!(
        stored_source_session_id,
        CLAUDE_LOCAL_AGENT_FIXTURE_SESSION_ID
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_ignores_claude_browser_session_storage_audit_files() {
    let root = temp_root("ingest-ignore-claude-browser-storage");
    let data_dir = root.join(".jottrace");
    let browser_audit = root
        .join("Library/Application Support/Claude/Session Storage")
        .join("audit.jsonl");
    write_text_file(
        &browser_audit,
        "{\"timestamp\":\"2026-05-06T02:20:00.000Z\",\"session_id\":\"browser-storage-session\",\"type\":\"user\"}\n",
    );

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 0"));
    assert!(ingest.contains("events: 0"));
    assert!(ingest.contains("inserted_events: 0"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_ignores_non_session_audit_files_under_claude_local_agent_root() {
    let root = temp_root("ingest-ignore-claude-local-agent-debug");
    let data_dir = root.join(".jottrace");
    let debug_audit = root
        .join("Library/Application Support/Claude/local-agent-mode-sessions/desktop-fixture/workspace-fixture/debug")
        .join("audit.jsonl");
    write_text_file(
        &debug_audit,
        "{\"timestamp\":\"2026-05-06T02:30:00.000Z\",\"session_id\":\"debug-session\",\"type\":\"user\"}\n",
    );

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 0"));
    assert!(ingest.contains("events: 0"));
    assert!(ingest.contains("inserted_events: 0"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

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
fn ingest_preserves_pi_agent_fixture_and_status_reports_counts() {
    let root = temp_root("ingest-pi-agent");
    let data_dir = root.join(".jottrace");
    let session_file = install_pi_agent_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 6"));
    assert!(ingest.contains("inserted_events: 6"));
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
             WHERE source = 'pi_agent'",
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
        .expect("pi agent session metadata");
    assert_eq!(source_session_id, PI_AGENT_FIXTURE_SESSION_ID);
    assert_eq!(cwd, "/Users/fixture/Workspace/jottrace");
    assert_eq!(started_at, "2026-05-06T02:00:00.000Z");
    assert_eq!(ended_at, "2026-05-06T02:00:05.000Z");
    assert_eq!(event_count, 6);
    assert_eq!(PathBuf::from(file_path), session_file);

    let event_types = decoded_event_types(&data_dir, "pi_agent", PI_AGENT_FIXTURE_SESSION_ID);
    assert_eq!(
        event_types,
        vec![
            "session",
            "model_change",
            "thinking_level_change",
            "message",
            "message",
            "message"
        ]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_is_idempotent_for_unchanged_pi_agent_fixture() {
    let root = temp_root("ingest-pi-agent-idempotent");
    let data_dir = root.join(".jottrace");
    install_pi_agent_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 6"));
    assert!(first.contains("inserted_events: 6"));

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 6"));
    assert!(second.contains("inserted_events: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let event_count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM events
             JOIN sessions ON sessions.id = events.session_id
             WHERE sessions.source = 'pi_agent'
               AND sessions.source_session_id = ?1",
            [PI_AGENT_FIXTURE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("pi event count");
    assert_eq!(event_count, 6);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_links_pi_agent_nested_run_session_to_parent() {
    let root = temp_root("ingest-pi-agent-nested-run");
    let data_dir = root.join(".jottrace");
    install_pi_agent_fixture(&root);
    let nested_session_file = install_pi_agent_nested_run_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (source_session_id, file_path, parent_source_session_id): (String, String, String) = conn
        .query_row(
            "SELECT child.source_session_id, child.file_path, parent.source_session_id
             FROM sessions AS child
             JOIN sessions AS parent ON parent.id = child.parent_session_id
             WHERE child.source = 'pi_agent' AND child.source_session_id = ?1",
            [PI_AGENT_NESTED_RUN_FIXTURE_SESSION_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("nested pi agent session should be linked to its parent");
    assert_eq!(source_session_id, PI_AGENT_NESTED_RUN_FIXTURE_SESSION_ID);
    assert_eq!(PathBuf::from(file_path), nested_session_file);
    assert_eq!(parent_source_session_id, PI_AGENT_FIXTURE_SESSION_ID);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_invalid_pi_agent_session_header_without_blocking_other_files() {
    let root = temp_root("ingest-pi-agent-invalid-header");
    let data_dir = root.join(".jottrace");
    let pi_session_file = install_pi_agent_fixture(&root);
    let bad_file = pi_session_file
        .parent()
        .expect("Pi fixture parent")
        .join("bad-pi-session.jsonl");
    write_text_file(
        &bad_file,
        "{\"type\":\"message\",\"id\":\"bad-pi-message\",\"timestamp\":\"2026-05-06T02:10:00.000Z\",\"message\":{\"role\":\"user\",\"timestamp\":\"2026-05-06T02:10:00.000Z\",\"content\":[{\"type\":\"text\",\"text\":\"fixture invalid header\"}]}}\n",
    );

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 6"));
    assert!(ingest.contains("inserted_events: 6"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "pi_agent");
    assert_eq!(error.source_session_id.as_deref(), Some("bad-pi-session"));
    assert_eq!(error.file_path, bad_file);
    assert_eq!(error.error_kind, "invalid_session_meta");
    assert!(
        error
            .message
            .contains("Pi agent session file does not start with a session event")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_unterminated_pi_agent_session_header_with_bounded_error() {
    let root = temp_root("ingest-pi-agent-unterminated-header");
    let data_dir = root.join(".jottrace");
    let bad_file = root
        .join(".pi/agent/sessions")
        .join("--Users-fixture-Workspace-jottrace--")
        .join("2026-05-06T02-20-00-000Z_bad-header.jsonl");
    write_text_file(&bad_file, &"x".repeat(70 * 1024));

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 0"));
    assert!(ingest.contains("inserted_events: 0"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "pi_agent");
    assert_eq!(error.source_session_id.as_deref(), Some("bad-header"));
    assert_eq!(error.file_path, bad_file);
    assert_eq!(error.error_kind, "invalid_session_meta");
    assert!(error.message.contains(
        "Pi agent session file has no committed session event line within the header limit"
    ));

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
fn ingest_preserves_gemini_cli_chat_json_session() {
    let root = temp_root("ingest-gemini-chat");
    let data_dir = root.join(".jottrace");
    let session_file = install_gemini_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 4"));
    assert!(ingest.contains("inserted_events: 4"));
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
             WHERE source = 'gemini_cli'",
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
        .expect("gemini session metadata");
    assert_eq!(source_session_id, GEMINI_FIXTURE_SESSION_ID);
    assert_eq!(cwd, "fixture-gemini-project-hash");
    assert_eq!(started_at, "2026-05-06T09:00:00.000Z");
    assert_eq!(ended_at, "2026-05-06T09:00:06.000Z");
    assert_eq!(event_count, 4);
    assert_eq!(PathBuf::from(file_path), session_file);

    let events: Vec<(i64, Option<String>, serde_json::Value)> = conn
        .prepare(
            "SELECT seq, ts, payload, codec
             FROM events
             JOIN sessions ON sessions.id = events.session_id
             WHERE sessions.source = 'gemini_cli'
             ORDER BY seq",
        )
        .expect("prepare gemini events")
        .query_map([], |row| {
            let payload: Vec<u8> = row.get(2)?;
            let codec: String = row.get(3)?;
            let decoded = jottrace::storage::decode_event_payload(&payload, &codec)
                .expect("decode gemini payload");
            let json = serde_json::from_slice(&decoded).expect("gemini payload should be json");
            Ok((row.get(0)?, row.get(1)?, json))
        })
        .expect("query gemini events")
        .map(|row| row.expect("gemini event"))
        .collect();
    assert_eq!(events.len(), 4);
    assert_eq!(
        events.iter().map(|event| event.0).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(events[0].1.as_deref(), Some("2026-05-06T09:00:01.000Z"));
    assert_eq!(events[0].2["type"], "user");
    assert_eq!(events[1].2["type"], "gemini");
    assert!(events[1].2.get("thoughts").is_some());
    assert!(events[1].2.get("tokens").is_some());
    assert!(events[1].2.get("toolCalls").is_some());
    assert_eq!(events[2].2["type"], "info");
    assert_eq!(events[3].2["content"], "Final sanitized response.");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_is_idempotent_for_unchanged_gemini_cli_fixture() {
    let root = temp_root("ingest-gemini-idempotent");
    let data_dir = root.join(".jottrace");
    install_gemini_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 4"));
    assert!(first.contains("inserted_events: 4"));

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 1"));
    assert!(second.contains("events: 4"));
    assert!(second.contains("inserted_events: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (current_generation, event_count): (i64, i64) = conn
        .query_row(
            "SELECT current_generation, event_count
             FROM sessions
             WHERE source = 'gemini_cli'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("gemini idempotent session state");
    assert_eq!(current_generation, 0);
    assert_eq!(event_count, 4);
    assert_eq!(generation_counts(&conn), vec![(0, 4)]);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_changed_gemini_cli_chat_as_next_generation() {
    let root = temp_root("ingest-gemini-rewrite");
    let data_dir = root.join(".jottrace");
    let session_file = install_gemini_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 4"));
    assert!(first.contains("inserted_events: 4"));

    let mut chat: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&session_file).expect("read gemini fixture for rewrite"),
    )
    .expect("parse gemini fixture for rewrite");
    chat["messages"]
        .as_array_mut()
        .expect("gemini messages array")
        .push(serde_json::json!({
            "id": "gemini-message-005",
            "timestamp": "2026-05-06T09:00:08.000Z",
            "type": "user",
            "content": [
                {
                    "text": "Please continue with the next fixture step."
                }
            ]
        }));
    chat["lastUpdated"] = serde_json::json!("2026-05-06T09:00:08.000Z");
    write_text_file(
        &session_file,
        &serde_json::to_string_pretty(&chat).expect("serialize rewritten gemini fixture"),
    );

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("events: 9"));
    assert!(second.contains("inserted_events: 5"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (current_generation, event_count, ended_at): (i64, i64, String) = conn
        .query_row(
            "SELECT current_generation, event_count, ended_at
             FROM sessions
             WHERE source = 'gemini_cli'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("gemini rewritten session state");
    assert_eq!(current_generation, 1);
    assert_eq!(event_count, 5);
    assert_eq!(ended_at, "2026-05-06T09:00:08.000Z");
    assert_eq!(generation_counts(&conn), vec![(0, 4), (1, 5)]);
    assert!(event_payload(&conn, 1, 4).contains("Please continue"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_opencode_sqlite_sessions_from_sanitized_fixture() {
    let root = temp_root("ingest-opencode-sqlite");
    let data_dir = root.join(".jottrace");
    let source_db = install_opencode_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 13"));
    assert!(ingest.contains("inserted_events: 13"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (child_parent_id, child_file_path, child_cwd, child_event_count): (
        Option<i64>,
        String,
        String,
        i64,
    ) = conn
        .query_row(
            "SELECT child.parent_session_id, child.file_path, child.cwd, child.event_count
             FROM sessions child
             WHERE child.source = 'opencode'
               AND child.source_session_id = ?1",
            [OPENCODE_CHILD_SESSION_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("OpenCode child session metadata");
    assert!(child_parent_id.is_some());
    assert_eq!(PathBuf::from(child_file_path), source_db);
    assert_eq!(child_cwd, "/Users/fixture/Workspace/jottrace");
    assert_eq!(child_event_count, 6);

    let parent_source_session_id: String = conn
        .query_row(
            "SELECT parent.source_session_id
             FROM sessions child
             JOIN sessions parent ON parent.id = child.parent_session_id
             WHERE child.source = 'opencode'
               AND child.source_session_id = ?1",
            [OPENCODE_CHILD_SESSION_ID],
            |row| row.get(0),
        )
        .expect("OpenCode parent session link");
    assert_eq!(parent_source_session_id, OPENCODE_PARENT_SESSION_ID);

    let mut child_events = Vec::new();
    jottrace::storage::for_each_decoded_event_payload_for_session(
        &db_path(&data_dir),
        "opencode",
        OPENCODE_CHILD_SESSION_ID,
        None,
        |payload| {
            child_events.push(
                serde_json::from_slice::<serde_json::Value>(payload)
                    .expect("OpenCode payload should be JSON"),
            );
            Ok(())
        },
    )
    .expect("decoded OpenCode child events");
    assert_eq!(
        child_events
            .iter()
            .map(|event| event["type"].as_str().expect("event type"))
            .collect::<Vec<_>>(),
        vec![
            "session",
            "message",
            "part",
            "message",
            "part",
            "session_message"
        ]
    );
    assert_eq!(child_events[1]["row"]["data"]["role"], "user");
    assert_eq!(child_events[2]["row"]["data"]["type"], "text");
    assert_eq!(
        child_events[2]["row"]["data"]["text"],
        "Please continue from the parent session."
    );
    assert_eq!(child_events[5]["row"]["source_type"], "checkpoint");
    assert_eq!(
        child_events[5]["row"]["data"]["status"],
        "synthetic child checkpoint"
    );

    let source_metadata: String = conn
        .query_row(
            "SELECT source_metadata
             FROM sessions
             WHERE source = 'opencode'
               AND source_session_id = ?1",
            [OPENCODE_CHILD_SESSION_ID],
            |row| row.get(0),
        )
        .expect("OpenCode source metadata");
    let source_metadata: serde_json::Value =
        serde_json::from_str(&source_metadata).expect("OpenCode metadata should be JSON");
    assert_eq!(source_metadata["project"]["name"], "fixture-jottrace");
    assert_eq!(source_metadata["session"]["slug"], "fixture-child-session");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_hermes_sqlite_sessions_from_sanitized_fixture() {
    let root = temp_root("ingest-hermes-sqlite");
    let data_dir = root.join(".jottrace");
    let source_db = install_hermes_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 7"));
    assert!(ingest.contains("inserted_events: 7"));
    assert!(ingest.contains("unresolved_ingest_errors: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (child_parent_id, child_file_path, child_event_count, child_ended_at): (
        Option<i64>,
        String,
        i64,
        String,
    ) = conn
        .query_row(
            "SELECT parent_session_id, file_path, event_count, ended_at
             FROM sessions
             WHERE source = 'hermes'
               AND source_session_id = ?1",
            [HERMES_CHILD_SESSION_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("Hermes child session metadata");
    assert!(child_parent_id.is_some());
    assert_eq!(PathBuf::from(child_file_path), source_db);
    assert_eq!(child_event_count, 4);
    assert_eq!(child_ended_at, "2026-02-02T02:42:10.000Z");

    let parent_source_session_id: String = conn
        .query_row(
            "SELECT parent.source_session_id
             FROM sessions child
             JOIN sessions parent ON parent.id = child.parent_session_id
             WHERE child.source = 'hermes'
               AND child.source_session_id = ?1",
            [HERMES_CHILD_SESSION_ID],
            |row| row.get(0),
        )
        .expect("Hermes parent session link");
    assert_eq!(parent_source_session_id, HERMES_PARENT_SESSION_ID);

    let mut child_events = Vec::new();
    jottrace::storage::for_each_decoded_event_payload_for_session(
        &db_path(&data_dir),
        "hermes",
        HERMES_CHILD_SESSION_ID,
        None,
        |payload| {
            child_events.push(
                serde_json::from_slice::<serde_json::Value>(payload)
                    .expect("Hermes payload should be JSON"),
            );
            Ok(())
        },
    )
    .expect("decoded Hermes child events");
    assert_eq!(
        child_events
            .iter()
            .map(|event| event["type"].as_str().expect("event type"))
            .collect::<Vec<_>>(),
        vec!["session", "message", "message", "message"]
    );
    assert_eq!(child_events[1]["row"]["id"], 12);
    assert_eq!(child_events[2]["row"]["id"], 10);
    assert_eq!(child_events[3]["row"]["id"], 11);
    assert_eq!(child_events[1]["row"]["role"], "user");
    assert_eq!(child_events[2]["row"]["tool_name"], "read_fixture_metadata");
    assert_eq!(
        child_events[3]["row"]["content"],
        "Hermes fixture import completed."
    );

    let source_metadata: String = conn
        .query_row(
            "SELECT source_metadata
             FROM sessions
             WHERE source = 'hermes'
               AND source_session_id = ?1",
            [HERMES_CHILD_SESSION_ID],
            |row| row.get(0),
        )
        .expect("Hermes source metadata");
    let source_metadata: serde_json::Value =
        serde_json::from_str(&source_metadata).expect("Hermes metadata should be JSON");
    assert_eq!(source_metadata["session"]["source"], "hermes-cli");
    assert_eq!(source_metadata["session"]["model"], "fixture-model");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_is_idempotent_for_unchanged_hermes_sqlite_fixture() {
    let root = temp_root("ingest-hermes-idempotent");
    let data_dir = root.join(".jottrace");
    install_hermes_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 7"));
    assert!(first.contains("inserted_events: 7"));

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 2"));
    assert!(second.contains("events: 7"));
    assert!(second.contains("inserted_events: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    assert_eq!(generation_counts(&conn), vec![(0, 7)]);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_changed_hermes_sqlite_session_as_next_generation() {
    let root = temp_root("ingest-hermes-rewrite");
    let data_dir = root.join(".jottrace");
    let source_db = install_hermes_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 7"));
    assert!(first.contains("inserted_events: 7"));

    Connection::open(&source_db)
        .expect("open source Hermes db")
        .execute_batch(
            "UPDATE messages
             SET content = 'Hermes fixture import completed after revision.',
                 timestamp = 1770000140.0
             WHERE id = 11;
             UPDATE sessions
             SET ended_at = 1770000140.0
             WHERE id = 'hermes_fixture_child_00000000000';",
        )
        .expect("rewrite Hermes fixture row");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 2"));
    assert!(second.contains("events: 11"));
    assert!(second.contains("inserted_events: 4"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (current_generation, event_count, ended_at): (i64, i64, String) = conn
        .query_row(
            "SELECT current_generation, event_count, ended_at
             FROM sessions
             WHERE source = 'hermes'
               AND source_session_id = ?1",
            [HERMES_CHILD_SESSION_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("Hermes rewritten session state");
    assert_eq!(current_generation, 1);
    assert_eq!(event_count, 4);
    assert_eq!(ended_at, "2026-02-02T02:42:20.000Z");
    assert_eq!(generation_counts(&conn), vec![(0, 7), (1, 4)]);

    assert!(event_payload(&conn, 1, 3).contains("after revision"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_invalid_hermes_sqlite_without_blocking_unrelated_files() {
    let root = temp_root("ingest-hermes-invalid");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let bad_db = root.join(".hermes/state.db");
    write_text_file(&bad_db, "not a sqlite database");

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 12"));
    assert!(ingest.contains("inserted_events: 12"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "hermes");
    assert_eq!(error.source_session_id.as_deref(), Some("state.db"));
    assert_eq!(error.file_path, bad_db);
    assert_eq!(error.error_kind, "invalid_session_meta");
    assert!(error.message.contains("Hermes SQLite SessionDB"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_is_idempotent_for_unchanged_opencode_sqlite_fixture() {
    let root = temp_root("ingest-opencode-idempotent");
    let data_dir = root.join(".jottrace");
    install_opencode_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 13"));
    assert!(first.contains("inserted_events: 13"));

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 2"));
    assert!(second.contains("events: 13"));
    assert!(second.contains("inserted_events: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    assert_eq!(generation_counts(&conn), vec![(0, 13)]);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_changed_opencode_sqlite_session_as_next_generation() {
    let root = temp_root("ingest-opencode-rewrite");
    let data_dir = root.join(".jottrace");
    let source_db = install_opencode_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("events: 13"));
    assert!(first.contains("inserted_events: 13"));

    Connection::open(&source_db)
        .expect("open source OpenCode db")
        .execute_batch(
            "UPDATE part
             SET data = '{\"type\":\"text\",\"text\":\"Synthetic child-session response, revised.\",\"time\":{\"start\":1770000005000,\"end\":1770000008000}}',
                 time_updated = 1770000008000
             WHERE id = 'prt_fixture_child_reply_0000000';
             UPDATE session
             SET time_updated = 1770000008000
             WHERE id = 'ses_fixture_child_000000000000';",
        )
        .expect("rewrite OpenCode fixture row");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 2"));
    assert!(second.contains("events: 19"));
    assert!(second.contains("inserted_events: 6"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (current_generation, event_count, ended_at): (i64, i64, String) = conn
        .query_row(
            "SELECT current_generation, event_count, ended_at
             FROM sessions
             WHERE source = 'opencode'
               AND source_session_id = ?1",
            [OPENCODE_CHILD_SESSION_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("OpenCode rewritten session state");
    assert_eq!(current_generation, 1);
    assert_eq!(event_count, 6);
    assert_eq!(ended_at, "2026-02-02T02:40:08.000Z");
    assert_eq!(generation_counts(&conn), vec![(0, 13), (1, 6)]);
    assert!(event_payload(&conn, 1, 4).contains("revised"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_invalid_opencode_sqlite_without_blocking_unrelated_files() {
    let root = temp_root("ingest-opencode-invalid");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let bad_db = root.join(".local/share/opencode/opencode.db");
    write_text_file(&bad_db, "not a sqlite database");

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 12"));
    assert!(ingest.contains("inserted_events: 12"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "opencode");
    assert_eq!(error.source_session_id.as_deref(), Some("opencode.db"));
    assert_eq!(error.file_path, bad_db);
    assert_eq!(error.error_kind, "invalid_session_meta");
    assert!(error.message.contains("OpenCode SQLite store"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_opencode_db_level_error_after_successful_import_under_fallback_identity() {
    let root = temp_root("ingest-opencode-invalid-after-success");
    let data_dir = root.join(".jottrace");
    let source_db = install_opencode_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("sessions: 2"));
    assert!(first.contains("events: 13"));
    assert!(first.contains("unresolved_ingest_errors: 0"));

    write_text_file(&source_db, "not a sqlite database anymore");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 3"));
    assert!(second.contains("events: 13"));
    assert!(second.contains("inserted_events: 0"));
    assert!(second.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "opencode");
    assert_eq!(error.source_session_id.as_deref(), Some("opencode.db"));
    assert_eq!(error.file_path, source_db);
    assert_eq!(error.error_kind, "invalid_session_meta");
    assert!(error.message.contains("OpenCode SQLite store"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_invalid_gemini_chat_json_without_blocking_unrelated_files() {
    let root = temp_root("ingest-gemini-invalid-json");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let bad_file = root
        .join(".gemini/tmp/bad-project/chats")
        .join("session-bad-gemini.json");
    write_text_file(&bad_file, "{\"sessionId\":");

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 2"));
    assert!(ingest.contains("events: 12"));
    assert!(ingest.contains("inserted_events: 12"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "gemini_cli");
    assert_eq!(
        error.source_session_id.as_deref(),
        Some("session-bad-gemini")
    );
    assert_eq!(error.file_path, bad_file);
    assert_eq!(error.error_kind, "invalid_session_meta");
    assert!(!error.message.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_invalid_gemini_chat_shape_with_readable_session_id() {
    let root = temp_root("ingest-gemini-invalid-shape");
    let data_dir = root.join(".jottrace");
    let bad_file = root
        .join(".gemini/tmp/bad-project/chats")
        .join("session-bad-gemini.json");
    write_text_file(
        &bad_file,
        r#"{"sessionId":"33333333-3333-4333-8333-333333333334","messages":{}}"#,
    );

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 0"));
    assert!(ingest.contains("inserted_events: 0"));
    assert!(ingest.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "gemini_cli");
    assert_eq!(
        error.source_session_id.as_deref(),
        Some("33333333-3333-4333-8333-333333333334")
    );
    assert_eq!(error.file_path, bad_file);
    assert_eq!(error.error_kind, "invalid_session_meta");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_invalid_gemini_rewrite_against_existing_session() {
    let root = temp_root("ingest-gemini-invalid-rewrite");
    let data_dir = root.join(".jottrace");
    let session_file = install_gemini_fixture(&root);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("sessions: 1"));
    assert!(first.contains("events: 4"));
    assert!(first.contains("unresolved_ingest_errors: 0"));

    write_text_file(&session_file, "{\"sessionId\":");

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 1"));
    assert!(second.contains("events: 4"));
    assert!(second.contains("inserted_events: 0"));
    assert!(second.contains("unresolved_ingest_errors: 1"));

    let errors = jottrace::storage::unresolved_ingest_errors_for_path(&db_path(&data_dir), 10)
        .expect("unresolved ingest errors");
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.source, "gemini_cli");
    assert_eq!(
        error.source_session_id.as_deref(),
        Some(GEMINI_FIXTURE_SESSION_ID)
    );
    assert_eq!(error.file_path, session_file);
    assert_eq!(error.error_kind, "invalid_session_meta");

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let session_ids: Vec<String> = conn
        .prepare(
            "SELECT source_session_id
             FROM sessions
             WHERE source = 'gemini_cli'
             ORDER BY source_session_id",
        )
        .expect("prepare gemini session ids")
        .query_map([], |row| row.get(0))
        .expect("query gemini session ids")
        .map(|row| row.expect("gemini session id"))
        .collect();
    assert_eq!(session_ids, vec![GEMINI_FIXTURE_SESSION_ID]);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_preserves_factory_session_events_from_sanitized_fixture() {
    let root = temp_root("ingest-factory-session");
    let data_dir = root.join(".jottrace");
    let session_file = install_factory_fixture(&root);

    let ingest = run_ingest(&root, &data_dir);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 5"));
    assert!(ingest.contains("inserted_events: 5"));
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
             WHERE source = 'factory'",
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
        .expect("factory session metadata");
    assert_eq!(source_session_id, FACTORY_FIXTURE_SESSION_ID);
    assert_eq!(cwd, "/Users/fixture/Workspace/jottrace");
    assert_eq!(started_at, "2026-05-06T10:00:01.000Z");
    assert_eq!(ended_at, "2026-05-06T10:00:04.000Z");
    assert_eq!(event_count, 5);
    assert_eq!(PathBuf::from(file_path), session_file);

    let mut event_types = Vec::new();
    jottrace::storage::for_each_decoded_event_payload_for_session(
        &db_path(&data_dir),
        "factory",
        FACTORY_FIXTURE_SESSION_ID,
        None,
        |payload| {
            event_types.push(
                serde_json::from_slice::<serde_json::Value>(payload)
                    .expect("factory payload should be json")["type"]
                    .as_str()
                    .expect("factory event type")
                    .to_string(),
            );
            Ok(())
        },
    )
    .expect("decoded factory payloads");
    assert_eq!(
        event_types,
        vec![
            "session_start",
            "message",
            "message",
            "todo_state",
            "compaction_state"
        ]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_links_factory_settings_as_source_metadata() {
    let root = temp_root("ingest-factory-settings");
    let data_dir = root.join(".jottrace");
    let session_file = install_factory_fixture(&root);
    let settings_file = session_file.with_extension("settings.json");

    run_ingest(&root, &data_dir);

    let source_metadata = factory_source_metadata(&data_dir);
    assert_eq!(
        source_metadata["settings_path"].as_str(),
        Some(settings_file.to_string_lossy().as_ref())
    );
    assert_eq!(
        source_metadata["settings_file_size"].as_i64(),
        Some(
            fs::metadata(&settings_file)
                .expect("settings metadata")
                .len() as i64
        )
    );
    assert!(
        source_metadata["settings_content_fingerprint"]
            .as_str()
            .is_some_and(|fingerprint| fingerprint.len() == 16)
    );
    assert_eq!(
        source_metadata["settings"]["model"].as_str(),
        Some("gpt-fixture")
    );
    assert_eq!(
        source_metadata["settings"]["reasoningEffort"].as_str(),
        Some("medium")
    );
    assert_eq!(
        source_metadata["settings"]["tokenUsage"]["totalTokens"].as_i64(),
        Some(1440)
    );
    assert!(source_metadata["settings_parse_error"].is_null());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_uses_factory_session_start_id_and_updates_settings_metadata_without_duplicate_events() {
    let root = temp_root("ingest-factory-settings-update");
    let data_dir = root.join(".jottrace");
    let session_file = factory_project_dir(&root).join("renamed-factory-session.jsonl");
    let settings_file = session_file.with_extension("settings.json");
    copy_reader_fixture(FACTORY_FIXTURE_SESSION, &session_file);
    copy_reader_fixture(FACTORY_FIXTURE_SETTINGS, &settings_file);
    set_modified(&session_file, 1_800_000_000);
    set_modified(&settings_file, 1_800_000_000);

    let first = run_ingest(&root, &data_dir);
    assert!(first.contains("sessions: 1"));
    assert!(first.contains("events: 5"));
    assert!(first.contains("inserted_events: 5"));
    let first_metadata = factory_source_metadata(&data_dir);

    write_text_file(
        &settings_file,
        "{\"model\":\"gpt-fixture-updated\",\"reasoningEffort\":\"high\"}\n",
    );
    set_modified(&settings_file, 1_800_000_100);

    let second = run_ingest(&root, &data_dir);
    assert!(second.contains("sessions: 1"));
    assert!(second.contains("events: 5"));
    assert!(second.contains("inserted_events: 0"));

    let conn = Connection::open(db_path(&data_dir)).expect("open preserved db");
    let (source_session_id, file_path): (String, String) = conn
        .query_row(
            "SELECT source_session_id, file_path
             FROM sessions
             WHERE source = 'factory'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("factory session identity");
    assert_eq!(source_session_id, FACTORY_FIXTURE_SESSION_ID);
    assert_eq!(PathBuf::from(file_path), session_file);

    let second_metadata = factory_source_metadata(&data_dir);
    assert_ne!(
        first_metadata["settings_content_fingerprint"],
        second_metadata["settings_content_fingerprint"]
    );
    assert_eq!(
        second_metadata["settings_path"].as_str(),
        Some(settings_file.to_string_lossy().as_ref())
    );
    assert_eq!(
        second_metadata["settings"]["model"].as_str(),
        Some("gpt-fixture-updated")
    );
    assert_eq!(
        second_metadata["settings"]["reasoningEffort"].as_str(),
        Some("high")
    );
    assert_eq!(generation_counts(&conn), vec![(0, 5)]);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ingest_records_invalid_factory_session_start_without_blocking_other_files() {
    let root = temp_root("ingest-factory-invalid-start");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    let bad_file = factory_project_dir(&root).join("bad-factory-session.jsonl");
    write_text_file(
        &bad_file,
        "{\"type\":\"message\",\"timestamp\":\"2026-05-06T10:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"bad start\"}]}}\n",
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
    assert_eq!(error.source, "factory");
    assert_eq!(
        error.source_session_id.as_deref(),
        Some("bad-factory-session")
    );
    assert_eq!(error.file_path, bad_file);
    assert_eq!(error.error_kind, "invalid_session_meta");
    assert!(error.message.contains("session_start"));

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
    assert!(stdout.contains("eligible_raw_events: 1"));
    assert!(stdout.contains("unresolved_ingest_errors: 0"));
    assert!(stdout.contains("next: rerun with `jottrace compact --apply`"));
    assert!(!stdout.contains("db: "));
    assert!(!stdout.contains("raw_events_before: "));
    assert!(!stdout.contains("zstd_events_before: "));
    assert!(!stdout.contains("converted_events: "));
    assert!(!stdout.contains("stored_bytes_before: "));
    assert!(!stdout.contains("sqlite_reclaimable_bytes: "));
    assert!(
        compact_metric(&stdout, "estimated_bytes_saved") > 0,
        "{stdout}"
    );

    let detailed = Command::new(binary())
        .args(["compact", "--details"])
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace compact --details");

    assert!(
        detailed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&detailed.stdout),
        String::from_utf8_lossy(&detailed.stderr)
    );
    let detailed_stdout =
        String::from_utf8(detailed.stdout).expect("compact stdout should be utf-8");
    assert!(detailed_stdout.contains(&format!("db: {}", db_path(&data_dir).display())));
    assert!(detailed_stdout.contains("raw_events_before: 1"));
    assert!(detailed_stdout.contains("zstd_events_before: 0"));
    assert!(detailed_stdout.contains("eligible_raw_events: 1"));
    assert!(detailed_stdout.contains("converted_events: 0"));
    assert!(
        compact_metric(&detailed_stdout, "stored_bytes_after")
            < compact_metric(&detailed_stdout, "stored_bytes_before"),
        "{detailed_stdout}"
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
        .args(["compact", "--apply", "--batch-size", "2", "--details"])
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
        .args(["compact", "--apply", "--details"])
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
        .args(["compact", "--apply", "--details"])
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
        .args(["compact", "--vacuum", "--details"])
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

fn factory_source_metadata(data_dir: &Path) -> serde_json::Value {
    let conn = Connection::open(db_path(data_dir)).expect("open preserved db");
    let source_metadata: String = conn
        .query_row(
            "SELECT source_metadata
             FROM sessions
             WHERE source = 'factory'
               AND source_session_id = ?1",
            [FACTORY_FIXTURE_SESSION_ID],
            |row| row.get(0),
        )
        .expect("factory source metadata");
    serde_json::from_str(&source_metadata).expect("factory source metadata should be json")
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

fn install_pi_agent_fixture(root: &Path) -> PathBuf {
    let session_file = root.join(".pi/agent").join(
        PI_AGENT_FIXTURE_SESSION
            .strip_prefix("pi-agent/")
            .expect("Pi fixture path should be under pi-agent"),
    );
    copy_reader_fixture(PI_AGENT_FIXTURE_SESSION, &session_file);
    session_file
}

fn install_pi_agent_nested_run_fixture(root: &Path) -> PathBuf {
    let session_file = root.join(".pi/agent").join(
        PI_AGENT_NESTED_RUN_FIXTURE_SESSION
            .strip_prefix("pi-agent/")
            .expect("Pi nested run fixture path should be under pi-agent"),
    );
    copy_reader_fixture(PI_AGENT_NESTED_RUN_FIXTURE_SESSION, &session_file);
    session_file
}

fn install_claude_local_agent_fixture(root: &Path) -> PathBuf {
    let base = root.join("Library/Application Support/Claude/local-agent-mode-sessions/desktop-fixture/workspace-fixture");
    let local_session = format!("local_{CLAUDE_LOCAL_AGENT_FIXTURE_SESSION_ID}");
    copy_reader_fixture(
        CLAUDE_LOCAL_AGENT_FIXTURE_METADATA,
        &base.join(format!("{local_session}.json")),
    );
    let audit_file = base.join(local_session).join("audit.jsonl");
    copy_reader_fixture(CLAUDE_LOCAL_AGENT_FIXTURE_AUDIT, &audit_file);
    audit_file
}

fn install_gemini_fixture(root: &Path) -> PathBuf {
    let session_file = root
        .join(".gemini/tmp/fixture-project/chats")
        .join("session-2026-05-06T09-00-gemini000.json");
    copy_reader_fixture(GEMINI_FIXTURE_SESSION, &session_file);
    session_file
}

fn install_factory_fixture(root: &Path) -> PathBuf {
    let session_file =
        factory_project_dir(root).join(format!("{FACTORY_FIXTURE_SESSION_ID}.jsonl"));
    let settings_file = session_file.with_extension("settings.json");
    copy_reader_fixture(FACTORY_FIXTURE_SESSION, &session_file);
    copy_reader_fixture(FACTORY_FIXTURE_SETTINGS, &settings_file);
    session_file
}

fn install_opencode_fixture(root: &Path) -> PathBuf {
    install_sqlite_fixture(
        root,
        ".local/share/opencode/opencode.db",
        OPENCODE_FIXTURE_SQL,
        "OpenCode",
    )
}

fn install_hermes_fixture(root: &Path) -> PathBuf {
    install_sqlite_fixture(root, ".hermes/state.db", HERMES_FIXTURE_SQL, "Hermes")
}

fn install_sqlite_fixture(
    root: &Path,
    db_relative_path: &str,
    fixture_relative_path: &str,
    label: &str,
) -> PathBuf {
    let db_path = root.join(db_relative_path);
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|_| panic!("create {label} fixture parent"));
    }
    let sql = fs::read_to_string(reader_fixture(fixture_relative_path))
        .unwrap_or_else(|_| panic!("read {label} fixture SQL"));
    let conn = Connection::open(&db_path).unwrap_or_else(|_| panic!("open {label} fixture db"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|_| panic!("load {label} fixture SQL"));
    db_path
}

fn replace_with_fixture(path: &Path, fixture_relative: &str) {
    copy_reader_fixture(fixture_relative, path);
}

fn claude_project_dir(root: &Path) -> PathBuf {
    root.join(".claude/projects/-Users-fixture-Workspace-jottrace")
}

fn factory_project_dir(root: &Path) -> PathBuf {
    root.join(".factory/sessions/-Users-fixture-Workspace-jottrace")
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

fn decoded_event_types(data_dir: &Path, source: &str, source_session_id: &str) -> Vec<String> {
    let mut event_types = Vec::new();
    jottrace::storage::for_each_decoded_event_payload_for_session(
        &db_path(data_dir),
        source,
        source_session_id,
        None,
        |payload| {
            let value: serde_json::Value =
                serde_json::from_slice(payload).expect("decoded event payload should be JSON");
            event_types.push(
                value["type"]
                    .as_str()
                    .expect("decoded event payload should have a type")
                    .to_string(),
            );
            Ok(())
        },
    )
    .expect("decoded event payloads");
    event_types
}

fn run_pack(data_dir: &Path, archive: &Path) -> String {
    let output = Command::new(binary())
        .args(["pack", "--output"])
        .arg(archive)
        .env("JOTTRACE_HOME", data_dir)
        .output()
        .expect("run jottrace pack");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf-8")
}

fn run_settle(data_dir: &Path, archive: &Path, force: bool) -> std::process::Output {
    let mut command = Command::new(binary());
    command
        .arg("settle")
        .arg(archive)
        .env("JOTTRACE_HOME", data_dir);
    if force {
        command.arg("--force");
    }
    command.output().expect("run jottrace settle")
}

#[test]
fn pack_then_settle_round_trips_ingested_claude_fixture() {
    let root = temp_root("pack-roundtrip");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let archive = root.join("journal.tar.gz");
    install_primary_claude_fixture(&root);

    let ingest = run_ingest(&root, &src_data);
    assert!(ingest.contains("sessions: 1"));
    assert!(ingest.contains("events: 12"));

    let pack_stdout = run_pack(&src_data, &archive);
    // The archive carries the same session/event totals the source DB reports,
    // so users can sanity-check the receipt before copying the file.
    assert!(pack_stdout.contains("sessions: 1"));
    assert!(pack_stdout.contains("events: 12"));
    assert!(
        archive.exists(),
        "pack must produce the archive at --output"
    );
    #[cfg(unix)]
    assert_eq!(
        mode(&archive),
        0o600,
        "archive permissions should be 0600 since it can contain private transcripts"
    );

    let settle = run_settle(&dst_data, &archive, false);
    assert!(
        settle.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&settle.stdout),
        String::from_utf8_lossy(&settle.stderr)
    );
    let settle_stdout = String::from_utf8_lossy(&settle.stdout);
    assert!(settle_stdout.contains("sessions: 1"));
    assert!(settle_stdout.contains("events: 12"));

    #[cfg(unix)]
    {
        assert_eq!(mode(&dst_data), 0o700);
        assert_eq!(mode(&db_path(&dst_data)), 0o600);
    }
    // The lock file is a runtime artifact and would cause `LockHeld` confusion
    // on the receiving machine if it travelled in the archive.
    assert!(!dst_data.join(jottrace::LOCK_FILE_NAME).exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn pack_refuses_to_overwrite_existing_archive() {
    let root = temp_root("pack-no-clobber");
    let data_dir = root.join(".jottrace");
    let archive = root.join("journal.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &data_dir);
    run_pack(&data_dir, &archive);

    let second = Command::new(binary())
        .args(["pack", "--output"])
        .arg(&archive)
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run second pack");
    assert!(!second.status.success(), "second pack should refuse");
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("already exists"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn settle_refuses_existing_journal_without_force() {
    let root = temp_root("settle-no-clobber");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let archive = root.join("journal.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &archive);

    // First settle populates the destination journal.
    let first = run_settle(&dst_data, &archive, false);
    assert!(first.status.success());

    // Second settle should refuse without --force.
    let refused = run_settle(&dst_data, &archive, false);
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("already contains journal data"));
    assert!(stderr.contains("--force"));

    // --force should succeed.
    let forced = run_settle(&dst_data, &archive, true);
    assert!(
        forced.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&forced.stdout),
        String::from_utf8_lossy(&forced.stderr)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn pack_help_does_not_initialize_database() {
    let root = temp_root("pack-help");
    let data_dir = root.join(".jottrace");

    for help_arg in ["-h", "--help"] {
        let output = Command::new(binary())
            .args(["pack", help_arg])
            .env("JOTTRACE_HOME", &data_dir)
            .output()
            .expect("run jottrace pack help");
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("jottrace pack"));
        assert!(stdout.contains("--output"));
        assert!(!db_path(&data_dir).exists());
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn settle_reports_missing_archive() {
    let root = temp_root("settle-missing");
    let data_dir = root.join(".jottrace");

    let output = Command::new(binary())
        .args(["settle"])
        .arg(root.join("does-not-exist.tar.gz"))
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace settle");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a readable archive file"));

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_rejects_archive_with_symlink_entry() {
    use std::os::unix::fs::symlink;

    let root = temp_root("settle-symlink");
    let staging = root.join("staging");
    let archive = root.join("evil.tar.gz");
    let data_dir = root.join(".jottrace");

    // A crafted archive containing a symlink that points outside the journal.
    // Without the file_type guard, settle's chmod walk would follow the link
    // and re-permission the target.
    fs::create_dir_all(&staging).expect("create staging");
    fs::write(staging.join("db.sqlite"), b"").expect("write placeholder db");
    symlink("/etc/passwd", staging.join("evil")).expect("create symlink");
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .args(["-C"])
        .arg(&staging)
        .arg(".")
        .status()
        .expect("run tar to build fixture");
    assert!(tar.success());

    let output = run_settle(&data_dir, &archive, false);
    assert!(
        !output.status.success(),
        "settle should refuse archives containing symlinks"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unsafe entry"));
    assert!(stderr.contains("symbolic link"));

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_never_writes_symlink_into_journal_home() {
    use std::os::unix::fs::symlink;

    let root = temp_root("settle-symlink-stage");
    let fixture = root.join("staging");
    let archive = root.join("evil.tar.gz");
    let data_dir = root.join(".jottrace");

    // Build an archive that contains BOTH a regular file and a malicious
    // symlink. With staging extraction the symlink should never appear under
    // the live journal directory — it only lands inside the disposable
    // staging subtree, which is removed when validation rejects it.
    fs::create_dir_all(&fixture).expect("create fixture");
    fs::write(fixture.join("db.sqlite"), b"").expect("write placeholder db");
    symlink("/etc/passwd", fixture.join("evil")).expect("create symlink");
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("run tar to build fixture");
    assert!(tar.success());

    let output = run_settle(&data_dir, &archive, false);
    assert!(!output.status.success());

    assert!(!data_dir.join("evil").exists());
    assert!(!data_dir.join("db.sqlite").exists());
    let symlinked = std::fs::symlink_metadata(data_dir.join("evil"));
    assert!(
        symlinked.is_err(),
        "leftover symlink must not exist after a failed settle"
    );
    // The staging dir must also be cleaned up so it does not block the next
    // settle attempt under the non-empty check.
    assert!(!data_dir.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_preserves_existing_journal_when_archive_is_corrupt() {
    let root = temp_root("settle-corrupt-archive");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let good_archive = root.join("good.tar.gz");
    let bad_archive = root.join("truncated.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &good_archive);
    run_settle(&dst_data, &good_archive, false);

    // Snapshot the live db.sqlite so we can prove --force did not touch it.
    let before = fs::read(db_path(&dst_data)).expect("read db before settle");

    // Truncate a real archive so tar errors midway. Without staging this
    // would happen AFTER `clear_journal_contents` and the user would lose
    // their previous journal to nothing.
    let bytes = fs::read(&good_archive).expect("read good archive");
    fs::write(&bad_archive, &bytes[..200]).expect("write truncated archive");

    let output = run_settle(&dst_data, &bad_archive, true);
    assert!(
        !output.status.success(),
        "tar must reject a truncated input"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.to_lowercase().contains("tar"));

    let after = fs::read(db_path(&dst_data)).expect("read db after failed settle");
    assert_eq!(
        before, after,
        "a failed settle must not modify the existing journal"
    );
    assert!(!dst_data.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_preserves_existing_journal_when_archive_lacks_database() {
    // A tarball with no `db.sqlite` is a plausible mistake (wrong archive,
    // partial pack on a different machine). Without validation, settle --force
    // would happily wipe the live journal, then `status_for_path` would create
    // a fresh empty database in its place — silent data loss with a 0-event
    // success report.
    let root = temp_root("settle-archive-no-db");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let good_archive = root.join("good.tar.gz");
    let empty_archive = root.join("no-db.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &good_archive);
    run_settle(&dst_data, &good_archive, false);

    let before = fs::read(db_path(&dst_data)).expect("read db before settle");

    // Build an archive that contains only an unrelated file.
    let fixture = root.join("decoy");
    fs::create_dir_all(&fixture).expect("create decoy");
    fs::write(fixture.join("README"), b"not a journal").expect("write decoy entry");
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&empty_archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("run tar to build decoy");
    assert!(tar.success());

    let output = run_settle(&dst_data, &empty_archive, true);
    assert!(
        !output.status.success(),
        "settle must refuse an archive that omits db.sqlite"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("db.sqlite"));
    assert!(stderr.contains("does not contain"));

    let after = fs::read(db_path(&dst_data)).expect("read db after failed settle");
    assert_eq!(
        before, after,
        "a failed settle must not modify the existing journal"
    );
    assert!(!dst_data.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_preserves_existing_journal_when_archive_database_is_blank() {
    // A zero-byte `db.sqlite` is treated by SQLite as a freshly created
    // database with `user_version = 0`. `storage::open_database` would migrate
    // it to LATEST_SCHEMA_VERSION and report success on the very kind of
    // broken archive we want to reject. The validator must reach into the
    // user_version pragma BEFORE we touch the live journal.
    let root = temp_root("settle-archive-blank-db");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let good_archive = root.join("good.tar.gz");
    let blank_archive = root.join("blank-db.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &good_archive);
    run_settle(&dst_data, &good_archive, false);

    let before = fs::read(db_path(&dst_data)).expect("read db before settle");

    let fixture = root.join("blank");
    fs::create_dir_all(&fixture).expect("create blank fixture");
    fs::write(fixture.join("db.sqlite"), b"").expect("write zero-byte db");
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&blank_archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("run tar to build blank fixture");
    assert!(tar.success());

    let output = run_settle(&dst_data, &blank_archive, true);
    assert!(
        !output.status.success(),
        "settle must refuse an archive whose db.sqlite has no Jottrace schema"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Jottrace"));

    let after = fs::read(db_path(&dst_data)).expect("read db after failed settle");
    assert_eq!(
        before, after,
        "a failed settle must not modify the existing journal"
    );
    assert!(!dst_data.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_preserves_existing_journal_when_archive_db_has_lookalike_tables() {
    // A `db.sqlite` whose `user_version` falls in Jottrace's accepted range
    // could still expose tables named `sessions`/`events`/`ingest_errors`
    // with completely unrelated columns. The name-only check would have
    // waved this archive through; the column-aware probe must refuse it
    // before `clear_journal_contents` wipes the live data.
    let root = temp_root("settle-archive-lookalike-tables");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let good_archive = root.join("good.tar.gz");
    let lookalike_archive = root.join("lookalike.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &good_archive);
    run_settle(&dst_data, &good_archive, false);

    let before = fs::read(db_path(&dst_data)).expect("read db before settle");

    let fixture = root.join("lookalike");
    fs::create_dir_all(&fixture).expect("create lookalike fixture");
    let lookalike_db_path = fixture.join("db.sqlite");
    {
        let conn = Connection::open(&lookalike_db_path).expect("create lookalike sqlite db");
        // Same table names, none of the columns Jottrace's queries touch.
        conn.execute("CREATE TABLE sessions (only TEXT)", [])
            .expect("create lookalike sessions");
        conn.execute("CREATE TABLE events (only TEXT)", [])
            .expect("create lookalike events");
        conn.execute("CREATE TABLE ingest_errors (only TEXT)", [])
            .expect("create lookalike ingest_errors");
        conn.pragma_update(None, "user_version", 5_i64)
            .expect("set user_version");
    }
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&lookalike_archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("run tar to build lookalike fixture");
    assert!(tar.success());

    let output = run_settle(&dst_data, &lookalike_archive, true);
    assert!(
        !output.status.success(),
        "settle must refuse a SQLite database whose tables have unrelated schemas"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Jottrace database")
            && (stderr.contains("no such column") || stderr.contains("unexpected schema")),
        "stderr should explain why the database was rejected, got: {stderr}"
    );

    let after = fs::read(db_path(&dst_data)).expect("read db after failed settle");
    assert_eq!(
        before, after,
        "a failed settle must not modify the existing journal"
    );
    assert!(!dst_data.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn pack_refuses_when_journal_has_no_database() {
    // A JOTTRACE_HOME without `db.sqlite` (e.g. auto-created by the updater
    // before any ingest has run) used to produce a "successful" archive that
    // `settle` later rejected — silent failure on the producing side. Pack
    // must surface the missing database immediately so the user does not ship
    // a useless tar.gz to another machine.
    let root = temp_root("pack-empty-journal");
    let data_dir = root.join(".jottrace");
    fs::create_dir_all(&data_dir).expect("create empty journal");
    #[cfg(unix)]
    {
        fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700))
            .expect("set private permissions on empty journal");
    }
    let archive = root.join("empty-journal.tar.gz");

    let output = Command::new(binary())
        .args(["pack", "--output"])
        .arg(&archive)
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace pack");
    assert!(
        !output.status.success(),
        "pack must refuse a journal directory with no db.sqlite"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no db.sqlite") || stderr.contains("has no db.sqlite"),
        "stderr should explain the missing database, got: {stderr}"
    );
    assert!(
        !archive.exists(),
        "pack must not leave a partial archive after refusing the journal"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn pack_excludes_stale_pending_settle_directory() {
    // A previous `settle` that crashed mid-flight can leave a `.pending-settle`
    // directory in the live journal. Packing it would yield an archive whose
    // own settle creates `.pending-settle/.pending-settle`, breaking the
    // staged-rename step after `clear_journal_contents` had already wiped
    // the receiving journal. Pack must skip the staging dir.
    let root = temp_root("pack-skip-pending-settle");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let archive = root.join("journal.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);

    // Simulate a crashed settle by hand: drop a stray `.pending-settle/`
    // entry into the source journal that pack would otherwise pick up.
    let stale = src_data.join(".pending-settle");
    fs::create_dir_all(&stale).expect("seed stale pending-settle dir");
    fs::write(stale.join("leftover"), b"crash debris").expect("seed stale file");

    run_pack(&src_data, &archive);

    // Listing the archive: the staging dir must not appear.
    let list = Command::new("tar")
        .args(["-tzf"])
        .arg(&archive)
        .output()
        .expect("list packed archive");
    assert!(list.status.success());
    let entries = String::from_utf8_lossy(&list.stdout);
    assert!(
        !entries.contains(".pending-settle"),
        "packed archive must not contain the staging dir, got entries:\n{entries}"
    );

    // And the resulting archive must settle cleanly end-to-end.
    let settle = run_settle(&dst_data, &archive, false);
    assert!(
        settle.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&settle.stdout),
        String::from_utf8_lossy(&settle.stderr)
    );

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_allows_destination_with_only_runtime_sentinels() {
    // On installer-managed binaries, `maybe_spawn_auto_update` can drop an
    // `auto-update-check` stamp into a freshly created `JOTTRACE_HOME`
    // before any ingest has run. The user's first settle then sees a
    // non-empty directory and refuses without `--force` — which would
    // suggest the new machine had real journal data worth protecting, even
    // though those files are runtime artefacts the receiving machine
    // re-creates as needed. The empty check must look past them.
    let root = temp_root("settle-with-only-runtime-sentinels");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let archive = root.join("journal.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &archive);

    // Seed only runtime sentinels on the destination — no real journal data.
    fs::create_dir_all(&dst_data).expect("create destination dir");
    fs::set_permissions(&dst_data, fs::Permissions::from_mode(0o700)).expect("set private mode");
    fs::write(dst_data.join("auto-update-check"), b"").expect("seed auto-update sentinel");

    let output = run_settle(&dst_data, &archive, false);
    assert!(
        output.status.success(),
        "settle without --force should treat a directory holding only runtime sentinels as empty; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(db_path(&dst_data).exists(), "db.sqlite should be restored");

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_rejects_archive_with_top_level_pending_settle_entry() {
    // A crafted (or older/broken) archive may carry a `.pending-settle/`
    // entry at the top level. Without preflight rejection, extraction would
    // create `data_dir/.pending-settle/.pending-settle/`, validation would
    // pass, `clear_journal_contents` would wipe the live journal, and the
    // rename of `staging/.pending-settle` over `data_dir/.pending-settle`
    // (the live staging dir) would fail — destroying the user's data.
    let root = temp_root("settle-archive-with-staging-entry");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let good_archive = root.join("good.tar.gz");
    let bad_archive = root.join("with-staging.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &good_archive);
    run_settle(&dst_data, &good_archive, false);

    let before = fs::read(db_path(&dst_data)).expect("read db before settle");

    // Build a fixture by extracting the good archive, planting a
    // `.pending-settle/` entry next to `db.sqlite`, and repacking.
    let fixture = root.join("fixture");
    fs::create_dir_all(&fixture).expect("create fixture root");
    let extract = Command::new("tar")
        .args(["-xzf"])
        .arg(&good_archive)
        .args(["-C"])
        .arg(&fixture)
        .status()
        .expect("extract good archive");
    assert!(extract.success());
    let stale = fixture.join(".pending-settle");
    fs::create_dir_all(&stale).expect("seed staging entry");
    fs::write(stale.join("leftover"), b"crashed settle").expect("seed stale file");
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&bad_archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("repack with staging entry");
    assert!(tar.success());

    let output = run_settle(&dst_data, &bad_archive, true);
    assert!(
        !output.status.success(),
        "settle must refuse an archive carrying a .pending-settle/ entry"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("reserved top-level entry"),
        "stderr should explain why the archive was rejected, got: {stderr}"
    );

    let after = fs::read(db_path(&dst_data)).expect("read db after failed settle");
    assert_eq!(
        before, after,
        "a failed settle must not modify the existing journal"
    );
    // Pre-flight rejection means staging is never even created on this path.
    assert!(!dst_data.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_rejects_archive_with_top_level_lock_file_entry() {
    // A hand-crafted archive (or a manual `tar` of `~/.jottrace` made without
    // jottrace's pack helper) could include a top-level `jottrace.lock` entry.
    // Promotion would either rename a staged regular file over the inode our
    // `_lock` is flocking — breaking mutual exclusion with concurrent
    // ingest/compact runs — or fail outright if the staged entry is the wrong
    // kind, after `clear_journal_contents` had already wiped the live data.
    let root = temp_root("settle-archive-with-lock-entry");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let good_archive = root.join("good.tar.gz");
    let bad_archive = root.join("with-lock.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &good_archive);
    run_settle(&dst_data, &good_archive, false);

    let before = fs::read(db_path(&dst_data)).expect("read db before settle");

    let fixture = root.join("fixture");
    fs::create_dir_all(&fixture).expect("create fixture root");
    let extract = Command::new("tar")
        .args(["-xzf"])
        .arg(&good_archive)
        .args(["-C"])
        .arg(&fixture)
        .status()
        .expect("extract good archive");
    assert!(extract.success());
    fs::write(fixture.join(jottrace::LOCK_FILE_NAME), b"pid=999").expect("seed lock-file entry");
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&bad_archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("repack with lock-file entry");
    assert!(tar.success());

    let output = run_settle(&dst_data, &bad_archive, true);
    assert!(
        !output.status.success(),
        "settle must refuse an archive carrying a top-level jottrace.lock entry"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("reserved top-level entry"),
        "stderr should explain why the archive was rejected, got: {stderr}"
    );

    let after = fs::read(db_path(&dst_data)).expect("read db after failed settle");
    assert_eq!(
        before, after,
        "a failed settle must not modify the existing journal"
    );
    assert!(!dst_data.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_preserves_existing_journal_when_archive_db_missing_post_v1_columns() {
    // An archive whose `db.sqlite` claims `user_version = LATEST_SCHEMA_VERSION`
    // but is missing a column added by a later migration (here:
    // `sessions.source_metadata` from migration 008) skips the migration
    // runner entirely. Without column-probe coverage of post-v1 schema, the
    // validator would have nodded the archive through. The receiving machine
    // would then fail on the first ingest after the live journal had already
    // been wiped.
    let root = temp_root("settle-archive-missing-late-column");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let good_archive = root.join("good.tar.gz");
    let stripped_archive = root.join("stripped.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &good_archive);
    run_settle(&dst_data, &good_archive, false);

    let before = fs::read(db_path(&dst_data)).expect("read db before settle");

    // Construct a Jottrace-shaped DB at LATEST_SCHEMA_VERSION but with the
    // `source_metadata` column omitted from `sessions`.
    let fixture = root.join("stripped");
    fs::create_dir_all(&fixture).expect("create stripped fixture");
    let stripped_db = fixture.join("db.sqlite");
    {
        let conn = Connection::open(&stripped_db).expect("create stripped sqlite db");
        conn.execute_batch(
            "CREATE TABLE sessions (
                id INTEGER PRIMARY KEY,
                source TEXT NOT NULL,
                source_session_id TEXT NOT NULL,
                file_path TEXT,
                cwd TEXT,
                parent_session_id INTEGER,
                started_at TEXT,
                ended_at TEXT,
                current_generation INTEGER NOT NULL DEFAULT 0,
                file_mtime INTEGER,
                file_size INTEGER,
                content_fingerprint TEXT,
                next_read_offset INTEGER NOT NULL DEFAULT 0,
                event_count INTEGER NOT NULL DEFAULT 0,
                last_read_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE events (
                session_id INTEGER NOT NULL,
                generation INTEGER NOT NULL,
                seq INTEGER NOT NULL,
                ts TEXT,
                payload BLOB NOT NULL,
                codec TEXT NOT NULL,
                payload_size INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (session_id, generation, seq)
            );
            CREATE TABLE ingest_errors (
                id INTEGER PRIMARY KEY,
                source TEXT NOT NULL,
                source_session_id TEXT,
                session_id INTEGER,
                file_path TEXT NOT NULL,
                generation INTEGER,
                byte_offset INTEGER,
                line_number INTEGER,
                error_kind TEXT NOT NULL,
                message TEXT NOT NULL,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                occurrence_count INTEGER NOT NULL DEFAULT 1,
                resolved_at TEXT,
                resolution_note TEXT
            );",
        )
        .expect("create stripped tables");
        // Claim the latest schema version so the migration runner skips and
        // only the column-aware probe can refuse this archive.
        conn.pragma_update(None, "user_version", 8_i64)
            .expect("set user_version");
    }
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&stripped_archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("repack stripped fixture");
    assert!(tar.success());

    let output = run_settle(&dst_data, &stripped_archive, true);
    assert!(
        !output.status.success(),
        "settle must refuse an archive missing post-v1 schema columns"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unexpected schema") && stderr.contains("source_metadata"),
        "stderr should explain the missing column, got: {stderr}"
    );

    let after = fs::read(db_path(&dst_data)).expect("read db after failed settle");
    assert_eq!(
        before, after,
        "a failed settle must not modify the existing journal"
    );
    assert!(!dst_data.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_rejects_symlink_archive_before_extraction() {
    // The threat: tar applies archive entries in stream order. A symlink
    // member followed by a regular file underneath that symlink (e.g.
    // `link -> /tmp/out` then `link/file`) lets the second write follow the
    // link and land outside the staging subtree on platforms where tar does
    // not refuse this by default. Pre-flight rejection means tar is never
    // invoked in extract mode for the archive at all, so .pending-settle is
    // never created and no on-disk symlink is ever materialised.
    use std::os::unix::fs::symlink;

    let root = temp_root("settle-symlink-pre-extract");
    let fixture = root.join("evil");
    let archive = root.join("evil.tar.gz");
    let data_dir = root.join(".jottrace");
    fs::create_dir_all(&fixture).expect("create fixture");
    fs::write(fixture.join("db.sqlite"), b"").expect("write placeholder db");
    symlink("/etc/passwd", fixture.join("link")).expect("create symlink");
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("run tar to build fixture");
    assert!(tar.success());

    let output = run_settle(&data_dir, &archive, false);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unsafe entry"));
    assert!(stderr.contains("symbolic link"));

    // With pre-flight inspection, staging is NEVER created on the rejection
    // path. The previous architecture created an empty `.pending-settle` and
    // relied on the StagingGuard drop to clean up; this is the strictly
    // stronger guarantee that catches a class of attacks tar could otherwise
    // commit between the create-staging and the validation step.
    assert!(
        !data_dir.join(".pending-settle").exists(),
        "rejected archives must not leave staging artifacts"
    );

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_preserves_existing_journal_when_archive_db_is_non_jottrace_sqlite() {
    // A SQLite file from another application can carry a `user_version` that
    // falls inside Jottrace's 1..=LATEST_SCHEMA_VERSION range and still lack
    // the tables Jottrace's queries need. Without a table-existence check,
    // the validator would have nodded the archive through, `clear_journal_contents`
    // would wipe the live journal, and the next reader would fail trying to
    // touch a `sessions` table that does not exist — by which time the
    // previous journal is already gone.
    let root = temp_root("settle-archive-non-jottrace-db");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let good_archive = root.join("good.tar.gz");
    let bogus_archive = root.join("non-jottrace.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &good_archive);
    run_settle(&dst_data, &good_archive, false);

    let before = fs::read(db_path(&dst_data)).expect("read db before settle");

    let fixture = root.join("non-jottrace");
    fs::create_dir_all(&fixture).expect("create non-jottrace fixture");
    let bogus_db_path = fixture.join("db.sqlite");
    {
        let conn = Connection::open(&bogus_db_path).expect("create bogus sqlite db");
        conn.execute("CREATE TABLE unrelated (k TEXT)", [])
            .expect("create unrelated table");
        // user_version sits inside Jottrace's accepted range so only a real
        // table-existence check can refuse this archive.
        conn.pragma_update(None, "user_version", 5_i64)
            .expect("set user_version");
    }
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&bogus_archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("run tar to build bogus fixture");
    assert!(tar.success());

    let output = run_settle(&dst_data, &bogus_archive, true);
    assert!(
        !output.status.success(),
        "settle must refuse a non-Jottrace SQLite database masquerading as a journal"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Jottrace database")
            && (stderr.contains("no such table") || stderr.contains("unexpected schema")),
        "stderr should explain why the database was rejected, got: {stderr}"
    );

    let after = fs::read(db_path(&dst_data)).expect("read db after failed settle");
    assert_eq!(
        before, after,
        "a failed settle must not modify the existing journal"
    );
    assert!(!dst_data.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn settle_rejects_archive_inside_journal_home() {
    let root = temp_root("settle-self-restore");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &data_dir);

    // Put the archive INSIDE the target journal — `--force` would otherwise
    // delete the archive via `clear_journal_contents` before tar could read
    // it, silently turning a plausible "restore from local backup" workflow
    // into data loss. Pack outside the journal (the producer-side check
    // refuses outputs landing under JOTTRACE_HOME) and then move the archive
    // in to simulate a user who copied a backup into their journal.
    let archive = data_dir.join("self-backup.tar.gz");
    let staging = root.join("self-backup.staging.tar.gz");
    run_pack(&data_dir, &staging);
    fs::rename(&staging, &archive).expect("move archive into journal");

    let output = run_settle(&data_dir, &archive, true);
    assert!(
        !output.status.success(),
        "settling an archive that lives inside JOTTRACE_HOME must fail fast"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("inside the target journal"));
    assert!(
        archive.exists(),
        "the rejection must happen before the archive is touched"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn pack_refuses_output_inside_source_journal() {
    // tar's `-C data_dir .` walk would otherwise include the partially-written
    // archive in its own output and race SQLite sidecars on names like
    // `db.sqlite-wal`. Pack must refuse the producer-side mistake of writing
    // its archive into the journal it is reading.
    let root = temp_root("pack-output-inside-journal");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &data_dir);

    let archive = data_dir.join("self.tar.gz");
    let output = Command::new(binary())
        .args(["pack", "--output"])
        .arg(&archive)
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace pack");
    assert!(
        !output.status.success(),
        "pack must refuse an --output path inside JOTTRACE_HOME"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("inside the source journal"),
        "stderr should explain the rejection, got: {stderr}"
    );
    assert!(
        !archive.exists(),
        "pack must not leave a partial archive after refusing the output path"
    );

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_rejects_archive_without_required_unique_index() {
    // An archive whose `db.sqlite` has the right columns and `user_version`
    // but is missing the unique `(source, source_session_id)` index would
    // pass earlier validation. The receiving machine's `INSERT OR IGNORE`
    // pattern would then silently degrade to a regular insert and grow
    // duplicate session rows on the next ingest. Validation must refuse the
    // archive before `clear_journal_contents` wipes the live data.
    let root = temp_root("settle-archive-without-unique-index");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let good_archive = root.join("good.tar.gz");
    let bad_archive = root.join("no-unique-index.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &good_archive);
    run_settle(&dst_data, &good_archive, false);

    let before = fs::read(db_path(&dst_data)).expect("read db before settle");

    // Build a Jottrace-shaped DB at LATEST_SCHEMA_VERSION with every column
    // the probes look for but WITHOUT the unique session index.
    let fixture = root.join("no-unique");
    fs::create_dir_all(&fixture).expect("create fixture");
    let stripped_db = fixture.join("db.sqlite");
    {
        let conn = Connection::open(&stripped_db).expect("create stripped sqlite db");
        conn.execute_batch(
            "CREATE TABLE sessions (
                id INTEGER PRIMARY KEY,
                source TEXT NOT NULL,
                source_session_id TEXT NOT NULL,
                file_path TEXT,
                cwd TEXT,
                parent_session_id INTEGER,
                started_at TEXT,
                ended_at TEXT,
                current_generation INTEGER NOT NULL DEFAULT 0,
                file_mtime INTEGER,
                file_size INTEGER,
                content_fingerprint TEXT,
                next_read_offset INTEGER NOT NULL DEFAULT 0,
                event_count INTEGER NOT NULL DEFAULT 0,
                last_read_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                source_metadata TEXT
            );
            CREATE TABLE events (
                session_id INTEGER NOT NULL,
                generation INTEGER NOT NULL,
                seq INTEGER NOT NULL,
                ts TEXT,
                payload BLOB NOT NULL,
                codec TEXT NOT NULL,
                payload_size INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (session_id, generation, seq)
            ) WITHOUT ROWID;
            CREATE TABLE ingest_errors (
                id INTEGER PRIMARY KEY,
                source TEXT NOT NULL,
                source_session_id TEXT,
                session_id INTEGER,
                file_path TEXT NOT NULL,
                generation INTEGER,
                byte_offset INTEGER,
                line_number INTEGER,
                error_kind TEXT NOT NULL,
                message TEXT NOT NULL,
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                occurrence_count INTEGER NOT NULL DEFAULT 1,
                resolved_at TEXT,
                resolution_note TEXT
            );",
        )
        .expect("create stripped tables");
        // Deliberately omit `CREATE UNIQUE INDEX idx_sessions_source_session_id`.
        conn.pragma_update(None, "user_version", 8_i64)
            .expect("set user_version");
    }
    let tar = Command::new("tar")
        .args(["-czf"])
        .arg(&bad_archive)
        .args(["-C"])
        .arg(&fixture)
        .arg(".")
        .status()
        .expect("repack stripped fixture");
    assert!(tar.success());

    let output = run_settle(&dst_data, &bad_archive, true);
    assert!(
        !output.status.success(),
        "settle must refuse an archive that omits the unique sessions index"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("idx_sessions_source_session_id"),
        "stderr should name the missing index, got: {stderr}"
    );

    let after = fs::read(db_path(&dst_data)).expect("read db after failed settle");
    assert_eq!(
        before, after,
        "a failed settle must not modify the existing journal"
    );
    assert!(!dst_data.join(".pending-settle").exists());

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn pack_removes_claimed_archive_when_lock_is_held() {
    let root = temp_root("pack-cleanup-on-failure");
    let data_dir = root.join(".jottrace");
    let archive = root.join("should-be-removed.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &data_dir);

    // Hold the data lock externally so `pack` fails between claiming the
    // output path and writing the tarball. Without the ArchiveClaim guard a
    // zero-byte archive would survive and block the user's next retry.
    let lock_file = jottrace::create_private_file(&data_dir.join(jottrace::LOCK_FILE_NAME))
        .expect("create held lock");
    let lock_result = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(lock_result, 0);

    let output = Command::new(binary())
        .args(["pack", "--output"])
        .arg(&archive)
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace pack");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("another jottrace DB-mutating command is already running"));
    assert!(
        !archive.exists(),
        "claimed archive must be removed when pack fails after claiming the path"
    );

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn settle_force_removes_stale_files_before_extracting() {
    let root = temp_root("settle-force-clean");
    let src_data = root.join("src/.jottrace");
    let dst_data = root.join("dst/.jottrace");
    let archive = root.join("journal.tar.gz");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &src_data);
    run_pack(&src_data, &archive);

    // Seed the destination, then drop a stale SQLite sidecar that is NOT in
    // the archive. The bug `--force` is meant to fix is that the sidecar
    // would otherwise survive and be paired with the restored `db.sqlite`.
    let first = run_settle(&dst_data, &archive, false);
    assert!(first.status.success());
    fs::write(dst_data.join("db.sqlite-wal"), b"stale wal frames").expect("seed stale sidecar");
    assert!(dst_data.join("db.sqlite-wal").exists());

    let forced = run_settle(&dst_data, &archive, true);
    assert!(
        forced.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&forced.stdout),
        String::from_utf8_lossy(&forced.stderr)
    );
    assert!(
        !dst_data.join("db.sqlite-wal").exists(),
        "--force should wipe stale files that the archive does not contain"
    );
    assert!(
        dst_data.join("db.sqlite").exists(),
        "the restored db.sqlite should still be present"
    );

    let _ = fs::remove_dir_all(root);
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
