mod common;

use common::{create_release_artifact, current_release_target, temp_root};
use std::fs;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[test]
fn installer_downloads_matching_release_artifact_to_local_bin() {
    let Some(target) = current_release_target() else {
        eprintln!("skipping installer smoke test on unsupported test host");
        return;
    };

    let root = temp_root("installer-smoke");
    let home = root.join("home");
    let releases = root.join("releases");
    let tag = "vtest";
    fs::create_dir_all(&home).expect("create home");
    create_release_artifact(
        &root,
        &releases,
        tag,
        target,
        "26.5.0",
        "installer test binary",
    );

    let install_dir = home.join(".local/bin");
    let output = Command::new("bash")
        .arg("install.sh")
        .env("HOME", &home)
        .env("PATH", format!("{}:/usr/bin:/bin", install_dir.display()))
        .env("JOTTRACE_VERSION", tag)
        .env(
            "JOTTRACE_RELEASE_BASE_URL",
            format!("file://{}", releases.display()),
        )
        .output()
        .expect("run installer");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let installed = install_dir.join("jottrace");
    assert!(
        installed.exists(),
        "installer should create ~/.local/bin/jottrace"
    );
    #[cfg(unix)]
    assert_ne!(
        fs::metadata(&installed).expect("installed metadata").mode() & 0o111,
        0,
        "installed jottrace should be executable"
    );

    let version = Command::new(&installed)
        .arg("--version")
        .output()
        .expect("run installed jottrace");
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "jottrace 26.5.0"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn installer_prints_path_hint_without_editing_shell_rc_files() {
    let Some(target) = current_release_target() else {
        eprintln!("skipping installer hint test on unsupported test host");
        return;
    };

    let root = temp_root("installer-path-hint");
    let home = root.join("home");
    let releases = root.join("releases");
    let tag = "vtest";
    fs::create_dir_all(&home).expect("create home");
    fs::write(home.join(".zshrc"), "# existing zsh config\n").expect("write zshrc");
    fs::write(home.join(".bashrc"), "# existing bash config\n").expect("write bashrc");
    create_release_artifact(
        &root,
        &releases,
        tag,
        target,
        "26.5.0",
        "installer test binary",
    );

    let output = Command::new("bash")
        .arg("install.sh")
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("JOTTRACE_VERSION", tag)
        .env(
            "JOTTRACE_RELEASE_BASE_URL",
            format!("file://{}", releases.display()),
        )
        .output()
        .expect("run installer");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("export PATH=\"$HOME/.local/bin:$PATH\""));
    assert_eq!(
        fs::read_to_string(home.join(".zshrc")).expect("read zshrc"),
        "# existing zsh config\n"
    );
    assert_eq!(
        fs::read_to_string(home.join(".bashrc")).expect("read bashrc"),
        "# existing bash config\n"
    );

    let _ = fs::remove_dir_all(root);
}
