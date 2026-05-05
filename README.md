# Jottrace

Jottrace preserves local AI coding-session transcripts into a private SQLite
journal you control.

Today it can ingest Claude CLI / Claude Code JSONL session files, including
subagent sidechain sessions, and report the local journal status from the
command line. Codex, Cursor, OpenCode, and other readers are tracked in the
design docs, but are not implemented as user-facing ingest sources yet.

## Features

- Install a standalone `jottrace` binary into `~/.local/bin`.
- Keep local state in `~/.jottrace/db.sqlite`, or another directory via
  `JOTTRACE_HOME`.
- Check the data directory and database with `jottrace doctor`.
- Ingest Claude session JSONL with `jottrace ingest`.
- Inspect stored session, event, schema, and ingest-error counts with
  `jottrace status`.
- Preserve Claude subagent sidechains as distinct child sessions linked to the
  parent session.
- Resume incremental JSONL imports, defer partial trailing lines, and preserve
  rewritten or truncated source files as new generations.
- Record corrupt or unreadable source-file errors without blocking unrelated
  sessions.

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
jottrace ingest
jottrace status
```

`jottrace doctor` creates or checks the local data directory at `~/.jottrace`
and reports whether its permissions are private. On Unix systems, the directory
is expected to use mode `0700`; the SQLite database is expected to use mode
`0600`. It also reports unresolved ingest errors and shows recent error details
when any exist.

`jottrace ingest` scans these Claude install directories under `HOME`:

- `~/.claude`
- `~/.claude-code`
- `~/.claude-local`
- `~/.claude-m2`
- `~/.claude-zai`

For each install directory, Jottrace reads Claude project session files under
`projects/` plus UUID-named flat-root JSONL session files. Source files are
read-only inputs: Jottrace does not move, edit, or delete Claude artifacts.

The ingest command stores raw JSONL event payloads and cheap deterministic
session metadata, then prints the database path, discovered file count, total
session/event counts, inserted event count, and unresolved ingest-error count.

`jottrace status` initializes `~/.jottrace/db.sqlite` if needed and reports
the schema version plus session, event, and unresolved ingest-error counts.

To use a different journal directory:

```sh
JOTTRACE_HOME=/path/to/private/journal jottrace ingest
JOTTRACE_HOME=/path/to/private/journal jottrace status
```

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
cargo run -- ingest
cargo run -- status
```

## Maintainer Release

Jottrace uses CalVer in `YY.M.PATCH` form. For example, the first release in
May 2026 is `v26.5.0`; later releases in the same month increment the patch
segment, such as `v26.5.2`.

`scripts/check-version.sh` enforces that `Cargo.toml` uses this shape and that
release tags match the Cargo package version.

The release workflow runs on version tags and publishes the artifacts consumed
by `install.sh`.

```sh
git checkout main
git pull --ff-only
git tag v26.5.2
git push origin v26.5.2
```

After the `Release` GitHub Action finishes, the install command above should
work end to end.
