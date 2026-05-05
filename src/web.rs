use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, params};
use std::fmt::Write as _;
use std::io::{self, Read, Write as IoWrite};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::storage::{
    IngestErrorSummary, LATEST_SCHEMA_VERSION, RAW_CODEC, sqlite_error,
    unresolved_ingest_errors_from_connection,
};
use crate::{JottraceError, Result};

const MAX_SESSIONS: usize = 200;
const MAX_EVENTS: usize = 200;
const MAX_INGEST_ERRORS: usize = 50;
const MIN_PAYLOAD_SEARCH_CHARS: usize = 3;
const PAYLOAD_PREVIEW_BYTES: usize = 4096;
const PAYLOAD_PREVIEW_CHARS: usize = 280;
const READ_LIMIT_BYTES: usize = 8192;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalQuery {
    pub selected_source: Option<String>,
    pub selected_source_session_id: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalView {
    pub db_path: PathBuf,
    pub sessions: Vec<JournalSession>,
    pub selected_session: Option<JournalSession>,
    pub events: Vec<JournalEvent>,
    pub unresolved_ingest_errors: Vec<IngestErrorSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalSession {
    pub source: String,
    pub source_session_id: String,
    pub file_path: Option<String>,
    pub cwd: Option<String>,
    pub parent_source_session_id: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub event_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEvent {
    pub generation: i64,
    pub seq: i64,
    pub ts: Option<String>,
    pub codec: String,
    pub payload_size: u64,
    pub payload_preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoadedSession {
    id: i64,
    session: JournalSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchPattern {
    like: Option<String>,
    include_payload: bool,
}

pub struct WebServer {
    db_path: PathBuf,
    listener: TcpListener,
}

impl WebServer {
    pub fn bind(db_path: PathBuf, port: u16) -> Result<Self> {
        drop(open_web_database(&db_path)?);
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|source| JottraceError::Io {
                path: db_path.clone(),
                source,
            })?;
        Ok(Self { db_path, listener })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn local_url(&self) -> String {
        match self.local_addr() {
            Ok(addr) => format!("http://{addr}"),
            Err(_) => "http://127.0.0.1".to_string(),
        }
    }

    pub fn serve_once(&self) -> Result<()> {
        let (mut stream, _) = self.listener.accept().map_err(|source| JottraceError::Io {
            path: self.db_path.clone(),
            source,
        })?;
        self.handle_stream(&mut stream)
    }

    pub fn serve_forever(&self) -> Result<()> {
        loop {
            self.serve_once()?;
        }
    }

    fn handle_stream(&self, stream: &mut TcpStream) -> Result<()> {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|source| self.io_error(source))?;
        let request = self.read_request(stream)?;
        let Some((method, target)) = request_line(&request) else {
            return self.write_response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                "bad request",
            );
        };

        if method != "GET" {
            return self.write_response(
                stream,
                "405 Method Not Allowed",
                "text/plain; charset=utf-8",
                "method not allowed",
            );
        }

        if target_path(target) != "/" {
            return self.write_response(
                stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                "not found",
            );
        }

        let query = journal_query_from_target(target);
        let body = match journal_view_for_path(&self.db_path, &query) {
            Ok(view) => render_home_page(&view, &query),
            Err(error) => render_error_page(&self.db_path, &error),
        };
        self.write_response(stream, "200 OK", "text/html; charset=utf-8", &body)
    }

    fn write_response(
        &self,
        stream: &mut TcpStream,
        status: &str,
        content_type: &str,
        body: &str,
    ) -> Result<()> {
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(header.as_bytes())
            .and_then(|_| stream.write_all(body.as_bytes()))
            .map_err(|source| self.io_error(source))
    }

    fn io_error(&self, source: io::Error) -> JottraceError {
        JottraceError::Io {
            path: self.db_path.clone(),
            source,
        }
    }

    fn read_request(&self, stream: &mut TcpStream) -> Result<String> {
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while request.len() < READ_LIMIT_BYTES {
            let len = stream
                .read(&mut buffer)
                .map_err(|source| self.io_error(source))?;
            if len == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..len]);
            if request.windows(4).any(|window| window == b"\r\n\r\n")
                || request.windows(2).any(|window| window == b"\n\n")
            {
                break;
            }
        }
        Ok(String::from_utf8_lossy(&request).into_owned())
    }
}

pub fn journal_view_for_path(path: &Path, query: &JournalQuery) -> Result<JournalView> {
    let conn = open_web_database(path)?;
    let search = search_pattern(query.search.as_deref());
    let loaded_sessions = load_sessions(path, &conn, &search)?;
    let selected_session = select_session(
        path,
        &conn,
        &loaded_sessions,
        query.selected_source.as_deref(),
        query.selected_source_session_id.as_deref(),
    )?;
    let events = match selected_session.as_ref() {
        Some(session) => load_events(path, &conn, session.id)?,
        None => Vec::new(),
    };
    let unresolved_ingest_errors =
        unresolved_ingest_errors_from_connection(path, &conn, MAX_INGEST_ERRORS)?;
    let sessions = loaded_sessions
        .into_iter()
        .map(|loaded| loaded.session)
        .collect();

    Ok(JournalView {
        db_path: path.to_path_buf(),
        sessions,
        selected_session: selected_session.map(|loaded| loaded.session),
        events,
        unresolved_ingest_errors,
    })
}

pub fn render_home_page(view: &JournalView, query: &JournalQuery) -> String {
    let mut html = String::new();
    let selected_source_session_id = view
        .selected_session
        .as_ref()
        .map(|session| session.source_session_id.as_str());
    let selected_source = view
        .selected_session
        .as_ref()
        .map(|session| session.source.as_str());
    let search = query.search.as_deref().unwrap_or("");

    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    html.push_str("<title>Jottrace Journal</title>");
    html.push_str("<style>");
    html.push_str(
        ":root{color-scheme:light dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif;line-height:1.45}\
         body{margin:0;background:#f7f7f3;color:#202124}\
         header{background:#113f3a;color:#fff;padding:20px 28px}\
         main{display:grid;grid-template-columns:minmax(280px,360px) 1fr;gap:0;min-height:calc(100vh - 96px)}\
         aside{border-right:1px solid #d8d5ca;background:#fffdf7;padding:18px;overflow:auto}\
         section{padding:22px 28px;overflow:auto}\
         h1,h2,h3{margin:0 0 12px}h1{font-size:1.35rem}h2{font-size:1.05rem}h3{font-size:.95rem}\
         .muted{color:#62645f}.db{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.82rem;word-break:break-all}\
         form{display:flex;gap:8px;margin:0 0 16px}input{flex:1;padding:8px 10px;border:1px solid #b9b7ad;border-radius:6px;background:#fff;color:#202124}\
         button{padding:8px 12px;border:0;border-radius:6px;background:#176b5f;color:#fff;font-weight:650}\
         a{color:#175e87;text-decoration:none}.session{display:block;border-top:1px solid #e6e2d8;padding:12px 0}.selected{font-weight:700}\
         dl{display:grid;grid-template-columns:max-content 1fr;gap:6px 12px;margin:0 0 18px}dt{font-weight:700}dd{margin:0;word-break:break-word}\
         table{width:100%;border-collapse:collapse;background:#fff}th,td{border-bottom:1px solid #e5e1d7;padding:8px;text-align:left;vertical-align:top}\
         th{font-size:.78rem;text-transform:uppercase;letter-spacing:.04em;background:#efeee7}pre{white-space:pre-wrap;word-break:break-word;margin:8px 0 0;font-size:.84rem}\
         details summary{cursor:pointer;color:#176b5f;font-weight:650}.error{color:#8a280d;font-weight:700}\
         @media(max-width:760px){main{display:block}aside{border-right:0;border-bottom:1px solid #d8d5ca}}",
    );
    html.push_str("</style></head><body>");
    html.push_str("<header><h1>Jottrace Journal</h1><div class=\"db\">");
    html_escape_into(&mut html, &view.db_path.display().to_string());
    html.push_str("</div></header><main><aside>");
    html.push_str("<form method=\"get\" action=\"/\"><input name=\"q\" type=\"search\" value=\"");
    html_escape_into(&mut html, search);
    html.push_str("\" placeholder=\"Search source, cwd, path, session id, payload\"><button type=\"submit\">Search</button></form>");
    html.push_str("<h2>Sessions</h2>");
    if view.sessions.is_empty() {
        html.push_str("<p class=\"muted\">No sessions found.</p>");
    }
    for session in &view.sessions {
        let class = if Some(session.source.as_str()) == selected_source
            && Some(session.source_session_id.as_str()) == selected_source_session_id
        {
            "session selected"
        } else {
            "session"
        };
        write!(html, "<a class=\"{}\" href=\"", class).expect("write html");
        html_escape_into(&mut html, &session_href(session, search));
        html.push_str("\">");
        html.push_str("<strong>");
        html_escape_into(&mut html, &session.source_session_id);
        html.push_str("</strong><br><span class=\"muted\">");
        html_escape_into(&mut html, &session.source);
        html.push_str(" · event count ");
        write!(html, "{}", session.event_count).expect("write html");
        html.push_str("</span>");
        if let Some(cwd) = &session.cwd {
            html.push_str("<br><span class=\"muted\">");
            html_escape_into(&mut html, cwd);
            html.push_str("</span>");
        }
        html.push_str("</a>");
    }
    html.push_str("</aside><section>");

    match &view.selected_session {
        Some(session) => {
            html.push_str("<h2>Selected session</h2><dl>");
            definition(&mut html, "source", &session.source);
            definition(&mut html, "session id", &session.source_session_id);
            optional_definition(&mut html, "cwd", session.cwd.as_deref());
            optional_definition(&mut html, "path", session.file_path.as_deref());
            optional_definition(
                &mut html,
                "parent",
                session.parent_source_session_id.as_deref(),
            );
            optional_definition(&mut html, "started", session.started_at.as_deref());
            optional_definition(&mut html, "ended", session.ended_at.as_deref());
            definition(&mut html, "event count", &session.event_count.to_string());
            html.push_str("</dl><h2>Events</h2>");
            render_events(&mut html, &view.events);
        }
        None => html.push_str("<h2>No session selected</h2>"),
    }

    html.push_str("<h2>Unresolved ingest errors</h2>");
    render_ingest_errors(&mut html, &view.unresolved_ingest_errors);
    html.push_str("</section></main></body></html>");
    html
}

fn open_web_database(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| sqlite_error(path, source))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|source| sqlite_error(path, source))?;
    let schema_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| sqlite_error(path, source))?;
    if schema_version > LATEST_SCHEMA_VERSION {
        return Err(JottraceError::UnsupportedSchemaVersion {
            path: path.to_path_buf(),
            actual: schema_version,
            supported: LATEST_SCHEMA_VERSION,
        });
    }
    Ok(conn)
}

