use rusqlite::{Connection, OptionalExtension, Transaction, named_params, params};
use serde::Deserialize;
use std::env;
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::storage::{
    DB_FILE_NAME, encode_event_payload, open_database, sqlite_error, status_from_connection,
};
use crate::{JottraceError, Result};

const CLAUDE_SOURCE: &str = "claude_cli";
const CODEX_SOURCE: &str = "codex_cli";
const CLAUDE_INSTALL_DIRS: &[&str] = &[
    ".claude",
    ".claude-code",
    ".claude-local",
    ".claude-m2",
    ".claude-zai",
];
const CODEX_INSTALL_DIRS: &[&str] = &[".codex", ".codex-local"];
const MAX_CODEX_SESSION_META_BYTES: u64 = 64 * 1024;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestReport {
    pub db_path: PathBuf,
    pub file_count: u64,
    pub session_count: u64,
    pub event_count: u64,
    pub inserted_event_count: u64,
    pub unresolved_ingest_error_count: u64,
}

#[derive(Debug, Clone)]
struct SourceFile {
    source: &'static str,
    source_session_id: String,
    source_session_id_kind: SourceSessionIdKind,
    parent_source_session_id: Option<String>,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceSessionIdKind {
    Known,
    CodexSessionMeta,
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
    next_read_offset: i64,
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

struct SessionUpdate<'a> {
    source_file: &'a SourceFile,
    session_id: i64,
    parent_session_id: Option<i64>,
    generation: i64,
    pass_size: i64,
    file_mtime: Option<i64>,
    content_fingerprint: &'a str,
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

struct SkippedSessionUpdate<'a> {
    source_file: &'a SourceFile,
    session_id: i64,
    parent_session_id: Option<i64>,
    checked_file: Option<CheckedFileState<'a>>,
}

struct CheckedFileState<'a> {
    pass_size: i64,
    file_mtime: Option<i64>,
    content_fingerprint: &'a str,
}

#[derive(Debug, Deserialize)]
struct EventHeader<'a> {
    #[serde(default, borrow, rename = "type")]
    event_type: Option<&'a str>,
    #[serde(default, borrow)]
    payload: Option<EventPayloadHeader<'a>>,
    #[serde(default, borrow)]
    cwd: Option<&'a str>,
    #[serde(default, borrow)]
    timestamp: Option<&'a str>,
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

pub fn run_ingest() -> Result<IngestReport> {
    let data_dir = crate::data_dir_from_env()?;
    let _lock = crate::acquire_data_lock(&data_dir)?;
    let source_files = discover_session_files()?;
    let db_path = data_dir.join(DB_FILE_NAME);
    let mut conn = open_database(&db_path)?;
    let mut inserted_event_count = 0;

    for source_file in &source_files {
        match ingest_jsonl_file(&mut conn, source_file) {
            Ok(inserted) => inserted_event_count += inserted,
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
        unresolved_ingest_error_count: status.unresolved_ingest_error_count,
    })
}

fn discover_session_files() -> Result<Vec<SourceFile>> {
    let mut source_files = discover_claude_session_files()?;
    source_files.extend(discover_codex_session_files()?);
    Ok(source_files)
}

