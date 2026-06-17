use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, named_params, params};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::HashSet;
use std::env;
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::storage::{
    DB_FILE_NAME, encode_event_payload, execute_sql, open_database, query_optional, sqlite_error,
    status_from_connection,
};
use crate::{JottraceError, Result, io_error};

const CLAUDE_SOURCE: &str = "claude_cli";
const CLAUDE_LOCAL_AGENT_SOURCE: &str = "claude_local_agent";
const CODEX_SOURCE: &str = "codex_cli";
const PI_AGENT_SOURCE: &str = "pi_agent";
const GEMINI_SOURCE: &str = "gemini_cli";
const FACTORY_SOURCE: &str = "factory";
const OPENCODE_SOURCE: &str = "opencode";
const HERMES_SOURCE: &str = "hermes";
const CLAUDE_INSTALL_DIRS: &[&str] = &[
    ".claude",
    ".claude-code",
    ".claude-local",
    ".claude-m2",
    ".claude-zai",
];
const CODEX_INSTALL_DIRS: &[&str] = &[".codex", ".codex-local"];
const PI_AGENT_SESSIONS_DIR: &str = ".pi/agent/sessions";
const CLAUDE_LOCAL_AGENT_SESSIONS_DIR: &str =
    "Library/Application Support/Claude/local-agent-mode-sessions";
const CLAUDE_LOCAL_AGENT_AUDIT_FILE_NAME: &str = "audit.jsonl";
const GEMINI_TMP_DIR: &str = ".gemini/tmp";
const FACTORY_INSTALL_DIRS: &[&str] = &[".factory"];
const OPENCODE_DB_PATH: &str = ".local/share/opencode/opencode.db";
const OPENCODE_DB_ERROR_SESSION_ID: &str = "opencode.db";
const HERMES_DB_PATH: &str = ".hermes/state.db";
const HERMES_DB_ERROR_SESSION_ID: &str = "state.db";
const MAX_SESSION_HEADER_BYTES: u64 = 64 * 1024;
const FACTORY_SESSION_START_MISSING_MESSAGE: &str =
    "factory session file has no committed session_start line within the header limit";
const PI_AGENT_SESSION_HEADER_MISSING_MESSAGE: &str =
    "Pi agent session file has no committed session event line within the header limit";
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const INVALID_JSON_ERROR_KIND: &str = "invalid_json";
const INVALID_SESSION_META_ERROR_KIND: &str = "invalid_session_meta";
const READ_ERROR_KIND: &str = "read_error";
const NOT_FILE_ERROR_KIND: &str = "not_file";
const SOURCE_FILE_INGESTED_SUCCESSFULLY_NOTE: &str = "source file ingested successfully";
const EMPTY_CODEX_SESSION_FILE_SKIPPED_NOTE: &str = "empty Codex session file skipped";
const INSERT_EVENT_SQL: &str = "INSERT OR IGNORE INTO events
    (session_id, generation, seq, ts, payload, codec, payload_size)
 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestReport {
    pub db_path: PathBuf,
    pub file_count: u64,
    pub session_count: u64,
    pub event_count: u64,
    pub inserted_event_count: u64,
    pub skipped_file_count: u64,
    pub unresolved_ingest_error_count: u64,
}

#[derive(Debug, Default)]
struct IngestState {
    unresolved_invalid_session_meta_paths: HashSet<(String, String)>,
    unresolved_invalid_json_paths: HashSet<(String, String)>,
}

#[derive(Debug, Clone)]
struct SourceFile {
    source: &'static str,
    source_session_id: String,
    source_session_id_kind: SourceSessionIdKind,
    parent_source_session_id: Option<String>,
    metadata_path: Option<PathBuf>,
    source_format: SourceFormat,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceSessionIdKind {
    Known,
    ClaudeLocalAgentAudit,
    CodexSessionMeta,
    GeminiChatJson,
    FactorySessionStart,
    PiAgentSessionHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceFormat {
    Jsonl,
    GeminiChatJson,
    OpenCodeSqlite,
    HermesSqlite,
}

#[derive(Debug, Clone)]
struct StoredSession {
    id: i64,
    source_session_id: String,
    file_path: Option<String>,
    parent_session_id: Option<i64>,
    current_generation: i64,
    file_size: Option<i64>,
    file_mtime: Option<i64>,
    content_fingerprint: Option<String>,
    source_metadata: Option<String>,
    next_read_offset: i64,
    event_count: i64,
    /// Fingerprint of the bytes already consumed (`buffer[..next_read_offset]`).
    /// Lets append resumes verify the prefix is unchanged before trusting the
    /// stored offset; a mismatch means the file was rewritten, not appended.
    prefix_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportMode {
    Append,
    Rewrite,
    Skip,
}

#[derive(Debug, Default)]
struct ParsedMetadata {
    cwd: Option<String>,
    started_at: Option<String>,
    ended_at: Option<String>,
}

impl ParsedMetadata {
    fn fill_missing_from(&mut self, other: ParsedMetadata) {
        fill_missing(&mut self.cwd, other.cwd);
        fill_missing(&mut self.started_at, other.started_at);
        fill_missing(&mut self.ended_at, other.ended_at);
    }
}

struct SessionUpdate<'a> {
    source_file: &'a SourceFile,
    session_id: i64,
    parent_session_id: Option<i64>,
    generation: i64,
    pass_size: i64,
    file_mtime: Option<i64>,
    content_fingerprint: &'a str,
    prefix_fingerprint: &'a str,
    source_metadata: Option<&'a str>,
    next_read_offset: i64,
    event_count: i64,
    metadata: &'a ParsedMetadata,
}

struct IngestErrorRecord<'a> {
    source_file: &'a SourceFile,
    session_id: Option<i64>,
    generation: Option<i64>,
    byte_offset: Option<i64>,
    line_number: Option<i64>,
    error_kind: &'a str,
    message: &'a str,
}

struct ResolvedSourceFile {
    source_file: SourceFile,
    buffer: Option<Vec<u8>>,
}

struct SkippedSessionUpdate<'a> {
    source_file: &'a SourceFile,
    session_id: i64,
    parent_session_id: Option<i64>,
    source_metadata: Option<&'a str>,
    checked_file: Option<CheckedFileState<'a>>,
}

struct CheckedFileState<'a> {
    pass_size: i64,
    file_mtime: Option<i64>,
    content_fingerprint: &'a str,
}

#[derive(Debug, Deserialize)]
struct EventHeader<'a> {
    #[serde(default, borrow)]
    id: Option<&'a str>,
    #[serde(default, borrow, rename = "type")]
    event_type: Option<&'a str>,
    #[serde(default, borrow)]
    payload: Option<EventPayloadHeader<'a>>,
    #[serde(default, borrow)]
    cwd: Option<&'a str>,
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
    #[serde(default, borrow, rename = "_audit_timestamp")]
    audit_timestamp: Option<&'a str>,
    #[serde(default, borrow)]
    snapshot: Option<SnapshotHeader<'a>>,
}

#[derive(Debug, Deserialize)]
struct SnapshotHeader<'a> {
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct EventPayloadHeader<'a> {
    #[serde(default, borrow)]
    id: Option<&'a str>,
    #[serde(default, borrow)]
    cwd: Option<&'a str>,
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct LegacyCodexHeader<'a> {
    #[serde(default, borrow)]
    id: Option<&'a str>,
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct ClaudeLocalAgentAuditHeader<'a> {
    #[serde(default, borrow, alias = "sessionId")]
    session_id: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct ClaudeLocalAgentMetadata<'a> {
    #[serde(
        default,
        borrow,
        alias = "workspace_path",
        alias = "workspacePath",
        alias = "project_path",
        alias = "projectPath"
    )]
    cwd: Option<&'a str>,
    #[serde(
        default,
        alias = "startedAt",
        alias = "created_at",
        alias = "createdAt"
    )]
    started_at: Option<serde_json::Value>,
    #[serde(
        default,
        alias = "endedAt",
        alias = "updated_at",
        alias = "updatedAt",
        alias = "last_activity_at",
        alias = "lastActivityAt"
    )]
    ended_at: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiChatFile<'a> {
    #[serde(borrow)]
    session_id: &'a str,
    #[serde(default, borrow)]
    project_hash: Option<&'a str>,
    #[serde(default, borrow)]
    start_time: Option<&'a str>,
    #[serde(default, borrow)]
    last_updated: Option<&'a str>,
    #[serde(borrow)]
    messages: Vec<&'a RawValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiChatIdentity<'a> {
    #[serde(borrow)]
    session_id: &'a str,
}

#[derive(Debug, Deserialize)]
struct GeminiMessageHeader<'a> {
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
}

#[derive(Debug)]
struct SqliteSessionSnapshot {
    events: Vec<SourceEvent>,
    metadata: ParsedMetadata,
    source_metadata: String,
}

#[derive(Debug)]
struct SourceEvent {
    sort_time: i64,
    rank: i64,
    id: String,
    ts: Option<String>,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
enum OpenCodeTable {
    Message,
    Part,
    SessionMessage,
}

impl OpenCodeTable {
    fn name(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Part => "part",
            Self::SessionMessage => "session_message",
        }
    }

