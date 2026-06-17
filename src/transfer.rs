//! Move a Jottrace journal between machines via `pack` and `settle`.
//!
//! `pack` collects `~/.jottrace/` into a single tar.gz archive after folding
//! the SQLite WAL into the main database file. `settle` unpacks that archive
//! into a target journal directory, enforces private permissions, and runs
//! schema migrations so the receiving machine can start using the journal
//! immediately.
//!
//! Both helpers shell out to the system `tar` for the same reason
//! `src/update.rs` does: it avoids pulling a compression dependency and the
//! installer already requires `tar` to extract the release artifact.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::update::{AUTO_UPDATE_LOCK_FILE, AUTO_UPDATE_STAMP_FILE};
use crate::{
    JottraceError, LOCK_FILE_NAME, PRIVATE_DIR_MODE, PRIVATE_FILE_MODE, Result, acquire_data_lock,
    data_dir_from_env, ensure_private_dir, io_error, storage,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// Subdirectory of the journal used to assemble a candidate settle. Restricting
/// extraction to this subtree keeps a malicious or truncated archive from
/// damaging the live journal: if anything fails the directory is removed
/// wholesale by `StagingGuard`.
const STAGING_DIR_NAME: &str = ".pending-settle";

/// Top-level names that are runtime artefacts of the live journal, never
/// user data. The non-empty guard ignores these so a journal directory that
/// only holds runtime sentinels (e.g. an auto-update stamp planted before
/// any ingest had run) still counts as restorable; the archive preflight
/// also refuses them so a crafted archive cannot smuggle one in and either
/// no-op-overwrite a sentinel or, worse, replace the inode the held data
/// lock is flocking.
const RUNTIME_TOP_LEVEL_ARTEFACTS: &[&str] = &[
    LOCK_FILE_NAME,
    AUTO_UPDATE_STAMP_FILE,
    AUTO_UPDATE_LOCK_FILE,
];

/// Returns true for top-level archive entry names that must not appear in a
/// settle archive — runtime artefacts above plus the staging directory the
/// settle flow itself maintains. `pack` excludes all of these.
fn is_reserved_archive_top_level(name: &OsStr) -> bool {
    if name == OsStr::new(STAGING_DIR_NAME) {
        return true;
    }
    RUNTIME_TOP_LEVEL_ARTEFACTS
        .iter()
        .any(|reserved| name == OsStr::new(reserved))
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackOptions {
    /// Output archive path. `None` chooses a deterministic timestamped path in
    /// the current working directory so repeated `pack` runs do not clobber.
    pub output: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackReport {
    pub data_dir: PathBuf,
    pub archive: PathBuf,
    pub archive_bytes: u64,
    pub schema_version: i64,
    pub session_count: u64,
    pub event_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettleOptions {
    pub archive: PathBuf,
    /// Allow overwriting an existing non-empty journal. The caller is expected
    /// to surface this as `--force` so the destructive intent stays visible.
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleReport {
    pub data_dir: PathBuf,
    pub schema_version: i64,
    pub session_count: u64,
    pub event_count: u64,
}

pub fn run_pack(options: PackOptions) -> Result<PackReport> {
    let data_dir = data_dir_from_env()?;
    // Refuse to create a journal directory implicitly: `pack` only makes sense
    // for an existing journal, and silently inventing one would hide typos in
    // `JOTTRACE_HOME`.
    match fs::metadata(&data_dir) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(JottraceError::NotDirectory(data_dir)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(JottraceError::NotDirectory(data_dir));
        }
        Err(source) => {
            return Err(io_error(&data_dir, source));
        }
    }

    let archive = options
        .output
        .unwrap_or_else(|| PathBuf::from(default_archive_name(SystemTime::now())));

    // Reject outputs that would land inside the journal we are about to tar.
    // Otherwise tar's `-C data_dir .` walk would race the SQLite sidecars
    // (`db.sqlite-wal` is a particularly cute name to collide with), include
    // the partially-written archive in its own output, and ship a truncated
    // file to the user. The default unsuffixed name in `~/.jottrace` falls
    // into the same trap if `pack` is run from inside `JOTTRACE_HOME`.
    if pack_output_inside_journal(&archive, &data_dir)? {
        return Err(JottraceError::PackOutputInsideJournal(archive));
    }

    // Claim the output path atomically with mode 0600 BEFORE running tar.
    // Two reasons: (1) `create_new` races a separate exists-check, closing the
    // TOCTOU window where another process could create the file between check
    // and write; (2) the file is never world-visible under a loose umask,
    // because the kernel applies mode 0600 at create time and tar's later
    // `O_WRONLY|O_TRUNC` reuses the inode without resetting the mode.
    let claim_file = claim_private_path(&archive)?;
    drop(claim_file);
    // From this point on, any early return must delete the empty claimed file
    // so a retry can succeed without the user manually removing a 0-byte
    // archive. `ArchiveClaim` enforces that on the Drop path.
    let mut claim = ArchiveClaim::new(&archive);

    // Hold the same lock as `ingest`/`compact` so a concurrent writer cannot
    // commit between the checkpoint and the tarball write.
    let _lock = acquire_data_lock(&data_dir)?;

    // Refuse to write an archive that `settle` would later reject. A journal
    // directory without `db.sqlite` (e.g. one auto-created by the updater
    // before any ingest has run) would otherwise pack happily, advertise
    // success with zero counts, and only fail on the receiving end.
    let db_path = data_dir.join(storage::DB_FILE_NAME);
    if !db_path.is_file() {
        return Err(JottraceError::PackNoDatabase(data_dir));
    }
    let conn = storage::open_database(&db_path)?;
    checkpoint_truncate(&db_path, &conn)?;
    let status = storage::status_from_connection(&db_path, &conn)?;
    let (schema_version, session_count, event_count) = (
        status.schema_version,
        status.session_count,
        status.event_count,
    );

    // tar with `-C <dir> .` archives directory contents without a leading
    // path component, so `settle` can drop them into any target directory.
    // `.pending-settle` is excluded explicitly: a previous settle that died
    // mid-flight could leave one behind, and packing it would produce an
    // archive whose own settle creates a `.pending-settle/.pending-settle`
    // and breaks the staged-rename step.
    let mut command = Command::new("tar");
    command
        .arg("-czf")
        .arg(&archive)
        .arg(format!("--exclude={LOCK_FILE_NAME}"))
        .arg(format!("--exclude={AUTO_UPDATE_STAMP_FILE}"))
        .arg(format!("--exclude={AUTO_UPDATE_LOCK_FILE}"))
        .arg(format!("--exclude={STAGING_DIR_NAME}"))
        .arg("-C")
        .arg(&data_dir)
        .arg(".");
    run_tar(&mut command)?;

    let archive_bytes = fs::metadata(&archive)
        .map_err(|source| io_error(&archive, source))?
        .len();

    claim.commit();
    Ok(PackReport {
        data_dir,
        archive,
        archive_bytes,
        schema_version,
        session_count,
        event_count,
    })
}

pub fn run_settle(options: SettleOptions) -> Result<SettleReport> {
    let archive = options.archive;
    match fs::metadata(&archive) {
        Ok(metadata) if metadata.is_file() => {}
        _ => return Err(JottraceError::ArchiveNotFound(archive)),
    }

    let data_dir = data_dir_from_env()?;
    // Refuse archives that live inside the target journal. `--force` would
    // otherwise wipe the archive in `clear_journal_contents` before tar could
    // open it, turning a plausible "restore from local backup" workflow into a
    // silent data-loss footgun.
    if archive_is_inside(&archive, &data_dir)? {
        return Err(JottraceError::ArchiveInsideJournal(archive));
    }

    ensure_private_dir(&data_dir)?;
    let _lock = acquire_data_lock(&data_dir)?;

    // Re-check non-empty AFTER taking the exclusive data lock. A racing
    // `ingest` could otherwise have populated the directory between the
    // previous check and our `acquire_data_lock`, and we would clobber its
    // fresh journal without `--force`.
    if directory_has_journal_content(&data_dir)? && !options.force {
        return Err(JottraceError::SettleNotEmpty(data_dir));
    }

    // Inspect the archive BEFORE creating staging or invoking tar to extract.
    // Once tar starts writing a crafted archive — say a symlink entry followed
    // by a member whose path traverses that symlink — it can land bytes
    // outside the staging subtree before any post-extract walk has a chance to
    // refuse it. Reject the entire archive up front if any entry is a symlink,
    // hard link, special file, has an absolute path, or contains `..`.
    inspect_archive_safety(&archive)?;

    // Build the new journal inside a staging subdirectory FIRST. This keeps
    // every dangerous intermediate state — partial extracts, crafted symlinks,
    // tar errors halfway through — confined to a directory we can throw away
    // on failure, leaving the user's previous journal intact.
    let staging = data_dir.join(STAGING_DIR_NAME);
    // A leftover staging dir from a crashed prior settle was already counted
    // as journal content by the empty check above, so reaching this point
    // means either the dir was clean or the caller passed `--force`.
    if let Err(source) = fs::remove_dir_all(&staging)
        && source.kind() != io::ErrorKind::NotFound
    {
        return Err(io_error(&staging, source));
    }
    fs::create_dir(&staging).map_err(|source| io_error(&staging, source))?;
    chmod(&staging, PRIVATE_DIR_MODE)?;
    let mut staging_guard = StagingGuard::new(&staging);

    // Extract into staging, NEVER directly into `data_dir`. Even crafted
    // archives with symlink prefixes (e.g. `link -> /tmp/target` followed by
    // `link/file`) can only land inside the staging subtree where the
    // validation pass below catches them before they touch the live journal.
    // `--no-same-owner` keeps extracted files owned by the current user.
    // `--no-same-permissions` lets the umask apply so `enforce_private_permissions`
    // is the single source of truth for journal permissions.
    let mut command = Command::new("tar");
    command
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&staging)
        .arg("--no-same-owner")
        .arg("--no-same-permissions");
    run_tar(&mut command)?;

    // Validate + chmod every entry inside staging. Rejects symlinks and
    // non-regular files outright. If anything here errors, `staging_guard`
    // removes the partial extraction on drop and the live journal stays put.
    enforce_private_permissions(&staging)?;

    // Then confirm the archive actually carries a usable Jottrace database
    // BEFORE wiping the live journal. A valid tarball without `db.sqlite`, or
    // one carrying a corrupt or non-Jottrace `db.sqlite`, would otherwise
    // pass extraction, clear the previous journal, and let the post-promote
    // `status_for_path` silently create a fresh empty database in its place.
    validate_staged_jottrace_database(&staging, &archive)?;

    // Only now is the archive proven readable, complete, and safe. Clear the
    // previous journal contents (skipping our lock and our staging dir) so a
    // stale `db.sqlite-wal` or future sidecar cannot survive next to the
    // restored `db.sqlite`. With the default non-force path the directory was
    // already verified empty above and this loop is a no-op.
    clear_journal_contents(&data_dir)?;

    // Promote staged entries into the live journal. Each `rename` is atomic
    // within a single filesystem (guaranteed since staging is a child of the
    // journal), and the files keep the 0600 mode set during validation.
    move_staged_into_place(&staging, &data_dir)?;
    fs::remove_dir(&staging).map_err(|source| io_error(&staging, source))?;
    staging_guard.commit();

    let db_path = data_dir.join(storage::DB_FILE_NAME);
    let status = storage::status_for_path(&db_path)?;

    Ok(SettleReport {
        data_dir,
        schema_version: status.schema_version,
        session_count: status.session_count,
        event_count: status.event_count,
    })
}

fn checkpoint_truncate(path: &Path, conn: &Connection) -> Result<()> {
    // PRAGMA wal_checkpoint(TRUNCATE) folds the WAL back into the main DB file
    // and resets the WAL length to zero, so the archive carries a single
    // self-contained `db.sqlite`.
    conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")
        .map_err(|source| JottraceError::Sqlite {
            path: path.to_path_buf(),
            source,
        })
}

fn directory_has_journal_content(path: &Path) -> Result<bool> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(io_error(path, source));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| io_error(path, source))?;
        // Skip runtime artefacts (lock + auto-update sentinels). A directory
        // that only holds these is, from a user's perspective, an empty
        // journal — for example, on installer-managed binaries
        // `maybe_spawn_auto_update` can drop an `auto-update-check` stamp
        // before any ingest has run, and forcing `--force` for a fresh
        // settle in that situation would defeat the safety check.
        let name = entry.file_name();
        if RUNTIME_TOP_LEVEL_ARTEFACTS
            .iter()
            .any(|artefact| name == OsStr::new(artefact))
        {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

fn clear_journal_contents(path: &Path) -> Result<()> {
    let entries = fs::read_dir(path).map_err(|source| io_error(path, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_error(path, source))?;
        let name = entry.file_name();
        // Preserve the lock file (settle is holding it; removing it would
        // orphan the OS flock) and the staging dir (it holds the validated
        // candidate journal that the caller is about to promote into place).
        if name == OsStr::new(LOCK_FILE_NAME) || name == OsStr::new(STAGING_DIR_NAME) {
            continue;
        }
        let child = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| io_error(&child, source))?;
        // remove_dir_all and remove_file both refuse to traverse symlinks, so
        // a stray link in the existing journal can only ever delete itself —
        // never the target it points at.
        let result = if file_type.is_dir() {
            fs::remove_dir_all(&child)
        } else {
            fs::remove_file(&child)
        };
        result.map_err(|source| io_error(&child, source))?;
    }
    Ok(())
}

fn enforce_private_permissions(root: &Path) -> Result<()> {
    chmod(root, PRIVATE_DIR_MODE)?;
    let entries = fs::read_dir(root).map_err(|source| io_error(root, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_error(root, source))?;
        let path = entry.path();
        // Use `file_type` (which does NOT follow symlinks) instead of
        // `metadata`. A crafted archive could otherwise place a symlink
        // pointing outside `JOTTRACE_HOME`; chmod follows symlinks, so the
        // permission change would land on the target file.
        let file_type = entry
            .file_type()
            .map_err(|source| io_error(&path, source))?;
        if file_type.is_symlink() {
            return Err(JottraceError::UnsafeArchiveEntry {
                path,
                kind: "symbolic link",
            });
        }
        if file_type.is_dir() {
            enforce_private_permissions(&path)?;
        } else if file_type.is_file() {
            chmod(&path, PRIVATE_FILE_MODE)?;
        } else {
            return Err(JottraceError::UnsafeArchiveEntry {
                path,
                kind: "non-regular file",
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn chmod(path: &Path, mode: u32) -> Result<()> {
    let permissions = fs::Permissions::from_mode(mode);
    fs::set_permissions(path, permissions).map_err(|source| io_error(path, source))
}

#[cfg(not(unix))]
fn chmod(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// Probe statements run against the staged database to confirm it carries the
/// foundational Jottrace schema, not just tables with the right names. Each
/// query references columns specific to the migration-001 definitions, so
/// `prepare()` fails fast on a SQLite file that happens to expose tables
/// called `sessions`/`events`/`ingest_errors` but with the wrong columns —
/// the situation an attacker (or another application) could construct under
/// a Jottrace-shaped `PRAGMA user_version`.
const REQUIRED_JOURNAL_SCHEMA_PROBES: &[(&str, &str)] = &[
    (
        "sessions",
        "SELECT id, source, source_session_id, file_path, cwd, parent_session_id, \
         started_at, ended_at, current_generation, file_mtime, file_size, \
         content_fingerprint, next_read_offset, event_count, last_read_at, \
         created_at, updated_at, source_metadata FROM sessions WHERE 0",
    ),
    (
        "events",
        "SELECT session_id, generation, seq, ts, payload, codec, payload_size, \
         created_at FROM events WHERE 0",
    ),
    (
        "ingest_errors",
        "SELECT id, source, source_session_id, session_id, file_path, generation, \
         byte_offset, line_number, error_kind, message, first_seen_at, last_seen_at, \
         occurrence_count, resolved_at, resolution_note FROM ingest_errors WHERE 0",
    ),
];

/// Pre-flight scan of the archive's table-of-contents to reject crafted
/// inputs without ever invoking the extracting tar. The threat: tar applies
/// archive entries in stream order, so a symlink member followed by a regular
/// file under that symlink lets the second write follow the link and land
/// outside the staging subtree on platforms where tar does not refuse this
/// (notably GNU tar with default options). Validating the listing first means
/// no symlink ever gets created on disk in the first place.
fn inspect_archive_safety(archive: &Path) -> Result<()> {
    // `-tvzf` prints a header per entry whose first column is the file-type
    // character. `-tzf` prints only the path. Both listings enumerate entries
    // in the same order, and a path-only line is unambiguous even for names
    // containing spaces (which would break naive whitespace parsing of `-tvzf`).
    let mut types_cmd = Command::new("tar");
    types_cmd.arg("-tvzf").arg(archive);
    let types_output = run_tar(&mut types_cmd)?;
    let types_stdout = String::from_utf8_lossy(&types_output.stdout);

    let mut paths_cmd = Command::new("tar");
    paths_cmd.arg("-tzf").arg(archive);
    let paths_output = run_tar(&mut paths_cmd)?;
    let paths_stdout = String::from_utf8_lossy(&paths_output.stdout);

    let types: Vec<char> = types_stdout
        .lines()
        .filter_map(|line| line.trim_start().chars().next())
        .collect();
    let paths: Vec<&str> = paths_stdout.lines().collect();
    if types.len() != paths.len() {
        return Err(JottraceError::UnsafeArchiveEntry {
            path: archive.to_path_buf(),
            kind: "inconsistent tar listing",
        });
    }

    for (type_char, path) in types.iter().zip(paths.iter()) {
        let unsafe_kind: Option<&'static str> = match type_char {
            '-' | 'd' => None,
            'l' => Some("symbolic link"),
            'h' => Some("hard link"),
            'c' | 'b' => Some("device node"),
            'p' => Some("named pipe"),
            's' => Some("socket"),
            _ => Some("unknown archive entry"),
        };
        if let Some(kind) = unsafe_kind {
            return Err(JottraceError::UnsafeArchiveEntry {
                path: PathBuf::from(*path),
                kind,
            });
        }
        let path_obj = Path::new(*path);
        if path_obj.is_absolute() {
            return Err(JottraceError::UnsafeArchiveEntry {
                path: PathBuf::from(*path),
                kind: "absolute path",
            });
        }
        // Iterate components to catch two distinct problems with a single pass:
        // `..` segments (which would let a relative path escape staging once
        // tar applies them) and a top-level entry that collides with our own
        // staging directory name. The latter is reachable from an older or
        // crafted archive that ships a `.pending-settle/` entry; extracting it
        // would create `data_dir/.pending-settle/.pending-settle/`, and the
        // subsequent rename of `staging/.pending-settle` over `data_dir/.pending-settle`
        // (the live staging) would fail after `clear_journal_contents` had
        // already wiped the live journal.
        let mut saw_first_real_segment = false;
        for component in path_obj.components() {
            match component {
                std::path::Component::ParentDir => {
                    return Err(JottraceError::UnsafeArchiveEntry {
                        path: PathBuf::from(*path),
                        kind: "parent traversal",
                    });
                }
                std::path::Component::Normal(name) if !saw_first_real_segment => {
                    saw_first_real_segment = true;
                    if is_reserved_archive_top_level(name) {
                        return Err(JottraceError::UnsafeArchiveEntry {
                            path: PathBuf::from(*path),
                            kind: "reserved top-level entry",
                        });
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Build a `JottraceError::ArchiveDatabaseInvalid` for a staged archive that
/// failed validation. Mirrors `io_error`/`sqlite_error` so the repeated
/// `archive: archive.to_path_buf()` struct literal does not recur at every
/// validation site in this module.
fn archive_database_invalid(archive: &Path, reason: String) -> JottraceError {
    JottraceError::ArchiveDatabaseInvalid {
        archive: archive.to_path_buf(),
        reason,
    }
}

/// Open the staged `db.sqlite` and confirm it is actually a Jottrace journal
/// at the *latest* schema version before we wipe the live journal. The
/// `user_version` range check intentionally runs against a bare
/// `Connection::open`: `storage::open_database` would happily upgrade an
/// empty SQLite file (or any file with `user_version = 0`) all the way to
/// `LATEST_SCHEMA_VERSION`, which is exactly the silent data-loss case this
/// guard exists to prevent. Once the version range is verified we *do* run
/// the standard migration runner so the column-aware probe queries can
/// check the current schema, not just whatever the archive happened to
/// claim with its `user_version` value.
fn validate_staged_jottrace_database(staging: &Path, archive: &Path) -> Result<()> {
    let staged_db = staging.join(storage::DB_FILE_NAME);
    match fs::symlink_metadata(&staged_db) {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => {
            // `enforce_private_permissions` already rejects non-regular files,
            // so reaching this arm would be a logic bug. Fail loudly with the
            // archive path so the user is not left wondering.
            return Err(archive_database_invalid(
                archive,
                format!("{} is not a regular file", storage::DB_FILE_NAME),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(JottraceError::ArchiveMissingDatabase(archive.to_path_buf()));
        }
        Err(source) => {
            return Err(io_error(&staged_db, source));
        }
    }

    let user_version: i64 = {
        let conn = Connection::open(&staged_db)
            .map_err(|source| archive_database_invalid(archive, source.to_string()))?;
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|source| archive_database_invalid(archive, source.to_string()))?
    };

    if user_version <= 0 {
        return Err(archive_database_invalid(
            archive,
            format!(
                "{} has no Jottrace schema (user_version={user_version})",
                storage::DB_FILE_NAME
            ),
        ));
    }
    if user_version > storage::LATEST_SCHEMA_VERSION {
        return Err(JottraceError::UnsupportedSchemaVersion {
            path: archive.to_path_buf(),
            actual: user_version,
            supported: storage::LATEST_SCHEMA_VERSION,
        });
    }

    // Bring the staged file up to `LATEST_SCHEMA_VERSION` via the standard
    // migration runner. This is the same code path the receiving machine uses
    // on first open, so a malformed schema (or a non-Jottrace file claiming a
    // mid-range user_version) will fail HERE — before we touch the live
    // journal — with a SQLite error we surface as `ArchiveDatabaseInvalid`.
    let conn = storage::open_database(&staged_db).map_err(|source| match source {
        JottraceError::Sqlite {
            source: rusqlite_err,
            ..
        } => archive_database_invalid(archive, rusqlite_err.to_string()),
        JottraceError::UnsupportedSchemaVersion {
            actual, supported, ..
        } => JottraceError::UnsupportedSchemaVersion {
            path: archive.to_path_buf(),
            actual,
            supported,
        },
        other => other,
    })?;

    // Probe statements for the *current* (post-migration) Jottrace schema.
    // A SQLite file from an unrelated app that happens to expose tables with
    // the right names but unrelated columns — or a Jottrace journal at
    // `user_version = LATEST` that is missing a column added after migration
    // 001 — would otherwise wave through here and break the receiving
    // machine after the live journal had already been wiped.
    for (table, probe_sql) in REQUIRED_JOURNAL_SCHEMA_PROBES {
        conn.prepare(probe_sql).map_err(|source| {
            archive_database_invalid(
                archive,
                format!("table `{table}` has unexpected schema: {source}"),
            )
        })?;
    }

    // Columns alone do not guarantee a usable journal: ingest relies on the
    // unique index on `(source, source_session_id)` for its INSERT-OR-IGNORE
    // pattern, and on the PRIMARY KEY of `events` for ON-CONFLICT updates.
    // A crafted archive (or one produced by a partial migration) could carry
    // all the expected columns but skip these constraints, after which the
    // receiving machine would silently grow duplicate session rows on the
    // next ingest.
    require_unique_index_named(&conn, archive, "sessions", "idx_sessions_source_session_id")?;
    require_primary_key_unique_index(&conn, archive, "events")?;

    drop(conn);

    // SQLite may have written a `-shm` (or touched `-wal`) during the open.
    // Those files were not part of the validated tree `enforce_private_permissions`
    // walked, so re-apply 0600 explicitly before they ride the rename into the
    // live journal under the process umask.
    for suffix in ["-shm", "-wal", "-journal"] {
        let sidecar = staging.join(format!("{}{suffix}", storage::DB_FILE_NAME));
        match fs::symlink_metadata(&sidecar) {
            Ok(meta) if meta.file_type().is_file() => chmod(&sidecar, PRIVATE_FILE_MODE)?,
            Ok(_) => {
                return Err(JottraceError::UnsafeArchiveEntry {
                    path: sidecar,
                    kind: "non-regular file",
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(io_error(&sidecar, source));
            }
        }
    }

    Ok(())
}

/// Verify a named unique index exists on `table`. Without the unique index
/// `INSERT OR IGNORE INTO sessions(source, source_session_id, ...)` collapses
/// to a regular insert and the receiving machine grows duplicate session rows
/// on subsequent ingests.
fn require_unique_index_named(
    conn: &Connection,
    archive: &Path,
    table: &str,
    index_name: &str,
) -> Result<()> {
    use rusqlite::OptionalExtension;
    let unique: Option<i64> = conn
        .query_row(
            "SELECT \"unique\" FROM pragma_index_list(?1) WHERE name = ?2",
            rusqlite::params![table, index_name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| archive_database_invalid(archive, source.to_string()))?;
    match unique {
        Some(value) if value != 0 => Ok(()),
        Some(_) => Err(archive_database_invalid(
            archive,
            format!("index `{index_name}` on `{table}` is not unique"),
        )),
        None => Err(archive_database_invalid(
            archive,
            format!("missing unique index `{index_name}` on `{table}`"),
        )),
    }
}

/// Verify that `table` was declared with a PRIMARY KEY (which SQLite enforces
/// via an implicit unique index labelled `origin='pk'` in `pragma_index_list`).
/// For `WITHOUT ROWID` tables like `events`, the PRIMARY KEY *is* the
/// uniqueness invariant; compactions rely on ON CONFLICT against it.
fn require_primary_key_unique_index(conn: &Connection, archive: &Path, table: &str) -> Result<()> {
    use rusqlite::OptionalExtension;
    let unique: Option<i64> = conn
        .query_row(
            "SELECT \"unique\" FROM pragma_index_list(?1) WHERE origin = 'pk' LIMIT 1",
            [table],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| archive_database_invalid(archive, source.to_string()))?;
    match unique {
        Some(value) if value != 0 => Ok(()),
        Some(_) => Err(archive_database_invalid(
            archive,
            format!("PRIMARY KEY index on `{table}` is not unique"),
        )),
        None => Err(archive_database_invalid(
            archive,
            format!("missing PRIMARY KEY index on `{table}`"),
        )),
    }
}

/// Atomically claim a new path with mode 0600 set at create time. Unlike
/// `crate::create_private_file`, this does NOT require the parent directory to
/// be private — `pack` writes archives into normal user directories like the
/// current working directory, which are typically 0755.
fn claim_private_path(path: &Path) -> Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(PRIVATE_FILE_MODE);
    options.open(path).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            JottraceError::PackOutputExists(path.to_path_buf())
        } else {
            io_error(path, source)
        }
    })
}

/// RAII guard that deletes a claimed archive path on drop unless committed.
/// `pack` allocates the output file before doing any work that might fail
/// (lock contention, DB error, tar failure); without this guard those early
/// returns would leave a 0-byte archive that blocks the user's next retry.
struct ArchiveClaim {
    path: Option<PathBuf>,
}

impl ArchiveClaim {
    fn new(path: &Path) -> Self {
        Self {
            path: Some(path.to_path_buf()),
        }
    }

    fn commit(&mut self) {
        self.path = None;
    }
}

impl Drop for ArchiveClaim {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

/// RAII guard that removes a staging directory on drop unless committed.
/// `settle` extracts the candidate journal into staging before touching the
/// live tree; on any failure between `create_dir` and the final `commit`, the
/// partial extraction is wiped so the previous journal stays intact.
struct StagingGuard {
    path: Option<PathBuf>,
}

impl StagingGuard {
    fn new(path: &Path) -> Self {
        Self {
            path: Some(path.to_path_buf()),
        }
    }

    fn commit(&mut self) {
        self.path = None;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// Move every entry inside `staging` up to `dest` using `rename`. Renames
/// within a single filesystem are atomic per entry, and the staging dir is a
/// child of `dest` so that guarantee always holds. The caller is responsible
/// for ensuring `dest` already has space for the renamed entries (typically by
/// running `clear_journal_contents` first).
fn move_staged_into_place(staging: &Path, dest: &Path) -> Result<()> {
    let entries = fs::read_dir(staging).map_err(|source| io_error(staging, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_error(staging, source))?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        fs::rename(&from, &to).map_err(|source| io_error(&from, source))?;
    }
    Ok(())
}

/// Canonicalise `path`, mapping a missing path to `None` rather than an error.
/// The `*_inside_journal` checks treat a not-yet-existing path as "nothing to be
/// inside of", so they short-circuit to `Ok(false)` when this returns `None`.
fn canonicalize_optional(path: &Path) -> Result<Option<PathBuf>> {
    match fs::canonicalize(path) {
        Ok(canonical) => Ok(Some(canonical)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error(path, source)),
    }
}

/// Decide whether `pack`'s output path would land inside the source journal.
/// Unlike `archive_is_inside`, the candidate file does not exist yet, so we
/// canonicalise the parent directory (which must exist) and rejoin the file
/// name to obtain a stable resolved path before comparing.
fn pack_output_inside_journal(output: &Path, data_dir: &Path) -> Result<bool> {
    let Some(canonical_data) = canonicalize_optional(data_dir)? else {
        return Ok(false);
    };
    let parent = output.parent().filter(|p| !p.as_os_str().is_empty());
    let canonical_parent = match parent {
        Some(parent) => match canonicalize_optional(parent)? {
            Some(canonical) => canonical,
            // If the parent does not exist there is nothing to be inside of
            // yet; the subsequent `claim_private_path` will turn that into a
            // proper I/O error with the right path.
            None => return Ok(false),
        },
        None => fs::canonicalize(".").map_err(|source| io_error(Path::new("."), source))?,
    };
    let file_name = match output.file_name() {
        Some(name) => name,
        None => return Ok(false),
    };
    let resolved = canonical_parent.join(file_name);
    Ok(resolved.starts_with(&canonical_data))
}

/// Decide whether `archive` lives inside `data_dir`. Both paths are
/// canonicalised so a user-supplied symlink or `..` segment cannot bypass the
/// check. If `data_dir` does not exist yet the check is a no-op: there is
/// nothing to be inside.
fn archive_is_inside(archive: &Path, data_dir: &Path) -> Result<bool> {
    let canonical_archive =
        fs::canonicalize(archive).map_err(|source| io_error(archive, source))?;
    let Some(canonical_data) = canonicalize_optional(data_dir)? else {
        return Ok(false);
    };
    Ok(canonical_archive.starts_with(canonical_data))
}

fn run_tar(command: &mut Command) -> Result<Output> {
    let output = command.output().map_err(|source| JottraceError::ToolIo {
        program: "tar",
        source,
    })?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(JottraceError::ToolFailed {
            program: "tar",
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Build a default archive filename of the form `jottrace-pack-YYYYMMDD-HHMMSSZ.tar.gz`
/// (UTC). The format sorts naturally and uses only filesystem-safe characters.
fn default_archive_name(now: SystemTime) -> String {
    let seconds = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (year, month, day, hour, minute, second) = civil_from_epoch(seconds);
    format!("jottrace-pack-{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}Z.tar.gz")
}

/// Days-from-civil algorithm by Howard Hinnant (public domain): converts
/// epoch seconds to (Y, M, D, h, m, s) without leap-second handling. We only
/// use this for human-readable filenames, so leap-second skew is acceptable.
fn civil_from_epoch(seconds: u64) -> (u64, u32, u32, u32, u32, u32) {
    let days = seconds / 86_400;
    let time_of_day = seconds % 86_400;
    let hour = (time_of_day / 3_600) as u32;
    let minute = ((time_of_day % 3_600) / 60) as u32;
    let second = (time_of_day % 60) as u32;

    // Shift the reference so March 1 starts each "year" — this collapses the
    // leap-day exception to the very end of the cycle and keeps the math
    // closed-form.
    let z = days as i64 + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { (y + 1) as u64 } else { y as u64 };
    (year, m, d, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_archive_name_uses_utc_iso_compact() {
        // 2026-05-18T15:30:45Z = 1779118245 epoch seconds.
        let stamp = UNIX_EPOCH + std::time::Duration::from_secs(1_779_118_245);
        assert_eq!(
            default_archive_name(stamp),
            "jottrace-pack-20260518-153045Z.tar.gz"
        );
    }

    #[test]
    fn civil_from_epoch_handles_unix_epoch() {
        assert_eq!(civil_from_epoch(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn civil_from_epoch_handles_leap_day() {
        // 2024-02-29T00:00:00Z = 1709164800.
        assert_eq!(civil_from_epoch(1_709_164_800), (2024, 2, 29, 0, 0, 0));
    }
}
