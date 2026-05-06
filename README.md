# Jottrace

Jottrace preserves local AI coding-session transcripts into a private SQLite
journal you control.

Today it can ingest Claude CLI / Claude Code and Codex CLI JSONL session files,
plus Gemini CLI chat JSON files. It also preserves Claude subagent sidechain
sessions and reports the local journal status from the command line. Cursor,
OpenCode, and other readers are tracked in the design docs, but are not
implemented as user-facing ingest sources yet.

## Features

- Install a standalone `jottrace` binary into `~/.local/bin`.
- Keep local state in `~/.jottrace/db.sqlite`, or another directory via
  `JOTTRACE_HOME`.
- Check the data directory and database with `jottrace doctor`.
- Ingest Claude, Codex, and Gemini session files with `jottrace ingest`.
- Inspect stored session, event, schema, and ingest-error counts with
  `jottrace status`.
- Preserve Claude subagent sidechains as distinct child sessions linked to the
  parent session.
- Resume incremental JSONL imports, defer partial trailing lines, and preserve
  rewritten or truncated source files as new generations.
- Record corrupt or unreadable source-file errors without blocking unrelated
  sessions.
- Browse preserved sessions, events, and unresolved ingest errors locally with
  `jottrace web`.
- Update the installed binary from GitHub Releases with `jottrace update`.

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
jottrace -h
jottrace --version
jottrace doctor
jottrace ingest
jottrace status
jottrace web
```

`jottrace -h` is the fastest CLI discovery path. It prints the top-level
commands and points to `jottrace <command> --help` for command-specific usage.

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

It also scans these Codex install directories under `HOME`:

- `~/.codex`
- `~/.codex-local`

For Codex, Jottrace reads session files under `sessions/` and
`archived_sessions/`, using each file's committed `session_meta` id as the
stable session id.

For Gemini CLI, Jottrace reads chat JSON files under
`~/.gemini/tmp/<project-or-hash>/chats/*.json`, using each file's `sessionId`
as the stable session id and preserving the ordered `messages[]` entries.

The ingest command stores raw source event/message payloads and cheap
deterministic session metadata, then prints the database path, discovered file
count, total session/event counts, inserted event count, and unresolved
ingest-error count.

`jottrace status` initializes `~/.jottrace/db.sqlite` if needed and reports
the schema version plus session, event, and unresolved ingest-error counts.

`jottrace web` starts a read-only web UI bound to `127.0.0.1`, prints the URL
and database path, and serves data from the existing SQLite journal. The UI lets
you filter sessions by source, cwd/path metadata, session id, and visible raw
payload text; selecting a session shows its preserved events and payload
previews. Unresolved ingest errors are shown on the page to help diagnose broken
source files.

The web UI is local-only. It does not mutate source files, delete journal rows,
or send transcript data to external services. To request a fixed port instead
of an available OS-assigned port:

```sh
jottrace web --port 7421
```

For scripted smoke checks, `jottrace web --once` serves one request and exits.

To use a different journal directory:

```sh
JOTTRACE_HOME=/path/to/private/journal jottrace ingest
JOTTRACE_HOME=/path/to/private/journal jottrace status
JOTTRACE_HOME=/path/to/private/journal jottrace web
```

## Update

To update the installed binary in place:

```sh
jottrace update
```

`jottrace upgrade` is supported as an alias. The command downloads the matching
GitHub Release artifact for your OS and CPU, reports the current version,
target version, install path, and final result. When a newer artifact is
available, it replaces only the installed binary. It does not read or mutate
data under `~/.jottrace`.

For deterministic release testing, `jottrace update` honors the same
`JOTTRACE_VERSION` and `JOTTRACE_RELEASE_BASE_URL` controls as `install.sh`.

If the update command is unavailable or fails before replacing the binary,
rerun the installer fallback:

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
cargo run -- web
```

## Maintainer Release

Jottrace uses CalVer in `YY.M.PATCH` form. For example, the first release in
May 2026 is `v26.5.0`; later releases in the same month increment the patch
segment, such as `v26.5.5`.

`scripts/check-version.sh` enforces that `Cargo.toml` uses this shape and that
release tags match the Cargo package version.

The release workflow runs on version tags and publishes the artifacts consumed
by `install.sh`.

```sh
git checkout main
git pull --ff-only
git tag v26.5.5
git push origin v26.5.5
```

After the `Release` GitHub Action finishes, the install command above should
work end to end.
