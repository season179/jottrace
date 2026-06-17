#![allow(dead_code)]

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub fn reader_fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/readers")
        .join(relative)
}

pub fn taste_fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/taste")
        .join(relative)
}

pub struct TempRoot {
    path: PathBuf,
}

impl Deref for TempRoot {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TempRoot {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn temp_root(name: &str) -> TempRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    TempRoot {
        path: std::env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id())),
    }
}

pub fn current_release_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-x86_64"),
        _ => None,
    }
}

pub fn create_release_artifact(
    root: &Path,
    releases: &Path,
    tag: &str,
    target: &str,
    version: &str,
    marker: &str,
) {
    let release_dir = releases.join(tag);
    let payload_dir = root.join(format!("payload-{target}"));
    fs::create_dir_all(&release_dir).expect("create release dir");
    fs::create_dir_all(&payload_dir).expect("create payload dir");

    write_fake_binary(&payload_dir.join("jottrace"), version, marker);

    let artifact = release_dir.join(format!("jottrace-{target}.tar.gz"));
    let tar_status = Command::new("tar")
        .args(["-C", payload_dir.to_str().expect("utf8 payload path")])
        .args(["-czf", artifact.to_str().expect("utf8 artifact path")])
        .arg("jottrace")
        .status()
        .expect("create release artifact");
    assert!(tar_status.success(), "tar should create test artifact");
}

pub fn write_fake_binary(path: &Path, version: &str, marker: &str) {
    fs::write(
        path,
        format!(
            "#!/usr/bin/env sh\nif [ \"$1\" = \"--version\" ]; then echo 'jottrace {version}'; else echo '{marker}'; fi\n"
        ),
    )
    .expect("write fake binary");
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod fake binary");
}
