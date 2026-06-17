use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::{
    fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    io::AsRawFd,
};

pub mod compact;
pub mod ingest;
pub mod storage;
pub mod taste;
pub mod transfer;
pub mod update;
pub mod web;
pub use compact::{CompactMode, CompactOptions, CompactReport, run_compact};
pub use ingest::{IngestReport, run_ingest};
pub use storage::{IngestErrorSummary, StatusReport, run_status};
pub use taste::{
    TasteExtractOptions, TasteExtractReport, TasteOutcomeCounts, TasteShowTimelineOptions,
    TasteStatusReport, TasteTimelineShowReport, run_taste_extract, run_taste_show_timeline,
    run_taste_status, show_timeline_for_data_dir, taste_status_for_data_dir,
};
pub use transfer::{PackOptions, PackReport, SettleOptions, SettleReport, run_pack, run_settle};
pub use update::{UpdateReport, run_update};

/// Default per-user data directory name for the current MVP.
pub const APP_DIR_NAME: &str = ".jottrace";
/// Single-instance guard for commands that mutate the local database.
pub const LOCK_FILE_NAME: &str = "jottrace.lock";
/// Session transcripts may contain private code, prompts, and paths, so the
/// default directory is readable only by the current user.
pub const PRIVATE_DIR_MODE: u32 = 0o700;
/// Files are kept even tighter than directories: readable and writable by the
/// current user, with no group/world access.
pub const PRIVATE_FILE_MODE: u32 = 0o600;
const DOCTOR_INGEST_ERROR_LIMIT: usize = 5;
static DATA_LOCK_PROCESS_PATHS: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug)]
pub enum JottraceError {
    /// Without a home directory or explicit override, there is no safe default
    /// place to put private journal state.
    MissingHome,
    Io {
        path: PathBuf,
        source: io::Error,
    },
    /// Refuse to reuse a path with the right name but the wrong kind; treating
    /// it as a directory would make later writes fail in surprising ways.
    NotDirectory(PathBuf),
    /// Refuse to reuse a filesystem node as a state file unless it is a regular
    /// file. SQLite needs a durable file path, not a directory or special file.
    NotFile(PathBuf),
    InvalidSessionMeta {
        path: PathBuf,
        message: String,
    },
    /// Existing loose permissions are surfaced instead of silently chmodded so
    /// the user can notice and decide whether the location is trustworthy.
    InsecureMode {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
    /// The local database was created by a newer Jottrace than this binary
    /// knows how to read safely.
    UnsupportedSchemaVersion {
        path: PathBuf,
        actual: i64,
        supported: i64,
    },
    UnsupportedEventPayloadCodec {
        codec: String,
    },
    EventPayloadCodec {
        codec: String,
        source: io::Error,
    },
    SessionNotFound {
        source: String,
        source_session_id: String,
    },
    /// `taste show timeline` found no materialized rows for the requested file.
    TimelineNotFound {
        source_session_id: String,
        file_path: String,
    },
    InvalidEventLimit {
        limit: i64,
    },
    InvalidCompactBatchSize {
        batch_size: usize,
        max: usize,
    },
    Output {
        source: io::Error,
    },
    /// A DB-mutating command is already active in this data directory.
    LockHeld(PathBuf),
    Sqlite {
        path: PathBuf,
        source: rusqlite::Error,
    },
    /// `pack` refuses to overwrite an existing output file so users do not lose
    /// a previous archive by accident.
    PackOutputExists(PathBuf),
    /// `settle` refuses to overwrite an existing non-empty journal unless the
    /// caller explicitly opts in with `--force`.
    SettleNotEmpty(PathBuf),
    /// `settle` cannot operate on a missing or non-file archive path.
    ArchiveNotFound(PathBuf),
    /// A spawned helper (e.g. `tar`) was not runnable on this system.
    ToolIo {
        program: &'static str,
        source: io::Error,
    },
    /// A spawned helper ran but exited non-zero; stderr is surfaced for triage.
    ToolFailed {
        program: &'static str,
        stderr: String,
    },
    /// `settle` refuses entries whose chmod or open would escape `JOTTRACE_HOME`
    /// (symlinks, hardlinks, fifos, sockets). A crafted archive is the only
    /// realistic source, so failing closed keeps the threat model honest.
    UnsafeArchiveEntry {
        path: PathBuf,
        kind: &'static str,
    },
    /// `settle` cannot read an archive that lives inside the target journal
    /// because `--force` would wipe the archive before tar could extract it.
    ArchiveInsideJournal(PathBuf),
    /// `settle` requires the archive to carry a `db.sqlite` entry. A valid
    /// tarball without it would otherwise wipe a live journal and silently
    /// replace it with the fresh empty database created by the post-promote
    /// status check.
    ArchiveMissingDatabase(PathBuf),
    /// `settle` could open the staged `db.sqlite` but it does not look like a
    /// Jottrace database (header magic failure, missing schema, etc.). The
    /// archive path is surfaced because the staging path is internal.
    ArchiveDatabaseInvalid {
        archive: PathBuf,
        reason: String,
    },
    /// `pack` refuses to produce an archive when the source journal has no
    /// `db.sqlite`. Such an archive would be advertised as a successful pack
    /// but would be rejected by `settle` as `ArchiveMissingDatabase`, leaving
    /// the user with an unusable file and no clear failure on the producer.
    PackNoDatabase(PathBuf),
    /// `pack` refuses to write its output inside the source journal. tar
    /// would otherwise see the archive in `-C data_dir .`, race the SQLite
    /// sidecars for names like `db.sqlite-wal`, and produce an archive whose
    /// content includes the (truncated/incomplete) archive itself.
    PackOutputInsideJournal(PathBuf),
}

impl fmt::Display for JottraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHome => write!(f, "HOME is not set and JOTTRACE_HOME was not provided"),
            Self::Io { path, source } => write!(f, "{}: {}", path.display(), source),
            Self::NotDirectory(path) => {
                write!(f, "{} exists but is not a directory", path.display())
            }
            Self::NotFile(path) => write!(f, "{} exists but is not a file", path.display()),
            Self::InvalidSessionMeta { path, message } => {
                write!(f, "{}: {}", path.display(), message)
            }
            Self::InsecureMode {
                path,
                expected,
                actual,
            } => write!(
                f,
                "{} has mode {:03o}; expected {:03o}",
                path.display(),
                actual,
                expected
            ),
            Self::UnsupportedSchemaVersion {
                path,
                actual,
                supported,
            } => write!(
                f,
                "{} has schema version {}; this binary supports up to {}",
                path.display(),
                actual,
                supported
            ),
            Self::UnsupportedEventPayloadCodec { codec } => {
                write!(f, "unsupported event payload codec: {codec}")
            }
            Self::EventPayloadCodec { codec, source } => {
                write!(f, "failed to process event payload codec {codec}: {source}")
            }
            Self::SessionNotFound {
                source,
                source_session_id,
            } => write!(
                f,
                "session not found: source={source} source_session_id={source_session_id}"
            ),
            Self::TimelineNotFound {
                source_session_id,
                file_path,
            } => write!(
                f,
                "timeline not found: session={source_session_id} file={file_path} (run `jottrace taste extract` first)"
            ),
            Self::InvalidEventLimit { limit } => {
                write!(f, "event limit must be at least 1; got {limit}")
            }
            Self::InvalidCompactBatchSize { batch_size, max } => write!(
                f,
                "compact batch size must be between 1 and {max}; got {batch_size}"
            ),
            Self::Output { source } => write!(f, "failed to write output: {source}"),
            Self::LockHeld(path) => write!(
                f,
                "another jottrace DB-mutating command is already running; lock: {}",
                path.display()
            ),
            Self::Sqlite { path, source } => write!(f, "{}: {}", path.display(), source),
            Self::PackOutputExists(path) => write!(
                f,
                "{} already exists; choose a different --output or remove the file",
                path.display()
            ),
            Self::SettleNotEmpty(path) => write!(
                f,
                "{} already contains journal data; rerun with --force to overwrite",
                path.display()
            ),
            Self::ArchiveNotFound(path) => {
                write!(f, "{} is not a readable archive file", path.display())
            }
            Self::ToolIo { program, source } => write!(f, "failed to run {program}: {source}"),
            Self::ToolFailed { program, stderr } => {
                let trimmed = stderr.trim();
                if trimmed.is_empty() {
                    write!(f, "{program} exited with a non-zero status")
                } else {
                    write!(f, "{program} failed: {trimmed}")
                }
            }
            Self::UnsafeArchiveEntry { path, kind } => write!(
                f,
                "archive contains unsafe entry ({kind}): {}",
                path.display()
            ),
            Self::ArchiveInsideJournal(path) => write!(
                f,
                "{} is inside the target journal; move it elsewhere and retry",
                path.display()
            ),
            Self::ArchiveMissingDatabase(path) => write!(
                f,
                "{} does not contain a {} entry; refusing to overwrite the live journal",
                path.display(),
                storage::DB_FILE_NAME
            ),
            Self::ArchiveDatabaseInvalid { archive, reason } => write!(
                f,
                "{} does not contain a usable Jottrace database: {reason}",
                archive.display()
            ),
            Self::PackNoDatabase(path) => write!(
                f,
                "{} has no {}; nothing to pack — run `jottrace ingest` first",
                path.display(),
                storage::DB_FILE_NAME
            ),
            Self::PackOutputInsideJournal(path) => write!(
                f,
                "{} is inside the source journal; choose a --output path outside JOTTRACE_HOME",
                path.display()
            ),
        }
    }
}