fn load_sessions(
    path: &Path,
    conn: &Connection,
    search: &SearchPattern,
) -> Result<Vec<LoadedSession>> {
    let mut statement = conn
        .prepare(
            "SELECT sessions.id, sessions.source, sessions.source_session_id,
                    sessions.file_path, sessions.cwd, parent.source_session_id,
                    sessions.started_at, sessions.ended_at, sessions.event_count
             FROM sessions
             LEFT JOIN sessions AS parent ON parent.id = sessions.parent_session_id
             WHERE ?1 IS NULL
                OR sessions.source LIKE ?1 ESCAPE '\\'
                OR sessions.source_session_id LIKE ?1 ESCAPE '\\'
                OR COALESCE(sessions.cwd, '') LIKE ?1 ESCAPE '\\'
                OR COALESCE(sessions.file_path, '') LIKE ?1 ESCAPE '\\'
                OR (?2 = 1 AND EXISTS (
                    SELECT 1
                    FROM events
                    WHERE events.session_id = sessions.id
                      AND events.codec = ?3
                      AND CAST(substr(events.payload, 1, ?4) AS TEXT) LIKE ?1 ESCAPE '\\'
                ))
             ORDER BY COALESCE(sessions.started_at, sessions.updated_at) DESC,
                      sessions.id DESC
             LIMIT ?5",
        )
        .map_err(|source| sqlite_error(path, source))?;

    statement
        .query_map(
            params![
                search.like.as_deref(),
                search.include_payload as i64,
                RAW_CODEC,
                PAYLOAD_PREVIEW_BYTES as i64,
                MAX_SESSIONS as i64,
            ],
            loaded_session_from_row,
        )
        .map_err(|source| sqlite_error(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| sqlite_error(path, source))
}

fn select_session(
    path: &Path,
    conn: &Connection,
    sessions: &[LoadedSession],
    selected_source: Option<&str>,
    selected_source_session_id: Option<&str>,
) -> Result<Option<LoadedSession>> {
    let Some(source_session_id) = selected_source_session_id else {
        return Ok(sessions.first().cloned());
    };

    if let Some(session) = sessions
        .iter()
        .find(|session| session_matches(&session.session, selected_source, source_session_id))
    {
        return Ok(Some(session.clone()));
    }

    load_session_by_source_session_id(path, conn, selected_source, source_session_id)
}

fn load_session_by_source_session_id(
    path: &Path,
    conn: &Connection,
    source: Option<&str>,
    source_session_id: &str,
) -> Result<Option<LoadedSession>> {
    conn.query_row(
        "SELECT sessions.id, sessions.source, sessions.source_session_id,
                sessions.file_path, sessions.cwd, parent.source_session_id,
                sessions.started_at, sessions.ended_at, sessions.event_count
         FROM sessions
         LEFT JOIN sessions AS parent ON parent.id = sessions.parent_session_id
         WHERE (?1 IS NULL OR sessions.source = ?1)
           AND sessions.source_session_id = ?2
         ORDER BY sessions.id DESC
         LIMIT 1",
        params![source, source_session_id],
        loaded_session_from_row,
    )
    .optional()
    .map_err(|source| sqlite_error(path, source))
}

fn session_matches(
    session: &JournalSession,
    selected_source: Option<&str>,
    source_session_id: &str,
) -> bool {
    session.source_session_id == source_session_id
        && selected_source.is_none_or(|source| session.source == source)
}

fn load_events(path: &Path, conn: &Connection, session_id: i64) -> Result<Vec<JournalEvent>> {
    let mut statement = conn
        .prepare(
            "SELECT generation, seq, ts, codec, payload_size,
                    CASE
                        WHEN codec = ?3 THEN substr(payload, 1, ?4)
                        ELSE x''
                    END AS payload_preview
             FROM events
             WHERE session_id = ?1
             ORDER BY generation, seq
             LIMIT ?2",
        )
        .map_err(|source| sqlite_error(path, source))?;

    statement
        .query_map(
            params![
                session_id,
                MAX_EVENTS as i64,
                RAW_CODEC,
                PAYLOAD_PREVIEW_BYTES as i64,
            ],
            journal_event_from_row,
        )
        .map_err(|source| sqlite_error(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| sqlite_error(path, source))
}

fn loaded_session_from_row(row: &Row<'_>) -> rusqlite::Result<LoadedSession> {
    let event_count: i64 = row.get("event_count")?;
    Ok(LoadedSession {
        id: row.get("id")?,
        session: JournalSession {
            source: row.get("source")?,
            source_session_id: row.get("source_session_id")?,
            file_path: row.get("file_path")?,
            cwd: row.get("cwd")?,
            parent_source_session_id: row.get(5)?,
            started_at: row.get("started_at")?,
            ended_at: row.get("ended_at")?,
            event_count: event_count as u64,
        },
    })
}

fn journal_event_from_row(row: &Row<'_>) -> rusqlite::Result<JournalEvent> {
    let payload_size: i64 = row.get("payload_size")?;
    let preview_bytes: Vec<u8> = row.get("payload_preview")?;
    let codec: String = row.get("codec")?;
    Ok(JournalEvent {
        generation: row.get("generation")?,
        seq: row.get("seq")?,
        ts: row.get("ts")?,
        payload_preview: payload_preview(&codec, payload_size as u64, &preview_bytes),
        codec,
        payload_size: payload_size as u64,
    })
}

fn search_pattern(search: Option<&str>) -> SearchPattern {
    let Some(search) = search.map(str::trim).filter(|search| !search.is_empty()) else {
        return SearchPattern {
            like: None,
            include_payload: false,
        };
    };
    SearchPattern {
        like: Some(like_contains_pattern(search)),
        include_payload: search.chars().count() >= MIN_PAYLOAD_SEARCH_CHARS,
    }
}

fn like_contains_pattern(search: &str) -> String {
    let mut pattern = String::from("%");
    for ch in search.chars() {
        match ch {
            '%' | '_' | '\\' => {
                pattern.push('\\');
                pattern.push(ch);
            }
            _ => pattern.push(ch),
        }
    }
    pattern.push('%');
    pattern
}

fn payload_preview(codec: &str, payload_size: u64, payload: &[u8]) -> String {
    if codec != RAW_CODEC {
        return format!("[{codec} payload: {payload_size} bytes]");
    }

    String::from_utf8_lossy(payload)
        .chars()
        .take(PAYLOAD_PREVIEW_CHARS)
        .collect()
}

fn render_events(html: &mut String, events: &[JournalEvent]) {
    if events.is_empty() {
        html.push_str("<p class=\"muted\">No events found for this session.</p>");
        return;
    }

    html.push_str("<table><thead><tr><th>generation</th><th>seq</th><th>time</th><th>payload</th></tr></thead><tbody>");
    for event in events {
        html.push_str("<tr><td>");
        write!(html, "{}", event.generation).expect("write html");
        html.push_str("</td><td>");
        write!(html, "{}", event.seq).expect("write html");
        html.push_str("</td><td>");
        html_escape_into(html, event.ts.as_deref().unwrap_or(""));
        html.push_str("</td><td><details><summary>");
        html_escape_into(
            html,
            &format!("{} bytes {}", event.payload_size, event.codec),
        );
        html.push_str("</summary><pre>");
        html_escape_into(html, &event.payload_preview);
        html.push_str("</pre></details></td></tr>");
    }
    html.push_str("</tbody></table>");
}

fn render_ingest_errors(html: &mut String, ingest_errors: &[IngestErrorSummary]) {
    if ingest_errors.is_empty() {
        html.push_str("<p class=\"muted\">No unresolved ingest errors.</p>");
        return;
    }

    html.push_str("<table><thead><tr><th>source</th><th>session</th><th>location</th><th>error</th></tr></thead><tbody>");
    for ingest_error in ingest_errors {
        html.push_str("<tr><td>");
        html_escape_into(html, &ingest_error.source);
        html.push_str("</td><td>");
        html_escape_into(
            html,
            ingest_error.source_session_id.as_deref().unwrap_or(""),
        );
        html.push_str("</td><td>");
        html_escape_into(html, &ingest_error.file_path.display().to_string());
        if let Some(line_number) = ingest_error.line_number {
            write!(html, "<br>line {line_number}").expect("write html");
        }
        html.push_str("</td><td><span class=\"error\">");
        html_escape_into(html, &ingest_error.error_kind);
        html.push_str("</span><br>");
        html_escape_into(html, &ingest_error.message);
        html.push_str("</td></tr>");
    }
    html.push_str("</tbody></table>");
}

fn definition(html: &mut String, label: &str, value: &str) {
    html.push_str("<dt>");
    html_escape_into(html, label);
    html.push_str("</dt><dd>");
    html_escape_into(html, value);
    html.push_str("</dd>");
}

fn optional_definition(html: &mut String, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        definition(html, label, value);
    }
}

fn session_href(session: &JournalSession, search: &str) -> String {
    let mut href = format!(
        "?source={}&session={}",
        url_encode(&session.source),
        url_encode(&session.source_session_id)
    );
    if !search.is_empty() {
        href.push_str("&q=");
        href.push_str(&url_encode(search));
    }
    href
}

fn html_escape_into(output: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(ch),
        }
    }
}

