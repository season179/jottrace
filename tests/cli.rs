use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[test]
fn version_prints_package_version() {
    // Run the compiled binary instead of the library so this test covers the
    // real command-line dispatch a user or script will exercise.
    let output = Command::new(binary())
        .arg("--version")
        .output()
        .expect("run jottrace --version");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("jottrace {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn doctor_creates_private_data_dir() {
    let root = temp_root("doctor-ok");
    let data_dir = root.join(".jottrace");

    // JOTTRACE_HOME keeps the integration test hermetic; it must never create
    // or chmod state under the developer's actual HOME.
    let output = Command::new(binary())
        .arg("doctor")
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace doctor");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("jottrace doctor"));
    assert!(stdout.contains("permissions: private (ok)"));

    #[cfg(unix)]
    assert_eq!(mode(&data_dir), 0o700);

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn doctor_rejects_insecure_existing_data_dir() {
    let root = temp_root("doctor-insecure");
    let data_dir = root.join(".jottrace");
    fs::create_dir_all(&data_dir).expect("create data dir");
    fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o755))
        .expect("set insecure dir mode");

    // This protects the privacy boundary across releases: an existing loose
    // directory must stay visible as a problem instead of being accepted.
    let output = Command::new(binary())
        .arg("doctor")
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace doctor");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jottrace doctor failed"));
    assert!(stderr.contains("expected 700"));

    let _ = fs::remove_dir_all(root);
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_jottrace")
}

fn temp_root(name: &str) -> std::path::PathBuf {
    // Cargo can run tests concurrently, so the temp path needs more entropy
    // than the test name alone.
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id()))
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    // Metadata includes file-type bits; masking leaves only chmod-style perms.
    fs::metadata(path).expect("metadata").mode() & 0o777
}
