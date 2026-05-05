use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

pub const APP_DIR_NAME: &str = ".jottrace";
pub const PRIVATE_DIR_MODE: u32 = 0o700;
pub const PRIVATE_FILE_MODE: u32 = 0o600;

#[derive(Debug)]
pub enum JottraceError {
    MissingHome,
    Io {
        path: PathBuf,
        source: io::Error,
    },
    NotDirectory(PathBuf),
    InsecureMode {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
}

impl fmt::Display for JottraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHome => write!(f, "HOME is not set and JOTTRACE_HOME was not provided"),
            Self::Io { path, source } => write!(f, "{}: {}", path.display(), source),
            Self::NotDirectory(path) => {
                write!(f, "{} exists but is not a directory", path.display())
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
        }
    }
}

impl std::error::Error for JottraceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, JottraceError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub data_dir: PathBuf,
}

pub fn data_dir_from_env() -> Result<PathBuf> {
    if let Some(path) = env::var_os("JOTTRACE_HOME") {
        return Ok(PathBuf::from(path));
    }

    let home = env::var_os("HOME").ok_or(JottraceError::MissingHome)?;
    Ok(PathBuf::from(home).join(APP_DIR_NAME))
}

pub fn run_doctor() -> Result<DoctorReport> {
    let data_dir = data_dir_from_env()?;
    ensure_private_dir(&data_dir)?;
    Ok(DoctorReport { data_dir })
}

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

pub fn create_private_file(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }

    let file = private_open_options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| JottraceError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    #[cfg(unix)]
    set_mode(path, PRIVATE_FILE_MODE)?;

    Ok(file)
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
    options.mode(PRIVATE_FILE_MODE);
    options
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
    fn ensure_private_dir_rejects_world_readable_directory() {
        let root = temp_root("insecure-dir");
        fs::create_dir_all(&root).expect("create temp dir");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755))
            .expect("set insecure permissions");

        let error = ensure_private_dir(&root).expect_err("reject insecure dir");
        assert!(error.to_string().contains("expected 700"));

        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id()))
    }
}
