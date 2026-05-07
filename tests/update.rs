mod common;

use common::{
    TempRoot, create_release_artifact, current_release_target, temp_root, write_fake_binary,
};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

const AUTO_UPDATE_LOCK_FILE: &str = "auto-update-check.lock";
const HIDDEN_AUTO_UPDATE_COMMAND: &str = "__jottrace-auto-update";

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

#[test]
fn update_help_aliases_print_usage_without_update_environment() {
    for command in ["update", "upgrade"] {
        let long = Command::new(binary())
            .args([command, "--help"])
            .output()
            .unwrap_or_else(|error| panic!("run jottrace {command} --help: {error}"));
        let short = Command::new(binary())
            .args([command, "-h"])
            .output()
            .unwrap_or_else(|error| panic!("run jottrace {command} -h: {error}"));

        assert!(
            long.status.success(),
            "{command} --help stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&long.stdout),
            String::from_utf8_lossy(&long.stderr)
        );
        assert!(
            short.status.success(),
            "{command} -h stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&short.stdout),
            String::from_utf8_lossy(&short.stderr)
        );
        assert_eq!(
            long.stdout, short.stdout,
            "{command} help aliases should match"
        );
        assert!(long.stderr.is_empty(), "{command} help should not warn");

        let stdout = String::from_utf8_lossy(&long.stdout);
        assert!(stdout.contains("jottrace update"));
        assert!(stdout.contains("Usage:"));
        assert!(stdout.contains("jottrace upgrade"));
    }
}

#[test]
fn update_rejects_unknown_options_before_running_update() {
    let output = Command::new(binary())
        .args(["update", "--definitely-not-an-option"])
        .output()
        .expect("run jottrace update with unknown option");

    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown update option: --definitely-not-an-option"));
    assert!(stderr.contains("jottrace update --help"));
}

#[test]
fn normal_command_spawns_quiet_background_update_for_installer_managed_binary() {
    let Some(fixture) = AutoUpdateFixture::new("auto-update-success") else {
        eprintln!("skipping auto-update test on unsupported test host");
        return;
    };

    let tag = "v26.5.6";
    fixture.create_release(tag, "26.5.6", "auto-updated binary");
    let output = fixture.run_status(tag);

    assert_quiet_status_success(&output);
    wait_for_version(&fixture.installed, "26.5.6");
    wait_for_missing_path(&fixture.auto_update_lock_path());
}

#[test]
fn auto_update_env_opt_out_disables_background_update() {
    let Some(fixture) = AutoUpdateFixture::new("auto-update-env-opt-out") else {
        eprintln!("skipping auto-update env opt-out test on unsupported test host");
        return;
    };

    let tag = "v26.5.6";
    fixture.create_release(tag, "26.5.6", "auto-updated binary");
    let mut command = fixture.status_command(&fixture.installed, tag);
    let output = command
        .env("JOTTRACE_AUTO_UPDATE", "0")
        .output()
        .expect("run opted-out jottrace status");

    assert_quiet_status_success(&output);
    assert_version_remains(&fixture.installed, env!("CARGO_PKG_VERSION"));
    wait_for_missing_path(&fixture.auto_update_lock_path());
}

#[test]
fn auto_update_config_opt_out_disables_background_update() {
    let Some(fixture) = AutoUpdateFixture::new("auto-update-config-opt-out") else {
        eprintln!("skipping auto-update config opt-out test on unsupported test host");
        return;
    };

    let tag = "v26.5.6";
    fixture.write_config_opt_out();
    fixture.create_release(tag, "26.5.6", "auto-updated binary");
    let output = fixture.run_status(tag);

    assert_quiet_status_success(&output);
    assert_version_remains(&fixture.installed, env!("CARGO_PKG_VERSION"));
    wait_for_missing_path(&fixture.auto_update_lock_path());
}

#[test]
fn auto_update_skips_non_installer_managed_binary() {
    let Some(fixture) = AutoUpdateFixture::new("auto-update-non-installer") else {
        eprintln!("skipping non-installer auto-update test on unsupported test host");
        return;
    };

    let custom_binary = fixture.home.join("custom/bin/jottrace");
    copy_current_test_binary_to(&custom_binary);
    let tag = "v26.5.6";
    fixture.create_release(tag, "26.5.6", "auto-updated binary");
    let output = fixture.run_status_from(&custom_binary, tag);

    assert_quiet_status_success(&output);
    assert_version_remains(&custom_binary, env!("CARGO_PKG_VERSION"));
}

