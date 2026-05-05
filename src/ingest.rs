use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use std::env;
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::storage::{DB_FILE_NAME, open_database, sqlite_error, status_from_connection};
use crate::{JottraceError, Result};

const CLAUDE_SOURCE: &str = "claude_cli";
const CLAUDE_INSTALL_DIRS: &[&str] = &[
    ".claude",
    ".claude-code",
    ".claude-local",
    ".claude-m2",
    ".claude-zai",
];
const RAW_CODEC: &str = "raw";
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
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct StoredSession {
    id: i64,
    file_path: Option<String>,
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
    session_id: i64,
    generation: i64,
    byte_offset: i64,
    line_number: i64,
    error_kind: &'a str,
    message: &'a str,
}

#[derive(Debug, Deserialize)]
struct EventHeader<'a> {
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

pub fn run_ingest() -> Result<IngestReport> {
    let data_dir = crate::data_dir_from_env()?;
    let _lock = crate::acquire_data_lock(&data_dir)?;
    let source_files = discover_claude_session_files()?;
    let db_path = data_dir.join(DB_FILE_NAME);
    let mut conn = open_database(&db_path)?;
    let mut inserted_event_count = 0;

    for source_file in &source_files {
        inserted_event_count += ingest_jsonl_file(&mut conn, source_file)?;
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

fn discover_claude_session_files() -> Result<Vec<SourceFile>> {
    let home = env::var_os("HOME").ok_or(JottraceError::MissingHome)?;
    let home = PathBuf::from(home);
    let mut paths = Vec::new();

    for install_dir in CLAUDE_INSTALL_DIRS {
        let root = home.join(install_dir);
        collect_jsonl_files(&root.join("projects"), true, &mut paths)?;
        collect_jsonl_files(&root, false, &mut paths)?;
    }

    paths.sort();
    paths.dedup();

    paths
        .into_iter()
        .map(|path| {
            Ok(SourceFile {
                source: CLAUDE_SOURCE,
                source_session_id: source_session_id_from_path(&path)?,
                path,
            })
        })
        .collect()
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

    if let Some(stored) = load_session(conn, source_file)?
        && unchanged_by_mtime(&stored, pass_size, file_mtime)
    {
        if stored.file_path.as_deref() == Some(source_file.path.to_string_lossy().as_ref()) {
            return Ok(0);
        }
        let tx = conn
            .transaction()
            .map_err(|source| sqlite_error(&source_file.path, source))?;
        update_skipped_session(&tx, source_file, stored.id)?;
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
    insert_session_if_missing(&tx, source_file)?;
    let stored = load_session(&tx, source_file)?.expect("session row should exist after insert");
    let import_mode = import_mode(&stored, pass_size, &content_fingerprint);

    if import_mode == ImportMode::Skip {
        update_skipped_session(&tx, source_file, stored.id)?;
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
        source_file,
        stored.id,
        generation,
        &buffer,
        start_offset,
        committed_len,
    )?;

    let event_count = generation_event_count(&tx, source_file, stored.id, generation)?;
    update_session_after_import(
        &tx,
        SessionUpdate {
            source_file,
            session_id: stored.id,
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
                capture_metadata(&mut metadata, &header);
                let ts = event_timestamp(&header);
                let inserted = event_insert
                    .execute(params![
                        session_id,
                        generation,
                        seq,
                        ts,
                        line,
                        RAW_CODEC,
                        i64_from_usize(line.len(), &source_file.path)?,
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
                        session_id,
                        generation,
                        byte_offset: byte_offset as i64,
                        line_number: seq,
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
    conn.query_row(
        "SELECT id, file_path, current_generation, file_size, file_mtime, content_fingerprint, next_read_offset
         FROM sessions
         WHERE source = ?1 AND source_session_id = ?2",
        params![source_file.source, source_file.source_session_id.as_str()],
        |row| {
            Ok(StoredSession {
                id: row.get(0)?,
                file_path: row.get(1)?,
                current_generation: row.get(2)?,
                file_size: row.get(3)?,
                file_mtime: row.get(4)?,
                content_fingerprint: row.get(5)?,
                next_read_offset: row.get(6)?,
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

fn update_skipped_session(
    tx: &Transaction<'_>,
    source_file: &SourceFile,
    session_id: i64,
) -> Result<()> {
    tx.execute(
        "UPDATE sessions
         SET file_path = ?1,
             last_read_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?2",
        params![source_file.path.to_string_lossy(), session_id],
    )
    .map_err(|source| sqlite_error(&source_file.path, source))?;
    Ok(())
}

fn update_session_after_import(tx: &Transaction<'_>, update: SessionUpdate<'_>) -> Result<()> {
    tx.execute(
        "UPDATE sessions
         SET file_path = ?1,
             cwd = COALESCE(?2, cwd),
             started_at = CASE
                 WHEN ?3 IS NULL THEN started_at
                 WHEN started_at IS NULL THEN ?3
                 WHEN ?3 < started_at THEN ?3
                 ELSE started_at
             END,
             ended_at = CASE
                 WHEN ?4 IS NULL THEN ended_at
                 WHEN ended_at IS NULL THEN ?4
                 WHEN ?4 > ended_at THEN ?4
                 ELSE ended_at
             END,
             current_generation = ?5,
             file_mtime = ?6,
             file_size = ?7,
             content_fingerprint = ?8,
             next_read_offset = ?9,
             event_count = ?10,
             last_read_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?11",
        params![
            update.source_file.path.to_string_lossy(),
            update.metadata.cwd.as_deref(),
            update.metadata.started_at.as_deref(),
            update.metadata.ended_at.as_deref(),
            update.generation,
            update.file_mtime,
            update.pass_size,
            update.content_fingerprint,
            update.next_read_offset,
            update.event_count,
            update.session_id,
        ],
    )
    .map_err(|source| sqlite_error(&update.source_file.path, source))?;
    Ok(())
}

fn record_ingest_error(tx: &Transaction<'_>, record: IngestErrorRecord<'_>) -> Result<()> {
    let updated = tx
        .execute(
            "UPDATE ingest_errors
             SET message = ?1,
                 last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 occurrence_count = occurrence_count + 1
             WHERE source = ?2
               AND source_session_id = ?3
               AND file_path = ?4
               AND generation = ?5
               AND byte_offset = ?6
               AND line_number = ?7
               AND error_kind = ?8
               AND resolved_at IS NULL",
            params![
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

fn capture_metadata(metadata: &mut ParsedMetadata, header: &EventHeader<'_>) {
    if metadata.cwd.is_none()
        && let Some(cwd) = header.cwd
    {
        metadata.cwd = Some(cwd.to_string());
    }

    if let Some(ts) = event_timestamp(header) {
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

fn event_timestamp<'a>(header: &'a EventHeader<'a>) -> Option<&'a str> {
    header.timestamp.or_else(|| {
        header
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.timestamp)
    })
}

fn source_session_id_from_path(path: &Path) -> Result<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .ok_or_else(|| invalid_path(path, "session file name is not valid UTF-8"))
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
