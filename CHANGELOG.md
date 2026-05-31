# Changelog

## v26.5.13 - 2026-05-31

### Summary

- Fixes two ingest bugs in how JSONL sessions resume across re-ingests, both
  surfaced by Claude Code workflow runner journals
  (`subagents/workflows/<wf-id>/journal.jsonl`). A rewritten file could log a
  permanent `invalid_json` error and stop ingesting, and two such journals could
  clobber the same session forever. Changes since `v26.5.12`.

### Changes

- `ingest` no longer resumes a JSONL session from its stored byte offset without
  verifying that the already-consumed bytes are unchanged. A new
  `prefix_fingerprint` column (migration `009`) records a hash of the consumed
  prefix; when it no longer matches, the file was rewritten rather than appended
  to, so the session is re-read from the start as a new generation instead of
  parsing from a now mid-record offset. Previously a rewrite left
  `next_read_offset` pointing inside a record, which logged an
  `invalid_json` error that never resolved and blocked the rest of the file.
- Sessions written before migration `009` have no stored prefix fingerprint and
  fall back to a structural check: a resume offset is trusted only when it sits
  immediately after a newline, the single position the importer ever records.
- Files nested under a session's `subagents/` tree that are not named `agent-*`
  (notably the workflow runner journals) now derive a path-qualified source
  session id of the form `<session-uuid>/subagents/<...>/journal` and link to
  their owning session. Previously every `journal.jsonl` collapsed to the bare
  id `journal`, so two journals in different workflow directories shared one
  session row and overwrote each other on every ingest, growing the session by
  two generations per run.

## v26.5.12 - 2026-05-18

### Summary

- Adds `pack` and `settle` so users can move a journal between machines
  without manually shelling around the SQLite WAL or restoring private file
  permissions. Changes since `v26.5.11`.

### Changes

- `jottrace pack` writes `~/.jottrace` into a `jottrace-pack-<utc>.tar.gz`
  archive after running `PRAGMA wal_checkpoint(TRUNCATE)`, holding the same
  lock as `ingest`/`compact` so concurrent writers cannot tear the snapshot.
  The output file is created with mode `0600` and never overwrites an existing
  path.
- `jottrace settle <archive>` extracts a pack archive into `JOTTRACE_HOME`,
  re-applies `0700`/`0600` permissions, and opens the database so schema
  migrations run before the receiving machine ingests again. Refuses to
  overwrite an existing non-empty journal unless `--force` is provided.
- The runtime lock file and auto-update sentinel are excluded from packed
  archives so they cannot follow a journal to a new host.
- `settle` rejects archives that contain symbolic links or other non-regular
  files. Following a crafted symlink during the post-extract chmod walk would
  otherwise re-permission files outside `JOTTRACE_HOME`.
- `settle --force` clears existing journal files (except the held lock) before
  extracting, so stale SQLite sidecars from the previous journal cannot
  survive next to the restored `db.sqlite`.
- `pack` claims its output path with an atomic `O_EXCL` open at mode `0600`,
  removing the small window where the archive existed with the process umask
  applied before the trailing chmod.
- `settle` extracts every archive into a `.pending-settle` staging
  subdirectory and only promotes the validated contents into the live journal
  after `enforce_private_permissions` clears them. A crafted symlink, a
  truncated tarball, a tar error halfway through, or any other extraction
  failure now leaves the previous journal byte-identical instead of partially
  overwritten.
- `settle` re-checks the journal-non-empty guard after acquiring the data
  lock, so a racing `ingest` cannot create a fresh journal between the empty
  check and the lock acquisition and have it silently overwritten.
- `pack` removes the claimed output file if it fails after creation (lock
  contention, DB open error, tar failure), so a retry with the same `--output`
  succeeds without manual cleanup of a zero-byte archive.
- `settle` refuses archives whose canonical path lives under
  `JOTTRACE_HOME`. `--force` would otherwise delete the archive in
  `clear_journal_contents` before tar could read it, silently destroying a
  user's local backup.
- `settle` validates the staged `db.sqlite` (existence and `user_version` in
  the supported range) before clearing the live journal. A valid tarball
  without `db.sqlite`, or one carrying a zero-byte or non-Jottrace
  `db.sqlite`, used to pass extraction cleanly and then be silently replaced
  by the fresh empty database `storage::status_for_path` opens at the end of
  settle — now it fails fast and the existing journal is left byte-identical.
- `settle` inspects the archive's table of contents (via `tar -tvzf` and
  `tar -tzf`) before invoking tar in extract mode. Symbolic links, hard
  links, device nodes, absolute paths, and `..` segments are now rejected up
  front so a crafted symlink-prefix archive cannot land bytes outside the
  staging subtree before the post-extract walk has a chance to refuse it.
  `.pending-settle` is never even created on the rejection path.
- `settle` also confirms the staged `db.sqlite` exposes the foundational
  Jottrace schema (column-aware probe queries against `sessions`, `events`,
  and `ingest_errors`) before clearing the live journal. Without this check,
  a SQLite file from an unrelated application that happens to have a
  `user_version` in Jottrace's accepted range — or that exposes tables with
  the right names but unrelated columns — used to pass validation and only
  fail later, after `clear_journal_contents` had already wiped the user's
  data.
- `pack` refuses to produce an archive when the source journal has no
  `db.sqlite`. The producer used to advertise success on directories that
  had been auto-created without an ingest, leaving the user with an archive
  `settle` would later reject as `ArchiveMissingDatabase`.