#[cfg(unix)]
#[test]
fn auto_update_skips_non_installer_binary_even_if_local_bin_symlinks_to_it() {
    let Some(fixture) = AutoUpdateFixture::new("auto-update-symlinked-non-installer") else {
        eprintln!("skipping symlinked non-installer auto-update test on unsupported test host");
        return;
    };

    let custom_binary = fixture.home.join("custom/bin/jottrace");
    copy_current_test_binary_to(&custom_binary);
    fs::remove_file(&fixture.installed).expect("remove installer-path test binary");
    std::os::unix::fs::symlink(&custom_binary, &fixture.installed)
        .expect("symlink installer path to custom binary");

    let tag = "v26.5.6";
    fixture.create_release(tag, "26.5.6", "auto-updated binary");
    let output = fixture.run_status_from(&custom_binary, tag);

    assert_quiet_status_success(&output);
    assert_version_remains(&custom_binary, env!("CARGO_PKG_VERSION"));
}

#[test]
fn auto_update_check_is_throttled_after_normal_startup() {
    let Some(fixture) = AutoUpdateFixture::new("auto-update-throttled") else {
        eprintln!("skipping auto-update throttle test on unsupported test host");
        return;
    };

    fixture.create_release("vcurrent", env!("CARGO_PKG_VERSION"), "current binary");
    let first = fixture.run_status("vcurrent");
    assert_quiet_status_success(&first);

    fixture.create_release("vnext", "26.5.6", "throttled newer binary");
    let second = fixture.run_status("vnext");
    assert_quiet_status_success(&second);

    assert_version_remains(&fixture.installed, env!("CARGO_PKG_VERSION"));
}

#[test]
fn failed_background_auto_update_leaves_installed_binary_usable() {
    let Some(fixture) = AutoUpdateFixture::new("auto-update-failed") else {
        eprintln!("skipping auto-update failure test on unsupported test host");
        return;
    };

    fs::create_dir_all(&fixture.releases).expect("create empty releases dir");
    let output = fixture.run_status("vmissing");

    assert_quiet_status_success(&output);
    assert_version_remains(&fixture.installed, env!("CARGO_PKG_VERSION"));
    wait_for_missing_path(&fixture.auto_update_lock_path());
}

#[test]
fn auto_update_reclaims_stale_lock_and_runs() {
    let Some(fixture) = AutoUpdateFixture::new("auto-update-stale-lock") else {
        eprintln!("skipping stale auto-update lock test on unsupported test host");
        return;
    };

    jottrace::ensure_private_dir(&fixture.data_dir).expect("create private data dir");
    fs::write(fixture.auto_update_lock_path(), "pid=1\ncreated_at=0\n")
        .expect("write stale auto-update lock");

    let tag = "v26.5.6";
    fixture.create_release(tag, "26.5.6", "auto-updated binary");
    let output = fixture.run_status(tag);

    assert_quiet_status_success(&output);
    wait_for_version(&fixture.installed, "26.5.6");
    wait_for_missing_path(&fixture.auto_update_lock_path());
}

#[test]
fn auto_update_does_not_run_for_command_help() {
    let Some(fixture) = AutoUpdateFixture::new("auto-update-help") else {
        eprintln!("skipping auto-update help test on unsupported test host");
        return;
    };

    fixture.create_release("v26.5.6", "26.5.6", "auto-updated binary");
    let mut command = fixture.status_command(&fixture.installed, "v26.5.6");
    let output = command
        .arg("--help")
        .output()
        .expect("run installer-managed jottrace status --help");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("jottrace status"));
    assert!(output.stderr.is_empty());
    assert_version_remains(&fixture.installed, env!("CARGO_PKG_VERSION"));
}

#[test]
fn hidden_auto_update_command_does_not_remove_untrusted_lock_path() {
    let root = temp_root("hidden-auto-update-untrusted-lock");
    let data_dir = root.join(".jottrace");
    let untrusted_lock_path = root.join("untrusted.lock");
    fs::create_dir_all(root.as_ref()).expect("create hidden auto-update test root");
    fs::write(&untrusted_lock_path, "not an auto-update lock").expect("write untrusted lock path");

    let output = Command::new(binary())
        .arg(HIDDEN_AUTO_UPDATE_COMMAND)
        .env("JOTTRACE_HOME", &data_dir)
        .env("JOTTRACE_AUTO_UPDATE_LOCK_PATH", &untrusted_lock_path)
        .env(
            "JOTTRACE_UPDATE_INSTALL_PATH",
            root.join("missing-jottrace"),
        )
        .output()
        .expect("run hidden auto-update command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        untrusted_lock_path.exists(),
        "hidden auto-update command should not remove arbitrary env-provided paths"
    );
}