    fn rank(self) -> i64 {
        match self {
            Self::Message => 1,
            Self::Part => 2,
            Self::SessionMessage => 3,
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Self::Message => {
                "SELECT id, session_id, NULL, NULL, time_created, time_updated,
                        strftime('%Y-%m-%dT%H:%M:%fZ', time_created / 1000.0, 'unixepoch'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', time_updated / 1000.0, 'unixepoch'),
                        data
                 FROM message
                 WHERE session_id = ?1"
            }
            Self::Part => {
                "SELECT id, session_id, message_id, NULL, time_created, time_updated,
                        strftime('%Y-%m-%dT%H:%M:%fZ', time_created / 1000.0, 'unixepoch'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', time_updated / 1000.0, 'unixepoch'),
                        data
                 FROM part
                 WHERE session_id = ?1"
            }
            Self::SessionMessage => {
                "SELECT id, session_id, NULL, type, time_created, time_updated,
                        strftime('%Y-%m-%dT%H:%M:%fZ', time_created / 1000.0, 'unixepoch'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', time_updated / 1000.0, 'unixepoch'),
                        data
                 FROM session_message
                 WHERE session_id = ?1"
            }
        }
    }
}

pub fn run_ingest() -> Result<IngestReport> {
    let data_dir = crate::data_dir_from_env()?;
    let _lock = crate::acquire_data_lock(&data_dir)?;
    let source_files = discover_session_files()?;
    let db_path = data_dir.join(DB_FILE_NAME);
    let mut conn = open_database(&db_path)?;
    let mut ingest_state = IngestState {
        unresolved_invalid_session_meta_paths: unresolved_source_file_error_paths(
            &db_path,
            &conn,
            INVALID_SESSION_META_ERROR_KIND,
        )?,
        unresolved_invalid_json_paths: unresolved_source_file_error_paths(
            &db_path,
            &conn,
            INVALID_JSON_ERROR_KIND,
        )?,
    };
    let mut inserted_event_count = 0;
    let mut skipped_file_count: u64 = 0;

    for source_file in &source_files {
        match ingest_source_file(&mut conn, &mut ingest_state, source_file) {
            Ok(inserted) => {
                if inserted == 0 {
                    skipped_file_count += 1;
                }
                inserted_event_count += inserted;
            }
            Err(error) if is_source_file_ingest_error(&error) => {
                record_source_file_ingest_error(&mut conn, source_file, &error)?;
            }
            Err(error) => return Err(error),
        }
    }

    let status = status_from_connection(&db_path, &conn)?;

    Ok(IngestReport {
        db_path,
        file_count: source_files.len() as u64,
        session_count: status.session_count,
        event_count: status.event_count,
        inserted_event_count,
        skipped_file_count,
        unresolved_ingest_error_count: status.unresolved_ingest_error_count,
    })
}

fn discover_session_files() -> Result<Vec<SourceFile>> {
    let mut source_files = discover_claude_session_files()?;
    source_files.extend(discover_claude_local_agent_session_files()?);
    source_files.extend(discover_codex_session_files()?);
    source_files.extend(discover_pi_agent_session_files()?);
    source_files.extend(discover_gemini_session_files()?);
    source_files.extend(discover_factory_session_files()?);
    source_files.extend(discover_opencode_session_files()?);
    source_files.extend(discover_hermes_session_files()?);
    Ok(source_files)
}

/// Build [`SourceFile`]s for a JSONL source whose session id derives purely from
/// the file path and which never carries a parent session (Codex, Gemini, Factory).
fn jsonl_source_files_from_paths(
    paths: Vec<PathBuf>,
    source: &'static str,
    source_session_id_kind: SourceSessionIdKind,
    source_format: SourceFormat,
) -> Result<Vec<SourceFile>> {
    paths
        .into_iter()
        .map(|path| {
            let (source_session_id, _) = source_session_ids_from_path(&path)?;
            Ok(SourceFile {
                source,
                source_session_id,
                source_session_id_kind,
                parent_source_session_id: None,
                metadata_path: None,
                source_format,
                path,
            })
        })
        .collect()
}

fn discover_claude_session_files() -> Result<Vec<SourceFile>> {
    let home = home_dir()?;
    let mut paths = Vec::new();

    for install_dir in CLAUDE_INSTALL_DIRS {
        let root = home.join(install_dir);
        collect_jsonl_files(&root.join("projects"), true, &mut paths)?;
        collect_flat_session_files(&root, &mut paths)?;
    }

    sort_dedup_paths(&mut paths);

    let mut source_files = paths
        .into_iter()
        .map(|path| {
            let (source_session_id, parent_source_session_id) =
                source_session_ids_from_path(&path)?;
            Ok(SourceFile {
                source: CLAUDE_SOURCE,
                source_session_id,
                source_session_id_kind: SourceSessionIdKind::Known,
                parent_source_session_id,
                metadata_path: None,
                source_format: SourceFormat::Jsonl,
                path,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    sort_source_files_with_parents_last(&mut source_files);
    Ok(source_files)
}

fn discover_claude_local_agent_session_files() -> Result<Vec<SourceFile>> {
    let root = home_dir()?.join(CLAUDE_LOCAL_AGENT_SESSIONS_DIR);
    let mut paths = Vec::new();
    collect_claude_local_agent_audit_files(&root, &mut paths)?;
    sort_dedup_paths(&mut paths);

    paths
        .into_iter()
        .map(|path| {
            Ok(SourceFile {
                source: CLAUDE_LOCAL_AGENT_SOURCE,
                source_session_id: claude_local_agent_fallback_source_session_id(&path)?,
                source_session_id_kind: SourceSessionIdKind::ClaudeLocalAgentAudit,
                parent_source_session_id: None,
                metadata_path: claude_local_agent_metadata_path(&path),
                source_format: SourceFormat::Jsonl,
                path,
            })
        })
        .collect()
}

fn discover_codex_session_files() -> Result<Vec<SourceFile>> {
    let home = home_dir()?;
    let mut paths = Vec::new();

    for install_dir in CODEX_INSTALL_DIRS {
        let root = home.join(install_dir);
        collect_jsonl_files(&root.join("sessions"), true, &mut paths)?;
        collect_jsonl_files(&root.join("archived_sessions"), false, &mut paths)?;
    }

    sort_dedup_paths(&mut paths);

    jsonl_source_files_from_paths(
        paths,
        CODEX_SOURCE,
        SourceSessionIdKind::CodexSessionMeta,
        SourceFormat::Jsonl,
    )
}

fn discover_gemini_session_files() -> Result<Vec<SourceFile>> {
    let root = home_dir()?.join(GEMINI_TMP_DIR);

    let mut paths = Vec::new();
    for_each_dir_entry(&root, |path, file_type| {
        if file_type.is_dir() {
            collect_json_files(&path.join("chats"), false, &mut paths)?;
        }
        Ok(())
    })?;

    sort_dedup_paths(&mut paths);

    jsonl_source_files_from_paths(
        paths,
        GEMINI_SOURCE,
        SourceSessionIdKind::GeminiChatJson,
        SourceFormat::GeminiChatJson,
    )
}

fn discover_pi_agent_session_files() -> Result<Vec<SourceFile>> {
    let home = home_dir()?;
    let mut paths = Vec::new();

    collect_jsonl_files(&home.join(PI_AGENT_SESSIONS_DIR), true, &mut paths)?;

    sort_dedup_paths(&mut paths);

    let mut source_files = paths
        .into_iter()
        .map(|path| {
            let (source_session_id, source_session_id_kind, parent_source_session_id) =
                if let Some(nested) = pi_agent_nested_run_info(&path) {
                    (
                        nested.placeholder_source_session_id,
                        SourceSessionIdKind::PiAgentSessionHeader,
                        Some(nested.parent_source_session_id),
                    )
                } else {
                    (
                        pi_source_session_id_from_path(&path)?,
                        SourceSessionIdKind::Known,
                        None,
                    )
                };
            Ok(SourceFile {
                source: PI_AGENT_SOURCE,
                source_session_id,
                source_session_id_kind,
                parent_source_session_id,
                metadata_path: None,
                source_format: SourceFormat::Jsonl,
                path,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    sort_source_files_with_parents_last(&mut source_files);
    Ok(source_files)
}

fn discover_factory_session_files() -> Result<Vec<SourceFile>> {
    let home = home_dir()?;
    let mut paths = Vec::new();

    for install_dir in FACTORY_INSTALL_DIRS {
        collect_jsonl_files(&home.join(install_dir).join("sessions"), true, &mut paths)?;
    }

    sort_dedup_paths(&mut paths);

    jsonl_source_files_from_paths(
        paths,
        FACTORY_SOURCE,
        SourceSessionIdKind::FactorySessionStart,
        SourceFormat::Jsonl,
    )
}

fn discover_opencode_session_files() -> Result<Vec<SourceFile>> {
    discover_sqlite_session_files(
        OPENCODE_DB_PATH,
        OPENCODE_SOURCE,
        SourceFormat::OpenCodeSqlite,
        OPENCODE_DB_ERROR_SESSION_ID,
        "SELECT id, parent_id
         FROM session
         ORDER BY parent_id IS NOT NULL, time_created, id",
        opencode_source_connection,
    )
}

fn discover_hermes_session_files() -> Result<Vec<SourceFile>> {
    discover_sqlite_session_files(
        HERMES_DB_PATH,
        HERMES_SOURCE,
        SourceFormat::HermesSqlite,
        HERMES_DB_ERROR_SESSION_ID,
        "SELECT id, parent_session_id
         FROM sessions
         ORDER BY parent_session_id IS NOT NULL, started_at, id",
        hermes_source_connection,
    )
}

fn discover_sqlite_session_files(
    db_relative_path: &str,
    source: &'static str,
    source_format: SourceFormat,
    error_session_id: &'static str,
    session_query: &str,
    open_connection: fn(&Path) -> Result<Connection>,
) -> Result<Vec<SourceFile>> {
    let path = home_dir()?.join(db_relative_path);
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Ok(sqlite_discovery_errors(
                source,
                error_session_id,
                source_format,
                path,
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_error(&path, source)),
    }

    // Any failure reading the store — open, prepare, query, a bad row, or an
    // empty session id — collapses to a single discovery error so the rest of
    // ingest can proceed.
    let Some(sessions) = read_sqlite_session_rows(&path, session_query, open_connection) else {
        return Ok(sqlite_discovery_errors(
            source,
            error_session_id,
            source_format,
            path,
        ));
    };

    Ok(sessions
        .into_iter()
        .map(|(source_session_id, parent_source_session_id)| SourceFile {
            source,
            source_session_id,
            source_session_id_kind: SourceSessionIdKind::Known,
            parent_source_session_id,
            metadata_path: None,
            source_format,
            path: path.clone(),
        })
        .collect())
}

/// Read `(source_session_id, parent_source_session_id)` rows from a source
/// SQLite store, returning `None` if the store cannot be opened or queried, or
/// yields a row with an empty session id. Callers translate `None` into a
/// discovery error placeholder.
fn read_sqlite_session_rows(
    path: &Path,
    session_query: &str,
    open_connection: fn(&Path) -> Result<Connection>,
) -> Option<Vec<(String, Option<String>)>> {
    let conn = open_connection(path).ok()?;
    let mut statement = conn.prepare(session_query).ok()?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .ok()?;

    let mut sessions = Vec::new();
    for row in rows {
        let (source_session_id, parent_source_session_id): (String, Option<String>) = row.ok()?;
        if source_session_id.trim().is_empty() {
            return None;
        }
        sessions.push((source_session_id, parent_source_session_id));
    }
    Some(sessions)
}

fn sqlite_discovery_errors(
    source: &'static str,
    error_session_id: &'static str,
    source_format: SourceFormat,
    path: PathBuf,
) -> Vec<SourceFile> {
    vec![sqlite_error_source_file(
        source,
        error_session_id,
        source_format,
        path,
    )]
}

fn sqlite_error_source_file(
    source: &'static str,
    error_session_id: &str,
    source_format: SourceFormat,
    path: PathBuf,
) -> SourceFile {
    SourceFile {
        source,
        source_session_id: error_session_id.to_string(),
        source_session_id_kind: SourceSessionIdKind::Known,
        parent_source_session_id: None,
        metadata_path: None,
        source_format,
        path,
    }
}

fn sort_dedup_paths(paths: &mut Vec<PathBuf>) {
    paths.sort();
    paths.dedup();
}

fn sort_source_files_with_parents_last(source_files: &mut [SourceFile]) {
    source_files.sort_by(|left, right| {
        left.parent_source_session_id
            .is_some()
            .cmp(&right.parent_source_session_id.is_some())
            .then_with(|| left.path.cmp(&right.path))
    });
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(JottraceError::MissingHome)
}

/// Iterate the entries of `root`, invoking `handle` with each entry's path and
/// file type. A missing directory is treated as empty (returns `Ok(())`),
/// matching the discovery callers that tolerate absent source roots; any other
/// `read_dir`/`file_type` failure becomes an [`io_error`]. Entries are read and
/// handled lazily so the first failure short-circuits exactly as an inline loop
/// would.
fn for_each_dir_entry(
    root: &Path,
    mut handle: impl FnMut(PathBuf, fs::FileType) -> Result<()>,
) -> Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(io_error(root, source));
        }
    };

    for entry in entries {
        let entry = entry.map_err(|source| io_error(root, source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| io_error(&path, source))?;
        handle(path, file_type)?;
    }

    Ok(())
}

fn collect_claude_local_agent_audit_files(root: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    for_each_dir_entry(root, |path, file_type| {
        if !file_type.is_dir() {
            return Ok(());
        }

        if is_claude_local_agent_session_dir(&path) {
            push_claude_local_agent_audit_file(&path, paths)?;
        } else {
            collect_claude_local_agent_audit_files(&path, paths)?;
        }
        Ok(())
    })
}

fn is_claude_local_agent_session_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("local_"))
}

fn push_claude_local_agent_audit_file(session_dir: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    let audit_path = session_dir.join(CLAUDE_LOCAL_AGENT_AUDIT_FILE_NAME);
    match fs::metadata(&audit_path) {
        Ok(metadata) if metadata.is_file() => paths.push(audit_path),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(io_error(&audit_path, source));
        }
    }
    Ok(())
}

fn collect_jsonl_files(root: &Path, recursive: bool, paths: &mut Vec<PathBuf>) -> Result<()> {
    collect_files_with_extension(root, "jsonl", recursive, paths)
}

fn collect_json_files(root: &Path, recursive: bool, paths: &mut Vec<PathBuf>) -> Result<()> {
    collect_files_with_extension(root, "json", recursive, paths)
}

fn collect_files_with_extension(
    root: &Path,
    extension: &str,
    recursive: bool,
    paths: &mut Vec<PathBuf>,
) -> Result<()> {
    for_each_dir_entry(root, |path, file_type| {
        if file_type.is_dir() {
            if recursive {
                collect_files_with_extension(&path, extension, true, paths)?;
            }
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|path_extension| path_extension == extension)
        {
            paths.push(path);
        }
        Ok(())
    })
}

fn collect_flat_session_files(root: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    let mut flat_paths = Vec::new();
    collect_jsonl_files(root, false, &mut flat_paths)?;
    paths.extend(flat_paths.into_iter().filter(|path| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(is_uuid_stem)
    }));
    Ok(())
}

fn ingest_source_file(
    conn: &mut Connection,
    ingest_state: &mut IngestState,
    source_file: &SourceFile,
) -> Result<u64> {
    let metadata =
        fs::metadata(&source_file.path).map_err(|source| io_error(&source_file.path, source))?;
    if !metadata.is_file() {
        return Err(JottraceError::NotFile(source_file.path.clone()));
    }

    let pass_size_bytes = metadata.len();
    let pass_size = i64_from_u64(pass_size_bytes, &source_file.path)?;
    let file_mtime = file_mtime(&metadata);
    let sqlite_snapshot = match source_file.source_format {
        SourceFormat::OpenCodeSqlite => Some(opencode_session_snapshot(source_file)?),
        SourceFormat::HermesSqlite => Some(hermes_session_snapshot(source_file)?),
        _ => None,
    };
    if let Some(snapshot) = sqlite_snapshot {
        return ingest_sqlite_session_snapshot(
            conn,
            ingest_state,
            source_file,
            pass_size,
            file_mtime,
            snapshot,
        );
    }

    let resolved_source_file = if source_file.source == CODEX_SOURCE && pass_size_bytes == 0 {
        match load_session_by_source_file_path(conn, source_file)? {
            Some(stored) if !is_empty_source_file_placeholder(&stored) => {
                resolved_known_source_file(source_file, stored.source_session_id, None)
            }
            stored => {
                resolve_invalid_session_meta_error(
                    conn,
                    ingest_state,
                    source_file,
                    EMPTY_CODEX_SESSION_FILE_SKIPPED_NOTE,
                )?;
                remove_empty_source_file_placeholder(conn, stored.as_ref(), source_file)?;
                return Ok(0);
            }
        }
    } else {
        resolve_source_file(conn, source_file, pass_size, pass_size_bytes, file_mtime)?
    };
    let ResolvedSourceFile {
        source_file,
        buffer: preloaded_buffer,
    } = resolved_source_file;
    let stored = load_session(conn, &source_file)?;
    let session_source_metadata = source_metadata_for_source_file(
        &source_file,
        stored
            .as_ref()
            .and_then(|stored| stored.source_metadata.as_deref()),
    )?;
    let retry_unresolved_invalid_json = ingest_state
        .unresolved_invalid_json_paths
        .contains(&source_file_error_path_key(&source_file));

    if let Some(stored) = &stored
        && !retry_unresolved_invalid_json
        && unchanged_by_mtime(stored, pass_size, file_mtime)
    {
        let parent_session_id = resolve_parent_session_id(conn, &source_file)?;
        if stored_jsonl_linked_session_matches(
            stored,
            &source_file,
            parent_session_id,
            session_source_metadata.as_deref(),
        ) {
            resolve_ingest_success_errors(
                conn,
                ingest_state,
                &source_file,
                stored.next_read_offset,
                pass_size,
            )?;
            return Ok(0);
        }

        let tx = begin_transaction(conn, &source_file.path)?;
        commit_skipped_session_refresh(
            tx,
            ingest_state,
            &SkippedSessionUpdate {
                source_file: &source_file,
                session_id: stored.id,
                parent_session_id,
                source_metadata: session_source_metadata.as_deref(),
                checked_file: None,
            },
            Some(JsonResolution {
                next_read_offset: stored.next_read_offset,
                read_boundary: pass_size,
            }),
        )?;
        return Ok(0);
    }

    validate_source_file_header(&source_file)?;
    let buffer = match preloaded_buffer {
        Some(buffer) => buffer,
        None => read_bounded(&source_file.path, pass_size_bytes)?,
    };
    let content_fingerprint = fingerprint(&buffer);
    let source_metadata = source_metadata(&source_file)?;

    let tx = begin_transaction(conn, &source_file.path)?;
    insert_session_if_missing(&tx, &source_file)?;
    let stored = load_session(&tx, &source_file)?.expect("session row should exist after insert");
    let prefix_intact = append_prefix_intact(
        stored.next_read_offset,
        stored.prefix_fingerprint.as_deref(),
        &buffer,
    );
    let read_boundary = invalid_json_resolution_boundary(&source_file, &buffer, pass_size)?;
    let import_mode = import_mode(
        &source_file,
        &stored,
        pass_size,
        &content_fingerprint,
        prefix_intact,
        retry_unresolved_invalid_json,
    );

    if import_mode == ImportMode::Skip {
        let parent_session_id = resolve_parent_session_id(&tx, &source_file)?;
        commit_skipped_session_refresh(
            tx,
            ingest_state,
            &SkippedSessionUpdate {
                source_file: &source_file,
                session_id: stored.id,
                parent_session_id,
                source_metadata: session_source_metadata.as_deref(),
                checked_file: Some(CheckedFileState {
                    pass_size,
                    file_mtime,
                    content_fingerprint: &content_fingerprint,
                }),
            },
            Some(JsonResolution {
                next_read_offset: stored.next_read_offset,
                read_boundary,
            }),
        )?;
        return Ok(0);
    }

    let generation = if import_mode == ImportMode::Rewrite {
        stored.current_generation + 1
    } else {
        stored.current_generation
    };
    let imported = match source_file.source_format {
        SourceFormat::Jsonl => {
            let committed_len = read_boundary as usize;
            let start_offset = if import_mode == ImportMode::Append {
                stored.next_read_offset.max(0) as usize
            } else {
                0
            }
            .min(committed_len);

            import_committed_lines(
                &tx,
                &source_file,
                stored.id,
                generation,
                &buffer,
                start_offset,
                committed_len,
            )?
        }
        SourceFormat::GeminiChatJson => {
            import_gemini_chat_json(&tx, &source_file, stored.id, generation, &buffer)?
        }
        SourceFormat::OpenCodeSqlite | SourceFormat::HermesSqlite => {
            unreachable!("SQLite sources are imported earlier")
        }
    };

    let mut metadata = imported.metadata;
    metadata.fill_missing_from(source_metadata);
    let event_count = generation_event_count(&tx, &source_file, stored.id, generation)?;
    let parent_session_id = resolve_parent_session_id(&tx, &source_file)?;
    let consumed = (imported.next_read_offset.max(0) as usize).min(buffer.len());
    let prefix_fingerprint = fingerprint(&buffer[..consumed]);
    update_session_after_import(
        &tx,
        SessionUpdate {
            source_file: &source_file,
            session_id: stored.id,
            parent_session_id,
            generation,
            pass_size,
            file_mtime,
            content_fingerprint: &content_fingerprint,
            prefix_fingerprint: &prefix_fingerprint,
            source_metadata: session_source_metadata.as_deref(),
            next_read_offset: imported.next_read_offset,
            event_count,
            metadata: &metadata,
        },
    )?;
    resolve_success_and_commit(
        tx,
        ingest_state,
        &source_file,
        imported.next_read_offset,
        read_boundary,
    )?;
    Ok(imported.inserted_event_count)
}

fn ingest_sqlite_session_snapshot(
    conn: &mut Connection,
    ingest_state: &mut IngestState,
    source_file: &SourceFile,
    pass_size: i64,
    file_mtime: Option<i64>,
    snapshot: SqliteSessionSnapshot,
) -> Result<u64> {
    let stored = load_session(conn, source_file)?;
    let parent_session_id = resolve_parent_session_id(conn, source_file)?;
    let content_fingerprint = source_events_fingerprint(&snapshot.events);
    if let Some(stored) = &stored
        && stored.content_fingerprint.as_deref() == Some(content_fingerprint.as_str())
    {
        if stored_sqlite_snapshot_matches(
            stored,
            source_file,
            pass_size,
            file_mtime,
            parent_session_id,
            &snapshot.source_metadata,
        ) {
            resolve_ingest_success_meta_error(conn, ingest_state, source_file)?;
            return Ok(0);
        }

        let tx = begin_transaction(conn, &source_file.path)?;
        commit_skipped_session_refresh(
            tx,
            ingest_state,
            &SkippedSessionUpdate {
                source_file,
                session_id: stored.id,
                parent_session_id,
                source_metadata: Some(snapshot.source_metadata.as_str()),
                checked_file: Some(CheckedFileState {
                    pass_size,
                    file_mtime,
                    content_fingerprint: &content_fingerprint,
                }),
            },
            None,
        )?;
        return Ok(0);
    }

    let generation = stored
        .as_ref()
        .filter(|stored| !is_empty_source_file_placeholder(stored))
        .map_or(0, |stored| stored.current_generation + 1);
    let tx = begin_transaction(conn, &source_file.path)?;
    insert_session_if_missing(&tx, source_file)?;
    let stored = load_session(&tx, source_file)?.expect("session row should exist after insert");
    let mut inserted_event_count = 0;
    {
        let mut event_insert = tx
            .prepare(INSERT_EVENT_SQL)
            .map_err(|source| sqlite_error(&source_file.path, source))?;
        for (seq, event) in snapshot.events.iter().enumerate() {
            inserted_event_count += insert_event_payload(
                &mut event_insert,
                source_file,
                stored.id,
                generation,
                i64_from_usize(seq, &source_file.path)?,
                event.ts.as_deref(),
                &event.payload,
            )?;
        }
    }
    update_session_after_import(
        &tx,
        SessionUpdate {
            source_file,
            session_id: stored.id,
            parent_session_id,
            generation,
            pass_size,
            file_mtime,
            content_fingerprint: &content_fingerprint,
            // SQLite snapshots are not byte-resumed, so the prefix fingerprint is
            // never read back for them (it is only consulted for JSONL sources).
            prefix_fingerprint: "",
            source_metadata: Some(snapshot.source_metadata.as_str()),
            next_read_offset: i64_from_usize(snapshot.events.len(), &source_file.path)?,
            event_count: generation_event_count(&tx, source_file, stored.id, generation)?,
            metadata: &snapshot.metadata,
        },
    )?;
    resolve_session_meta_and_commit(tx, ingest_state, source_file)?;
    Ok(inserted_event_count)
}

fn known_source_file(source_file: &SourceFile, source_session_id: impl Into<String>) -> SourceFile {
    SourceFile {
        source_session_id: source_session_id.into(),
        source_session_id_kind: SourceSessionIdKind::Known,
        ..source_file.clone()
    }
}

fn resolve_source_file(
    conn: &Connection,
    source_file: &SourceFile,
    pass_size: i64,
    pass_size_bytes: u64,
    file_mtime: Option<i64>,
) -> Result<ResolvedSourceFile> {
    if source_file.source_session_id_kind == SourceSessionIdKind::Known {
        return Ok(ResolvedSourceFile {
            source_file: source_file.clone(),
            buffer: None,
        });
    }

    let stored_by_path = load_session_by_source_file_path(conn, source_file)?;
    if let Some(stored) = &stored_by_path
        && unchanged_by_mtime(stored, pass_size, file_mtime)
    {
        return Ok(resolved_known_source_file(
            source_file,
            stored.source_session_id.clone(),
            None,
        ));
    }

    let (source_session_id, buffer) = match source_file.source_session_id_kind {
        SourceSessionIdKind::ClaudeLocalAgentAudit => (
            claude_local_agent_source_session_id_from_file(&source_file.path)?,
            None,
        ),
        SourceSessionIdKind::CodexSessionMeta => {
            (codex_source_session_id_from_file(&source_file.path)?, None)
        }
        SourceSessionIdKind::GeminiChatJson => {
            let buffer = read_bounded(&source_file.path, pass_size_bytes)?;
            (
                gemini_source_session_id_from_buffer(&source_file.path, &buffer)?,
                Some(buffer),
            )
        }
        SourceSessionIdKind::FactorySessionStart => (
            factory_source_session_id_from_file(&source_file.path)?,
            None,
        ),
        SourceSessionIdKind::PiAgentSessionHeader => (
            pi_agent_source_session_id_from_file(&source_file.path)?,
            None,
        ),
        SourceSessionIdKind::Known => unreachable!("known source files return early"),
    };
    reuse_source_file_session_identity(
        conn,
        stored_by_path.as_ref(),
        source_file,
        &source_session_id,
    )?;

    Ok(resolved_known_source_file(
        source_file,
        source_session_id,
        buffer,
    ))
}

fn resolved_known_source_file(
    source_file: &SourceFile,
    source_session_id: String,
    buffer: Option<Vec<u8>>,
) -> ResolvedSourceFile {
    ResolvedSourceFile {
        source_file: known_source_file(source_file, source_session_id),
        buffer,
    }
}

#[derive(Debug)]
struct ImportResult {
    inserted_event_count: u64,
    next_read_offset: i64,
    metadata: ParsedMetadata,
}

fn import_committed_lines(
    tx: &Transaction<'_>,
    source_file: &SourceFile,
    session_id: i64,
    generation: i64,
    buffer: &[u8],
    start_offset: usize,
    committed_len: usize,
) -> Result<ImportResult> {
    let mut metadata = ParsedMetadata::default();
    let mut inserted_event_count = 0;
    let mut next_read_offset = start_offset as i64;
    let mut byte_offset = start_offset;
    let mut seq = line_number_at(buffer, start_offset);
    let mut event_insert = tx
        .prepare(INSERT_EVENT_SQL)
        .map_err(|source| sqlite_error(&source_file.path, source))?;

    while byte_offset < committed_len {
        let line_end = next_line_end(buffer, byte_offset, committed_len);
        let line = &buffer[byte_offset..line_end];

        match serde_json::from_slice::<EventHeader<'_>>(line) {
            Ok(header) => {
                capture_metadata(&mut metadata, source_file.source, &header);
                let ts = event_timestamp(source_file.source, &header);
                inserted_event_count += insert_event_payload(
                    &mut event_insert,
                    source_file,
                    session_id,
                    generation,
                    seq,
                    ts,
                    line,
                )?;
                next_read_offset = (line_end + 1) as i64;
            }
            Err(error) => {
                record_ingest_error(
                    tx,
                    IngestErrorRecord {
                        source_file,
                        session_id: Some(session_id),
                        generation: Some(generation),
                        byte_offset: Some(byte_offset as i64),
                        line_number: Some(seq + 1),
                        error_kind: INVALID_JSON_ERROR_KIND,
                        message: &error.to_string(),
                    },
                )?;
                break;
            }
        }

        byte_offset = line_end + 1;
        seq += 1;
    }

    Ok(ImportResult {
        inserted_event_count,
        next_read_offset,
        metadata,
    })
}

fn insert_event_payload(
    event_insert: &mut rusqlite::Statement<'_>,
    source_file: &SourceFile,
    session_id: i64,
    generation: i64,
    seq: i64,
    ts: Option<&str>,
    payload: &[u8],
) -> Result<u64> {
    let encoded = encode_event_payload(payload)?;
    let inserted = event_insert
        .execute(params![
            session_id,
            generation,
            seq,
            ts,
            encoded.payload,
            encoded.codec,
            i64_from_usize(encoded.payload_size, &source_file.path)?,
        ])
        .map_err(|source| sqlite_error(&source_file.path, source))?;
    Ok(inserted as u64)
}

fn import_gemini_chat_json(
    tx: &Transaction<'_>,
    source_file: &SourceFile,
    session_id: i64,
    generation: i64,
    buffer: &[u8],
) -> Result<ImportResult> {
    let chat = gemini_chat_from_buffer(&source_file.path, buffer)?;
    let mut metadata = ParsedMetadata::default();
    if let Some(project_hash) = chat.project_hash {
        metadata.cwd = Some(project_hash.to_string());
    }
    if let Some(start_time) = chat.start_time {
        capture_metadata_timestamp(&mut metadata, start_time);
    }
    if let Some(last_updated) = chat.last_updated {
        capture_metadata_timestamp(&mut metadata, last_updated);
    }

    let mut inserted_event_count = 0;
    let mut event_insert = tx
        .prepare(INSERT_EVENT_SQL)
        .map_err(|source| sqlite_error(&source_file.path, source))?;

    for (index, message) in chat.messages.iter().enumerate() {
        let header = serde_json::from_str::<GeminiMessageHeader<'_>>(message.get())
            .map_err(|source| invalid_session_meta(&source_file.path, source.to_string()))?;
        if let Some(ts) = header.timestamp {
            capture_metadata_timestamp(&mut metadata, ts);
        }

        let payload = message.get().as_bytes();
        inserted_event_count += insert_event_payload(
            &mut event_insert,
            source_file,
            session_id,
            generation,
            i64_from_usize(index, &source_file.path)?,
            header.timestamp,
            payload,
        )?;
    }

    Ok(ImportResult {
        inserted_event_count,
        next_read_offset: i64_from_usize(buffer.len(), &source_file.path)?,
        metadata,
    })
}

fn opencode_session_snapshot(source_file: &SourceFile) -> Result<SqliteSessionSnapshot> {
    let conn = opencode_source_connection(&source_file.path)?;
    let (mut events, metadata, source_metadata) = opencode_session_event(&conn, source_file)?;
    for table in [
        OpenCodeTable::Message,
        OpenCodeTable::Part,
        OpenCodeTable::SessionMessage,
    ] {
        events.extend(opencode_row_events(&conn, source_file, table)?);
    }
    events.sort_by(|left, right| {
        left.sort_time
            .cmp(&right.sort_time)
            .then_with(|| left.rank.cmp(&right.rank))
            .then_with(|| left.id.cmp(&right.id))
    });

    Ok(SqliteSessionSnapshot {
        events,
        metadata,
        source_metadata,
    })
}

fn opencode_source_connection(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| opencode_sqlite_error(path, source))
}

fn hermes_source_connection(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| hermes_sqlite_error(path, source))
}

fn opencode_session_event(
    conn: &Connection,
    source_file: &SourceFile,
) -> Result<(Vec<SourceEvent>, ParsedMetadata, String)> {
    let event = conn
        .query_row(
            "SELECT s.id, s.project_id, s.parent_id, s.slug, s.directory, s.title,
                    s.version, s.share_url, s.summary_additions, s.summary_deletions,
                    s.summary_files, s.summary_diffs, s.revert, s.permission,
                    s.time_created, s.time_updated, s.time_compacting, s.time_archived,
                    s.workspace_id,
                    strftime('%Y-%m-%dT%H:%M:%fZ', s.time_created / 1000.0, 'unixepoch'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', s.time_updated / 1000.0, 'unixepoch'),
                    p.id, p.worktree, p.vcs, p.name, p.icon_url, p.icon_color,
                    p.time_created, p.time_updated, p.time_initialized, p.sandboxes,
                    p.commands,
                    strftime('%Y-%m-%dT%H:%M:%fZ', p.time_created / 1000.0, 'unixepoch'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', p.time_updated / 1000.0, 'unixepoch'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', p.time_initialized / 1000.0, 'unixepoch')
             FROM session s
             LEFT JOIN project p ON p.id = s.project_id
             WHERE s.id = ?1",
            [source_file.source_session_id.as_str()],
            |row| {
                let session_id: String = row.get(0)?;
                let project_id: String = row.get(1)?;
                let parent_id: Option<String> = row.get(2)?;
                let slug: String = row.get(3)?;
                let directory: String = row.get(4)?;
                let title: String = row.get(5)?;
                let version: String = row.get(6)?;
                let share_url: Option<String> = row.get(7)?;
                let summary_additions: Option<i64> = row.get(8)?;
                let summary_deletions: Option<i64> = row.get(9)?;
                let summary_files: Option<i64> = row.get(10)?;
                let summary_diffs: Option<String> = row.get(11)?;
                let revert: Option<String> = row.get(12)?;
                let permission: Option<String> = row.get(13)?;
                let time_created: i64 = row.get(14)?;
                let time_updated: i64 = row.get(15)?;
                let time_compacting: Option<i64> = row.get(16)?;
                let time_archived: Option<i64> = row.get(17)?;
                let workspace_id: Option<String> = row.get(18)?;
                let created_ts: Option<String> = row.get(19)?;
                let updated_ts: Option<String> = row.get(20)?;
                let event_id = session_id.clone();
                let event_ts = created_ts.clone();
                let metadata = ParsedMetadata {
                    cwd: Some(directory.clone()),
                    started_at: created_ts.clone(),
                    ended_at: updated_ts.clone(),
                };
                let project_row = serde_json::json!({
                    "id": row.get::<_, Option<String>>(21)?,
                    "worktree": row.get::<_, Option<String>>(22)?,
                    "vcs": row.get::<_, Option<String>>(23)?,
                    "name": row.get::<_, Option<String>>(24)?,
                    "icon_url": row.get::<_, Option<String>>(25)?,
                    "icon_color": row.get::<_, Option<String>>(26)?,
                    "time_created": row.get::<_, Option<i64>>(27)?,
                    "time_updated": row.get::<_, Option<i64>>(28)?,
                    "time_initialized": row.get::<_, Option<i64>>(29)?,
                    "sandboxes": json_text_column(row.get::<_, Option<String>>(30)?),
                    "commands": json_text_column(row.get::<_, Option<String>>(31)?),
                    "time_created_iso": row.get::<_, Option<String>>(32)?,
                    "time_updated_iso": row.get::<_, Option<String>>(33)?,
                    "time_initialized_iso": row.get::<_, Option<String>>(34)?,
                });
                let session_row = serde_json::json!({
                    "id": session_id,
                    "project_id": project_id,
                    "parent_id": parent_id,
                    "slug": slug,
                    "directory": directory,
                    "title": title,
                    "version": version,
                    "share_url": share_url,
                    "summary_additions": summary_additions,
                    "summary_deletions": summary_deletions,
                    "summary_files": summary_files,
                    "summary_diffs": json_text_column(summary_diffs),
                    "revert": json_text_column(revert),
                    "permission": json_text_column(permission),
                    "time_created": time_created,
                    "time_updated": time_updated,
                    "time_compacting": time_compacting,
                    "time_archived": time_archived,
                    "workspace_id": workspace_id,
                    "time_created_iso": created_ts,
                    "time_updated_iso": updated_ts,
                });
                let source_metadata = serde_json::json!({
                    "session": session_row.clone(),
                    "project": project_row.clone(),
                })
                .to_string();
                let payload = serde_json::json!({
                    "type": "session",
                    "table": "session",
                    "row": session_row,
                    "project": project_row,
                });
                Ok((
                    SourceEvent {
                        sort_time: time_created,
                        rank: 0,
                        id: event_id,
                        ts: event_ts,
                        payload: serde_json::to_vec(&payload).expect("serialize OpenCode payload"),
                    },
                    metadata,
                    source_metadata,
                ))
            },
        )
        .optional()
        .map_err(|source| opencode_sqlite_error(&source_file.path, source))?
        .ok_or_else(|| {
            invalid_session_meta(
                &source_file.path,
                format!(
                    "OpenCode session {} is missing",
                    source_file.source_session_id
                ),
            )
        })?;

    Ok((vec![event.0], event.1, event.2))
}

fn opencode_row_events(
    conn: &Connection,
    source_file: &SourceFile,
    table: OpenCodeTable,
) -> Result<Vec<SourceEvent>> {
    let table_name = table.name();
    let mut statement = conn
        .prepare(table.sql())
        .map_err(|source| opencode_sqlite_error(&source_file.path, source))?;
    let rows = statement
        .query_map([source_file.source_session_id.as_str()], |row| {
            let id: String = row.get(0)?;
            let session_id: String = row.get(1)?;
            let message_id: Option<String> = row.get(2)?;
            let source_type: Option<String> = row.get(3)?;
            let time_created: i64 = row.get(4)?;
            let time_updated: i64 = row.get(5)?;
            let created_ts: Option<String> = row.get(6)?;
            let updated_ts: Option<String> = row.get(7)?;
            let data: String = row.get(8)?;
            let event_id = id.clone();
            let event_ts = created_ts.clone();
            let payload = serde_json::json!({
                "type": table_name,
                "table": table_name,
                "row": {
                    "id": id,
                    "session_id": session_id,
                    "message_id": message_id,
                    "source_type": source_type,
                    "time_created": time_created,
                    "time_updated": time_updated,
                    "time_created_iso": created_ts,
                    "time_updated_iso": updated_ts,
                    "data": json_text_column(Some(data)),
                },
            });
            Ok(SourceEvent {
                sort_time: time_created,
                rank: table.rank(),
                id: event_id,
                ts: event_ts,
                payload: serde_json::to_vec(&payload).expect("serialize OpenCode payload"),
            })
        })
        .map_err(|source| opencode_sqlite_error(&source_file.path, source))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| opencode_sqlite_error(&source_file.path, source))
}

fn hermes_session_snapshot(source_file: &SourceFile) -> Result<SqliteSessionSnapshot> {
    let conn = hermes_source_connection(&source_file.path)?;
    let (mut events, metadata, source_metadata) = hermes_session_event(&conn, source_file)?;
    events.extend(hermes_message_events(&conn, source_file)?);
    events.sort_by(|left, right| {
        left.sort_time
            .cmp(&right.sort_time)
            .then_with(|| left.rank.cmp(&right.rank))
            .then_with(|| left.id.cmp(&right.id))
    });

    Ok(SqliteSessionSnapshot {
        events,
        metadata,
        source_metadata,
    })
}

fn hermes_session_event(
    conn: &Connection,
    source_file: &SourceFile,
) -> Result<(Vec<SourceEvent>, ParsedMetadata, String)> {
    let event = conn
        .query_row(
            "SELECT id, source, user_id, model, model_config, system_prompt,
                    parent_session_id, started_at, ended_at, end_reason,
                    message_count, tool_call_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_write_tokens, reasoning_tokens,
                    billing_provider, billing_base_url, billing_mode,
                    estimated_cost_usd, actual_cost_usd, cost_status, cost_source,
                    pricing_version, title, api_call_count,
                    strftime('%Y-%m-%dT%H:%M:%fZ', started_at, 'unixepoch'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', ended_at, 'unixepoch'),
                    CAST(started_at * 1000000 AS INTEGER)
             FROM sessions
             WHERE id = ?1",
            [source_file.source_session_id.as_str()],
            |row| {
                let session_id: String = row.get(0)?;
                let source_name: String = row.get(1)?;
                let started_ts: Option<String> = row.get(27)?;
                let ended_ts: Option<String> = row.get(28)?;
                let sort_time: i64 = row.get(29)?;
                let metadata = ParsedMetadata {
                    cwd: None,
                    started_at: started_ts.clone(),
                    ended_at: ended_ts.clone(),
                };
                let session_row = serde_json::json!({
                    "id": session_id,
                    "source": source_name,
                    "user_id": row.get::<_, Option<String>>(2)?,
                    "model": row.get::<_, Option<String>>(3)?,
                    "model_config": json_text_column(row.get::<_, Option<String>>(4)?),
                    "system_prompt": row.get::<_, Option<String>>(5)?,
                    "parent_session_id": row.get::<_, Option<String>>(6)?,
                    "started_at": row.get::<_, f64>(7)?,
                    "ended_at": row.get::<_, Option<f64>>(8)?,
                    "end_reason": row.get::<_, Option<String>>(9)?,
                    "message_count": row.get::<_, Option<i64>>(10)?,
                    "tool_call_count": row.get::<_, Option<i64>>(11)?,
                    "input_tokens": row.get::<_, Option<i64>>(12)?,
                    "output_tokens": row.get::<_, Option<i64>>(13)?,
                    "cache_read_tokens": row.get::<_, Option<i64>>(14)?,
                    "cache_write_tokens": row.get::<_, Option<i64>>(15)?,
                    "reasoning_tokens": row.get::<_, Option<i64>>(16)?,
                    "billing_provider": row.get::<_, Option<String>>(17)?,
                    "billing_base_url": row.get::<_, Option<String>>(18)?,
                    "billing_mode": row.get::<_, Option<String>>(19)?,
                    "estimated_cost_usd": row.get::<_, Option<f64>>(20)?,
                    "actual_cost_usd": row.get::<_, Option<f64>>(21)?,
                    "cost_status": row.get::<_, Option<String>>(22)?,
                    "cost_source": row.get::<_, Option<String>>(23)?,
                    "pricing_version": row.get::<_, Option<String>>(24)?,
                    "title": row.get::<_, Option<String>>(25)?,
                    "api_call_count": row.get::<_, Option<i64>>(26)?,
                    "started_at_iso": started_ts,
                    "ended_at_iso": ended_ts,
                });
                let source_metadata = serde_json::json!({
                    "session": {
                        "id": session_row["id"].clone(),
                        "source": session_row["source"].clone(),
                        "model": session_row["model"].clone(),
                        "parent_session_id": session_row["parent_session_id"].clone(),
                        "title": session_row["title"].clone(),
                        "message_count": session_row["message_count"].clone(),
                        "tool_call_count": session_row["tool_call_count"].clone(),
                        "started_at_iso": session_row["started_at_iso"].clone(),
                        "ended_at_iso": session_row["ended_at_iso"].clone(),
                    },
                })
                .to_string();
                let payload = serde_json::json!({
                    "type": "session",
                    "table": "sessions",
                    "row": session_row,
                });
                Ok((
                    SourceEvent {
                        sort_time,
                        rank: 0,
                        id: source_file.source_session_id.clone(),
                        ts: metadata.started_at.clone(),
                        payload: serde_json::to_vec(&payload).expect("serialize Hermes payload"),
                    },
                    metadata,
                    source_metadata,
                ))
            },
        )
        .optional()
        .map_err(|source| hermes_sqlite_error(&source_file.path, source))?
        .ok_or_else(|| {
            invalid_session_meta(
                &source_file.path,
                format!(
                    "Hermes session {} is missing",
                    source_file.source_session_id
                ),
            )
        })?;

    Ok((vec![event.0], event.1, event.2))
}

fn hermes_message_events(conn: &Connection, source_file: &SourceFile) -> Result<Vec<SourceEvent>> {
    let mut statement = conn
        .prepare(
            "SELECT id, session_id, role, content, tool_call_id, tool_calls,
                    tool_name, timestamp, token_count, finish_reason, reasoning,
                    reasoning_details, codex_reasoning_items, reasoning_content,
                    codex_message_items,
                    strftime('%Y-%m-%dT%H:%M:%fZ', timestamp, 'unixepoch'),
                    CAST(timestamp * 1000000 AS INTEGER)
             FROM messages
             WHERE session_id = ?1",
        )
        .map_err(|source| hermes_sqlite_error(&source_file.path, source))?;
    let rows = statement
        .query_map([source_file.source_session_id.as_str()], |row| {
            let id: i64 = row.get(0)?;
            let session_id: String = row.get(1)?;
            let role: String = row.get(2)?;
            let event_ts: Option<String> = row.get(15)?;
            let sort_time: i64 = row.get(16)?;
            let payload = serde_json::json!({
                "type": "message",
                "table": "messages",
                "row": {
                    "id": id,
                    "session_id": session_id,
                    "role": role,
                    "content": row.get::<_, Option<String>>(3)?,
                    "tool_call_id": row.get::<_, Option<String>>(4)?,
                    "tool_calls": json_text_column(row.get::<_, Option<String>>(5)?),
                    "tool_name": row.get::<_, Option<String>>(6)?,
                    "timestamp": row.get::<_, f64>(7)?,
                    "token_count": row.get::<_, Option<i64>>(8)?,
                    "finish_reason": row.get::<_, Option<String>>(9)?,
                    "reasoning": row.get::<_, Option<String>>(10)?,
                    "reasoning_details": json_text_column(row.get::<_, Option<String>>(11)?),
                    "codex_reasoning_items": json_text_column(row.get::<_, Option<String>>(12)?),
                    "reasoning_content": row.get::<_, Option<String>>(13)?,
                    "codex_message_items": json_text_column(row.get::<_, Option<String>>(14)?),
                    "timestamp_iso": event_ts.clone(),
                },
            });
            Ok(SourceEvent {
                sort_time,
                rank: 1,
                id: format!("{id:020}"),
                ts: event_ts,
                payload: serde_json::to_vec(&payload).expect("serialize Hermes payload"),
            })
        })
        .map_err(|source| hermes_sqlite_error(&source_file.path, source))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| hermes_sqlite_error(&source_file.path, source))
}

fn json_text_column(value: Option<String>) -> serde_json::Value {
    value.map_or(serde_json::Value::Null, |value| {
        serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
    })
}

fn source_events_fingerprint(events: &[SourceEvent]) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    for event in events {
        update_fingerprint(&mut hash, &event.payload);
        update_fingerprint(&mut hash, b"\n");
    }
    format!("{hash:016x}")
}

fn update_fingerprint(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn opencode_sqlite_error(path: &Path, source: rusqlite::Error) -> JottraceError {
    invalid_session_meta(
        path,
        format!("failed to read OpenCode SQLite store: {source}"),
    )
}

fn hermes_sqlite_error(path: &Path, source: rusqlite::Error) -> JottraceError {
    invalid_session_meta(
        path,
        format!("failed to read Hermes SQLite SessionDB: {source}"),
    )
}

fn record_source_file_ingest_error(
    conn: &mut Connection,
    source_file: &SourceFile,
    error: &JottraceError,
) -> Result<()> {
    let tx = begin_transaction(conn, &source_file.path)?;
    let stored_by_path = if matches!(
        source_file.source_format,
        SourceFormat::OpenCodeSqlite | SourceFormat::HermesSqlite
    ) {
        load_session(&tx, source_file)?
    } else {
        load_session_by_source_file_path(&tx, source_file)?
    };
    let (error_source_file, stored) = if let Some(stored) = stored_by_path {
        (
            known_source_file(source_file, stored.source_session_id.clone()),
            stored,
        )
    } else {
        let error_source_file = source_file_for_ingest_error(source_file);
        insert_session_if_missing(&tx, &error_source_file)?;
        let stored =
            load_session(&tx, &error_source_file)?.expect("session row should exist after insert");
        (error_source_file, stored)
    };
    let message = error.to_string();
    record_ingest_error(
        &tx,
        IngestErrorRecord {
            source_file: &error_source_file,
            session_id: Some(stored.id),
            generation: Some(stored.current_generation),
            byte_offset: None,
            line_number: None,
            error_kind: source_file_error_kind(error),
            message: &message,
        },
    )?;
    commit_ingest_transaction(tx, &error_source_file.path)
}

fn source_file_for_ingest_error(source_file: &SourceFile) -> SourceFile {
    if source_file.source_session_id_kind == SourceSessionIdKind::GeminiChatJson
        && let Some(source_session_id) = recover_gemini_source_session_id(source_file)
    {
        return known_source_file(source_file, source_session_id);
    }
    source_file.clone()
}

fn recover_gemini_source_session_id(source_file: &SourceFile) -> Option<String> {
    let metadata = fs::metadata(&source_file.path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let buffer = read_bounded(&source_file.path, metadata.len()).ok()?;
    gemini_source_session_id_from_buffer(&source_file.path, &buffer).ok()
}

fn is_source_file_ingest_error(error: &JottraceError) -> bool {
    matches!(
        error,
        JottraceError::Io { .. }
            | JottraceError::NotFile(_)
            | JottraceError::InvalidSessionMeta { .. }
    )
}

fn source_file_error_kind(error: &JottraceError) -> &'static str {
    match error {
        JottraceError::InvalidSessionMeta { .. } => INVALID_SESSION_META_ERROR_KIND,
        JottraceError::Io { .. } => READ_ERROR_KIND,
        JottraceError::NotFile(_) => NOT_FILE_ERROR_KIND,
        _ => "ingest_error",
    }
}

fn insert_session_if_missing(tx: &Transaction<'_>, source_file: &SourceFile) -> Result<()> {
    execute_sql(
        &source_file.path,
        tx,
        "INSERT OR IGNORE INTO sessions (source, source_session_id, file_path)
         VALUES (?1, ?2, ?3)",
        params![
            source_file.source,
            source_file.source_session_id.as_str(),
            source_file.path.to_string_lossy(),
        ],
    )?;
    Ok(())
}

fn load_session(conn: &Connection, source_file: &SourceFile) -> Result<Option<StoredSession>> {
    load_session_by_source_session_id(
        conn,
        &source_file.path,
        source_file.source,
        &source_file.source_session_id,
    )
}

fn load_session_by_source_session_id(
    conn: &Connection,
    error_path: &Path,
    source: &str,
    source_session_id: &str,
) -> Result<Option<StoredSession>> {
    query_optional(
        error_path,
        conn,
        "SELECT id, source_session_id, file_path, parent_session_id, current_generation, file_size, file_mtime,
                content_fingerprint, source_metadata, next_read_offset, event_count, prefix_fingerprint
         FROM sessions
         WHERE source = ?1 AND source_session_id = ?2",
        params![source, source_session_id],
        |row| {
            Ok(StoredSession {
                id: row.get(0)?,
                source_session_id: row.get(1)?,
                file_path: row.get(2)?,
                parent_session_id: row.get(3)?,
                current_generation: row.get(4)?,
                file_size: row.get(5)?,
                file_mtime: row.get(6)?,
                content_fingerprint: row.get(7)?,
                source_metadata: row.get(8)?,
                next_read_offset: row.get(9)?,
                event_count: row.get(10)?,
                prefix_fingerprint: row.get(11)?,
            })
        },
    )
}

fn load_session_by_source_file_path(
    conn: &Connection,
    source_file: &SourceFile,
) -> Result<Option<StoredSession>> {
    query_optional(
        &source_file.path,
        conn,
        "SELECT id, source_session_id, file_path, parent_session_id, current_generation, file_size, file_mtime,
                content_fingerprint, source_metadata, next_read_offset, event_count, prefix_fingerprint
         FROM sessions
         WHERE source = ?1 AND file_path = ?2",
        params![
            source_file.source,
            source_file.path.to_string_lossy().as_ref(),
        ],
        |row| {
            Ok(StoredSession {
                id: row.get(0)?,
                source_session_id: row.get(1)?,
                file_path: row.get(2)?,
                parent_session_id: row.get(3)?,
                current_generation: row.get(4)?,
                file_size: row.get(5)?,
                file_mtime: row.get(6)?,
                content_fingerprint: row.get(7)?,
                source_metadata: row.get(8)?,
                next_read_offset: row.get(9)?,
                event_count: row.get(10)?,
                prefix_fingerprint: row.get(11)?,
            })
        },
    )
}

fn stored_session_path_matches(stored: &StoredSession, source_file: &SourceFile) -> bool {
    stored.file_path.as_deref() == Some(source_file.path.to_string_lossy().as_ref())
}

fn stored_session_metadata_matches(stored: &StoredSession, source_metadata: Option<&str>) -> bool {
    stored.source_metadata.as_deref() == source_metadata
}

fn stored_jsonl_linked_session_matches(
    stored: &StoredSession,
    source_file: &SourceFile,
    parent_session_id: Option<i64>,
    session_source_metadata: Option<&str>,
) -> bool {
    stored_session_path_matches(stored, source_file)
        && stored.parent_session_id == parent_session_id
        && stored_session_metadata_matches(stored, session_source_metadata)
}

fn stored_sqlite_snapshot_matches(
    stored: &StoredSession,
    source_file: &SourceFile,
    pass_size: i64,
    file_mtime: Option<i64>,
    parent_session_id: Option<i64>,
    source_metadata: &str,
) -> bool {
    stored_session_path_matches(stored, source_file)
        && stored.file_size == Some(pass_size)
        && stored.file_mtime == file_mtime
        && stored.parent_session_id == parent_session_id
        && stored_session_metadata_matches(stored, Some(source_metadata))
}

fn unchanged_by_mtime(stored: &StoredSession, pass_size: i64, file_mtime: Option<i64>) -> bool {
    stored.file_size == Some(pass_size)
        && file_mtime.is_some()
        && stored.file_mtime == file_mtime
        && stored.content_fingerprint.is_some()
}

fn import_mode(
    source_file: &SourceFile,
    stored: &StoredSession,
    pass_size: i64,
    content_fingerprint: &str,
    prefix_intact: bool,
    retry_unresolved_invalid_json: bool,
) -> ImportMode {
    match stored.file_size {
        Some(file_size)
            if file_size == pass_size
                && stored.content_fingerprint.as_deref() == Some(content_fingerprint) =>
        {
            ImportMode::Skip
        }
        None => ImportMode::Append,
        // The consumed prefix changed, so the stored offset no longer points at a
        // record boundary. JSONL sources are not guaranteed to be append-only
        // (e.g. workflow journals get rewritten); re-read from the start instead
        // of resuming mid-record, which would log a spurious invalid_json error.
        Some(_) if source_file.source_format == SourceFormat::Jsonl && !prefix_intact => {
            ImportMode::Rewrite
        }
        Some(file_size)
            if source_file.source_format == SourceFormat::Jsonl
                && ((retry_unresolved_invalid_json && pass_size >= stored.next_read_offset)
                    || pass_size > file_size) =>
        {
            ImportMode::Append
        }
        Some(_) => ImportMode::Rewrite,
    }
}

/// Whether the bytes already consumed (`buffer[..next_read_offset]`) still match
/// what they were when the offset was recorded — i.e. the file was appended to,
/// not rewritten. Falls back to a structural boundary check for sessions written
/// before `prefix_fingerprint` was tracked.
fn append_prefix_intact(
    next_read_offset: i64,
    prefix_fingerprint: Option<&str>,
    buffer: &[u8],
) -> bool {
    let offset = next_read_offset.max(0) as usize;
    if offset == 0 {
        return true;
    }
    if offset > buffer.len() {
        return false;
    }
    match prefix_fingerprint {
        Some(expected) => fingerprint(&buffer[..offset]) == expected,
        // Legacy rows have no stored prefix fingerprint: a resume offset must at
        // least sit immediately after a newline, since that is the only position
        // `import_committed_lines` ever records.
        None => buffer[offset - 1] == b'\n',
    }
}

fn invalid_json_resolution_boundary(
    source_file: &SourceFile,
    buffer: &[u8],
    pass_size: i64,
) -> Result<i64> {
    if source_file.source_format != SourceFormat::Jsonl {
        return Ok(pass_size);
    }
    i64_from_usize(committed_len(buffer), &source_file.path)
}

fn begin_transaction<'a>(conn: &'a mut Connection, path: &Path) -> Result<Transaction<'a>> {
    conn.transaction()
        .map_err(|source| sqlite_error(path, source))
}