fn discover_claude_session_files() -> Result<Vec<SourceFile>> {
    let home = home_dir()?;
    let mut paths = Vec::new();

    for install_dir in CLAUDE_INSTALL_DIRS {
        let root = home.join(install_dir);
        collect_jsonl_files(&root.join("projects"), true, &mut paths)?;
        collect_flat_session_files(&root, &mut paths)?;
    }

    paths.sort();
    paths.dedup();

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
                path,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    source_files.sort_by(|left, right| {
        left.parent_source_session_id
            .is_some()
            .cmp(&right.parent_source_session_id.is_some())
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(source_files)
}

fn discover_codex_session_files() -> Result<Vec<SourceFile>> {
    let home = home_dir()?;
    let mut paths = Vec::new();

    for install_dir in CODEX_INSTALL_DIRS {
        let root = home.join(install_dir);
        collect_jsonl_files(&root.join("sessions"), true, &mut paths)?;
        collect_jsonl_files(&root.join("archived_sessions"), false, &mut paths)?;
    }

    paths.sort();
    paths.dedup();

    paths
        .into_iter()
        .map(|path| {
            let (source_session_id, _) = source_session_ids_from_path(&path)?;
            Ok(SourceFile {
                source: CODEX_SOURCE,
                source_session_id,
                source_session_id_kind: SourceSessionIdKind::CodexSessionMeta,
                parent_source_session_id: None,
                path,
            })
        })
        .collect()
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(JottraceError::MissingHome)
}

fn collect_jsonl_files(root: &Path, recursive: bool, paths: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(JottraceError::Io {
                path: root.to_path_buf(),
                source,
            });
        }
    };

    for entry in entries {
        let entry = entry.map_err(|source| JottraceError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| JottraceError::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            if recursive {
                collect_jsonl_files(&path, true, paths)?;
            }
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
        {
            paths.push(path);
        }
    }

    Ok(())
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

fn ingest_jsonl_file(conn: &mut Connection, source_file: &SourceFile) -> Result<u64> {
    let metadata = fs::metadata(&source_file.path).map_err(|source| JottraceError::Io {
        path: source_file.path.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(JottraceError::NotFile(source_file.path.clone()));
    }

    let pass_size_bytes = metadata.len();
    let pass_size = i64_from_u64(pass_size_bytes, &source_file.path)?;
    let file_mtime = file_mtime(&metadata);

    let source_file = resolve_source_file(conn, source_file, pass_size, file_mtime)?;

    if let Some(stored) = load_session(conn, &source_file)?
        && unchanged_by_mtime(&stored, pass_size, file_mtime)
    {
        let path_is_current =
            stored.file_path.as_deref() == Some(source_file.path.to_string_lossy().as_ref());
        let parent_session_id = resolve_parent_session_id(conn, &source_file)?;
        if path_is_current && stored.parent_session_id == parent_session_id {
            return Ok(0);
        }

        let tx = conn
            .transaction()
            .map_err(|source| sqlite_error(&source_file.path, source))?;
        update_skipped_session(
            &tx,
            SkippedSessionUpdate {
                source_file: &source_file,
                session_id: stored.id,
                parent_session_id,
                checked_file: None,
            },
        )?;
        tx.commit()
            .map_err(|source| sqlite_error(&source_file.path, source))?;
        return Ok(0);
    }

    let buffer = read_bounded(&source_file.path, pass_size_bytes)?;
    let content_fingerprint = fingerprint(&buffer);
    let committed_len = committed_len(&buffer);

    let tx = conn
        .transaction()
        .map_err(|source| sqlite_error(&source_file.path, source))?;
    insert_session_if_missing(&tx, &source_file)?;
    let stored = load_session(&tx, &source_file)?.expect("session row should exist after insert");
    let import_mode = import_mode(&stored, pass_size, &content_fingerprint);

    if import_mode == ImportMode::Skip {
        let parent_session_id = resolve_parent_session_id(&tx, &source_file)?;
        update_skipped_session(
            &tx,
            SkippedSessionUpdate {
                source_file: &source_file,
                session_id: stored.id,
                parent_session_id,
                checked_file: Some(CheckedFileState {
                    pass_size,
                    file_mtime,
                    content_fingerprint: &content_fingerprint,
                }),
            },
        )?;
        tx.commit()
            .map_err(|source| sqlite_error(&source_file.path, source))?;
        return Ok(0);
    }

    let generation = match import_mode {
        ImportMode::Append | ImportMode::Skip => stored.current_generation,
        ImportMode::Rewrite => stored.current_generation + 1,
    };
    let start_offset = match import_mode {
        ImportMode::Append => stored.next_read_offset.max(0) as usize,
        ImportMode::Rewrite | ImportMode::Skip => 0,
    }
    .min(committed_len);

    let imported = import_committed_lines(
        &tx,
        &source_file,
        stored.id,
        generation,
        &buffer,
        start_offset,
        committed_len,
    )?;

    let event_count = generation_event_count(&tx, &source_file, stored.id, generation)?;
    let parent_session_id = resolve_parent_session_id(&tx, &source_file)?;
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
            next_read_offset: imported.next_read_offset,
            event_count,
            metadata: &imported.metadata,
        },
    )?;

    tx.commit()
        .map_err(|source| sqlite_error(&source_file.path, source))?;
    Ok(imported.inserted_event_count)
}

fn resolve_source_file(
    conn: &Connection,
    source_file: &SourceFile,
    pass_size: i64,
    file_mtime: Option<i64>,
) -> Result<SourceFile> {
    if source_file.source_session_id_kind == SourceSessionIdKind::Known {
        return Ok(source_file.clone());
    }

    if let Some(stored) = load_session_by_source_file_path(conn, source_file)?
        && unchanged_by_mtime(&stored, pass_size, file_mtime)
    {
        return Ok(SourceFile {
            source_session_id: stored.source_session_id,
            source_session_id_kind: SourceSessionIdKind::Known,
            ..source_file.clone()
        });
    }

    Ok(SourceFile {
        source_session_id: codex_source_session_id_from_file(&source_file.path)?,
        source_session_id_kind: SourceSessionIdKind::Known,
        ..source_file.clone()
    })
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
        .prepare(
            "INSERT OR IGNORE INTO events
                (session_id, generation, seq, ts, payload, codec, payload_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(|source| sqlite_error(&source_file.path, source))?;

    while byte_offset < committed_len {
        let line_end = next_line_end(buffer, byte_offset, committed_len);
        let line = &buffer[byte_offset..line_end];

        match serde_json::from_slice::<EventHeader<'_>>(line) {
            Ok(header) => {
                capture_metadata(&mut metadata, source_file.source, &header);
                let ts = event_timestamp(source_file.source, &header);
                let encoded = encode_event_payload(line)?;
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
                inserted_event_count += inserted as u64;
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
                        error_kind: "invalid_json",
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

fn record_source_file_ingest_error(
    conn: &mut Connection,
    source_file: &SourceFile,
    error: &JottraceError,
) -> Result<()> {
    let tx = conn
        .transaction()
        .map_err(|source| sqlite_error(&source_file.path, source))?;
    insert_session_if_missing(&tx, source_file)?;
    let stored = load_session(&tx, source_file)?.expect("session row should exist after insert");
    let message = error.to_string();
    record_ingest_error(
        &tx,
        IngestErrorRecord {
            source_file,
            session_id: Some(stored.id),
            generation: Some(stored.current_generation),
            byte_offset: None,
            line_number: None,
            error_kind: source_file_error_kind(error),
            message: &message,
        },
    )?;
    tx.commit()
        .map_err(|source| sqlite_error(&source_file.path, source))
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
        JottraceError::InvalidSessionMeta { .. } => "invalid_session_meta",
        JottraceError::Io { .. } => "read_error",
        JottraceError::NotFile(_) => "not_file",
        _ => "ingest_error",
    }
}

fn insert_session_if_missing(tx: &Transaction<'_>, source_file: &SourceFile) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO sessions (source, source_session_id, file_path)
         VALUES (?1, ?2, ?3)",
        params![
            source_file.source,
            source_file.source_session_id.as_str(),
            source_file.path.to_string_lossy(),
        ],
    )
    .map_err(|source| sqlite_error(&source_file.path, source))?;
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
    conn.query_row(
        "SELECT id, source_session_id, file_path, parent_session_id, current_generation, file_size, file_mtime,
                content_fingerprint, next_read_offset
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
                next_read_offset: row.get(8)?,
            })
        },
    )
    .optional()
    .map_err(|source| sqlite_error(error_path, source))
}

fn load_session_by_source_file_path(
    conn: &Connection,
    source_file: &SourceFile,
) -> Result<Option<StoredSession>> {
    conn.query_row(
        "SELECT id, source_session_id, file_path, parent_session_id, current_generation, file_size, file_mtime,
                content_fingerprint, next_read_offset
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
                next_read_offset: row.get(8)?,
            })
        },
    )
    .optional()
    .map_err(|source| sqlite_error(&source_file.path, source))
}

