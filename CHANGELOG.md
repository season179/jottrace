# Changelog

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