fn commit_ingest_transaction(tx: Transaction<'_>, path: &Path) -> Result<()> {
    tx.commit().map_err(|source| sqlite_error(path, source))
}

/// Bounds for resolving a stale `invalid_json` ingest error after a JSONL pass:
/// the read offset reached and the byte boundary up to which content is committed.
/// Absent for SQLite snapshots, which are atomic and cannot raise `invalid_json`.
struct JsonResolution {
    next_read_offset: i64,
    read_boundary: i64,
}

fn commit_skipped_session_refresh(
    tx: Transaction<'_>,
    ingest_state: &mut IngestState,
    update: &SkippedSessionUpdate<'_>,
    json_resolution: Option<JsonResolution>,
) -> Result<()> {
    update_skipped_session(&tx, update)?;
    if let Some(JsonResolution {
        next_read_offset,
        read_boundary,
    }) = json_resolution
    {
        resolve_success_and_commit(
            tx,
            ingest_state,
            update.source_file,
            next_read_offset,
            read_boundary,
        )
    } else {
        resolve_session_meta_and_commit(tx, ingest_state, update.source_file)
    }
}

fn resolve_success_and_commit(
    tx: Transaction<'_>,
    ingest_state: &mut IngestState,
    source_file: &SourceFile,
    next_read_offset: i64,
    read_boundary: i64,
) -> Result<()> {
    resolve_ingest_success_errors(
        &tx,
        ingest_state,
        source_file,
        next_read_offset,
        read_boundary,
    )?;
    commit_ingest_transaction(tx, &source_file.path)
}

