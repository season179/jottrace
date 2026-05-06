use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const REPO: &str = "season179/jottrace";
const VERSION_ENV: &str = "JOTTRACE_VERSION";
const RELEASE_BASE_URL_ENV: &str = "JOTTRACE_RELEASE_BASE_URL";
const UPDATE_INSTALL_PATH_ENV: &str = "JOTTRACE_UPDATE_INSTALL_PATH";

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
