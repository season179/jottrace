mod common;

use common::reader_fixture;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

const CLAUDE_FIXTURE_SESSION: &str = "claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021.jsonl";
const CLAUDE_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000021";
const CORRUPT_FIXTURE_SESSION: &str = "edge-cases/corrupt-line.jsonl";
const CORRUPT_FIXTURE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000000";

#[test]
fn web_journal_view_reads_sessions_events_and_ingest_errors() {
    let root = temp_root("web-view");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    install_claude_fixture(&root, CORRUPT_FIXTURE_SESSION_ID, CORRUPT_FIXTURE_SESSION);
    run_ingest(&root, &data_dir);

    let query = jottrace::web::JournalQuery {
        selected_source: Some("claude_cli".to_string()),
        selected_source_session_id: Some(CLAUDE_FIXTURE_SESSION_ID.to_string()),
        search: None,
        ..Default::default()
    };
    let view = jottrace::web::journal_view_for_path(&db_path(&data_dir), &query)
        .expect("load web journal view");

    assert_eq!(view.sessions.len(), 2);
    let session = view
        .sessions
        .iter()
        .find(|session| session.source_session_id == CLAUDE_FIXTURE_SESSION_ID)
        .expect("primary session should be listed");
    assert_eq!(session.source, "claude_cli");
    assert_eq!(
        session.cwd.as_deref(),
        Some("/Users/fixture/Workspace/jottrace")
    );
    assert!(
        session
            .file_path
            .as_deref()
            .is_some_and(|path| path.ends_with(&format!("/{CLAUDE_FIXTURE_SESSION_ID}.jsonl")))
    );
    assert_eq!(session.event_count, 12);
    assert_eq!(
        session.started_at.as_deref(),
        Some("2026-05-05T01:00:00.000Z")
    );
    assert_eq!(
        session.ended_at.as_deref(),
        Some("2026-05-05T01:00:08.000Z")
    );

    assert_eq!(
        view.selected_session
            .as_ref()
            .map(|session| session.source_session_id.as_str()),
        Some(CLAUDE_FIXTURE_SESSION_ID)
    );
    assert_eq!(view.events.len(), 12);
    assert_eq!(view.events[0].seq, 0);
    assert!(view.events[0].payload_preview.is_empty());

    let query = jottrace::web::JournalQuery {
        selected_source: Some("claude_cli".to_string()),
        selected_source_session_id: Some(CLAUDE_FIXTURE_SESSION_ID.to_string()),
        expanded_event: Some(jottrace::web::EventKey {
            generation: 0,
            seq: 0,
        }),
        ..Default::default()
    };
    let view = jottrace::web::journal_view_for_path(&db_path(&data_dir), &query)
        .expect("load expanded web journal view");
    assert!(
        view.expanded_event
            .as_ref()
            .is_some_and(|event| event.payload_preview.contains("permissionMode"))
    );

    assert_eq!(view.unresolved_ingest_errors.len(), 1);
    let ingest_error = &view.unresolved_ingest_errors[0];
    assert_eq!(
        ingest_error.source_session_id.as_deref(),
        Some(CORRUPT_FIXTURE_SESSION_ID)
    );
    assert_eq!(ingest_error.error_kind, "invalid_json");
    assert_eq!(ingest_error.line_number, Some(2));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn web_home_page_renders_journal_data_and_search_controls() {
    let root = temp_root("web-render");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    install_claude_fixture(&root, CORRUPT_FIXTURE_SESSION_ID, CORRUPT_FIXTURE_SESSION);
    run_ingest(&root, &data_dir);

    let query = jottrace::web::JournalQuery {
        selected_source: Some("claude_cli".to_string()),
        selected_source_session_id: Some(CLAUDE_FIXTURE_SESSION_ID.to_string()),
        search: Some("sanitized reader fixture".to_string()),
        include_payload_search: true,
        expanded_event: Some(jottrace::web::EventKey {
            generation: 0,
            seq: 2,
        }),
        ..Default::default()
    };
    let view = jottrace::web::journal_view_for_path(&db_path(&data_dir), &query)
        .expect("load web journal view");
    let html = jottrace::web::render_home_page(&view, &query);

    assert!(html.contains("<form"));
    assert!(html.contains("name=\"q\""));
    assert!(html.contains("sanitized reader fixture"));
    assert!(html.contains(CLAUDE_FIXTURE_SESSION_ID));
    assert!(html.contains("/Users/fixture/Workspace/jottrace"));
    assert!(html.contains("event count"));
    assert!(html.contains("Please implement the sanitized reader fixture corpus."));
    assert!(html.contains("Unresolved ingest errors"));
    assert!(html.contains("invalid_json"));
    assert!(!html.contains("<script"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn web_journal_view_filters_sessions_by_metadata_and_payload_text() {
    let root = temp_root("web-search");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    install_claude_fixture(&root, CORRUPT_FIXTURE_SESSION_ID, CORRUPT_FIXTURE_SESSION);
    run_ingest(&root, &data_dir);
    let db_path = db_path(&data_dir);

    assert_eq!(matching_session_ids(&db_path, "claude_cli").len(), 2);
    assert_eq!(
        matching_session_ids(&db_path, "Workspace/jottrace").len(),
        1
    );
    assert_eq!(
        matching_session_ids(&db_path, CLAUDE_FIXTURE_SESSION_ID),
        vec![CLAUDE_FIXTURE_SESSION_ID.to_string()]
    );
    assert_eq!(
        matching_session_ids(&db_path, "projects/-Users-fixture-Workspace-jottrace").len(),
        2
    );
    assert_eq!(
        payload_matching_session_ids(&db_path, "sanitized reader fixture corpus"),
        vec![CLAUDE_FIXTURE_SESSION_ID.to_string()]
    );
    assert_eq!(
        matching_session_ids(&db_path, "sanitized reader fixture corpus"),
        Vec::<String>::new()
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn web_journal_view_treats_search_wildcards_as_literal_text() {
    let root = temp_root("web-literal-search");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &data_dir);
    let db_path = db_path(&data_dir);

    assert_eq!(matching_session_ids(&db_path, "%"), Vec::<String>::new());
    assert_eq!(
        matching_session_ids(&db_path, "claude%cli"),
        Vec::<String>::new()
    );
    assert_eq!(
        matching_session_ids(&db_path, "claude_cli"),
        vec![CLAUDE_FIXTURE_SESSION_ID.to_string()]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn web_journal_view_keeps_initial_large_journal_page_bounded_and_payload_light() {
    let root = temp_root("web-large-initial");
    let data_dir = root.join(".jottrace");
    let db_path = db_path(&data_dir);
    jottrace::storage::status_for_path(&db_path).expect("initialize database");
    populate_large_journal(&db_path);

    let query = jottrace::web::JournalQuery::default();
    let view =
        jottrace::web::journal_view_for_path(&db_path, &query).expect("load large web journal");
    let html = jottrace::web::render_home_page(&view, &query);

    assert_eq!(view.sessions.len(), 50);
    assert_eq!(
        view.selected_session
            .as_ref()
            .map(|session| session.source_session_id.as_str()),
        Some("session-079")
    );
    assert_eq!(view.events.len(), 25);
    assert!(
        view.events
            .iter()
            .all(|event| event.payload_preview.is_empty())
    );
    assert!(!html.contains("SENSITIVE_PAYLOAD_MARKER"));
    assert!(html.contains("Next sessions"));
    assert!(html.contains("href=\"?session_offset=50\""));
    assert!(html.contains("Next events"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn web_journal_view_pages_large_journal_and_expands_one_event_payload() {
    let root = temp_root("web-large-pages");
    let data_dir = root.join(".jottrace");
    let db_path = db_path(&data_dir);
    jottrace::storage::status_for_path(&db_path).expect("initialize database");
    populate_large_journal(&db_path);

    let query = jottrace::web::JournalQuery {
        session_offset: 50,
        ..Default::default()
    };
    let view =
        jottrace::web::journal_view_for_path(&db_path, &query).expect("load second session page");
    assert_eq!(view.sessions.len(), 30);
    assert!(view.session_page.has_previous);
    assert!(!view.session_page.has_next);
    assert_eq!(view.sessions[0].source_session_id, "session-029");

    let query = jottrace::web::JournalQuery {
        selected_source: Some("claude_cli".to_string()),
        selected_source_session_id: Some("session-079".to_string()),
        event_offset: 25,
        expanded_event: Some(jottrace::web::EventKey {
            generation: 0,
            seq: 26,
        }),
        ..Default::default()
    };
    let view =
        jottrace::web::journal_view_for_path(&db_path, &query).expect("load second event page");
    assert_eq!(
        view.events
            .first()
            .map(|event| (event.generation, event.seq)),
        Some((0, 25))
    );
    assert_eq!(view.events.len(), 25);
    assert!(
        view.events
            .iter()
            .all(|event| event.payload_preview.is_empty())
    );
    assert!(
        view.expanded_event
            .as_ref()
            .is_some_and(|event| event.payload_preview.contains("event 026"))
    );
    assert!(view.event_page.expect("event page").has_previous);
    let html = jottrace::web::render_home_page(&view, &query);
    assert!(html.contains("Previous events"));
    assert!(html.contains("Next events"));
    assert!(html.contains("SENSITIVE_PAYLOAD_MARKER event 026"));
    assert!(!html.contains("SENSITIVE_PAYLOAD_MARKER event 027"));

    let query = jottrace::web::JournalQuery {
        selected_source_session_id: Some("session-079".to_string()),
        ..Default::default()
    };
    let view = jottrace::web::journal_view_for_path(&db_path, &query)
        .expect("load source-less selected large journal");
    let html = jottrace::web::render_home_page(&view, &query);
    assert!(
        html.contains("href=\"?source=claude_cli&amp;session=session-079&amp;session_offset=50\"")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn web_journal_view_selects_session_by_source_and_source_session_id() {
    let root = temp_root("web-source-qualified-selection");
    let data_dir = root.join(".jottrace");
    let db_path = db_path(&data_dir);
    jottrace::storage::status_for_path(&db_path).expect("initialize database");
    let conn = Connection::open(&db_path).expect("open database");
    conn.execute(
        "INSERT INTO sessions (source, source_session_id, cwd, event_count, started_at)
         VALUES ('claude_cli', 'shared-session-id', '/tmp/claude', 1, '2026-05-05T01:00:00Z')",
        [],
    )
    .expect("insert claude session");
    conn.execute(
        "INSERT INTO sessions (source, source_session_id, cwd, event_count, started_at)
         VALUES ('codex_cli', 'shared-session-id', '/tmp/codex', 1, '2026-05-05T01:00:01Z')",
        [],
    )
    .expect("insert codex session");
    conn.execute(
        "INSERT INTO events (session_id, generation, seq, payload, codec, payload_size)
         VALUES (1, 0, 0, x'7B22636C61756465223A747275657D', 'raw', 15)",
        [],
    )
    .expect("insert claude event");
    conn.execute(
        "INSERT INTO events (session_id, generation, seq, payload, codec, payload_size)
         VALUES (2, 0, 0, x'7B22636F646578223A747275657D', 'raw', 14)",
        [],
    )
    .expect("insert codex event");
    drop(conn);

    let query = jottrace::web::JournalQuery {
        selected_source: Some("claude_cli".to_string()),
        selected_source_session_id: Some("shared-session-id".to_string()),
        search: None,
        expanded_event: Some(jottrace::web::EventKey {
            generation: 0,
            seq: 0,
        }),
        ..Default::default()
    };
    let view =
        jottrace::web::journal_view_for_path(&db_path, &query).expect("load source-qualified view");

    let selected = view.selected_session.expect("selected session");
    assert_eq!(selected.source, "claude_cli");
    assert_eq!(selected.cwd.as_deref(), Some("/tmp/claude"));
    assert!(
        view.expanded_event
            .as_ref()
            .is_some_and(|event| event.payload_preview.contains("claude"))
    );
    let view = jottrace::web::journal_view_for_path(&db_path, &query)
        .expect("reload source-qualified view");
    let html = jottrace::web::render_home_page(&view, &query);
    assert!(html.contains(
        "<a class=\"session selected\" href=\"?source=claude_cli&amp;session=shared-session-id\""
    ));
    assert!(
        html.contains(
            "<a class=\"session\" href=\"?source=codex_cli&amp;session=shared-session-id\""
        )
    );
    assert!(html.contains("?source=codex_cli&amp;session=shared-session-id"));
    assert!(html.contains("?source=claude_cli&amp;session=shared-session-id"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn web_server_serves_local_journal_html() {
    let root = temp_root("web-server");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &data_dir);

    let server = jottrace::web::WebServer::bind(db_path(&data_dir), 0).expect("bind web server");
    let url = server.local_url();
    assert!(url.starts_with("http://127.0.0.1:"));
    let address = server.local_addr().expect("local address");
    let handle = thread::spawn(move || server.serve_once().expect("serve one request"));

    let mut stream = TcpStream::connect(address).expect("connect to web server");
    write!(
        stream,
        "GET /?session={CLAUDE_FIXTURE_SESSION_ID}&q=sanitized%20reader&payload=1&event_generation=0&event_seq=2 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .expect("write http request");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish http request");
    let mut response_bytes = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(len) => response_bytes.extend_from_slice(&buffer[..len]),
            Err(error) if error.kind() == ErrorKind::ConnectionReset => break,
            Err(error) => panic!("read http response: {error}"),
        }
    }
    handle.join().expect("web server thread");
    let response = String::from_utf8(response_bytes).expect("response should be utf-8");

    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains("Content-Type: text/html; charset=utf-8"));
    assert!(
        response.contains(CLAUDE_FIXTURE_SESSION_ID),
        "response:\n{response}"
    );
    assert!(response.contains("Please implement the sanitized reader fixture corpus."));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn web_cli_prints_url_and_db_path_before_serving() {
    let root = temp_root("web-cli");
    let data_dir = root.join(".jottrace");
    install_primary_claude_fixture(&root);
    run_ingest(&root, &data_dir);

    let mut child = Command::new(binary())
        .args(["web", "--port", "0", "--once"])
        .env("JOTTRACE_HOME", &data_dir)
        .stdout(Stdio::piped())
        .spawn()
        .expect("run jottrace web");
    let stdout = child.stdout.take().expect("web stdout");
    let mut stdout = BufReader::new(stdout);
    let mut title_line = String::new();
    let mut url_line = String::new();
    let mut db_line = String::new();
    stdout.read_line(&mut title_line).expect("read title line");
    stdout.read_line(&mut url_line).expect("read url line");
    stdout.read_line(&mut db_line).expect("read db line");

    assert_eq!(title_line.trim_end(), "jottrace web");
    assert!(url_line.starts_with("url: http://127.0.0.1:"));
    assert_eq!(
        db_line.trim_end(),
        format!("db: {}", db_path(&data_dir).display())
    );

    let address = url_line
        .trim()
        .strip_prefix("url: http://")
        .expect("url prefix");
    let mut stream = TcpStream::connect(address).expect("connect to web cli");
    write!(
        stream,
        "GET /?session={CLAUDE_FIXTURE_SESSION_ID} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .expect("write http request");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish http request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read http response");
    assert!(response.contains(CLAUDE_FIXTURE_SESSION_ID));

    let status = child.wait().expect("wait for jottrace web");
    assert!(status.success());

    let _ = fs::remove_dir_all(root);
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_jottrace")
}

fn matching_session_ids(db_path: &Path, search: &str) -> Vec<String> {
    let query = jottrace::web::JournalQuery {
        selected_source: None,
        selected_source_session_id: None,
        search: Some(search.to_string()),
        ..Default::default()
    };
    jottrace::web::journal_view_for_path(db_path, &query)
        .expect("load web journal view")
        .sessions
        .into_iter()
        .map(|session| session.source_session_id)
        .collect()
}

fn payload_matching_session_ids(db_path: &Path, search: &str) -> Vec<String> {
    let query = jottrace::web::JournalQuery {
        selected_source: None,
        selected_source_session_id: None,
        search: Some(search.to_string()),
        include_payload_search: true,
        ..Default::default()
    };
    jottrace::web::journal_view_for_path(db_path, &query)
        .expect("load web journal view")
        .sessions
        .into_iter()
        .map(|session| session.source_session_id)
        .collect()
}

fn run_ingest(home: &Path, data_dir: &Path) {
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
}

fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join(jottrace::storage::DB_FILE_NAME)
}

fn install_primary_claude_fixture(root: &Path) -> PathBuf {
    install_claude_fixture(root, CLAUDE_FIXTURE_SESSION_ID, CLAUDE_FIXTURE_SESSION)
}

fn install_claude_fixture(root: &Path, session_id: &str, fixture_relative: &str) -> PathBuf {
    let session_file = root
        .join(".claude/projects/-Users-fixture-Workspace-jottrace")
        .join(format!("{session_id}.jsonl"));
    if let Some(parent) = session_file.parent() {
        fs::create_dir_all(parent).expect("create fixture destination parent");
    }
    fs::copy(reader_fixture(fixture_relative), &session_file).expect("copy fixture");
    session_file
}

fn populate_large_journal(db_path: &Path) {
    let mut conn = Connection::open(db_path).expect("open large journal database");
    let tx = conn.transaction().expect("start large journal transaction");
    {
        let mut insert_session = tx
            .prepare(
                "INSERT INTO sessions (
                    source, source_session_id, file_path, cwd, event_count, started_at, ended_at
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .expect("prepare large journal session insert");
        for index in 0..80 {
            insert_session
                .execute(params![
                    "claude_cli",
                    format!("session-{index:03}"),
                    format!("/Users/fixture/.claude/session-{index:03}.jsonl"),
                    format!("/Users/fixture/project-{index:03}"),
                    if index == 79 { 80 } else { 0 },
                    fixture_timestamp(1, index, 0),
                    fixture_timestamp(1, index, 30),
                ])
                .expect("insert large journal session");
        }
    }

    let latest_session_id = tx.last_insert_rowid();
    {
        let mut insert_event = tx
            .prepare(
                "INSERT INTO events (session_id, generation, seq, ts, payload, codec, payload_size)
                 VALUES (?1, 0, ?2, ?3, ?4, 'raw', ?5)",
            )
            .expect("prepare large journal event insert");
        for seq in 0..80 {
            let payload = format!(
                "{{\"message\":\"SENSITIVE_PAYLOAD_MARKER event {seq:03} with enough transcript text to be noticeable\"}}"
            );
            insert_event
                .execute(params![
                    latest_session_id,
                    seq as i64,
                    fixture_timestamp(2, seq, 0),
                    payload.as_bytes(),
                    payload.len() as i64,
                ])
                .expect("insert large journal event");
        }
    }
    tx.commit().expect("commit large journal transaction");
}

fn fixture_timestamp(start_hour: usize, index: usize, second: usize) -> String {
    let hour = start_hour + (index / 60);
    let minute = index % 60;
    format!("2026-05-05T{hour:02}:{minute:02}:{second:02}.000Z")
}

fn temp_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id()))
}