fn resolve_session_meta_and_commit(
    tx: Transaction<'_>,
    ingest_state: &mut IngestState,
    source_file: &SourceFile,
) -> Result<()> {
    resolve_ingest_success_meta_error(&tx, ingest_state, source_file)?;
    commit_ingest_transaction(tx, &source_file.path)
}

fn update_skipped_session(tx: &Transaction<'_>, update: &SkippedSessionUpdate<'_>) -> Result<()> {
    let checked_file = update.checked_file.as_ref();
    let has_checked_file = checked_file.is_some();
    let file_mtime = checked_file.and_then(|state| state.file_mtime);
    let pass_size = checked_file.map(|state| state.pass_size);
    let content_fingerprint = checked_file.map(|state| state.content_fingerprint);
    let file_path = update.source_file.path.to_string_lossy();
    let has_parent_source = update.source_file.parent_source_session_id.is_some();

    execute_sql(
        &update.source_file.path,
        tx,
        "UPDATE sessions
         SET file_path = :file_path,
             parent_session_id = CASE
                 WHEN :has_parent_source THEN :parent_session_id
                 ELSE NULL
             END,
             source_metadata = :source_metadata,
             file_mtime = CASE WHEN :has_checked_file THEN :file_mtime ELSE file_mtime END,
             file_size = CASE WHEN :has_checked_file THEN :pass_size ELSE file_size END,
             content_fingerprint = CASE
                 WHEN :has_checked_file THEN :content_fingerprint
                 ELSE content_fingerprint
             END,
             last_read_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = :session_id",
        named_params! {
            ":file_path": file_path,
            ":has_parent_source": has_parent_source,
            ":parent_session_id": update.parent_session_id,
            ":source_metadata": update.source_metadata,
            ":has_checked_file": has_checked_file,
            ":file_mtime": file_mtime,
            ":pass_size": pass_size,
            ":content_fingerprint": content_fingerprint,
            ":session_id": update.session_id,
        },
    )?;
    Ok(())
}