- `pack` excludes any leftover `.pending-settle` staging directory from the
  archive. A previous settle that crashed mid-flight could otherwise leave
  one behind; packing it would yield an archive whose own settle creates
  `.pending-settle/.pending-settle` and breaks the staged-rename step
  after `clear_journal_contents` had already wiped the receiving journal.
- `settle` rejects archives whose top-level path collides with a runtime
  artefact of the live journal — `.pending-settle/`, `jottrace.lock`, or
  either auto-update sentinel — before extraction. A top-level
  `jottrace.lock` entry was particularly dangerous: promoting it would
  either fail outright after `clear_journal_contents` had already wiped the
  live data, or replace the inode the held data lock was flocking and break
  mutual exclusion with concurrent `ingest`/`compact`. Such entries are now
  refused up front under "reserved top-level entry".
- `settle` now brings the staged database up to `LATEST_SCHEMA_VERSION`
  via the standard migration runner before probing required tables, and the
  probe set covers every column the current code reads or writes —
  including v1 columns like `sessions.file_size`, `sessions.file_mtime`,
  `sessions.content_fingerprint`, `sessions.last_read_at`, and
  `sessions.updated_at` that a crafted archive could omit alongside a
  Jottrace-shaped `user_version`. An archive claiming `user_version =
  LATEST` while missing any column used by ingest/compact/web used to wave
  through and only fail later, after the live journal had been overwritten.
- `settle`'s non-empty guard now ignores runtime sentinels
  (`auto-update-check` and `auto-update-check.lock`) the way it already
  ignored the held `jottrace.lock`. On installer-managed binaries
  `maybe_spawn_auto_update` can plant the stamp into a freshly created
  `JOTTRACE_HOME` before any ingest runs, which used to make the first
  settle on the new machine refuse without `--force` for no real reason.
- `pack` refuses an `--output` path that resolves inside the source journal.
  tar's `-C data_dir .` walk would otherwise see the partially-written
  archive in its own input and race SQLite sidecars on names like
  `db.sqlite-wal`, producing a self-referential, truncated archive.
- `settle` verifies that the staged database carries the unique
  `idx_sessions_source_session_id` index and the `events` PRIMARY KEY
  before promoting it. A crafted or partially migrated archive with the
  right columns but no uniqueness constraint used to wave through; the
  receiving machine's `INSERT OR IGNORE` would then silently degrade to a
  regular insert and grow duplicate session rows on subsequent ingests.

## v26.5.11 - 2026-05-15

### Summary

- Replaces the brittle DB-mutating command sentinel with OS-level locking on
  Unix, so stale `jottrace.lock` metadata no longer blocks future ingests after
  an interrupted process. Changes since `v26.5.10`.

### Changes

- `ingest`, `compact --apply`, and `compact --vacuum` now hold a non-blocking
  Unix `flock` on `~/.jottrace/jottrace.lock`; the file contents are diagnostic
  metadata rather than the authoritative lock state.
- Same-process callers are also guarded by an in-process path lock so
  multi-threaded use preserves the old atomic single-writer behavior even on
  platforms with different duplicate-lock semantics.
- Stale lock metadata is overwritten once the OS lock is acquired, and clean
  shutdown removes metadata only when it still belongs to the current process.
- Non-Unix builds keep the previous atomic `create_new` lock-file behavior
  instead of failing with an unsupported-platform error.

### Commits

- Add OS-level data lock recovery for DB-mutating commands

### Verification

- `cargo fmt --check`
- `cargo test acquire_data_lock`
- `cargo test ingest_reports_lock_contention_as_clear_cli_failure`
- `cargo run -- ingest` live verification with `unresolved_ingest_errors: 0`

## v26.5.10 - 2026-05-10

### Summary

- Restores Pi agent ingestion for nested subagent run sessions, which were
  previously rejected with `invalid_session_meta` errors. Changes since
  `v26.5.9`.

### Changes

- Pi agent discovery now recognises nested subagent runs at
  `~/.pi/agent/sessions/<encoded-cwd>/<timestamp>_<parent-session-id>/<short>/run-N/session.jsonl`
  and resolves their session id from the first JSONL `session` event instead
  of the literal `session` filename stem.
- Nested run sessions are linked to their parent session via the parent UUID
  extracted from the run directory's grandparent, and pi-agent files are now
  ingested in parent-first order so linkage resolves on the first pass.
- Existing `invalid_session_meta` ingest errors recorded against nested
  Pi agent run files self-resolve on the next successful ingest of each
  affected session.
- Reader source inventory documents the nested run shape alongside the
  top-level layout.

### Commits

- Ingest pi-agent nested subagent run sessions (c629f3b)

## v26.5.9 - 2026-05-09

### Summary

- Fixes OpenCode SQLite ingestion after upstream renamed the per-session entry
  table to `session_message`. Changes since `v26.5.8`.

### Changes

- The OpenCode reader now queries `session_message` instead of the removed
  `session_entry` table, restoring ingestion for opencode databases on the
  current schema (drizzle migration `20260312043431_session_message_cursor`
  and later).
- OpenCode reader fixture, fixture corpus, CLI test expectations, and the
  reader design notes have been updated to match the renamed table.
- Existing `invalid_session_meta` ingest errors recorded against the missing
  `session_entry` table self-resolve on the next successful ingest of each
  affected session.

### Commits

- Rename OpenCode session_entry to session_message (ebcdd66)

### Verification

- `scripts/check-version.sh v26.5.9`
- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `jottrace ingest` live verification with `unresolved_ingest_errors: 0`

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