fn unchanged_by_mtime(stored: &StoredSession, pass_size: i64, file_mtime: Option<i64>) -> bool {
    stored.file_size == Some(pass_size)
        && file_mtime.is_some()
        && stored.file_mtime == file_mtime
        && stored.content_fingerprint.is_some()
}

fn import_mode(stored: &StoredSession, pass_size: i64, content_fingerprint: &str) -> ImportMode {
    match stored.file_size {
        None => ImportMode::Append,
        Some(file_size)
            if file_size == pass_size
                && stored.content_fingerprint.as_deref() == Some(content_fingerprint) =>
        {
            ImportMode::Skip
        }
        Some(file_size) if pass_size > file_size => ImportMode::Append,
        Some(_) => ImportMode::Rewrite,
    }
}

fn update_skipped_session(tx: &Transaction<'_>, update: SkippedSessionUpdate<'_>) -> Result<()> {
    let checked_file = update.checked_file.as_ref();
    let has_checked_file = checked_file.is_some();
    let file_mtime = checked_file.and_then(|state| state.file_mtime);
    let pass_size = checked_file.map(|state| state.pass_size);
    let content_fingerprint = checked_file.map(|state| state.content_fingerprint);
    let file_path = update.source_file.path.to_string_lossy();
    let has_parent_source = update.source_file.parent_source_session_id.is_some();

    tx.execute(
        "UPDATE sessions
         SET file_path = :file_path,
             parent_session_id = CASE
                 WHEN :has_parent_source THEN :parent_session_id
                 ELSE NULL
             END,
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
            ":has_checked_file": has_checked_file,
            ":file_mtime": file_mtime,
            ":pass_size": pass_size,
            ":content_fingerprint": content_fingerprint,
            ":session_id": update.session_id,
        },
    )
    .map_err(|source| sqlite_error(&update.source_file.path, source))?;
    Ok(())
}