fn update_session_after_import(tx: &Transaction<'_>, update: SessionUpdate<'_>) -> Result<()> {
    let file_path = update.source_file.path.to_string_lossy();
    let has_parent_source = update.source_file.parent_source_session_id.is_some();

    execute_sql(
        &update.source_file.path,
        tx,
        "UPDATE sessions
         SET file_path = :file_path,
             parent_session_id = CASE
                 WHEN :has_parent_source THEN :parent_session_id
                 ELSE NULL
             END,
             cwd = COALESCE(:cwd, cwd),
             started_at = CASE
                 WHEN :started_at IS NULL THEN started_at
                 WHEN started_at IS NULL THEN :started_at
                 WHEN :started_at < started_at THEN :started_at
                 ELSE started_at
             END,
             ended_at = CASE
                 WHEN :ended_at IS NULL THEN ended_at
                 WHEN ended_at IS NULL THEN :ended_at
                 WHEN :ended_at > ended_at THEN :ended_at
                 ELSE ended_at
             END,
             source_metadata = :source_metadata,
             current_generation = :current_generation,
             file_mtime = :file_mtime,
             file_size = :file_size,
             content_fingerprint = :content_fingerprint,
             prefix_fingerprint = :prefix_fingerprint,
             next_read_offset = :next_read_offset,
             event_count = :event_count,
             last_read_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = :session_id",
        named_params! {
            ":file_path": file_path,
            ":has_parent_source": has_parent_source,
            ":parent_session_id": update.parent_session_id,
            ":cwd": update.metadata.cwd.as_deref(),
            ":started_at": update.metadata.started_at.as_deref(),
            ":ended_at": update.metadata.ended_at.as_deref(),
            ":source_metadata": update.source_metadata,
            ":current_generation": update.generation,
            ":file_mtime": update.file_mtime,
            ":file_size": update.pass_size,
            ":content_fingerprint": update.content_fingerprint,
            ":prefix_fingerprint": update.prefix_fingerprint,
            ":next_read_offset": update.next_read_offset,
            ":event_count": update.event_count,
            ":session_id": update.session_id,
        },
    )?;
    Ok(())
}

