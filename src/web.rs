use rusqlite::{Connection, Row, params};
use std::fmt::Write as _;
use std::io::{self, Read, Write as IoWrite};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::storage::{
    IngestErrorSummary, LATEST_SCHEMA_VERSION, RAW_CODEC, ZSTD_CODEC, decode_event_payload,
    decode_event_payload_prefix, open_readonly_database, query_collect, query_one, query_optional,
    row_value, sqlite_error, unresolved_ingest_errors_from_connection,
};
use crate::{JottraceError, Result, io_error};

const SESSION_PAGE_SIZE: usize = 50;
const EVENT_PAGE_SIZE: usize = 25;
const MAX_INGEST_ERRORS: usize = 50;
const MAX_QUERY_OFFSET: usize = 100_000;
const MIN_PAYLOAD_SEARCH_CHARS: usize = 3;
const PAYLOAD_PREVIEW_BYTES: usize = 4096;
const PAYLOAD_PREVIEW_CHARS: usize = 280;
const READ_LIMIT_BYTES: usize = 8192;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalQuery {
    pub selected_source: Option<String>,
    pub selected_source_session_id: Option<String>,
    pub search: Option<String>,
    pub include_payload_search: bool,
    pub session_offset: usize,
    pub event_offset: usize,
    pub expanded_event: Option<EventKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventKey {
    pub generation: i64,
    pub seq: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageInfo {
    pub offset: usize,
    pub limit: usize,
    pub has_previous: bool,
    pub has_next: bool,
}

fn paginate<T>(mut items: Vec<T>, offset: usize, limit: usize) -> (Vec<T>, PageInfo) {
    let has_next = items.len() > limit;
    if has_next {
        items.truncate(limit);
    }
    (
        items,
        PageInfo {
            offset,
            limit,
            has_previous: offset > 0,
            has_next,
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalView {
    pub db_path: PathBuf,
    pub sessions: Vec<JournalSession>,
    pub session_page: PageInfo,
    pub selected_session: Option<JournalSession>,
    pub events: Vec<JournalEvent>,
    pub event_page: Option<PageInfo>,
    pub expanded_event: Option<JournalEvent>,
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
    pub payload_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoadedSession {
    id: i64,
    session: JournalSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchPattern {
    like: Option<String>,
    payload_needle: Option<String>,
    decoded_payload_session_ids: Vec<i64>,
}

pub struct WebServer {
    db_path: PathBuf,
    listener: TcpListener,
}

impl WebServer {
    pub fn bind(db_path: PathBuf, port: u16) -> Result<Self> {
        drop(open_web_database(&db_path)?);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .map_err(|source| io_error(&db_path, source))?;
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
        let mut stream = self.accept_stream()?;
        self.handle_stream(&mut stream)
    }

    pub fn serve_forever(&self) -> Result<()> {
        loop {
            let mut stream = self.accept_stream()?;
            if self.handle_stream(&mut stream).is_err() {
                continue;
            }
        }
    }

    fn accept_stream(&self) -> Result<TcpStream> {
        self.listener
            .accept()
            .map(|(stream, _)| stream)
            .map_err(|source| io_error(&self.db_path, source))
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

        let (status, content_type, body) = match target_path(target) {
            "/" => {
                let query = journal_query_from_target(target);
                let body = match journal_view_for_path(&self.db_path, &query) {
                    Ok(view) => render_home_page(&view, &query),
                    Err(error) => render_error_page(&self.db_path, &error),
                };
                ("200 OK", "text/html; charset=utf-8", body)
            }
            "/payload" => match event_payload_text_for_path(&self.db_path, target) {
                Ok(Some(payload)) => ("200 OK", "text/plain; charset=utf-8", payload),
                Ok(None) => (
                    "404 Not Found",
                    "text/plain; charset=utf-8",
                    "payload not found".to_string(),
                ),
                Err(error) => (
                    "500 Internal Server Error",
                    "text/plain; charset=utf-8",
                    error.to_string(),
                ),
            },
            _ => (
                "404 Not Found",
                "text/plain; charset=utf-8",
                "not found".to_string(),
            ),
        };
        self.write_response(stream, status, content_type, &body)
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
        io_error(&self.db_path, source)
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
    let mut search = search_pattern(query.search.as_deref(), query.include_payload_search);
    if search.payload_needle.is_some() {
        search.decoded_payload_session_ids =
            decoded_payload_matching_session_ids(path, &conn, &search)?;
    }
    let session_offset = bounded_offset(query.session_offset);
    let event_offset = bounded_offset(query.event_offset);
    let (loaded_sessions, session_page) = load_sessions(path, &conn, &search, session_offset)?;
    let selected_session = select_session(
        path,
        &conn,
        &loaded_sessions,
        query.selected_source.as_deref(),
        query.selected_source_session_id.as_deref(),
    )?;
    let (events, event_page, expanded_event) = match selected_session.as_ref() {
        Some(session) => {
            let (events, page) = load_events(path, &conn, session.id, event_offset)?;
            let expanded_event = match query.expanded_event {
                Some(key) => load_event_payload(path, &conn, session.id, key)?,
                None => None,
            };
            (events, Some(page), expanded_event)
        }
        None => (Vec::new(), None, None),
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
        session_page,
        selected_session: selected_session.map(|loaded| loaded.session),
        events,
        event_page,
        expanded_event,
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
    let payload_checked = if query.include_payload_search {
        " checked"
    } else {
        ""
    };

    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    html.push_str("<title>Jottrace Journal</title>");
    html.push_str("<style>");
    html.push_str(
        ":root{color-scheme:light;--ink:#151715;--paper:#eef2ee;--line:#c9cec7;--muted:#5d665f;--green:#0e5c4f;--acid:#d8ff3d;--rust:#a93f22;font-family:\"Avenir Next\",\"Gill Sans\",Verdana,sans-serif;line-height:1.45}\
         *{box-sizing:border-box}body{margin:0;background:linear-gradient(90deg,rgba(21,23,21,.045) 1px,transparent 1px) 0 0/34px 34px,linear-gradient(0deg,rgba(21,23,21,.035) 1px,transparent 1px) 0 0/34px 34px,var(--paper);color:var(--ink)}\
         .topbar{display:grid;grid-template-columns:1fr minmax(260px,42vw);gap:24px;align-items:end;background:var(--ink);color:#f8faef;padding:24px 30px 22px;border-bottom:5px solid var(--acid)}\
         .kicker{margin:0 0 5px;font-size:.72rem;text-transform:uppercase;letter-spacing:.18em;color:var(--acid);font-weight:800}.topbar h1{margin:0;font-family:Georgia,\"Times New Roman\",serif;font-size:clamp(2rem,4vw,4.2rem);font-weight:900;line-height:.92}\
         .db{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.78rem;word-break:break-all;color:#dfe9df;text-align:right}.shell{display:grid;grid-template-columns:minmax(300px,390px) 1fr;min-height:calc(100vh - 118px)}\
         .rail{border-right:2px solid var(--line);background:rgba(251,251,247,.92);padding:18px;overflow:auto}.workspace{padding:24px 30px 40px;overflow:auto}\
         h2,h3{margin:0 0 12px;line-height:1.1}h2{font-size:1.02rem;text-transform:uppercase;letter-spacing:.12em}h3{font-size:1rem}.muted{color:var(--muted)}\
         .search{display:grid;grid-template-columns:1fr auto;gap:9px;margin:0 0 18px}.search input[type=search]{min-width:0;padding:10px 11px;border:2px solid var(--ink);border-radius:0;background:#fff;color:var(--ink);font:inherit}.search button{padding:10px 14px;border:2px solid var(--ink);border-radius:0;background:var(--acid);color:var(--ink);font-weight:900;text-transform:uppercase;letter-spacing:.06em}.payload-toggle{grid-column:1/3;display:flex;gap:8px;align-items:center;font-size:.86rem;color:var(--muted)}\
         a{color:var(--green);text-decoration:none}.session{display:block;border-top:1px solid var(--line);border-left:5px solid transparent;padding:12px 10px 12px 12px;color:var(--ink)}.session:hover{background:#fff;border-left-color:var(--rust)}.session.selected{background:var(--ink);border-left-color:var(--acid);color:#f8faef}.session strong{display:block;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.84rem;overflow-wrap:anywhere}.session .line{display:block;margin-top:3px;font-size:.82rem;color:inherit;opacity:.72}.pager{display:flex;flex-wrap:wrap;gap:8px;margin:14px 0 4px}.pager a,.inspect{display:inline-flex;align-items:center;min-height:32px;border:2px solid var(--ink);padding:5px 10px;background:#fff;color:var(--ink);font-weight:850;text-transform:uppercase;letter-spacing:.05em;font-size:.76rem}.pager a:hover,.inspect:hover{background:var(--acid)}\
         .summary{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:12px;margin:0 0 22px}.metric{border-top:5px solid var(--ink);background:rgba(255,255,255,.7);padding:10px 0}.metric b{display:block;font-size:1.35rem;line-height:1}.metric span{display:block;margin-top:2px;color:var(--muted);font-size:.78rem;text-transform:uppercase;letter-spacing:.08em}\
         dl{display:grid;grid-template-columns:max-content 1fr;gap:7px 14px;margin:0 0 24px;max-width:980px}dt{font-weight:900;text-transform:uppercase;letter-spacing:.08em;font-size:.72rem;color:var(--muted)}dd{margin:0;word-break:break-word}\
         .table-scroll{max-width:100%;overflow-x:auto}table{width:100%;min-width:640px;border-collapse:collapse;background:rgba(255,255,255,.78);border-top:3px solid var(--ink)}th,td{border-bottom:1px solid var(--line);padding:9px 8px;text-align:left;vertical-align:top}th{font-size:.72rem;text-transform:uppercase;letter-spacing:.12em;color:var(--muted);background:#e2e8e2}td{font-size:.9rem}.number{font-family:ui-monospace,SFMono-Regular,Menlo,monospace}.payload-panel{margin:14px 0 22px;border-left:6px solid var(--rust);background:#fff;padding:14px 16px;box-shadow:8px 8px 0 rgba(21,23,21,.08)}pre{white-space:pre-wrap;word-break:break-word;margin:8px 0 0;font-size:.84rem;font-family:ui-monospace,SFMono-Regular,Menlo,monospace}.error{color:#8a280d;font-weight:900}\
         @media(max-width:860px){.topbar{display:block;padding:20px}.db{text-align:left;margin-top:12px}.shell{display:block}.rail{border-right:0;border-bottom:2px solid var(--line)}.workspace{padding:20px}.summary{grid-template-columns:repeat(2,minmax(0,1fr))}table{font-size:.88rem}}",
    );
    html.push_str("</style></head><body>");
    html.push_str("<header class=\"topbar\"><div><p class=\"kicker\">local-only session browser</p><h1>Jottrace</h1></div><div class=\"db\">");
    html_escape_into(&mut html, &view.db_path.display().to_string());
    html.push_str("</div></header><main class=\"shell\"><aside class=\"rail\">");
    html.push_str("<form class=\"search\" method=\"get\" action=\"/\"><input name=\"q\" type=\"search\" value=\"");
    html_escape_into(&mut html, search);
    html.push_str("\" placeholder=\"Search sessions\"><button type=\"submit\">Search</button><label class=\"payload-toggle\"><input name=\"payload\" type=\"checkbox\" value=\"1\"");
    html.push_str(payload_checked);
    html.push_str("> include payload text</label></form>");
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
        html_escape_into(&mut html, &session_href(session, query));
        html.push_str("\">");
        html.push_str("<strong>");
        html_escape_into(&mut html, &session.source_session_id);
        html.push_str("</strong><span class=\"line\">");
        html_escape_into(&mut html, &session.source);
        html.push_str(" / ");
        write!(html, "{}", session.event_count).expect("write html");
        html.push_str(" events</span>");
        if let Some(cwd) = &session.cwd {
            html.push_str("<span class=\"line\">");
            html_escape_into(&mut html, cwd);
            html.push_str("</span>");
        }
        html.push_str("</a>");
    }
    render_session_pagination(&mut html, view, query);
    html.push_str("</aside><section class=\"workspace\">");

    match &view.selected_session {
        Some(session) => {
            html.push_str("<div class=\"summary\"><div class=\"metric\"><b>");
            write!(html, "{}", view.sessions.len()).expect("write html");
            html.push_str("</b><span>visible sessions</span></div><div class=\"metric\"><b>");
            write!(html, "{}", session.event_count).expect("write html");
            html.push_str("</b><span>session events</span></div><div class=\"metric\"><b>");
            write!(html, "{}", view.events.len()).expect("write html");
            html.push_str("</b><span>visible events</span></div><div class=\"metric\"><b>");
            write!(html, "{}", view.unresolved_ingest_errors.len()).expect("write html");
            html.push_str("</b><span>open errors</span></div></div>");
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
            render_events(&mut html, &view.events, query, session);
            if let Some(event) = &view.expanded_event {
                render_payload_panel(&mut html, event, session);
            }
            if let Some(page) = view.event_page {
                render_event_pagination(&mut html, page, query, session);
            }
        }
        None => html.push_str("<h2>No session selected</h2>"),
    }

    html.push_str("<h2>Unresolved ingest errors</h2>");
    render_ingest_errors(&mut html, &view.unresolved_ingest_errors);
    html.push_str("</section></main></body></html>");
    html
}

fn open_web_database(path: &Path) -> Result<Connection> {
    let conn = open_readonly_database(path, sqlite_error)?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|source| sqlite_error(path, source))?;
    let schema_version: i64 = query_one(path, &conn, "PRAGMA user_version", [], |row| row.get(0))?;
    if schema_version > LATEST_SCHEMA_VERSION {
        return Err(JottraceError::UnsupportedSchemaVersion {
            path: path.to_path_buf(),
            actual: schema_version,
            supported: LATEST_SCHEMA_VERSION,
        });
    }
    Ok(conn)
}

fn event_payload_text_for_path(path: &Path, target: &str) -> Result<Option<String>> {
    let query = journal_query_from_target(target);
    let Some(source_session_id) = query.selected_source_session_id.as_deref() else {
        return Ok(None);
    };
    let Some(key) = query.expanded_event else {
        return Ok(None);
    };

    let conn = open_web_database(path)?;
    let Some(session) = load_session_by_source_session_id(
        path,
        &conn,
        query.selected_source.as_deref(),
        source_session_id,
    )?
    else {
        return Ok(None);
    };

    load_event_payload_text(path, &conn, session.id, key)
}

fn load_sessions(
    path: &Path,
    conn: &Connection,
    search: &SearchPattern,
    offset: usize,
) -> Result<(Vec<LoadedSession>, PageInfo)> {
    let limit = SESSION_PAGE_SIZE;
    let mut sql = String::from(
        "SELECT sessions.id, sessions.source, sessions.source_session_id,
                sessions.file_path, sessions.cwd, parent.source_session_id,
                sessions.started_at, sessions.ended_at, sessions.event_count
         FROM sessions
         LEFT JOIN sessions AS parent ON parent.id = sessions.parent_session_id
         WHERE ?1 IS NULL
            OR sessions.source LIKE ?1 ESCAPE '\\'
            OR sessions.source_session_id LIKE ?1 ESCAPE '\\'
            OR COALESCE(sessions.cwd, '') LIKE ?1 ESCAPE '\\'
            OR COALESCE(sessions.file_path, '') LIKE ?1 ESCAPE '\\'",
    );
    if !search.decoded_payload_session_ids.is_empty() {
        sql.push_str(" OR sessions.id IN (");
        for index in 0..search.decoded_payload_session_ids.len() {
            if index > 0 {
                sql.push_str(", ");
            }
            write!(sql, "{}", search.decoded_payload_session_ids[index])
                .expect("write SQL session id");
        }
        sql.push(')');
    }
    sql.push_str(
        " ORDER BY COALESCE(sessions.started_at, sessions.updated_at) DESC,
                  sessions.id DESC
          LIMIT ?2 OFFSET ?3",
    );

    let sessions = query_collect(
        path,
        conn,
        &sql,
        params![search.like.as_deref(), (limit + 1) as i64, offset as i64],
        loaded_session_from_row,
    )?;
    Ok(paginate(sessions, offset, limit))
}

fn load_event_payload_text(
    path: &Path,
    conn: &Connection,
    session_id: i64,
    key: EventKey,
) -> Result<Option<String>> {
    let row = query_optional(
        path,
        conn,
        "SELECT payload, codec
             FROM events
             WHERE session_id = ?1
               AND generation = ?2
               AND seq = ?3
             LIMIT 1",
        params![session_id, key.generation, key.seq],
        |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
    )?;

    row.map(|(payload, codec)| {
        decode_event_payload(&payload, &codec)
            .map(|decoded| String::from_utf8_lossy(&decoded).into_owned())
    })
    .transpose()
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
    query_optional(
        path,
        conn,
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
}

fn session_matches(
    session: &JournalSession,
    selected_source: Option<&str>,
    source_session_id: &str,
) -> bool {
    session.source_session_id == source_session_id
        && selected_source.is_none_or(|source| session.source == source)
}

fn load_events(
    path: &Path,
    conn: &Connection,
    session_id: i64,
    offset: usize,
) -> Result<(Vec<JournalEvent>, PageInfo)> {
    let limit = EVENT_PAGE_SIZE;
    let events = query_collect(
        path,
        conn,
        "SELECT generation, seq, ts, codec, payload_size
         FROM events
         WHERE session_id = ?1
         ORDER BY generation, seq
         LIMIT ?2 OFFSET ?3",
        params![session_id, (limit + 1) as i64, offset as i64],
        journal_event_metadata_from_row,
    )?;
    Ok(paginate(events, offset, limit))
}

fn load_event_payload(
    path: &Path,
    conn: &Connection,
    session_id: i64,
    key: EventKey,
) -> Result<Option<JournalEvent>> {
    let row = query_optional(
        path,
        conn,
        "SELECT generation, seq, ts, codec, payload_size,
                CASE
                    WHEN codec = ?4 THEN substr(payload, 1, ?5)
                    ELSE payload
                END AS payload_preview
         FROM events
         WHERE session_id = ?1
           AND generation = ?2
           AND seq = ?3
         LIMIT 1",
        params![
            session_id,
            key.generation,
            key.seq,
            RAW_CODEC,
            PAYLOAD_PREVIEW_BYTES as i64,
        ],
        |row| {
            let payload_size: i64 = row.get("payload_size")?;
            Ok((
                row.get("generation")?,
                row.get("seq")?,
                row.get("ts")?,
                row.get("codec")?,
                payload_size as u64,
                row.get("payload_preview")?,
            ))
        },
    )?;

    row.map(
        |(generation, seq, ts, codec, payload_size, payload_preview)| {
            journal_event_with_payload(generation, seq, ts, codec, payload_size, payload_preview)
        },
    )
    .transpose()
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

fn journal_event_metadata_from_row(row: &Row<'_>) -> rusqlite::Result<JournalEvent> {
    journal_event_from_row(row, |_, _| String::new())
}

fn journal_event_from_row(
    row: &Row<'_>,
    payload_preview_from: impl FnOnce(&str, u64) -> String,
) -> rusqlite::Result<JournalEvent> {
    let payload_size: i64 = row.get("payload_size")?;
    let codec: String = row.get("codec")?;
    let payload_size = payload_size as u64;
    Ok(JournalEvent {
        generation: row.get("generation")?,
        seq: row.get("seq")?,
        ts: row.get("ts")?,
        payload_preview: payload_preview_from(&codec, payload_size),
        payload_error: None,
        codec,
        payload_size,
    })
}

fn journal_event_with_payload(
    generation: i64,
    seq: i64,
    ts: Option<String>,
    codec: String,
    payload_size: u64,
    payload: Vec<u8>,
) -> Result<JournalEvent> {
    let payload_preview = match decode_event_payload_prefix(&payload, &codec, PAYLOAD_PREVIEW_BYTES)
    {
        Ok(decoded) => String::from_utf8_lossy(&decoded)
            .chars()
            .take(PAYLOAD_PREVIEW_CHARS)
            .collect(),
        Err(error) => {
            return Ok(JournalEvent {
                generation,
                seq,
                ts,
                payload_preview: String::new(),
                payload_error: Some(error.to_string()),
                codec,
                payload_size,
            });
        }
    };
    Ok(JournalEvent {
        generation,
        seq,
        ts,
        payload_preview,
        payload_error: None,
        codec,
        payload_size,
    })
}

fn search_pattern(search: Option<&str>, include_payload_search: bool) -> SearchPattern {
    let Some(search) = search.map(str::trim).filter(|search| !search.is_empty()) else {
        return SearchPattern {
            like: None,
            payload_needle: None,
            decoded_payload_session_ids: Vec::new(),
        };
    };
    let include_payload =
        include_payload_search && search.chars().count() >= MIN_PAYLOAD_SEARCH_CHARS;
    SearchPattern {
        like: Some(like_contains_pattern(search)),
        payload_needle: include_payload.then(|| search.to_lowercase()),
        decoded_payload_session_ids: Vec::new(),
    }
}

fn bounded_offset(offset: usize) -> usize {
    if offset <= MAX_QUERY_OFFSET {
        offset
    } else {
        0
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

fn decoded_payload_matching_session_ids(
    path: &Path,
    conn: &Connection,
    search: &SearchPattern,
) -> Result<Vec<i64>> {
    let Some(needle) = search.payload_needle.as_deref() else {
        return Ok(Vec::new());
    };

    let mut statement = conn
        .prepare(
            "SELECT session_id,
                    CASE
                        WHEN codec = ?1 THEN substr(payload, 1, ?3)
                        ELSE payload
                    END AS payload_prefix,
                    codec
             FROM events
             WHERE codec IN (?1, ?2)
             ORDER BY session_id, generation, seq",
        )
        .map_err(|source| sqlite_error(path, source))?;
    let mut rows = statement
        .query(params![RAW_CODEC, ZSTD_CODEC, PAYLOAD_PREVIEW_BYTES as i64])
        .map_err(|source| sqlite_error(path, source))?;
    let mut session_ids = Vec::new();
    let mut matched_session_id = None;
    while let Some(row) = rows.next().map_err(|source| sqlite_error(path, source))? {
        let session_id: i64 = row_value(path, row, 0)?;
        if matched_session_id == Some(session_id) {
            continue;
        }
        let payload: Vec<u8> = row_value(path, row, 1)?;
        let codec: String = row_value(path, row, 2)?;
        let Ok(decoded) = decode_event_payload_prefix(&payload, &codec, PAYLOAD_PREVIEW_BYTES)
        else {
            continue;
        };
        let preview = String::from_utf8_lossy(&decoded).to_lowercase();
        if preview.contains(needle) {
            matched_session_id = Some(session_id);
            session_ids.push(session_id);
        }
    }
    Ok(session_ids)
}

fn render_events(
    html: &mut String,
    events: &[JournalEvent],
    query: &JournalQuery,
    session: &JournalSession,
) {
    if events.is_empty() {
        html.push_str("<p class=\"muted\">No events found for this session.</p>");
        return;
    }

    html.push_str("<div class=\"table-scroll\"><table><thead><tr><th>generation</th><th>seq</th><th>time</th><th>payload</th></tr></thead><tbody>");
    for event in events {
        html.push_str("<tr><td class=\"number\">");
        write!(html, "{}", event.generation).expect("write html");
        html.push_str("</td><td class=\"number\">");
        write!(html, "{}", event.seq).expect("write html");
        html.push_str("</td><td>");
        html_escape_into(html, event.ts.as_deref().unwrap_or(""));
        html.push_str("</td><td><a class=\"inspect\" href=\"");
        html_escape_into(html, &event_payload_href(query, session, event));
        html.push_str("\">Inspect ");
        write!(html, "{} bytes ", event.payload_size).expect("write html");
        html_escape_into(html, &event.codec);
        html.push_str("</a></td></tr>");
    }
    html.push_str("</tbody></table></div>");
}

fn render_payload_panel(html: &mut String, event: &JournalEvent, session: &JournalSession) {
    html.push_str(
        "<div class=\"payload-panel\"><h3>Payload preview</h3><div class=\"muted\">generation ",
    );
    write!(html, "{}", event.generation).expect("write html");
    html.push_str(" / seq ");
    write!(html, "{}", event.seq).expect("write html");
    html.push_str(" / ");
    write!(html, "{} bytes ", event.payload_size).expect("write html");
    html_escape_into(html, &event.codec);
    html.push_str("</div>");
    if let Some(error) = &event.payload_error {
        html.push_str("<p class=\"error\">Payload unavailable</p><pre>");
        html_escape_into(html, error);
        html.push_str("</pre></div>");
        return;
    }

    html.push_str("<pre>");
    html_escape_into(html, &event.payload_preview);
    html.push_str("</pre>");
    html.push_str("<p><a class=\"inspect\" href=\"");
    html_escape_into(html, &event_payload_export_href(session, event));
    html.push_str("\">Export full decoded payload</a></p>");
    html.push_str("</div>");
}

fn render_ingest_errors(html: &mut String, ingest_errors: &[IngestErrorSummary]) {
    if ingest_errors.is_empty() {
        html.push_str("<p class=\"muted\">No unresolved ingest errors.</p>");
        return;
    }

    html.push_str("<div class=\"table-scroll\"><table><thead><tr><th>source</th><th>session</th><th>location</th><th>error</th></tr></thead><tbody>");
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
    html.push_str("</tbody></table></div>");
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

fn render_session_pagination(html: &mut String, view: &JournalView, query: &JournalQuery) {
    let selected_session = explicit_selected_session_key(view, query);
    let page = view.session_page;
    render_pager(
        html,
        page,
        "Session pages",
        page.has_previous.then(|| {
            state_href(
                selected_session,
                query,
                page.offset.saturating_sub(page.limit),
                0,
                None,
            )
        }),
        "Previous sessions",
        page.has_next
            .then(|| state_href(selected_session, query, page.offset + page.limit, 0, None)),
        "Next sessions",
    );
}

fn explicit_selected_session_key<'a>(
    view: &'a JournalView,
    query: &JournalQuery,
) -> Option<SelectedSessionKey<'a>> {
    query
        .selected_source_session_id
        .as_ref()
        .and(view.selected_session.as_ref())
        .map(SelectedSessionKey::from)
}

fn render_event_pagination(
    html: &mut String,
    page: PageInfo,
    query: &JournalQuery,
    session: &JournalSession,
) {
    let session_key = Some(SelectedSessionKey::from(session));
    render_pager(
        html,
        page,
        "Event pages",
        page.has_previous.then(|| {
            state_href(
                session_key,
                query,
                query.session_offset,
                page.offset.saturating_sub(page.limit),
                None,
            )
        }),
        "Previous events",
        page.has_next.then(|| {
            state_href(
                session_key,
                query,
                query.session_offset,
                page.offset + page.limit,
                None,
            )
        }),
        "Next events",
    );
}

fn render_pager(
    html: &mut String,
    page: PageInfo,
    aria_label: &str,
    previous_href: Option<String>,
    previous_label: &str,
    next_href: Option<String>,
    next_label: &str,
) {
    if !page.has_previous && !page.has_next {
        return;
    }

    html.push_str("<nav class=\"pager\" aria-label=\"");
    html_escape_into(html, aria_label);
    html.push_str("\">");
    if let Some(href) = previous_href {
        push_pager_link(html, &href, previous_label);
    }
    if let Some(href) = next_href {
        push_pager_link(html, &href, next_label);
    }
    html.push_str("</nav>");
}

fn push_pager_link(html: &mut String, href: &str, label: &str) {
    html.push_str("<a href=\"");
    html_escape_into(html, href);
    html.push_str("\">");
    html.push_str(label);
    html.push_str("</a>");
}

fn session_href(session: &JournalSession, query: &JournalQuery) -> String {
    state_href(
        Some(SelectedSessionKey::from(session)),
        query,
        query.session_offset,
        0,
        None,
    )
}

fn event_payload_href(
    query: &JournalQuery,
    session: &JournalSession,
    event: &JournalEvent,
) -> String {
    state_href(
        Some(SelectedSessionKey::from(session)),
        query,
        query.session_offset,
        query.event_offset,
        Some(EventKey {
            generation: event.generation,
            seq: event.seq,
        }),
    )
}

fn event_payload_export_href(session: &JournalSession, event: &JournalEvent) -> String {
    let mut href = String::from("/payload");
    let mut first = true;
    append_url_param(&mut href, &mut first, "source", &session.source);
    append_url_param(&mut href, &mut first, "session", &session.source_session_id);
    append_url_param(
        &mut href,
        &mut first,
        "event_generation",
        &event.generation.to_string(),
    );
    append_url_param(&mut href, &mut first, "event_seq", &event.seq.to_string());
    href
}

#[derive(Debug, Clone, Copy)]
struct SelectedSessionKey<'a> {
    source: Option<&'a str>,
    source_session_id: &'a str,
}

impl<'a> From<&'a JournalSession> for SelectedSessionKey<'a> {
    fn from(session: &'a JournalSession) -> Self {
        Self {
            source: Some(&session.source),
            source_session_id: &session.source_session_id,
        }
    }
}

fn state_href(
    selected_session: Option<SelectedSessionKey<'_>>,
    query: &JournalQuery,
    session_offset: usize,
    event_offset: usize,
    expanded_event: Option<EventKey>,
) -> String {
    let mut href = String::new();
    let mut first = true;
    if let Some(session) = selected_session {
        if let Some(source) = session.source {
            append_url_param(&mut href, &mut first, "source", source);
        }
        append_url_param(&mut href, &mut first, "session", session.source_session_id);
    }
    if let Some(search) = query.search.as_deref().filter(|search| !search.is_empty()) {
        append_url_param(&mut href, &mut first, "q", search);
    }
    if query.include_payload_search {
        append_url_param(&mut href, &mut first, "payload", "1");
    }
    if session_offset > 0 {
        append_url_param(
            &mut href,
            &mut first,
            "session_offset",
            &session_offset.to_string(),
        );
    }
    if event_offset > 0 {
        append_url_param(
            &mut href,
            &mut first,
            "event_offset",
            &event_offset.to_string(),
        );
    }
    if let Some(event) = expanded_event {
        append_url_param(
            &mut href,
            &mut first,
            "event_generation",
            &event.generation.to_string(),
        );
        append_url_param(&mut href, &mut first, "event_seq", &event.seq.to_string());
    }
    if href.is_empty() {
        "/".to_string()
    } else {
        href
    }
}

fn append_url_param(href: &mut String, first: &mut bool, key: &str, value: &str) {
    href.push(if *first { '?' } else { '&' });
    *first = false;
    href.push_str(key);
    href.push('=');
    href.push_str(&url_encode(value));
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
    let mut event_generation = None;
    let mut event_seq = None;
    for pair in raw_query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "source" => query.selected_source = Some(percent_decode(value)),
            "session" => query.selected_source_session_id = Some(percent_decode(value)),
            "q" => query.search = Some(percent_decode(value)),
            "payload" => query.include_payload_search = value == "1" || value == "true",
            "session_offset" => query.session_offset = parse_offset(value),
            "event_offset" => query.event_offset = parse_offset(value),
            "event_generation" => event_generation = percent_decode(value).parse().ok(),
            "event_seq" => event_seq = percent_decode(value).parse().ok(),
            _ => {}
        }
    }
    if let (Some(generation), Some(seq)) = (event_generation, event_seq) {
        query.expanded_event = Some(EventKey { generation, seq });
    }
    query
}

fn parse_offset(value: &str) -> usize {
    percent_decode(value)
        .parse()
        .ok()
        .map(bounded_offset)
        .unwrap_or(0)
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
