# Jottrace

Jottrace preserves local AI coding-session transcripts into a journal you
control. The first CLI surface is intentionally small: install the binary,
check the version, and run `doctor` to verify the local data directory is
private enough for transcript data.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/season179/jottrace/main/install.sh | bash
```

The installer detects your OS and CPU architecture, downloads the matching
GitHub Release artifact, and installs `jottrace` to:

```sh
~/.local/bin/jottrace
```

If `~/.local/bin` is not on your `PATH`, the installer prints a shell snippet
you can paste into your shell rc file. It does not edit shell configuration
files automatically.

The installer needs a published GitHub Release to download from. If no release
has been published yet, the script is present but installation will fail at the
download step.

## Verify

After installing, run:

```sh
jottrace --version
jottrace doctor
```

`jottrace doctor` creates or checks the local data directory at `~/.jottrace`
and reports whether its permissions are private. On Unix systems, the directory
is expected to use mode `0700`.

## Update

To update Jottrace, rerun the installer:

```sh
curl -fsSL https://raw.githubusercontent.com/season179/jottrace/main/install.sh | bash
```

## Build From Source

```sh
cargo build
cargo test
```

Run the development binary with:

```sh
cargo run -- doctor
```

## Maintainer Release

Jottrace uses CalVer in `YY.M.PATCH` form. For example, the first release in
May 2026 is `v26.5.0`; later releases in the same month increment the patch
segment, such as `v26.5.1`.

`scripts/check-version.sh` enforces that `Cargo.toml` uses this shape and that
release tags match the Cargo package version.

The release workflow runs on version tags and publishes the artifacts consumed
by `install.sh`.

```sh
git checkout main
git pull --ff-only
git tag v26.5.0
git push origin v26.5.0
```

After the `Release` GitHub Action finishes, the install command above should
work end to end.