fn reuse_source_file_session_identity(
    conn: &Connection,
    stored_by_path: Option<&StoredSession>,
    source_file: &SourceFile,
    source_session_id: &str,
) -> Result<()> {
    let Some(stored) = stored_by_path else {
        return Ok(());
    };
    if stored.source_session_id == source_session_id {
        return Ok(());
    }
    if !is_empty_source_file_placeholder(stored) {
        return Ok(());
    }
    if load_session_by_source_session_id(
        conn,
        &source_file.path,
        source_file.source,
        source_session_id,
    )?
    .is_some()
    {
        return Ok(());
    }

    execute_sql(
        &source_file.path,
        conn,
        "UPDATE sessions
         SET source_session_id = ?1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?2",
        params![source_session_id, stored.id],
    )?;
    Ok(())
}

fn resolve_parent_session_id(conn: &Connection, source_file: &SourceFile) -> Result<Option<i64>> {
    let Some(parent_source_session_id) = &source_file.parent_source_session_id else {
        return Ok(None);
    };

    load_session_by_source_session_id(
        conn,
        &source_file.path,
        source_file.source,
        parent_source_session_id,
    )
    .map(|session| session.map(|session| session.id))
}

fn is_empty_source_file_placeholder(stored: &StoredSession) -> bool {
    stored.current_generation == 0
        && stored.next_read_offset == 0
        && stored.event_count == 0
        && stored.content_fingerprint.is_none()
}

fn source_metadata_for_source_file(
    source_file: &SourceFile,
    stored_source_metadata: Option<&str>,
) -> Result<Option<String>> {
    if source_file.source != FACTORY_SOURCE {
        return Ok(None);
    }

    let settings_path = source_file.path.with_extension("settings.json");
    let metadata = match fs::metadata(&settings_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(io_error(&settings_path, source));
        }
    };
    if !metadata.is_file() {
        return Err(JottraceError::NotFile(settings_path));
    }

    let pass_size_bytes = metadata.len();
    let pass_size = i64_from_u64(pass_size_bytes, &settings_path)?;
    let file_mtime = file_mtime(&metadata);
    let settings_path_text = settings_path.to_string_lossy().into_owned();
    if let Some(stored_source_metadata) = stored_source_metadata
        && factory_settings_metadata_matches(
            stored_source_metadata,
            &settings_path_text,
            pass_size,
            file_mtime,
        )
    {
        return Ok(Some(stored_source_metadata.to_string()));
    }

    let buffer = read_bounded(&settings_path, pass_size_bytes)?;
    let (settings, settings_parse_error) =
        match serde_json::from_slice::<serde_json::Value>(&buffer) {
            Ok(settings) => (Some(settings), None),
            Err(error) => (None, Some(error.to_string())),
        };
    let source_metadata = serde_json::json!({
        "settings_path": settings_path_text,
        "settings_file_size": pass_size,
        "settings_file_mtime": file_mtime,
        "settings_content_fingerprint": fingerprint(&buffer),
        "settings": settings,
        "settings_parse_error": settings_parse_error,
    });
    Ok(Some(source_metadata.to_string()))
}

fn factory_settings_metadata_matches(
    stored_source_metadata: &str,
    settings_path: &str,
    settings_file_size: i64,
    settings_file_mtime: Option<i64>,
) -> bool {
    let Ok(stored) = serde_json::from_str::<serde_json::Value>(stored_source_metadata) else {
        return false;
    };
    stored.get("settings_path").and_then(|value| value.as_str()) == Some(settings_path)
        && stored
            .get("settings_file_size")
            .and_then(|value| value.as_i64())
            == Some(settings_file_size)
        && json_i64_option(stored.get("settings_file_mtime")) == Some(settings_file_mtime)
        && (stored.get("settings").is_some() || stored.get("settings_parse_error").is_some())
}

fn json_i64_option(value: Option<&serde_json::Value>) -> Option<Option<i64>> {
    match value {
        Some(serde_json::Value::Number(number)) => number.as_i64().map(Some),
        Some(serde_json::Value::Null) => Some(None),
        _ => None,
    }
}

fn unresolved_source_file_error_paths(
    db_path: &Path,
    conn: &Connection,
    error_kind: &str,
) -> Result<HashSet<(String, String)>> {
    let mut statement = conn
        .prepare(
            "SELECT source, file_path
             FROM ingest_errors
             WHERE error_kind = ?1
               AND resolved_at IS NULL",
        )
        .map_err(|source| sqlite_error(db_path, source))?;
    let rows = statement
        .query_map([error_kind], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|source| sqlite_error(db_path, source))?;
    let mut paths = HashSet::new();
    for row in rows {
        paths.insert(row.map_err(|source| sqlite_error(db_path, source))?);
    }
    Ok(paths)
}

