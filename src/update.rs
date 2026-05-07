use serde::Deserialize;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const REPO: &str = "season179/jottrace";
const VERSION_ENV: &str = "JOTTRACE_VERSION";
const RELEASE_BASE_URL_ENV: &str = "JOTTRACE_RELEASE_BASE_URL";
const UPDATE_INSTALL_PATH_ENV: &str = "JOTTRACE_UPDATE_INSTALL_PATH";
const AUTO_UPDATE_ENV: &str = "JOTTRACE_AUTO_UPDATE";
const AUTO_UPDATE_LOCK_PATH_ENV: &str = "JOTTRACE_AUTO_UPDATE_LOCK_PATH";
const AUTO_UPDATE_COMMAND: &str = "__jottrace-auto-update";
const AUTO_UPDATE_CONFIG_FILE: &str = "config.json";
const AUTO_UPDATE_STAMP_FILE: &str = "auto-update-check";
const AUTO_UPDATE_LOCK_FILE: &str = "auto-update-check.lock";
const AUTO_UPDATE_INTERVAL_SECS: u64 = 24 * 60 * 60;
const AUTO_UPDATE_LOCK_STALE_SECS: u64 = 60 * 60;

#[derive(Debug, Deserialize)]
struct JottraceConfig {
    auto_update: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    pub current_version: String,
    pub target_version: String,
    pub install_path: PathBuf,
    pub result: UpdateResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateResult {
    Updated,
    UpToDate,
}

impl UpdateResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Updated => "updated",
            Self::UpToDate => "up-to-date",
        }
    }
}

#[derive(Debug)]
pub enum UpdateError {
    UnsupportedPlatform {
        os: String,
        arch: String,
    },
    MissingInstallParent(PathBuf),
    Io {
        path: PathBuf,
        source: io::Error,
    },
    CommandIo {
        program: &'static str,
        source: io::Error,
    },
    CommandFailed {
        program: &'static str,
        stderr: String,
    },
    InvalidArtifact(String),
    InvalidVersionOutput {
        path: PathBuf,
        stdout: String,
    },
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform { os, arch } => {
                write!(f, "unsupported platform {os}/{arch}")
            }
            Self::MissingInstallParent(path) => {
                write!(
                    f,
                    "install path has no parent directory: {}",
                    path.display()
                )
            }
            Self::Io { path, source } => write!(f, "{}: {}", path.display(), source),
            Self::CommandIo { program, source } => write!(f, "failed to run {program}: {source}"),
            Self::CommandFailed { program, stderr } => {
                let stderr = stderr.trim();
                if stderr.is_empty() {
                    write!(f, "{program} failed")
                } else {
                    write!(f, "{program} failed: {stderr}")
                }
            }
            Self::InvalidArtifact(message) => write!(f, "invalid release artifact: {message}"),
            Self::InvalidVersionOutput { path, stdout } => write!(
                f,
                "{} printed an invalid version: {}",
                path.display(),
                stdout.trim()
            ),
        }
    }
}

impl std::error::Error for UpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::CommandIo { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, UpdateError>;

pub fn is_auto_update_command(command: &str) -> bool {
    command == AUTO_UPDATE_COMMAND
}

pub fn maybe_spawn_auto_update() {
    let _ = spawn_auto_update_if_due();
}

pub fn run_auto_update_silent() {
    let lock_path = trusted_auto_update_lock_path();
    let _ = run_update();
    if let Some(lock_path) = lock_path {
        let _ = fs::remove_file(lock_path);
    }
}

pub fn run_update() -> Result<UpdateReport> {
    let target = current_release_target()?;
    let install_path = install_path()?;
    let current_version = binary_version(&install_path)?;
    let version = env_value_or_default(VERSION_ENV, "latest");
    let artifact_url = release_url(target, &version);
    let temp_dir = TempDir::new("jottrace-update")?;
    let archive = temp_dir.path.join("jottrace.tar.gz");

    download_archive(&artifact_url, &archive)?;
    extract_archive(&archive, &temp_dir.path)?;

    let candidate = temp_dir.path.join("jottrace");
    let target_version = artifact_binary_version(&candidate)?;
    let result = if current_version == target_version {
        UpdateResult::UpToDate
    } else {
        replace_installed_binary(&candidate, &install_path)?;
        UpdateResult::Updated
    };

    Ok(UpdateReport {
        current_version,
        target_version,
        install_path,
        result,
    })
}

fn current_release_target() -> Result<&'static str> {
    match (env::consts::OS, env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("macos", "x86_64") => Ok("darwin-x86_64"),
        (os, arch) => Err(UpdateError::UnsupportedPlatform {
            os: os.to_string(),
            arch: arch.to_string(),
        }),
    }
}

fn install_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os(UPDATE_INSTALL_PATH_ENV) {
        return Ok(PathBuf::from(path));
    }

    env::current_exe().map_err(|source| UpdateError::Io {
        path: PathBuf::from("current executable"),
        source,
    })
}

fn release_url(target: &str, version: &str) -> String {
    let base_url = env_value(RELEASE_BASE_URL_ENV);
    release_url_for_base(target, version, base_url.as_deref())
}

