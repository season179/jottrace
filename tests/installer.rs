use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[test]
fn installer_downloads_matching_release_artifact_to_local_bin() {
    let Some(target) = current_installer_target() else {
        eprintln!("skipping installer smoke test on unsupported test host");
        return;
    };

    let root = temp_root("installer-smoke");
    let home = root.join("home");
    let releases = root.join("releases");
    let tag = "vtest";
    fs::create_dir_all(&home).expect("create home");
    create_release_artifact(&root, &releases, tag, target);

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
        "jottrace 0.1.0"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn installer_prints_path_hint_without_editing_shell_rc_files() {
    let Some(target) = current_installer_target() else {
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
    create_release_artifact(&root, &releases, tag, target);

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

fn create_release_artifact(
    root: &std::path::Path,
    releases: &std::path::Path,
    tag: &str,
    target: &str,
) {
    let release_dir = releases.join(tag);
    let payload_dir = root.join(format!("payload-{target}"));
    fs::create_dir_all(&release_dir).expect("create release dir");
    fs::create_dir_all(&payload_dir).expect("create payload dir");

    let fake_binary = payload_dir.join("jottrace");
    fs::write(
        &fake_binary,
        "#!/usr/bin/env sh\nif [ \"$1\" = \"--version\" ]; then echo 'jottrace 0.1.0'; else exit 64; fi\n",
    )
    .expect("write fake binary");
    #[cfg(unix)]
    fs::set_permissions(&fake_binary, fs::Permissions::from_mode(0o755))
        .expect("chmod fake binary");

    let artifact = release_dir.join(format!("jottrace-{target}.tar.gz"));
    let tar_status = Command::new("tar")
        .args(["-C", payload_dir.to_str().expect("utf8 payload path")])
        .args(["-czf", artifact.to_str().expect("utf8 artifact path")])
        .arg("jottrace")
        .status()
        .expect("create release artifact");
    assert!(tar_status.success(), "tar should create test artifact");
}

fn current_installer_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-x86_64"),
        _ => None,
    }
}

fn temp_root(name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id()))
}