fn resolve_ingest_success_errors(
    conn: &Connection,
    ingest_state: &mut IngestState,
    source_file: &SourceFile,
    next_read_offset: i64,
    pass_size: i64,
) -> Result<()> {
    resolve_ingest_success_meta_error(conn, ingest_state, source_file)?;
    resolve_invalid_json_error_if_fully_read(
        conn,
        ingest_state,
        source_file,
        next_read_offset,
        pass_size,
        SOURCE_FILE_INGESTED_SUCCESSFULLY_NOTE,
    )
}

fn resolve_ingest_success_meta_error(
    conn: &Connection,
    ingest_state: &mut IngestState,
    source_file: &SourceFile,
) -> Result<()> {
    resolve_invalid_session_meta_error(
        conn,
        ingest_state,
        source_file,
        SOURCE_FILE_INGESTED_SUCCESSFULLY_NOTE,
    )
}

fn resolve_invalid_session_meta_error(
    conn: &Connection,
    ingest_state: &mut IngestState,
    source_file: &SourceFile,
    resolution_note: &str,
) -> Result<()> {
    resolve_source_file_error(
        conn,
        &mut ingest_state.unresolved_invalid_session_meta_paths,
        source_file,
        INVALID_SESSION_META_ERROR_KIND,
        resolution_note,
    )
}

fn resolve_invalid_json_error_if_fully_read(
    conn: &Connection,
    ingest_state: &mut IngestState,
    source_file: &SourceFile,
    next_read_offset: i64,
    pass_size: i64,
    resolution_note: &str,
) -> Result<()> {
    if next_read_offset < pass_size || ingest_state.unresolved_invalid_json_paths.is_empty() {
        return Ok(());
    }
    resolve_source_file_error(
        conn,
        &mut ingest_state.unresolved_invalid_json_paths,
        source_file,
        INVALID_JSON_ERROR_KIND,
        resolution_note,
    )
}

fn resolve_source_file_error(
    conn: &Connection,
    unresolved_paths: &mut HashSet<(String, String)>,
    source_file: &SourceFile,
    error_kind: &str,
    resolution_note: &str,
) -> Result<()> {
    let path_key = source_file_error_path_key(source_file);
    if !unresolved_paths.remove(&path_key) {
        return Ok(());
    }

    execute_sql(
        &source_file.path,
        conn,
        "UPDATE ingest_errors
         SET resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             resolution_note = ?1
         WHERE source = ?2
           AND file_path = ?3
           AND error_kind = ?4
           AND resolved_at IS NULL",
        params![
            resolution_note,
            source_file.source,
            source_file.path.to_string_lossy(),
            error_kind,
        ],
    )?;
    Ok(())
}

fn source_file_error_path_key(source_file: &SourceFile) -> (String, String) {
    (
        source_file.source.to_string(),
        source_file.path.to_string_lossy().into_owned(),
    )
}

fn remove_empty_source_file_placeholder(
    conn: &Connection,
    stored_by_path: Option<&StoredSession>,
    source_file: &SourceFile,
) -> Result<()> {
    let Some(stored) = stored_by_path else {
        return Ok(());
    };
    if !is_empty_source_file_placeholder(stored) {
        return Ok(());
    }

    execute_sql(
        &source_file.path,
        conn,
        "DELETE FROM sessions WHERE id = ?1",
        [stored.id],
    )?;
    Ok(())
}

fn record_ingest_error(tx: &Transaction<'_>, record: IngestErrorRecord<'_>) -> Result<()> {
    let updated = execute_sql(
        &record.source_file.path,
        tx,
        "UPDATE ingest_errors
             SET session_id = ?1,
                 message = ?2,
                 last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 occurrence_count = occurrence_count + 1
             WHERE source = ?3
               AND source_session_id = ?4
               AND file_path = ?5
               AND generation IS ?6
               AND byte_offset IS ?7
               AND line_number IS ?8
               AND error_kind = ?9
               AND resolved_at IS NULL",
        params![
            record.session_id,
            record.message,
            record.source_file.source,
            record.source_file.source_session_id.as_str(),
            record.source_file.path.to_string_lossy(),
            record.generation,
            record.byte_offset,
            record.line_number,
            record.error_kind,
        ],
    )?;

    if updated > 0 {
        return Ok(());
    }

    execute_sql(
        &record.source_file.path,
        tx,
        "INSERT INTO ingest_errors
            (source, source_session_id, session_id, file_path, generation, byte_offset,
             line_number, error_kind, message)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            record.source_file.source,
            record.source_file.source_session_id.as_str(),
            record.session_id,
            record.source_file.path.to_string_lossy(),
            record.generation,
            record.byte_offset,
            record.line_number,
            record.error_kind,
            record.message,
        ],
    )?;
    Ok(())
}

fn generation_event_count(
    tx: &Transaction<'_>,
    source_file: &SourceFile,
    session_id: i64,
    generation: i64,
) -> Result<i64> {
    tx.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND generation = ?2",
        [session_id, generation],
        |row| row.get(0),
    )
    .map_err(|source| sqlite_error(&source_file.path, source))
}

fn read_bounded(path: &Path, pass_size: u64) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut reader = file.take(pass_size);
    let mut buffer = Vec::with_capacity(pass_size.min(1024 * 1024) as usize);
    reader
        .read_to_end(&mut buffer)
        .map_err(|source| io_error(path, source))?;
    Ok(buffer)
}

fn committed_len(buffer: &[u8]) -> usize {
    buffer
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1)
}

fn next_line_end(buffer: &[u8], start: usize, committed_len: usize) -> usize {
    start
        + buffer[start..committed_len]
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("committed range should contain a newline")
}

fn line_number_at(buffer: &[u8], offset: usize) -> i64 {
    buffer[..offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as i64
}

fn capture_metadata(metadata: &mut ParsedMetadata, source: &str, header: &EventHeader<'_>) {
    if metadata.cwd.is_none()
        && let Some(cwd) = header.cwd
    {
        metadata.cwd = Some(cwd.to_string());
    }
    if source == CODEX_SOURCE
        && metadata.cwd.is_none()
        && let Some(cwd) = header.payload.as_ref().and_then(|payload| payload.cwd)
    {
        metadata.cwd = Some(cwd.to_string());
    }

    if let Some(ts) = event_timestamp(source, header) {
        capture_metadata_timestamp(metadata, ts);
    }
}

fn capture_metadata_timestamp(metadata: &mut ParsedMetadata, ts: &str) {
    let replace_started = metadata
        .started_at
        .as_deref()
        .is_none_or(|current| ts < current);
    let replace_ended = metadata
        .ended_at
        .as_deref()
        .is_none_or(|current| ts > current);

    match (replace_started, replace_ended) {
        (true, true) => {
            let ts = ts.to_string();
            metadata.started_at = Some(ts.clone());
            metadata.ended_at = Some(ts);
        }
        (true, false) => metadata.started_at = Some(ts.to_string()),
        (false, true) => metadata.ended_at = Some(ts.to_string()),
        (false, false) => {}
    }
}

fn event_timestamp<'a>(source: &str, header: &'a EventHeader<'a>) -> Option<&'a str> {
    let ts = header.timestamp.or_else(|| {
        header
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.timestamp)
    });
    if ts.is_some() {
        return ts;
    }

    if source == CLAUDE_LOCAL_AGENT_SOURCE {
        return header.audit_timestamp;
    }

    if source == CODEX_SOURCE {
        return header
            .payload
            .as_ref()
            .and_then(|payload| payload.timestamp);
    }

    None
}

fn source_metadata(source_file: &SourceFile) -> Result<ParsedMetadata> {
    if source_file.source == CLAUDE_LOCAL_AGENT_SOURCE {
        return claude_local_agent_metadata(source_file.metadata_path.as_deref());
    }
    Ok(ParsedMetadata::default())
}

fn claude_local_agent_metadata(metadata_path: Option<&Path>) -> Result<ParsedMetadata> {
    let Some(path) = metadata_path else {
        return Ok(ParsedMetadata::default());
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ParsedMetadata::default());
        }
        Err(source) => {
            return Err(io_error(path, source));
        }
    };
    let metadata = parse_session_json::<ClaudeLocalAgentMetadata<'_>>(path, &bytes)?;
    let started_at = claude_local_agent_metadata_timestamp(path, "createdAt", metadata.started_at)?;
    let ended_at =
        claude_local_agent_metadata_timestamp(path, "lastActivityAt", metadata.ended_at)?;

    Ok(ParsedMetadata {
        cwd: metadata.cwd.map(str::to_string),
        started_at,
        ended_at,
    })
}

fn claude_local_agent_metadata_timestamp(
    path: &Path,
    field: &str,
    value: Option<serde_json::Value>,
) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value)),
        Some(serde_json::Value::Number(value)) => {
            let epoch_millis = json_timestamp_number_to_epoch_millis(&value).ok_or_else(|| {
                invalid_session_meta(
                    path,
                    format!("claude local-agent {field} timestamp is outside supported range"),
                )
            })?;
            epoch_millis_to_utc_iso(epoch_millis)
                .map(Some)
                .ok_or_else(|| {
                    invalid_session_meta(
                        path,
                        format!("claude local-agent {field} timestamp is outside supported range"),
                    )
                })
        }
        Some(_) => Err(invalid_session_meta(
            path,
            format!("claude local-agent {field} timestamp must be a string or number"),
        )),
    }
}

fn json_timestamp_number_to_epoch_millis(value: &serde_json::Number) -> Option<i128> {
    if let Some(value) = value.as_i64() {
        return integer_timestamp_to_epoch_millis(i128::from(value));
    }
    if let Some(value) = value.as_u64() {
        return integer_timestamp_to_epoch_millis(i128::from(value));
    }

    float_timestamp_to_epoch_millis(value.as_f64()?)
}

fn integer_timestamp_to_epoch_millis(value: i128) -> Option<i128> {
    let magnitude = if value < 0 {
        value.checked_neg()?
    } else {
        value
    };
    if magnitude >= 10_000_000_000 {
        Some(value)
    } else {
        value.checked_mul(1_000)
    }
}

fn float_timestamp_to_epoch_millis(value: f64) -> Option<i128> {
    if !value.is_finite() {
        return None;
    }
    let scaled = if value.abs() >= 10_000_000_000_f64 {
        value
    } else {
        value * 1_000.0
    };
    if !scaled.is_finite() || scaled < i128::MIN as f64 || scaled > i128::MAX as f64 {
        return None;
    }
    Some(scaled.round() as i128)
}

fn epoch_millis_to_utc_iso(epoch_millis: i128) -> Option<String> {
    let seconds = epoch_millis.div_euclid(1_000);
    let millisecond = epoch_millis.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days)?;
    if !(0_i128..=9999_i128).contains(&year) {
        return None;
    }

    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}Z"
    ))
}

fn civil_from_days(days_since_unix_epoch: i128) -> Option<(i128, i128, i128)> {
    let z = days_since_unix_epoch.checked_add(719_468)?;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    Some((year, month, day))
}

fn fill_missing(slot: &mut Option<String>, value: Option<String>) {
    if slot.is_none() {
        *slot = value;
    }
}

fn claude_local_agent_source_session_id_from_file(path: &Path) -> Result<String> {
    let Some(first_line) = first_committed_line(
        path,
        MAX_SESSION_HEADER_BYTES,
        "claude local-agent audit has no committed header line within the header limit",
    )?
    else {
        return claude_local_agent_fallback_source_session_id(path);
    };
    let header = parse_session_json::<ClaudeLocalAgentAuditHeader<'_>>(path, &first_line)?;

    header
        .session_id
        .filter(|session_id| !session_id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid_session_meta(path, "claude local-agent audit session_id is missing"))
}

fn codex_source_session_id_from_file(path: &Path) -> Result<String> {
    let first_line = first_committed_line(
        path,
        MAX_SESSION_HEADER_BYTES,
        "codex session file has no committed session_meta line within the header limit",
    )?
    .ok_or_else(|| {
        invalid_session_meta(
            path,
            "codex session file has no committed session_meta line within the header limit",
        )
    })?;

    let header = parse_session_json::<EventHeader<'_>>(path, &first_line)?;
    if header.event_type != Some("session_meta") {
        if header.event_type.is_none()
            && let Ok(legacy_header) = serde_json::from_slice::<LegacyCodexHeader<'_>>(&first_line)
            && let Some(id) = legacy_header.id
            && legacy_header.timestamp.is_some()
            && is_uuid_stem(id)
        {
            return Ok(id.to_string());
        }

        return Err(invalid_session_meta(
            path,
            "codex session file does not start with session_meta or legacy id header",
        ));
    }
    header
        .payload
        .and_then(|payload| payload.id.map(str::to_string))
        .ok_or_else(|| invalid_session_meta(path, "codex session_meta payload id is missing"))
}