fn release_url_for_base(target: &str, version: &str, base_url: Option<&str>) -> String {
    let artifact = format!("jottrace-{target}.tar.gz");
    if let Some(base_url) = base_url.filter(|value| !value.is_empty()) {
        format!("{}/{version}/{artifact}", base_url.trim_end_matches('/'))
    } else if version == "latest" {
        format!("https://github.com/{REPO}/releases/latest/download/{artifact}")
    } else {
        format!("https://github.com/{REPO}/releases/download/{version}/{artifact}")
    }
}

fn env_value(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn env_value_or_default(name: &str, default: &str) -> String {
    env_value(name).unwrap_or_else(|| default.to_string())
}

fn download_archive(url: &str, archive: &Path) -> Result<()> {
    let mut command = Command::new("curl");
    command.args(["-fsSL", url, "-o"]).arg(archive);
    run_checked_command("curl", &mut command).map(|_| ())
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<()> {
    let mut command = Command::new("tar");
    command
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .arg("jottrace");
    run_checked_command("tar", &mut command)
        .map(|_| ())
        .map_err(|error| match error {
            UpdateError::CommandFailed { program: "tar", .. } => UpdateError::InvalidArtifact(
                format!("artifact did not contain a runnable jottrace binary ({error})"),
            ),
            error => error,
        })
}

fn run_checked_command(program: &'static str, command: &mut Command) -> Result<Output> {
    let output = command
        .output()
        .map_err(|source| UpdateError::CommandIo { program, source })?;

    if output.status.success() {
        Ok(output)
    } else {
        Err(UpdateError::CommandFailed {
            program,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn binary_version(path: &Path) -> Result<String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|source| UpdateError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    if !output.status.success() {
        return Err(UpdateError::InvalidArtifact(format!(
            "{} did not run successfully with --version",
            path.display()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .strip_prefix("jottrace ")
        .map(ToOwned::to_owned)
        .ok_or_else(|| UpdateError::InvalidVersionOutput {
            path: path.to_path_buf(),
            stdout: stdout.into_owned(),
        })
}

fn artifact_binary_version(path: &Path) -> Result<String> {
    binary_version(path).map_err(|error| {
        UpdateError::InvalidArtifact(format!(
            "artifact did not contain a runnable jottrace binary ({error})"
        ))
    })
}

fn spawn_auto_update_if_due() -> io::Result<bool> {
    if auto_update_disabled_by_env() {
        return Ok(false);
    }

    let exe = env::current_exe()?;
    if !is_installer_managed_binary(&exe) {
        return Ok(false);
    }

    let Ok(data_dir) = crate::data_dir_from_env() else {
        return Ok(false);
    };
    if crate::ensure_private_dir(&data_dir).is_err() {
        return Ok(false);
    }

    let stamp = data_dir.join(AUTO_UPDATE_STAMP_FILE);
    if !auto_update_due(&stamp) {
        return Ok(false);
    }
    let Some(lock) = AutoUpdateLock::acquire(&data_dir)? else {
        return Ok(false);
    };
    if !auto_update_due(&stamp) {
        return Ok(false);
    }
    if !config_allows_auto_update(&data_dir) {
        return Ok(false);
    }
    write_auto_update_stamp(&stamp)?;

    let lock_path = lock.into_path();
    let child = Command::new(&exe)
        .arg(AUTO_UPDATE_COMMAND)
        .env(UPDATE_INSTALL_PATH_ENV, &exe)
        .env(AUTO_UPDATE_LOCK_PATH_ENV, &lock_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    let mut child = match child {
        Ok(child) => child,
        Err(source) => {
            let _ = fs::remove_file(lock_path);
            return Err(source);
        }
    };

    let _ = thread::Builder::new()
        .name("jottrace-auto-update-wait".to_string())
        .spawn(move || {
            let _ = child.wait();
        });

    Ok(true)
}

fn auto_update_disabled_by_env() -> bool {
    env_value(AUTO_UPDATE_ENV).as_deref() == Some("0")
}

fn config_allows_auto_update(data_dir: &Path) -> bool {
    let path = data_dir.join(AUTO_UPDATE_CONFIG_FILE);
    match fs::read_to_string(path) {
        Ok(contents) if contents.trim().is_empty() => true,
        Ok(contents) => serde_json::from_str::<JottraceConfig>(&contents)
            .map(|config| config.auto_update.unwrap_or(true))
            .unwrap_or(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

fn is_installer_managed_binary(exe: &Path) -> bool {
    let Some(home) = env::var_os("HOME") else {
        return false;
    };
    let expected = PathBuf::from(home).join(".local/bin/jottrace");
    if fs::symlink_metadata(&expected).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return false;
    }
    same_parent_and_file_name(exe, &expected)
}

fn trusted_auto_update_lock_path() -> Option<PathBuf> {
    let lock_path = env::var_os(AUTO_UPDATE_LOCK_PATH_ENV).map(PathBuf::from)?;
    let data_dir = crate::data_dir_from_env().ok()?;
    let expected = data_dir.join(AUTO_UPDATE_LOCK_FILE);
    same_parent_and_file_name(&lock_path, &expected).then_some(lock_path)
}

fn same_parent_and_file_name(left: &Path, right: &Path) -> bool {
    if left.file_name() != right.file_name() {
        return false;
    }

    let Some(left_parent) = left.parent() else {
        return false;
    };
    let Some(right_parent) = right.parent() else {
        return false;
    };

    match (
        fs::canonicalize(left_parent),
        fs::canonicalize(right_parent),
    ) {
        (Ok(left_parent), Ok(right_parent)) => left_parent == right_parent,
        _ => left_parent == right_parent,
    }
}

fn auto_update_due(path: &Path) -> bool {
    match fs::read_to_string(path) {
        Ok(contents) => contents
            .trim()
            .parse::<u64>()
            .map(|last_checked| {
                now_unix_seconds().saturating_sub(last_checked) >= AUTO_UPDATE_INTERVAL_SECS
            })
            .unwrap_or(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

fn write_auto_update_stamp(path: &Path) -> io::Result<()> {
    crate::ensure_private_file(path).map_err(jottrace_error_to_io)?;
    fs::write(path, format!("{}\n", now_unix_seconds()))
}

struct AutoUpdateLock {
    path: Option<PathBuf>,
}

impl AutoUpdateLock {
    fn acquire(data_dir: &Path) -> io::Result<Option<Self>> {
        let path = data_dir.join(AUTO_UPDATE_LOCK_FILE);
        let mut file = match Self::create_file(&path)? {
            Some(file) => file,
            None if remove_stale_auto_update_lock(&path)? => {
                let Some(file) = Self::create_file(&path)? else {
                    return Ok(None);
                };
                file
            }
            None => return Ok(None),
        };

        if let Err(source) = writeln!(
            file,
            "pid={}\ncreated_at={}",
            std::process::id(),
            now_unix_seconds()
        ) {
            let _ = fs::remove_file(&path);
            return Err(source);
        }

        Ok(Some(Self { path: Some(path) }))
    }

    fn create_file(path: &Path) -> io::Result<Option<fs::File>> {
        let file = match crate::create_private_file(path) {
            Ok(file) => file,
            Err(crate::JottraceError::Io { source, .. })
                if source.kind() == io::ErrorKind::AlreadyExists =>
            {
                return Ok(None);
            }
            Err(error) => return Err(jottrace_error_to_io(error)),
        };
        Ok(Some(file))
    }

    fn into_path(mut self) -> PathBuf {
        self.path.take().expect("auto-update lock path")
    }
}

impl Drop for AutoUpdateLock {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
    }
}

fn remove_stale_auto_update_lock(path: &Path) -> io::Result<bool> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(_) => return Ok(false),
    };

    let Some(created_at) = lock_created_at(&contents) else {
        return Ok(false);
    };
    if now_unix_seconds().saturating_sub(created_at) < AUTO_UPDATE_LOCK_STALE_SECS {
        return Ok(false);
    }

    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn lock_created_at(contents: &str) -> Option<u64> {
    contents
        .lines()
        .find_map(|line| line.strip_prefix("created_at="))
        .and_then(|value| value.parse().ok())
}

fn jottrace_error_to_io(error: crate::JottraceError) -> io::Error {
    io::Error::other(error.to_string())
}

fn replace_installed_binary(candidate: &Path, install_path: &Path) -> Result<()> {
    let parent = install_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| UpdateError::MissingInstallParent(install_path.to_path_buf()))?;
    let staged = parent.join(format!(
        ".jottrace-update-{}-{}",
        std::process::id(),
        unique_suffix()
    ));

    fs::copy(candidate, &staged).map_err(|source| UpdateError::Io {
        path: staged.clone(),
        source,
    })?;

    #[cfg(unix)]
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).map_err(|source| {
        let _ = fs::remove_file(&staged);
        UpdateError::Io {
            path: staged.clone(),
            source,
        }
    })?;

    fs::rename(&staged, install_path).map_err(|source| {
        let _ = fs::remove_file(&staged);
        UpdateError::Io {
            path: install_path.to_path_buf(),
            source,
        }
    })
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Result<Self> {
        let path = env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir(&path).map_err(|source| UpdateError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Self { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_url_uses_github_latest_when_base_url_is_unset_or_empty() {
        assert_eq!(
            release_url_for_base("linux-x86_64", "latest", None),
            "https://github.com/season179/jottrace/releases/latest/download/jottrace-linux-x86_64.tar.gz"
        );
        assert_eq!(
            release_url_for_base("linux-x86_64", "latest", Some("")),
            "https://github.com/season179/jottrace/releases/latest/download/jottrace-linux-x86_64.tar.gz"
        );
    }

    #[test]
    fn release_url_uses_base_url_when_non_empty() {
        assert_eq!(
            release_url_for_base("darwin-arm64", "v26.5.5", Some("file:///tmp/releases/")),
            "file:///tmp/releases/v26.5.5/jottrace-darwin-arm64.tar.gz"
        );
    }
}