impl std::error::Error for JottraceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::EventPayloadCodec { source, .. } => Some(source),
            Self::Output { source } => Some(source),
            Self::Sqlite { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, JottraceError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub data_dir: PathBuf,
    pub unresolved_ingest_error_count: u64,
    pub recent_ingest_errors: Vec<IngestErrorSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoctorOptions {
    pub include_recent_errors: bool,
}

impl Default for DoctorOptions {
    fn default() -> Self {
        Self {
            include_recent_errors: true,
        }
    }
}

/// Resolve the data directory from the environment.
///
/// `JOTTRACE_HOME` comes first because tests and future integrations need a
/// deterministic sandbox that never touches the user's real journal.
pub fn data_dir_from_env() -> Result<PathBuf> {
    if let Some(path) = env::var_os("JOTTRACE_HOME") {
        return Ok(PathBuf::from(path));
    }

    let home = env::var_os("HOME").ok_or(JottraceError::MissingHome)?;
    Ok(PathBuf::from(home).join(APP_DIR_NAME))
}

/// Verify the local runtime can safely create and protect its private state.
pub fn run_doctor() -> Result<DoctorReport> {
    run_doctor_with_options(DoctorOptions::default())
}

pub fn run_doctor_with_options(options: DoctorOptions) -> Result<DoctorReport> {
    let data_dir = data_dir_from_env()?;
    ensure_private_dir(&data_dir)?;
    let db_path = data_dir.join(storage::DB_FILE_NAME);
    let conn = storage::open_database(&db_path)?;
    let unresolved_ingest_error_count =
        storage::unresolved_ingest_error_count_from_connection(&db_path, &conn)?;
    let ingest_errors = if options.include_recent_errors {
        storage::unresolved_ingest_errors_from_connection(
            &db_path,
            &conn,
            DOCTOR_INGEST_ERROR_LIMIT,
        )?
    } else {
        Vec::new()
    };
    Ok(DoctorReport {
        data_dir,
        unresolved_ingest_error_count,
        recent_ingest_errors: ingest_errors,
    })
}

// The in-process guard plus OS/file lock are authoritative. `jottrace.lock`
// stores diagnostic metadata for humans and is removed on clean shutdown for
// tidiness; its mere presence is not a Unix lock.
pub(crate) struct DataLock {
    // Field order matters: `_file` must drop before `_process_guard` so the OS
    // lock releases before another acquirer can pass the in-process check.
    _file: File,
    _process_guard: ProcessDataLock,
    path: PathBuf,
    token: String,
}

struct ProcessDataLock {
    path: PathBuf,
}

pub(crate) fn acquire_data_lock(data_dir: &Path) -> Result<DataLock> {
    ensure_private_dir(data_dir)?;
    let path = data_dir.join(LOCK_FILE_NAME);
    let process_guard = acquire_process_data_lock(&path)?;
    let token = lock_token();

    let file = acquire_data_lock_file(&path, &token)?;

    Ok(DataLock {
        _file: file,
        _process_guard: process_guard,
        path,
        token,
    })
}

fn acquire_process_data_lock(path: &Path) -> Result<ProcessDataLock> {
    // `flock` behavior for duplicate locks inside one process varies by
    // platform and kernel. Track the path in-process too, preserving the old
    // atomic `create_new` behavior for same-process callers.
    let mut paths = locked_process_paths();
    if !paths.insert(path.to_path_buf()) {
        return Err(JottraceError::LockHeld(path.to_path_buf()));
    }
    Ok(ProcessDataLock {
        path: path.to_path_buf(),
    })
}

fn locked_process_paths() -> std::sync::MutexGuard<'static, HashSet<PathBuf>> {
    DATA_LOCK_PROCESS_PATHS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

impl Drop for ProcessDataLock {
    fn drop(&mut self) {
        let mut paths = locked_process_paths();
        paths.remove(&self.path);
    }
}

#[cfg(unix)]
fn acquire_data_lock_file(path: &Path, token: &str) -> Result<File> {
    ensure_private_file(path)?;
    // The private data dir is 0700, so opening an existing file is within the
    // same-user trust model; the OS lock below decides ownership.
    let mut file = private_open_options()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|source| JottraceError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    acquire_os_file_lock(&file, path)?;
    write_data_lock_metadata(&mut file, path, token)?;
    Ok(file)
}

#[cfg(not(unix))]
fn acquire_data_lock_file(path: &Path, token: &str) -> Result<File> {
    let mut file = match create_private_file(path) {
        Ok(file) => file,
        Err(JottraceError::Io {
            path: error_path,
            source,
        }) if source.kind() == io::ErrorKind::AlreadyExists => {
            return Err(JottraceError::LockHeld(error_path));
        }
        Err(error) => return Err(error),
    };
    write_data_lock_metadata(&mut file, path, token)?;
    Ok(file)
}

fn write_data_lock_metadata(file: &mut File, path: &Path, token: &str) -> Result<()> {
    let metadata = format!("{token}\n");

    // Write first, then trim. That avoids the empty-file window that
    // truncate-then-write would create when replacing stale metadata.
    let result: io::Result<()> = (|| {
        file.seek(SeekFrom::Start(0))?;
        file.write_all(metadata.as_bytes())?;
        file.set_len(metadata.len() as u64)?;
        Ok(())
    })();

    result.map_err(|source| {
        let _ = fs::remove_file(path);
        JottraceError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(unix)]
fn acquire_os_file_lock(file: &File, path: &Path) -> Result<()> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(());
    }

    let source = io::Error::last_os_error();
    if source.kind() == io::ErrorKind::WouldBlock {
        return Err(JottraceError::LockHeld(path.to_path_buf()));
    }
    Err(JottraceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn remove_data_lock_file_if_owned(path: &Path, token: &str) {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return,
    };
    if contents.trim_end() == token {
        let _ = fs::remove_file(path);
    }
}

impl Drop for DataLock {
    fn drop(&mut self) {
        // The process guard and OS lock are authoritative. The file content is
        // diagnostic metadata for humans and is removed on clean shutdown.
        remove_data_lock_file_if_owned(&self.path, &self.token);
        // Dropping `_file` closes the fd and releases the OS lock.
    }
}

/// Ensure a directory exists and is private enough for transcript data.
pub fn ensure_private_dir(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Err(JottraceError::NotDirectory(path.to_path_buf()));
            }
            ensure_dir_mode(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_private_dir(path)?;
            ensure_dir_mode(path)
        }
        Err(source) => Err(JottraceError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Create a new private file without overwriting an existing one.
pub fn create_private_file(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }

    // `create_new` is intentional: a caller creating durable journal state
    // should not accidentally truncate an existing transcript or database.
    let file = private_open_options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| JottraceError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    #[cfg(unix)]
    // The open mode is the first line of defense, but chmod after creation
    // corrects for umask and keeps behavior stable across Unix environments.
    if let Err(error) = set_mode(path, PRIVATE_FILE_MODE) {
        let _ = fs::remove_file(path);
        return Err(error);
    }

    Ok(file)
}

/// Ensure a regular file exists and is private enough for local state.
pub fn ensure_private_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }

    match fs::metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(JottraceError::NotFile(path.to_path_buf()));
            }
            ensure_file_mode(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            drop(create_private_file(path)?);
            Ok(())
        }
        Err(source) => Err(JottraceError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    builder.mode(PRIVATE_DIR_MODE);
    builder.create(path).map_err(|source| JottraceError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    // DirBuilder's mode is affected by the process umask, so enforce the final
    // permission after the directory exists.
    set_mode(path, PRIVATE_DIR_MODE)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| JottraceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn ensure_dir_mode(path: &Path) -> Result<()> {
    let actual = mode(path)?;
    if actual != PRIVATE_DIR_MODE {
        return Err(JottraceError::InsecureMode {
            path: path.to_path_buf(),
            expected: PRIVATE_DIR_MODE,
            actual,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_dir_mode(_path: &Path) -> Result<()> {
    // The numeric Unix mode contract does not apply on Windows; platform-
    // specific ACL hardening can be added behind this same check later.
    Ok(())
}

#[cfg(unix)]
fn ensure_file_mode(path: &Path) -> Result<()> {
    let actual = mode(path)?;
    if actual != PRIVATE_FILE_MODE {
        return Err(JottraceError::InsecureMode {
            path: path.to_path_buf(),
            expected: PRIVATE_FILE_MODE,
            actual,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_file_mode(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, expected: u32) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .map_err(|source| JottraceError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    permissions.set_mode(expected);
    fs::set_permissions(path, permissions).map_err(|source| JottraceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn mode(path: &Path) -> Result<u32> {
    // Mask out file-type bits so callers compare only the familiar chmod mode.
    Ok(fs::metadata(path)
        .map_err(|source| JottraceError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .permissions()
        .mode()
        & 0o777)
}

fn private_open_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    // File permissions have to be attached before `open`; setting them only
    // afterwards would leave a small window with process-default permissions.
    options.mode(PRIVATE_FILE_MODE);
    options
}

fn lock_token() -> String {
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("pid={}\nstarted_at_ns={started_at}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn create_private_file_uses_private_mode() {
        let root = temp_root("private-file");
        let file_path = root.join("db.sqlite");

        // Exercise the public helper rather than hand-creating parents, because
        // the privacy guarantee is the contract this crate is meant to provide.
        let mut file = create_private_file(&file_path).expect("create private file");
        file.write_all(b"sqlite placeholder").expect("write file");

        #[cfg(unix)]
        {
            assert_eq!(mode(&root).expect("dir mode"), PRIVATE_DIR_MODE);
            assert_eq!(mode(&file_path).expect("file mode"), PRIVATE_FILE_MODE);
        }

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn acquire_data_lock_reports_live_contention() {
        let root = temp_root("live-lock");
        fs::create_dir_all(&root).expect("create root");
        fs::set_permissions(&root, fs::Permissions::from_mode(PRIVATE_DIR_MODE))
            .expect("set private permissions");
        let _held = acquire_data_lock(&root).expect("hold lock");

        let Err(error) = acquire_data_lock(&root) else {
            panic!("live lock should be held");
        };
        assert!(matches!(error, JottraceError::LockHeld(_)));

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn acquire_data_lock_ignores_stale_metadata_without_os_lock() {
        let root = temp_root("stale-metadata");
        fs::create_dir_all(&root).expect("create root");
        fs::set_permissions(&root, fs::Permissions::from_mode(PRIVATE_DIR_MODE))
            .expect("set private permissions");
        let lock_path = root.join(LOCK_FILE_NAME);
        fs::write(&lock_path, "pid=123\nstarted_at_ns=0").expect("write lock");
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))
            .expect("set private file permissions");

        let lock = acquire_data_lock(&root).expect("stale metadata should not block");

        assert_eq!(
            fs::read_to_string(&lock_path)
                .expect("read lock")
                .trim_end(),
            lock.token
        );

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_private_dir_rejects_world_readable_directory() {
        let root = temp_root("insecure-dir");
        fs::create_dir_all(&root).expect("create temp dir");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755))
            .expect("set insecure permissions");

        // Rejecting this path is deliberate: a world-readable transcript store
        // should require an explicit human fix, not an invisible repair.
        let error = ensure_private_dir(&root).expect_err("reject insecure dir");
        assert!(error.to_string().contains("expected 700"));

        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(name: &str) -> PathBuf {
        // Include the process id and a high-resolution timestamp so parallel
        // test runs do not collide in the shared temp directory.
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id()))
    }
}