fn update_session_after_import(tx: &Transaction<'_>, update: SessionUpdate<'_>) -> Result<()> {
    let file_path = update.source_file.path.to_string_lossy();
    let has_parent_source = update.source_file.parent_source_session_id.is_some();

    tx.execute(
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
             current_generation = :current_generation,
             file_mtime = :file_mtime,
             file_size = :file_size,
             content_fingerprint = :content_fingerprint,
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
            ":current_generation": update.generation,
            ":file_mtime": update.file_mtime,
            ":file_size": update.pass_size,
            ":content_fingerprint": update.content_fingerprint,
            ":next_read_offset": update.next_read_offset,
            ":event_count": update.event_count,
            ":session_id": update.session_id,
        },
    )
    .map_err(|source| sqlite_error(&update.source_file.path, source))?;
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

fn record_ingest_error(tx: &Transaction<'_>, record: IngestErrorRecord<'_>) -> Result<()> {
    let updated = tx
        .execute(
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
        )
        .map_err(|source| sqlite_error(&record.source_file.path, source))?;

    if updated > 0 {
        return Ok(());
    }

    tx.execute(
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
    )
    .map_err(|source| sqlite_error(&record.source_file.path, source))?;
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
    let file = File::open(path).map_err(|source| JottraceError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = file.take(pass_size);
    let mut buffer = Vec::with_capacity(pass_size.min(1024 * 1024) as usize);
    reader
        .read_to_end(&mut buffer)
        .map_err(|source| JottraceError::Io {
            path: path.to_path_buf(),
            source,
        })?;
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
        let ts = ts.to_string();
        if metadata
            .started_at
            .as_ref()
            .is_none_or(|current| ts < *current)
        {
            metadata.started_at = Some(ts.clone());
        }
        if metadata
            .ended_at
            .as_ref()
            .is_none_or(|current| ts > *current)
        {
            metadata.ended_at = Some(ts);
        }
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

    if source == CODEX_SOURCE {
        return header
            .payload
            .as_ref()
            .and_then(|payload| payload.timestamp);
    }

    None
}

fn codex_source_session_id_from_file(path: &Path) -> Result<String> {
    let file = File::open(path).map_err(|source| JottraceError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = BufReader::new(file).take(MAX_CODEX_SESSION_META_BYTES);
    let mut first_line = Vec::new();
    let read = reader
        .read_until(b'\n', &mut first_line)
        .map_err(|source| JottraceError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if read == 0 || !first_line.ends_with(b"\n") {
        return Err(invalid_session_meta(
            path,
            "codex session file has no committed session_meta line within the header limit",
        ));
    }
    first_line.pop();

    let header = serde_json::from_slice::<EventHeader<'_>>(&first_line)
        .map_err(|source| invalid_session_meta(path, source.to_string()))?;
    if header.event_type != Some("session_meta") {
        return Err(invalid_session_meta(
            path,
            "codex session file does not start with session_meta",
        ));
    }
    header
        .payload
        .and_then(|payload| payload.id.map(str::to_string))
        .ok_or_else(|| invalid_session_meta(path, "codex session_meta payload id is missing"))
}

fn source_session_ids_from_path(path: &Path) -> Result<(String, Option<String>)> {
    let file_stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .ok_or_else(|| invalid_path(path, "session file name is not valid UTF-8"))?;

    let parent_source_session_id = parent_source_session_id_from_path(path, &file_stem);
    let source_session_id = if let Some(parent_source_session_id) = &parent_source_session_id {
        format!("{parent_source_session_id}/subagents/{file_stem}")
    } else {
        file_stem
    };

    Ok((source_session_id, parent_source_session_id))
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
    for byte in buffer {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
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
    JottraceError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, message),
    }
}

fn invalid_session_meta(path: &Path, message: impl Into<String>) -> JottraceError {
    JottraceError::InvalidSessionMeta {
        path: path.to_path_buf(),
        message: message.into(),
    }
}
