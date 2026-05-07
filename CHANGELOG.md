# Changelog

## v26.5.8 - 2026-05-07

### Summary

- Adds `skipped_files` to the ingest report so unchanged source files are
  visible at a glance. Changes since `v26.5.7`.

### Changes

- `jottrace ingest` now prints `skipped_files:` in its output, showing how
  many source files were checked but produced no new inserts. This makes
  idempotent reruns (especially large Hermes session lists) clearly
  distinguishable from passes that actually imported new data.

### Commits

- Add skipped_files to ingest report (c99c772)

### Verification


## v26.5.7 - 2026-05-07

### Summary

- Fixes ingest recovery for previously unresolved source-file errors. Changes
  since `v26.5.6`.

### Changes

- `jottrace ingest` now resolves prior `invalid_json` rows after all committed
  JSONL lines parse successfully, including when a new unterminated tail is
  still waiting for a newline.
- Unchanged corrupt JSONL files keep their existing unresolved error without
  re-recording the same failure on every ingest run.
- Claude local-agent audit ingestion now handles the live `_audit_timestamp`
  shape and numeric metadata timestamps.
- Claude local-agent audit files with oversized first records fall back to the
  path-derived session identity, while short uncommitted headers remain visible
  as unresolved ingest errors and self-heal when committed.
- Updated Claude local-agent fixtures to match the live metadata and audit
  shapes.

### Commits

- Fix ingest error recovery (b582ddc)

### Verification

- `scripts/check-version.sh v26.5.7`
- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `bash -n install.sh scripts/check-version.sh`
- `cargo test`
- `jottrace ingest` live verification with `unresolved_ingest_errors: 0`
- Release preflight script

## v26.5.6 - 2026-05-07

### Summary

- Expands Jottrace's preserved source coverage across local agent tools and
  tightens the CLI surface for daily use. Changes since `v26.5.5`.

### Changes

- Added reader support for Pi agent JSONL sessions, Claude local-agent audit
  JSONL files, Gemini CLI chat JSON files, Factory / Droid-style JSONL sessions,
  OpenCode SQLite sessions, and Hermes SQLite SessionDB rows.
- Added fixture-backed reader documentation and inventory updates that make
  ignored source boundaries explicit.
- `jottrace -h` and command help are now easier to discover from the CLI.
- CLI output for high-frequency commands is compact by default, with
  `--details` preserving the longer diagnostic output.
- Background auto-update checks can now be disabled with
  `JOTTRACE_AUTO_UPDATE=0` or persistent config.
- Codex ingest handles legacy edge cases more reliably.
- Updated README release examples for `v26.5.6`.

### Commits

- Add Hermes SQLite SessionDB reader (6129e1f)
- Make CLI output compact by default (31ef71a)
- Add opt-out background auto-update (65f01e9)
- Capture OpenCode SQLite sessions (7b4c0ea)
- Add Factory JSONL ingest (a2e8bbc)
- Capture Gemini CLI chat sessions (f7b6ebb)
- Capture Claude local-agent audit sessions (6230e62)
- Capture Pi agent sessions (0122251)
- Document ignored reader source boundaries (21ea342)
- Make CLI help discoverable (a3e8fe0)
- Add OpenCode reader fixture shape (f697e19)
- Handle legacy Codex ingest edge cases (96ec5d9)
- Document reader fixture gate (3913051)

### Verification

- `scripts/check-version.sh v26.5.6`
- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `bash -n install.sh scripts/check-version.sh`
- `cargo test`
- Release preflight script

## v26.5.5 - 2026-05-06

### Summary

- Adds Codex CLI session ingestion and a first-class installed-binary update
  command. Changes since `v26.5.4`.

### Changes

- `jottrace ingest` now discovers Codex CLI session files under `.codex` and
  `.codex-local`, including archived sessions.
- Codex sessions use the committed `session_meta.payload.id` as their stable
  source session id, so moved archived files do not become duplicate sessions.
- Invalid Codex session metadata is recorded as a non-blocking ingest error so
  unrelated source files still import.
- Added `jottrace update`, with `jottrace upgrade` as an alias, to replace the
  installed binary from GitHub Release artifacts.
- Update failures leave the existing installed binary usable when the download
  or release artifact is missing or invalid.
- Updated README release examples and ingest documentation for `v26.5.5`.

### Commits

- Add first-class update command (07999dd)
- Add Codex CLI session ingest (0a9d9b7)

### Verification

- `scripts/check-version.sh v26.5.5`
- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `bash -n install.sh scripts/check-version.sh`
- `cargo test`
- Release preflight script

## v26.5.3 - 2026-05-06

Changes since `v26.5.2`.

### Added

- Added `jottrace web`, a read-only local web UI bound to `127.0.0.1` for
  browsing the preserved SQLite journal.
- Added session browsing with source, session id, cwd/path metadata, event
  counts, timestamps, and parent-session context.
- Added selected-session event views with raw payload previews and payload
  size/codec details.
- Added journal search across source, session id, cwd, file path, and visible
  raw payload text, with SQL wildcard characters treated as literal input.
- Added unresolved ingest-error rendering in the web UI so broken source files
  are visible next to the preserved journal data.
- Added `jottrace web --port <port>` for fixed-port local serving and
  `jottrace web --once` for scripted smoke checks.
- Added web UI tests covering journal loading, HTML rendering, local HTTP
  serving, CLI startup output, source-qualified session selection, payload
  search, literal wildcard search, and ingest-error display.

### Changed

- Documented `jottrace web` in the README and architecture design notes.
- Moved the raw payload codec constant into storage so ingestion and web
  rendering share the same codec name.
- Updated the release workflow to publish the matching changelog entry as the
  GitHub Release notes.