#[test]
fn manual_update_runs_even_when_auto_update_is_opted_out() {
    let Some(target) = current_release_target() else {
        eprintln!("skipping manual update opt-out test on unsupported test host");
        return;
    };

    let root = temp_root("manual-update-auto-opt-out");
    let home = root.join("home");
    let releases = root.join("releases");
    let data_dir = home.join(".jottrace");
    let install_dir = root.join("bin");
    let installed = install_dir.join("jottrace");
    let tag = "v26.5.6";
    fs::create_dir_all(&install_dir).expect("create install dir");
    jottrace::ensure_private_dir(&data_dir).expect("create private data dir");
    fs::write(data_dir.join("config.json"), "{\"auto_update\": false}\n")
        .expect("write auto-update config opt-out");
    write_fake_binary(&installed, "26.5.4", "old binary");
    create_release_artifact(&root, &releases, tag, target, "26.5.6", "updated binary");

    let output = Command::new(binary())
        .arg("update")
        .env("HOME", &home)
        .env("JOTTRACE_HOME", &data_dir)
        .env("JOTTRACE_AUTO_UPDATE", "0")
        .env("JOTTRACE_UPDATE_INSTALL_PATH", &installed)
        .env("JOTTRACE_VERSION", tag)
        .env(
            "JOTTRACE_RELEASE_BASE_URL",
            format!("file://{}", releases.display()),
        )
        .output()
        .expect("run manual jottrace update with auto-update opt-outs");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("jottrace update"));
    assert!(stdout.contains("result: updated"));
    assert_eq!(installed_version(&installed), "26.5.6");
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

struct AutoUpdateFixture {
    root: TempRoot,
    home: std::path::PathBuf,
    releases: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    installed: std::path::PathBuf,
    target: &'static str,
}

impl AutoUpdateFixture {
    fn new(name: &str) -> Option<Self> {
        let target = current_release_target()?;
        let root = temp_root(name);
        let home = root.join("home");
        let releases = root.join("releases");
        let data_dir = home.join(".jottrace");
        let installed = install_current_test_binary(&home);

        Some(Self {
            root,
            home,
            releases,
            data_dir,
            installed,
            target,
        })
    }

    fn create_release(&self, tag: &str, version: &str, marker: &str) {
        create_release_artifact(
            self.root.as_ref(),
            &self.releases,
            tag,
            self.target,
            version,
            marker,
        );
    }

    fn write_config_opt_out(&self) {
        jottrace::ensure_private_dir(&self.data_dir).expect("create private data dir");
        fs::write(
            self.data_dir.join("config.json"),
            "{\"auto_update\": false}\n",
        )
        .expect("write auto-update config opt-out");
    }

    fn run_status(&self, tag: &str) -> Output {
        self.run_status_from(&self.installed, tag)
    }

    fn run_status_from(&self, binary: &Path, tag: &str) -> Output {
        self.status_command(binary, tag)
            .output()
            .expect("run jottrace status")
    }

    fn status_command(&self, binary: &Path, tag: &str) -> Command {
        let mut command = Command::new(binary);
        command
            .arg("status")
            .env("HOME", &self.home)
            .env("JOTTRACE_HOME", &self.data_dir)
            .env("JOTTRACE_VERSION", tag)
            .env(
                "JOTTRACE_RELEASE_BASE_URL",
                format!("file://{}", self.releases.display()),
            );
        command
    }

    fn auto_update_lock_path(&self) -> std::path::PathBuf {
        self.data_dir.join(AUTO_UPDATE_LOCK_FILE)
    }
}

fn assert_quiet_status_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("jottrace status"));
    assert!(
        !stdout.contains("jottrace update"),
        "foreground stdout should stay script-friendly"
    );
    assert!(
        output.stderr.is_empty(),
        "foreground stderr should not include background update status"
    );
}

fn install_current_test_binary(home: &Path) -> std::path::PathBuf {
    let install_dir = home.join(".local/bin");
    let installed = install_dir.join("jottrace");
    copy_current_test_binary_to(&installed);
    installed
}

fn copy_current_test_binary_to(path: &Path) {
    fs::create_dir_all(path.parent().expect("test binary path has parent"))
        .expect("create test binary dir");
    fs::copy(binary(), path).expect("copy current test binary");
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod copied test binary");
}

fn wait_for_version(path: &Path, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if installed_version(path) == expected {
            return;
        }
        if Instant::now() >= deadline {
            panic!(
                "expected {} to update to {expected}, got {}",
                path.display(),
                installed_version(path)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_version_remains(path: &Path, expected: &str) {
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        assert_eq!(installed_version(path), expected);
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_missing_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while path.exists() {
        if Instant::now() >= deadline {
            panic!("expected {} to be removed", path.display());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn installed_version(path: &Path) -> String {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("run {} --version: {error}", path.display()));
    assert!(
        output.status.success(),
        "{} --version failed: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .strip_prefix("jottrace ")
        .unwrap_or_else(|| panic!("unexpected version output from {}", path.display()))
        .to_string()
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_jottrace")
}
