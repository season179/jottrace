mod common;

use common::{create_release_artifact, current_release_target, temp_root, write_fake_binary};
use std::fs;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[test]
fn update_replaces_installed_binary_from_release_artifact() {
    let Some(target) = current_release_target() else {
        eprintln!("skipping update test on unsupported test host");
        return;
    };

    let root = temp_root("update-success");
    let releases = root.join("releases");
    let install_dir = root.join("bin");
    let installed = install_dir.join("jottrace");
    let data_dir = root.join(".jottrace");
    let data_sentinel = data_dir.join("db.sqlite");
    let tag = "v26.5.5";

    fs::create_dir_all(&install_dir).expect("create install dir");
    fs::create_dir_all(&data_dir).expect("create data dir");
    fs::write(&data_sentinel, "preserved journal data").expect("write data sentinel");
    write_fake_binary(&installed, "26.5.3", "old binary");
    create_release_artifact(&root, &releases, tag, target, "26.5.5", "updated binary");

    let output = Command::new(binary())
        .arg("update")
        .env("JOTTRACE_UPDATE_INSTALL_PATH", &installed)
        .env("JOTTRACE_VERSION", tag)
        .env(
            "JOTTRACE_RELEASE_BASE_URL",
            format!("file://{}", releases.display()),
        )
        .env("JOTTRACE_HOME", &data_dir)
        .output()
        .expect("run jottrace update");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("jottrace update"));
    assert!(stdout.contains("current_version: 26.5.3"));
    assert!(stdout.contains("target_version: 26.5.5"));
    assert!(stdout.contains(&format!("install_path: {}", installed.display())));
    assert!(stdout.contains("result: updated"));

    let version = Command::new(&installed)
        .arg("--version")
        .output()
        .expect("run updated jottrace");
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "jottrace 26.5.5"
    );

    assert_eq!(
        fs::read_to_string(&data_sentinel).expect("read data sentinel"),
        "preserved journal data"
    );

    #[cfg(unix)]
    assert_ne!(
        fs::metadata(&installed).expect("installed metadata").mode() & 0o111,
        0,
        "updated jottrace should be executable"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn update_reports_up_to_date_without_replacing_installed_binary() {
    let Some(target) = current_release_target() else {
        eprintln!("skipping up-to-date update test on unsupported test host");
        return;
    };

    let root = temp_root("update-up-to-date");
    let releases = root.join("releases");
    let install_dir = root.join("bin");
    let installed = install_dir.join("jottrace");
    let tag = "v26.5.5";

    fs::create_dir_all(&install_dir).expect("create install dir");
    write_fake_binary(&installed, "26.5.5", "old binary");
    let before = fs::read_to_string(&installed).expect("read installed binary before update");
    create_release_artifact(&root, &releases, tag, target, "26.5.5", "new binary");

    let output = Command::new(binary())
        .arg("update")
        .env("JOTTRACE_UPDATE_INSTALL_PATH", &installed)
        .env("JOTTRACE_VERSION", tag)
        .env(
            "JOTTRACE_RELEASE_BASE_URL",
            format!("file://{}", releases.display()),
        )
        .output()
        .expect("run jottrace update for current version");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("current_version: 26.5.5"));
    assert!(stdout.contains("target_version: 26.5.5"));
    assert!(stdout.contains("result: up-to-date"));
    assert_eq!(
        fs::read_to_string(&installed).expect("read installed binary after update"),
        before
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn empty_version_env_uses_latest_release_artifact() {
    let Some(target) = current_release_target() else {
        eprintln!("skipping empty version update test on unsupported test host");
        return;
    };

    let root = temp_root("update-empty-version");
    let releases = root.join("releases");
    let install_dir = root.join("bin");
    let installed = install_dir.join("jottrace");

    fs::create_dir_all(&install_dir).expect("create install dir");
    write_fake_binary(&installed, "26.5.3", "old binary");
    create_release_artifact(
        &root,
        &releases,
        "latest",
        target,
        "26.5.5",
        "latest binary",
    );

    let output = Command::new(binary())
        .arg("update")
        .env("JOTTRACE_UPDATE_INSTALL_PATH", &installed)
        .env("JOTTRACE_VERSION", "")
        .env(
            "JOTTRACE_RELEASE_BASE_URL",
            format!("file://{}", releases.display()),
        )
        .output()
        .expect("run jottrace update with empty version env");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("target_version: 26.5.5"));
    assert!(stdout.contains("result: updated"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_update_leaves_installed_binary_usable() {
    let Some(_target) = current_release_target() else {
        eprintln!("skipping update failure test on unsupported test host");
        return;
    };

    let root = temp_root("update-failed-download");
    let releases = root.join("releases");
    let install_dir = root.join("bin");
    let installed = install_dir.join("jottrace");

    fs::create_dir_all(&install_dir).expect("create install dir");
    fs::create_dir_all(&releases).expect("create empty releases dir");
    write_fake_binary(&installed, "26.5.3", "old binary");

    let output = Command::new(binary())
        .arg("update")
        .env("JOTTRACE_UPDATE_INSTALL_PATH", &installed)
        .env("JOTTRACE_VERSION", "vmissing")
        .env(
            "JOTTRACE_RELEASE_BASE_URL",
            format!("file://{}", releases.display()),
        )
        .output()
        .expect("run jottrace update with missing artifact");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jottrace update failed"));

    let version = Command::new(&installed)
        .arg("--version")
        .output()
        .expect("run preserved jottrace");
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "jottrace 26.5.3"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn invalid_update_artifact_leaves_installed_binary_usable() {
    let Some(target) = current_release_target() else {
        eprintln!("skipping invalid update artifact test on unsupported test host");
        return;
    };

    let root = temp_root("update-invalid-artifact");
    let releases = root.join("releases");
    let install_dir = root.join("bin");
    let installed = install_dir.join("jottrace");
    let tag = "vbad";

    fs::create_dir_all(&install_dir).expect("create install dir");
    write_fake_binary(&installed, "26.5.3", "old binary");
    create_invalid_release_artifact(&root, &releases, tag, target);

    let output = Command::new(binary())
        .arg("update")
        .env("JOTTRACE_UPDATE_INSTALL_PATH", &installed)
        .env("JOTTRACE_VERSION", tag)
        .env(
            "JOTTRACE_RELEASE_BASE_URL",
            format!("file://{}", releases.display()),
        )
        .output()
        .expect("run jottrace update with invalid artifact");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jottrace update failed"));
    assert!(stderr.contains("invalid release artifact"));

    let version = Command::new(&installed)
        .arg("--version")
        .output()
        .expect("run preserved jottrace");
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "jottrace 26.5.3"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn upgrade_is_an_update_alias() {
    let Some(target) = current_release_target() else {
        eprintln!("skipping upgrade alias test on unsupported test host");
        return;
    };

    let root = temp_root("upgrade-alias");
    let releases = root.join("releases");
    let install_dir = root.join("bin");
    let installed = install_dir.join("jottrace");
    let tag = "v26.5.6";

    fs::create_dir_all(&install_dir).expect("create install dir");
    write_fake_binary(&installed, "26.5.4", "old binary");
    create_release_artifact(&root, &releases, tag, target, "26.5.6", "upgraded binary");

    let output = Command::new(binary())
        .arg("upgrade")
        .env("JOTTRACE_UPDATE_INSTALL_PATH", &installed)
        .env("JOTTRACE_VERSION", tag)
        .env(
            "JOTTRACE_RELEASE_BASE_URL",
            format!("file://{}", releases.display()),
        )
        .output()
        .expect("run jottrace upgrade");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let version = Command::new(&installed)
        .arg("--version")
        .output()
        .expect("run upgraded jottrace");
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "jottrace 26.5.6"
    );

    let _ = fs::remove_dir_all(root);
}

fn create_invalid_release_artifact(
    root: &std::path::Path,
    releases: &std::path::Path,
    tag: &str,
    target: &str,
) {
    let release_dir = releases.join(tag);
    let payload_dir = root.join(format!("invalid-payload-{target}"));
    fs::create_dir_all(&release_dir).expect("create release dir");
    fs::create_dir_all(&payload_dir).expect("create payload dir");
    fs::write(payload_dir.join("not-jottrace"), "not the binary").expect("write invalid payload");

    let artifact = release_dir.join(format!("jottrace-{target}.tar.gz"));
    let tar_status = Command::new("tar")
        .args(["-C", payload_dir.to_str().expect("utf8 payload path")])
        .args(["-czf", artifact.to_str().expect("utf8 artifact path")])
        .arg("not-jottrace")
        .status()
        .expect("create invalid release artifact");
    assert!(
        tar_status.success(),
        "tar should create invalid test artifact"
    );
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_jottrace")
}