fn factory_source_session_id_from_file(path: &Path) -> Result<String> {
    let first_line = first_committed_line(
        path,
        MAX_SESSION_HEADER_BYTES,
        FACTORY_SESSION_START_MISSING_MESSAGE,
    )?
    .ok_or_else(|| invalid_session_meta(path, FACTORY_SESSION_START_MISSING_MESSAGE))?;
    let header = parse_session_json::<EventHeader<'_>>(path, &first_line)?;
    if header.event_type != Some("session_start") {
        return Err(invalid_session_meta(
            path,
            "factory session file does not start with session_start",
        ));
    }
    header
        .id
        .map(str::to_string)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| invalid_session_meta(path, "factory session_start id is missing"))
}

fn first_committed_line(
    path: &Path,
    byte_limit: u64,
    missing_line_message: &str,
) -> Result<Option<Vec<u8>>> {
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut reader = BufReader::new(file).take(byte_limit);
    let mut first_line = Vec::new();
    let read = reader
        .read_until(b'\n', &mut first_line)
        .map_err(|source| io_error(path, source))?;
    if read == 0 || !first_line.ends_with(b"\n") {
        if read as u64 == byte_limit {
            return Ok(None);
        }
        return Err(invalid_session_meta(path, missing_line_message));
    }
    first_line.pop();
    Ok(Some(first_line))
}

fn validate_source_file_header(source_file: &SourceFile) -> Result<()> {
    if source_file.source == PI_AGENT_SOURCE {
        validate_pi_source_session_header(&source_file.path, &source_file.source_session_id)?;
    }
    Ok(())
}

fn validate_pi_source_session_header(path: &Path, source_session_id: &str) -> Result<()> {
    let id = pi_agent_source_session_id_from_file(path)?;
    if id != source_session_id {
        return Err(invalid_session_meta(
            path,
            "Pi agent session id does not match filename session id",
        ));
    }
    Ok(())
}

fn claude_local_agent_fallback_source_session_id(path: &Path) -> Result<String> {
    let dir_name = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid_path(path, "local-agent audit path is not valid UTF-8"))?;
    Ok(dir_name
        .strip_prefix("local_")
        .unwrap_or(dir_name)
        .to_string())
}

fn claude_local_agent_metadata_path(path: &Path) -> Option<PathBuf> {
    path.parent().map(|parent| parent.with_extension("json"))
}

fn gemini_source_session_id_from_buffer(path: &Path, buffer: &[u8]) -> Result<String> {
    let identity = parse_session_json::<GeminiChatIdentity<'_>>(path, buffer)?;
    if identity.session_id.trim().is_empty() {
        return Err(invalid_session_meta(
            path,
            "gemini chat sessionId is missing",
        ));
    }
    Ok(identity.session_id.to_string())
}

fn gemini_chat_from_buffer<'a>(path: &Path, buffer: &'a [u8]) -> Result<GeminiChatFile<'a>> {
    let chat = parse_session_json::<GeminiChatFile<'a>>(path, buffer)?;
    if chat.session_id.trim().is_empty() {
        return Err(invalid_session_meta(
            path,
            "gemini chat sessionId is missing",
        ));
    }
    Ok(chat)
}

fn source_session_ids_from_path(path: &Path) -> Result<(String, Option<String>)> {
    let file_stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .ok_or_else(|| invalid_path(path, "session file name is not valid UTF-8"))?;

    if let Some(parent_source_session_id) = parent_source_session_id_from_path(path, &file_stem) {
        let source_session_id = format!("{parent_source_session_id}/subagents/{file_stem}");
        return Ok((source_session_id, Some(parent_source_session_id)));
    }

    // Files nested deeper under a session's `subagents/` tree (e.g. the workflow
    // runner journals at `subagents/workflows/<wf-id>/journal.jsonl`) are not named
    // `agent-*`, so they are not caught above. Qualify them with their path relative
    // to the session dir; otherwise a bare stem like "journal" collides across every
    // workflow directory and the colliding sessions clobber each other on every run.
    if let Some((parent_source_session_id, source_session_id)) =
        nested_subagents_source_session_id(path, &file_stem)
    {
        return Ok((source_session_id, Some(parent_source_session_id)));
    }

    Ok((file_stem, None))
}

/// Build a unique source session id for a file living below a session's
/// `subagents/` directory but not directly inside it. Returns the owning session
/// id and a fully-qualified id of the form
/// `<session-uuid>/subagents/<...nested dirs...>/<file-stem>`.
fn nested_subagents_source_session_id(path: &Path, file_stem: &str) -> Option<(String, String)> {
    let components: Vec<&str> = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    let subagents_index = components
        .iter()
        .position(|component| *component == "subagents")?;
    // The owning session must be the UUID directory directly above `subagents`,
    // and there must be at least one directory between `subagents` and the file
    // (otherwise the `agent-*` branch already handled it).
    let parent_source_session_id = *components.get(subagents_index.checked_sub(1)?)?;
    if !is_uuid_stem(parent_source_session_id) {
        return None;
    }
    let nested = &components[subagents_index + 1..];
    if nested.len() < 2 {
        return None;
    }

    // Qualified id: <uuid>/subagents/<nested dirs>/<stem>. The final nested
    // component is the file name, which we drop in favour of the stem.
    let mut parts = vec![parent_source_session_id, "subagents"];
    parts.extend_from_slice(&nested[..nested.len() - 1]);
    parts.push(file_stem);
    Some((parent_source_session_id.to_string(), parts.join("/")))
}

fn pi_source_session_id_from_path(path: &Path) -> Result<String> {
    let file_stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| invalid_path(path, "session file name is not valid UTF-8"))?;
    let source_session_id = file_stem
        .rsplit_once('_')
        .map_or(file_stem, |(_, suffix)| suffix);
    if source_session_id.is_empty() {
        return Err(invalid_path(path, "Pi agent session id suffix is empty"));
    }
    Ok(source_session_id.to_string())
}

fn pi_agent_source_session_id_from_file(path: &Path) -> Result<String> {
    let first_line = first_committed_line(
        path,
        MAX_SESSION_HEADER_BYTES,
        PI_AGENT_SESSION_HEADER_MISSING_MESSAGE,
    )?
    .ok_or_else(|| invalid_session_meta(path, PI_AGENT_SESSION_HEADER_MISSING_MESSAGE))?;
    let header = parse_session_json::<EventHeader<'_>>(path, &first_line)?;
    if header.event_type != Some("session") {
        return Err(invalid_session_meta(
            path,
            "Pi agent session file does not start with a session event",
        ));
    }
    header
        .id
        .map(str::to_string)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| invalid_session_meta(path, "Pi agent session id is missing"))
}

struct PiAgentNestedRunInfo {
    parent_source_session_id: String,
    placeholder_source_session_id: String,
}

fn pi_agent_nested_run_info(path: &Path) -> Option<PiAgentNestedRunInfo> {
    if path.file_stem()?.to_str()? != "session" {
        return None;
    }
    let run_dir = path.parent()?;
    let run_name = run_dir.file_name()?.to_str()?;
    let run_index = run_name.strip_prefix("run-")?;
    if run_index.is_empty() || !run_index.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let short_dir = run_dir.parent()?;
    let short_name = short_dir.file_name()?.to_str()?;
    let parent_dir = short_dir.parent()?;
    let parent_dir_name = parent_dir.file_name()?.to_str()?;
    let (_, parent_uuid) = parent_dir_name.rsplit_once('_')?;
    if !is_uuid_stem(parent_uuid) {
        return None;
    }
    Some(PiAgentNestedRunInfo {
        parent_source_session_id: parent_uuid.to_string(),
        placeholder_source_session_id: format!("{parent_dir_name}/{short_name}/{run_name}"),
    })
}

fn parent_source_session_id_from_path(path: &Path, file_stem: &str) -> Option<String> {
    if !file_stem.starts_with("agent-") {
        return None;
    }

    let subagents_dir = path.parent()?;
    if subagents_dir.file_name()? != "subagents" {
        return None;
    }

    let parent_source_session_id = subagents_dir
        .parent()?
        .file_name()?
        .to_str()
        .map(str::to_string)?;

    is_uuid_stem(&parent_source_session_id).then_some(parent_source_session_id)
}

fn is_uuid_stem(value: &str) -> bool {
    value.len() == 36
        && value
            .char_indices()
            .all(|(index, ch)| matches!(index, 8 | 13 | 18 | 23) == (ch == '-'))
        && value
            .chars()
            .filter(|ch| *ch != '-')
            .all(|ch| ch.is_ascii_hexdigit())
}

fn file_mtime(metadata: &Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
}

fn fingerprint(buffer: &[u8]) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    update_fingerprint(&mut hash, buffer);
    format!("{hash:016x}")
}

fn i64_from_u64(value: u64, path: &Path) -> Result<i64> {
    value
        .try_into()
        .map_err(|_| invalid_path(path, "file is too large to index"))
}

fn i64_from_usize(value: usize, path: &Path) -> Result<i64> {
    value
        .try_into()
        .map_err(|_| invalid_path(path, "line is too large to index"))
}

fn invalid_path(path: &Path, message: &str) -> JottraceError {
    io_error(path, io::Error::new(io::ErrorKind::InvalidData, message))
}

fn invalid_session_meta(path: &Path, message: impl Into<String>) -> JottraceError {
    JottraceError::InvalidSessionMeta {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

fn parse_session_json<'a, T>(path: &Path, bytes: &'a [u8]) -> Result<T>
where
    T: Deserialize<'a>,
{
    serde_json::from_slice::<T>(bytes)
        .map_err(|source| invalid_session_meta(path, source.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID_A: &str = "95d258fd-2c38-4e97-97cb-8fff360d3de3";

    fn ids(path: &str) -> (String, Option<String>) {
        source_session_ids_from_path(Path::new(path)).expect("derive session ids")
    }

    #[test]
    fn workflow_journals_in_different_dirs_get_distinct_ids() {
        let (one, parent_one) = ids(&format!(
            "/home/u/.claude/projects/-proj/{UUID_A}/subagents/workflows/wf_ea989e2b-1d9/journal.jsonl"
        ));
        let (two, parent_two) = ids(&format!(
            "/home/u/.claude/projects/-proj/{UUID_A}/subagents/workflows/wf_ab3bff12-fa4/journal.jsonl"
        ));

        // Both belong to the same session but must not collide on a bare "journal".
        assert_ne!(one, two);
        assert_eq!(parent_one.as_deref(), Some(UUID_A));
        assert_eq!(parent_two.as_deref(), Some(UUID_A));
        assert_eq!(
            one,
            format!("{UUID_A}/subagents/workflows/wf_ea989e2b-1d9/journal")
        );
        assert_eq!(
            two,
            format!("{UUID_A}/subagents/workflows/wf_ab3bff12-fa4/journal")
        );
    }

    #[test]
    fn direct_subagent_files_keep_their_existing_id_shape() {
        let (id, parent) = ids(&format!(
            "/home/u/.claude/projects/-proj/{UUID_A}/subagents/agent-a000000000000021.jsonl"
        ));
        assert_eq!(parent.as_deref(), Some(UUID_A));
        assert_eq!(id, format!("{UUID_A}/subagents/agent-a000000000000021"));
    }

    #[test]
    fn top_level_session_files_keep_a_bare_stem_and_no_parent() {
        let (id, parent) = ids(&format!("/home/u/.claude/projects/-proj/{UUID_A}.jsonl"));
        assert_eq!(id, UUID_A);
        assert_eq!(parent, None);
    }

    #[test]
    fn prefix_is_trivially_intact_at_offset_zero() {
        // Nothing consumed yet, so there is no prefix to invalidate.
        assert!(append_prefix_intact(0, None, b""));
        assert!(append_prefix_intact(0, Some("does-not-matter"), b"line\n"));
    }

    #[test]
    fn prefix_is_stale_when_offset_exceeds_buffer() {
        // The file shrank below the recorded offset: it was rewritten, not appended.
        assert!(!append_prefix_intact(100, None, b"short\n"));
    }

    #[test]
    fn prefix_fingerprint_match_decides_when_present() {
        let buffer = b"first\nsecond\n";
        let consumed = fingerprint(&buffer[..6]); // "first\n"
        assert!(append_prefix_intact(6, Some(&consumed), buffer));
        assert!(!append_prefix_intact(6, Some("0000000000000000"), buffer));
    }

    #[test]
    fn legacy_rows_fall_back_to_a_newline_boundary_check() {
        let buffer = b"first\nsecond\n";
        // Offset 6 sits right after a newline (a real record boundary).
        assert!(append_prefix_intact(6, None, buffer));
        // Offset 3 lands mid-record, so the stored offset must be stale.
        assert!(!append_prefix_intact(3, None, buffer));
    }
}