fn url_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => write!(encoded, "%{byte:02X}").expect("write url encoding"),
        }
    }
    encoded
}

fn request_line(request: &str) -> Option<(&str, &str)> {
    let line = request.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    Some((method, target))
}

fn target_path(target: &str) -> &str {
    target.split_once('?').map_or(target, |(path, _query)| path)
}

fn journal_query_from_target(target: &str) -> JournalQuery {
    let Some((_path, raw_query)) = target.split_once('?') else {
        return JournalQuery::default();
    };

    let mut query = JournalQuery::default();
    for pair in raw_query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "source" => query.selected_source = Some(percent_decode(value)),
            "session" => query.selected_source_session_id = Some(percent_decode(value)),
            "q" => query.search = Some(percent_decode(value)),
            _ => {}
        }
    }
    query
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                if let (Some(high), Some(low)) =
                    (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
                {
                    decoded.push(high * 16 + low);
                    index += 3;
                    continue;
                }
                decoded.push(bytes[index]);
            }
            b'+' => decoded.push(b' '),
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn render_error_page(db_path: &Path, error: &JottraceError) -> String {
    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    html.push_str("<title>Jottrace Journal Error</title></head><body><h1>Jottrace Journal</h1>");
    html.push_str("<p>Unable to read the local journal database.</p><dl><dt>db</dt><dd>");
    html_escape_into(&mut html, &db_path.display().to_string());
    html.push_str("</dd><dt>error</dt><dd>");
    html_escape_into(&mut html, &error.to_string());
    html.push_str("</dd></dl></body></html>");
    html
}
