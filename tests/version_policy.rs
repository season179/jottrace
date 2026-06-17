use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn cargo_version_matches_changelog_head() {
    let manifest = fs::read_to_string("Cargo.toml").expect("read Cargo.toml");
    let cargo_version = manifest
        .lines()
        .find_map(|line| line.strip_prefix("version = \"").and_then(|rest| rest.strip_suffix('"')))
        .expect("Cargo.toml package version");

    let changelog = fs::read_to_string("CHANGELOG.md").expect("read CHANGELOG.md");
    let changelog_version = changelog
        .lines()
        .find_map(|line| {
            line.strip_prefix("## v")
                .and_then(|rest| rest.split_whitespace().next())
        })
        .expect("CHANGELOG head version");

    assert_eq!(
        cargo_version, changelog_version,
        "Cargo.toml version must match the top CHANGELOG entry"
    );
}

#[test]
fn version_policy_accepts_yy_month_patch_and_matching_tag() {
    let root = temp_root("version-valid");
    let manifest = root.join("Cargo.toml");
    fs::create_dir_all(&root).expect("create temp root");
    fs::write(&manifest, manifest_with_version("26.5.0")).expect("write manifest");

    let output = Command::new("bash")
        .arg("scripts/check-version.sh")
        .arg("v26.5.0")
        .env("JOTTRACE_CARGO_TOML", &manifest)
        .output()
        .expect("run version check");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn version_policy_rejects_four_numeric_segments() {
    let root = temp_root("version-four-segments");
    let manifest = root.join("Cargo.toml");
    fs::create_dir_all(&root).expect("create temp root");
    fs::write(&manifest, manifest_with_version("26.5.5.0")).expect("write manifest");

    let output = Command::new("bash")
        .arg("scripts/check-version.sh")
        .env("JOTTRACE_CARGO_TOML", &manifest)
        .output()
        .expect("run version check");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("YY.M.PATCH"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn version_policy_rejects_tag_that_does_not_match_cargo_version() {
    let root = temp_root("version-tag-mismatch");
    let manifest = root.join("Cargo.toml");
    fs::create_dir_all(&root).expect("create temp root");
    fs::write(&manifest, manifest_with_version("26.5.0")).expect("write manifest");

    let output = Command::new("bash")
        .arg("scripts/check-version.sh")
        .arg("v26.5.1")
        .env("JOTTRACE_CARGO_TOML", &manifest)
        .output()
        .expect("run version check");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must match"));

    let _ = fs::remove_dir_all(root);
}

fn manifest_with_version(version: &str) -> String {
    format!("[package]\nname = \"jottrace\"\nversion = \"{version}\"\nedition = \"2024\"\n")
}

fn temp_root(name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("jottrace-{name}-{}-{unique}", std::process::id()))
}
