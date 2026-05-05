---
title: "Jottrace architecture"
status: DRAFT
updated: 2026-05-05
repo: github.com/season179/jottrace
---

# Jottrace

Local tool that turns AI coding-session transcripts (Claude CLI, Codex,
Cursor, OpenCode, and others) into a queryable journal.

This document is the source of truth for the current architecture. Earlier
v3.x designs (office-hours / grill-with-docs era) were scrapped on
2026-05-03; recoverable from git history if needed.

## Why this exists

AI coding-session files live outside our control. The host tools own them:
they can be rotated, compacted, or deleted by a future tool version, by an
OS reinstall, or by the user themselves. Jottrace's first job is
**preservation** — extract the raw event stream from each AI tool's
session files and store it in a place the user controls
(`~/.jottrace/db.sqlite`) before the source ever vanishes.

The journal use case (recall, daily/topic notes, search) is the eventual
payoff. Preservation is what justifies running jottrace tonight.

## Stages

The system has **four stages**: reader, processor, writer, and a
scheduler that orchestrates the other three. We use the word **stage**
because the units form a directional flow; "module" would describe their
code shape but not their role. The arrow diagram below shows only the
three data-flow stages; the scheduler is not in the data flow, it
triggers the others.

```
[reader] -> [processor] -> [writer]
              ^              ^
              |              |
            scheduler triggers all three
```

1. **Reader** — reads sessions from disk and stores them in SQLite in a
   deterministic way. There are many reader variants (Claude CLI, Codex
   CLI, Cursor, OpenCode, ...). Each reader handles its own source
   format and its own deduplication. Readers do **not** interpret
   content; they store the full event stream losslessly plus cheap
   metadata.

2. **Processor** — an LLM agent that takes the unstructured event content
   the reader stored and produces structured output (summaries, topics,
   decisions, dead ends, etc.). The session shape is deterministic; the
   *content* isn't — that's the processor's job.

3. **Writer** — takes processed content and writes it to a configured path
   (Markdown files in a vault, etc.).

4. **Scheduler** — orchestrates reader, processor, and writer runs.

The Claude CLI reader is the first one to ship. Codex CLI follows
immediately after. Cursor and OpenCode are designed at stub level so the
schema decisions they expose (in-place SQLite edits, JSON-file iteration
order) are not surprises later.

## Implementation

- **Language: Rust.** Chosen for a single static binary, predictable
  performance on large JSONL files, and a strong SQLite ecosystem.
- **Storage:** SQLite at `~/.jottrace/db.sqlite`. The DB is the contract
  between stages, not in-process function calls.
- **MVP command surface:** `jottrace ingest`, `jottrace status`, and
  `jottrace doctor`.
- Specific crate choices (rusqlite vs. sqlx, sync vs. async, CLI framework,
  config loader) are not yet decided — see Open seams.

### Commands

The first executable surface is intentionally small:

- `jottrace ingest` — runs reader ingestion for configured or discovered
  sources and preserves new source events into SQLite.
- `jottrace status` — reports session counts, event counts, and unresolved
  ingest errors from the local DB.
- `jottrace doctor` — checks the install, data directory permissions,
  source path visibility, and basic runtime health.

Processor, writer, scheduler, and recall commands come later.

## Distribution

Single static binary, installed via a shell script piped to bash, matching
the Claude Code UX:

    curl -fsSL https://raw.githubusercontent.com/season179/jottrace/main/install.sh | bash

The `install.sh` script is hosted directly from the GitHub repo (raw URL in
MVP; a custom domain like `install.jottrace.dev` is a notational improvement,
not a prerequisite). It detects OS+arch, downloads the matching binary from
GitHub Releases, and places it at `~/.local/bin/jottrace`. If
`~/.local/bin` is not on PATH, the script prints a one-liner the user can
paste into their shell rc; it does not edit shell configs automatically.

Cargo, npm, and Homebrew are not the primary install path. They may be added
later as alternates, but they are not what we optimize for.

Implications:

- **GitHub Releases is the binary store.** Every tagged release publishes
  per-platform artifacts. MVP target list: darwin-arm64, darwin-x86_64,
  linux-x86_64. Linux arm64 is best-effort. Windows is out of scope.
- **CI builds and ships releases on tag push.** GitHub Actions with a
  platform build matrix. No manual release step. `cargo-dist` can own
  release artifacts, checksums, and release CI underneath the public
  `install.sh` UX; it is plumbing, not the user-facing install contract.
- **The binary must be self-contained.** SQLite is statically linked (e.g.,
  `rusqlite` with the `bundled` feature). No runtime library dependencies.
- **Update path is "re-run the installer."** No `jottrace update`
  self-replace command in MVP.
- **Data dir is separate from binary path.** Binary at
  `~/.local/bin/jottrace`; data at `~/.jottrace/` (DB, single-instance
  lockfile, future config). The two are independent.

Real-world distribution caveats (macOS code signing, Linux glibc
compatibility, install.sh checksum verification) are tracked in Open
seams. They are not blocking the architecture; they need a conscious
choice before the first tag is cut.

Release CI includes an installer smoke test: install from the staged or
published release artifacts, then assert `jottrace --version` runs.

## Local data, permissions, and locking

The data directory contains private AI transcript history, so it is private
by default:

- `~/.jottrace/` is created with mode `0700` on Unix.
- DB, lock, and future config files are created with mode `0600` on Unix.
- Permission behavior is covered by tests on Unix targets.

DB-mutating commands acquire one single-instance process lock before running
migrations, ingest, processor, or writer work. Read-only commands can skip
the lock. SQLite's write lock still protects the DB file, but the process
lock gives clearer contention errors and protects source-scan/offset
decisions from racing.

## Reader contract

Every reader, regardless of source, implements the same six
responsibilities. This contract is what makes the architecture's
"many readers" claim honest.

1. **Discover.** Probe known on-disk locations for session files
   belonging to its source. Each reader knows its source's path
   conventions and any alternate installation directories.

2. **Identify.** Derive a stable source session id per session. The DB has
   its own internal `sessions.id`; source-level dedup is enforced by
   `UNIQUE(source, source_session_id)`. The source id must be stable across
   re-runs (same source artifact → same id), so dedup works.

3. **Detect change.** A cheap signal that says "look closer at this
   session." For append-only file sources (JSONL): `(file_size,
   file_mtime)`, with a cheap content fingerprint checked when `mtime`
   changes but size does not. For in-place-edited sources (SQLite): a
   content hash on the session's row set, or a per-message updated_at
   timestamp.

4. **Read.** Produce the event stream for one session, ordered by an
   opaque per-source `seq` key. For JSONL sources, `seq` is the line
   number. For SQLite sources, `seq` is the rowid or a stable index
   derived from the source's natural ordering. For JSON-file sources,
   `seq` is the iteration order over the directory or the file's
   internal array.

5. **Dedup.** Idempotent at event grain. PK = `(session_id, generation,
   seq)`. A re-read that sees no new events writes nothing. A re-read that
   sees N new events writes exactly N rows. (We rejected session-level
   dedup — replace-the-row-on-each-run — because it forces the processor to
   redo work and loses event-level history.)

6. **Stay safe.** Never write to, move, or delete source files. The DB
   row outlives the source: if a session's source file disappears, the
   stored events stay valid, and no DB row is deleted in response.

Readers do **not** interpret content. They preserve raw source events plus
cheap deterministic metadata. Anything that requires meaning extraction
(topics, decisions, dead ends, summaries) is processor work. The reader stays
dumb on purpose.

### Shared JSONL ingest core

Claude CLI and Codex CLI are different source adapters over the same JSONL
ingest core. The adapters own discovery, source session id extraction, and
source-specific metadata. The shared core owns the dangerous state machine:
bounded reads, newline handling, offsets, generations, fingerprint checks,
compression, transaction boundaries, ingest errors, and idempotent inserts.

Each file import runs in one SQLite transaction with prepared statements.
Event rows and session metadata commit together. If a line cannot be
preserved (invalid JSON, invalid UTF-8 where the source requires UTF-8, too
large for configured limits, compression failure, etc.), the reader records a
visible ingest error, does not advance `next_read_offset` past unpreserved
data, and continues scanning unrelated files.

SQLite uses boring local pragmas suitable for a single-user CLI, including a
busy timeout and WAL mode unless a target platform forces a different choice.

### Migrations

Schema changes are explicit SQL migrations embedded in the binary and tracked
with SQLite `PRAGMA user_version`. Migrations run transactionally before
DB-mutating commands. Tests cover both a fresh DB and upgrade paths from every
prior schema version; old preserved event payloads must still decode after
upgrade.

## Schema

One DB per user at `~/.jottrace/db.sqlite`. The schema below is shared by
all readers; it is the contract between readers and the rest of the pipeline.

Core tables:

- `sessions` — one row per logical source session. Columns include:
  - `id` — internal DB key used by other tables.
  - `source` — e.g. `claude_cli`, `codex_cli`, `cursor`, `opencode`.
  - `source_session_id` — stable id from the source. The unique source
    identity is `(source, source_session_id)`.
  - `file_path` — current path to the source artifact when applicable.
  - `cwd`, `parent_session_id` (nullable), `started_at`, `ended_at`.
  - `current_generation` — starts at 0 and increments when a source artifact
    is truncated or rewritten in a way that would otherwise collide with old
    `seq` values.
  - `file_mtime`, `file_size`, `content_fingerprint`, `next_read_offset`,
    `event_count`, `last_read_at`.

  `file_size` answers "did the file change?"; `content_fingerprint` answers
  suspicious same-size changes; `next_read_offset` is the byte position right
  after the last newline we successfully preserved for the current generation.
  The offset can be less than `file_size` when a partial line is sitting on
  disk mid-write.

- `events` — one row per preserved source event. Primary key is
  `(session_id, generation, seq)` where `seq` is an opaque per-source
  ordering key chosen by the reader and stable within that generation. For
  JSONL sources (Claude CLI, Codex CLI), `seq` is the 0-based line number in
  the file. Other readers derive it differently when their source isn't
  line-shaped; the schema doesn't care, as long as `seq` is unique within a
  session generation and stable.

  Columns:
  - `ts` — extracted timestamp, for cheap ordering and date filtering.
  - `payload` (BLOB) — the raw source event bytes as captured from the source,
    stored exactly enough to preserve source fidelity after decompression.
  - `codec` — per-row payload codec. MVP supports `raw` for small events and
    `zstd` for larger events; the threshold is benchmarked against the fixture
    corpus and can change without rewriting old rows.

- `ingest_errors` — visible per-file/per-session preservation failures.
  Columns include the source, source session id when known, file path, byte
  offset or line number when known, error kind, message, first/last seen times,
  and resolved marker. One bad file must not block ingestion of unrelated
  files, but it must be visible to `status` and `doctor`.

Each reader populates a small number of source-specific extra columns it needs
at metadata grain (e.g., model id, tool-call counts). The set of columns
evolves through migrations.

Initial indexes cover common metadata and status paths:

- unique `(source, source_session_id)` on `sessions`
- event ordering by `(session_id, generation, seq)`
- event time reads by `(session_id, ts)`
- unresolved ingest errors by source/session/path

## Query path

SQL filters always run on the metadata columns (`session_id`, `generation`,
`seq`, `ts`, plus reader-specific extras). The `payload` is opaque to SQL —
we don't query into it. To read event content, the application fetches the
matching rows and decodes each `payload` BLOB according to its `codec` before
handing the structured event to the processor.

If a future caller needs SQL-level filtering on a specific event field
(e.g. only `tool_use` events), we extract that field as its own
metadata column at ingest. We don't try to query into the compressed
payload.

## Reader: Claude CLI

### Source

- Primary location:
  `~/.claude/projects/<encoded-cwd>/<session-uuid>.jsonl`
- Alt installation paths to probe (some users run multiple Claude Code
  installs side-by-side; reading only `~/.claude/` misses sessions on
  those machines): `~/.claude-code/`, `~/.claude-local/`, `~/.claude-m2/`,
  `~/.claude-zai/`, each with the same `projects/<encoded-cwd>/` shape.
- Two project directory shapes coexist: the new one
  (`projects/<encoded-cwd>/<session-uuid>.jsonl`) and an older flat
  layout (sessions sit directly in the install dir as `*.jsonl`). The
  reader handles both.
- One JSONL file per session, append-only.
- Each line is one event (JSON object). Event shape varies by `type`
  (`user`, `assistant`, `system`, `file-history-snapshot`, ...).
- The session UUID in the filename is the canonical `source_session_id`.
- The encoded-cwd directory name is *not* reliably round-trippable, so
  the reader extracts the real `cwd` from event payloads instead of
  decoding the directory name.
- `tool_use` events are nested inside an assistant event's `content`
  array, not as top-level events. The reader stores the full event
  payload regardless; this only matters for downstream metadata
  extraction.

### Subagent sidechain files

Claude CLI emits a separate JSONL file for subagent (Task tool)
sidechains, named `agent-<uuid>.jsonl` in the same project directory.
Each subagent file is stored as its own row in `sessions`, with a
nullable `parent_session_id` column linking back to the spawning
session. Subagent events use the same `(session_id, generation, seq)` PK
shape as regular sessions; they are not folded into the parent's event
stream. Re-read behavior is identical to a regular session.

Rationale: matches how Claude CLI writes them on disk (separate files),
keeps session-level metadata intact (each subagent has its own
start/end/event count), and defers any cross-session view question to
the processor.

### Per-event metadata

The reader extracts only **cheap deterministic** metadata at session
grain:

- `source_session_id` (filename), `source` (`claude_cli`), `file_path`
- `cwd` (from the first event with a `cwd` field)
- `started_at` / `ended_at` (min/max of top-level `timestamp` across
  events; some events nest it under `snapshot.timestamp` — also
  accepted)
- `file_mtime`, `file_size`, `event_count`

Per event: an extracted `ts`, plus the raw source line bytes stored in
`payload` with per-row `codec` (`raw` or `zstd`). The reader parses JSON only
to extract metadata and validate that a committed line is a complete source
event. It does not replace the preserved payload with canonicalized JSON.

### Re-read behavior

A pass over one session file works like this:

1. **Snapshot.** Stat the file once and record the current `file_size`
   as `pass_size`. The pass only reads up to `pass_size`; anything the
   writer appends during the pass is left for the next pass. This
   avoids races with a live session.

2. **Decide.** Compare `pass_size` and `file_mtime` against the stored row:

   | File state vs. stored row                          | Action                                             |
   | -------------------------------------------------- | -------------------------------------------------- |
   | No row                                             | Generation 0 full import from byte 0.              |
   | `pass_size == file_size` and fingerprint unchanged | Skip.                                              |
   | `pass_size == file_size` and fingerprint changed   | Increment generation, import from byte 0.          |
   | `pass_size > file_size`                            | Resume from `next_read_offset`, parse forward.     |
   | `pass_size < file_size`                            | Increment generation, import from byte 0.          |

3. **Parse.** Split on `\n` and only commit lines terminated by `\n`. An
   unterminated tail (writer flushed mid-line) is left for a future
   pass. After commit, `next_read_offset` is set to the byte position
   right after the last `\n` consumed — which may be less than
   `pass_size` when a partial tail is on disk.

4. **Update.** In the same transaction as inserted events, save
   `file_size = pass_size` plus the new `content_fingerprint`,
   `next_read_offset`, `file_mtime`, `event_count`, `ended_at`, and
   `last_read_at` on the `sessions` row.

Append works because JSONL is append-only and line numbers are stable.
Truncation or same-size rewrite is treated as a new generation, not a rebuild.
Previously preserved events are never deleted just because the source artifact
changed.

### Source-file safety

Two unconditional rules (reader contract, restated for emphasis):

- The reader never writes to, moves, or deletes any file under
  `~/.claude/projects/` (or any of the alt install paths).
- If a known session's source file disappears (user wiped the
  directory, rotated, etc.), the reader leaves the DB row alone.
  Stored events stay valid. The DB is meant to outlive the source.

## Reader: Codex CLI

### Source

- Primary locations: recursive `~/.codex/sessions/YYYY/MM/DD/` trees and
  flat `~/.codex/archived_sessions/`. Each file is a "rollout" JSONL
  containing one session.
- Alt installation paths to probe: `~/.codex-local/`. Same
  `sessions/` and `archived_sessions/` subdirectory shape.
- One JSONL file per session, append-only (live sessions write into
  `sessions/`; closed sessions are moved to `archived_sessions/` by
  the host tool).
- Each line is one event. The relevant top-level event types:
  - `session_meta` — always the first event; carries the canonical
    session id and metadata in its `payload` field.
  - `event_msg` — carries actual conversation events. The payload
    has its own `type` field with subtypes like `user_message`,
    `agent_message`, plus tool-result variants.
- Canonical `source_session_id` is the value in the `session_meta`
  payload, **not** the filename. Filename and meta-id usually agree
  but the meta-id is authoritative.

### Re-read behavior

Same shape as Claude CLI: snapshot `file_size`, decide based on stored
row, parse forward, update. JSONL append-only invariant holds; line
numbers are stable within a generation; the events PK shape
`(session_id, generation, seq=line_number)` works unchanged.

### Differences from Claude CLI

- **Session id comes from event payload, not filename.** The reader
  must read the first event of a fresh file before it can write the
  `sessions` row.
- **Two directory shapes to scan.** `sessions/` is recursive by date;
  `archived_sessions/` is currently flat. Files may move from `sessions/`
  to `archived_sessions/` between passes; the reader uses the canonical
  session id (from `session_meta`) for dedup, so the move is invisible to
  the DB. The `file_path` column is updated on each pass.
- **No subagent sidechain files** in Codex CLI's session shape.

### Source-file safety

Same as Claude CLI. Reader is read-only on `~/.codex/sessions/` and
`~/.codex/archived_sessions/`; missing source file does not delete the
DB row.

## Reader: Cursor (SQLite shape, designed stub)

### Source

- macOS: `~/Library/Application Support/Cursor/`. Specifically, two
  storage shapes coexist depending on Cursor version:
  - Workspace storage at
    `User/workspaceStorage/<workspace-id>/state.vscdb` (older Chat
    mode).
  - Composer/Agent storage at `User/globalStorage/cursorDiskKV/`
    (newer Composer/Agent versions; further variants between v1.x and
    v2.0).
- Linux/Windows paths follow the same VSCode-derived conventions.
- Storage format is **SQLite**, not JSONL. The reader queries tables
  rather than reading lines.
- Canonical `source_session_id` candidates (settle when a real fixture
  is available): the composer id, the chat/conversation primary key,
  or a `(workspace_id, chat_id)` composite. Whichever is chosen, it
  must be stable across Cursor restarts and remain unique within the
  reader's installation.

### Change-detection challenge

SQLite databases can be edited in place — a session's content can change
without the file size changing. `(file_size, file_mtime)` is **not** a
sufficient change signal. Options:

- **Per-session content hash** stored alongside `file_size`/`file_mtime`.
  Cheaper to compute incrementally (hash only the rows belonging to one
  session, not the whole DB).
- **Trust message-level timestamps** in the source schema, if reliable.
  Cursor's tables include creation/update timestamps; the reader can
  filter "rows updated after `last_read_at`" rather than full scans.

Either works. The schema's `next_read_offset` column is a no-op for
this reader (or repurposed as "last-seen rowid" or similar
SQLite-friendly cursor).

### Ordering key

`seq` derives from the source's natural ordering. Likely candidates:
SQLite rowid, message creation timestamp, or an explicit per-session
sequence number if the source schema has one. Whichever is chosen, it
must be stable across re-reads.

### Status

Designed-stub. Full implementation is post-MVP and will surface quirks
(multi-version schema, encrypted columns, workspace-id discovery) that
aren't visible from this distance. Goal of including it here: validate
that the events-table schema and the reader contract handle a non-JSONL
source without modification.

## Reader: OpenCode (JSON-files shape, designed stub)

### Source

- macOS CLI: `~/Library/Application Support/opencode/storage/`
- Linux CLI: `~/.local/share/opencode/storage/`
- Desktop variants: same parent directory but `ai.opencode.app` instead
  of `opencode`.
- Storage format is a directory tree of JSON files (not JSONL). Each
  session is a subdirectory containing separate files for sessions,
  messages, and parts, plus Tauri `.dat` files for the desktop variant.
- Sessions can have parent/child relationships (similar to Claude CLI's
  subagent sidechains, but folded into the storage tree shape rather
  than expressed via filenames). The shared `parent_session_id` column
  on `sessions` carries the link.

### Change-detection

Per-file `mtime` over the session's subdirectory is the cheap signal.
The reader can compute it directly on the relevant subdirectory rather
than scanning the entire tree.

### Ordering key

`seq` is the iteration order over the session's message/part files,
sorted by some stable key (filename, creation order, or an explicit
sequence field in the JSON). To be confirmed against a real fixture.

### Status

Designed-stub. Full implementation needs a real fixture to settle the
file-iteration order question and the parent/child session linkage.

## Other readers (named, not designed)

These exist in the wild and are tracked so the architecture's "many
readers" claim is honest. Each will get its own designed section when
the user starts using that source or when implementation begins.

- **Continue.** JSON files at `~/.continue/sessions/`. One file per
  session. Similar to OpenCode's shape, simpler.
- **Gemini CLI.** JSON files at `~/.gemini/tmp/<hash>/chats/`.
  Workspace-bucketed by hash. Includes "thoughts" (reasoning steps with
  timestamps) alongside conversation.
- **Windsurf.** SQLite (VSCode-derived format) at
  `~/Library/Application Support/Windsurf/`. Shape mirrors Cursor;
  treat as a Cursor-family reader sharing change-detection strategy.
- **Trae.** Mixed JSONL + SQLite at `~/.trae/` and
  `~/Library/Application Support/Trae/`. Will need both code paths
  (JSONL read for chat, SQLite read for agent data).

Reference for paths, event shapes, and edge cases:
<https://github.com/0xSero/ai-data-extraction> (no license on the
repo; use for knowledge only — paths, formats, edge cases — not for
code reuse).

## Test strategy

The first implementation slice starts with fixtures, not guesses. The fixture
corpus is sanitized real source data, committed under the repo's test
fixtures, and includes:

- Claude CLI normal session.
- Claude CLI subagent sidechain.
- Codex CLI nested `sessions/YYYY/MM/DD/` rollout.
- Codex CLI archived rollout.
- Partial JSONL tail.
- Corrupt line or invalid event.
- Truncated/re-written source artifact that must become a new generation.

Unit tests cover the shared JSONL ingest core: full import, append resume,
partial tail handling, same-size `mtime` fingerprint checks, generation
increments, raw/zstd codec threshold behavior, per-file ingest errors,
transaction boundaries, and idempotency.

Migration tests cover every schema version. Each migration test builds or
loads an old-version DB, runs migrations, and asserts preserved event payloads
still decode.

CLI integration tests run the compiled binary against temp homes and temp
source directories. They assert exit codes, stdout/stderr, DB effects, lock
contention, data-dir permissions, `status`, and `doctor`.

Release CI includes an installer smoke test. The test installs from staged or
published release artifacts and asserts the installed binary can run
`jottrace --version`.

## Open seams (not yet designed)

Listed so we don't pretend they're decided.

- **Processor.** What model, what prompt shape, what input window
  (whole session? last N events? per-event?), what output schema, how
  does the writer consume it.
- **Writer.** Output format (Markdown shape), path layout,
  regenerate-vs-append semantics.
- **Scheduler.** Trigger model (cron, watch, manual, on-demand),
  per-stage vs. whole-pipeline runs, partial-failure recovery.
- **Multi-source orchestration.** Whether all readers run inside one
  binary or as separate processes invoked by the scheduler. (The
  shared-DB question is decided: one DB per user, see Schema.)
- **Subagent / parent-child linkage derivation.** For Claude CLI's
  `agent-<uuid>.jsonl` files and OpenCode's parent/child sessions:
  what's the cheapest deterministic way to derive `parent_session_id`
  from the file plus the parent's events? Likely the parent's
  `parentUuid` chain or a `Task` tool_use event referencing the
  subagent's UUID. Decide after looking at real fixtures; the schema
  column already exists.
- **Distribution caveats.** macOS Gatekeeper quarantine on unsigned
  binaries (ad-hoc sign vs. notarize vs. document `xattr -d`
  workaround); Linux glibc compatibility (build on oldest-supported
  runner vs. `musl` static target); install.sh checksum/signature
  verification of the downloaded binary. None of these are blocking;
  all need a conscious choice before the first release tag.
- **`recall`.** A query interface over the journal exists in the older
  design but is intentionally not yet placed in this architecture. The
  bet: build the data layer cleanly, design recall against real stored
  data later.
- **Compression tuning.** The architecture is decided: per-row `raw` or
  `zstd`. The exact threshold and zstd level are benchmarked against the
  fixture corpus.
- **Rust crate stack.** SQLite driver (`rusqlite` vs. `sqlx`), runtime
  model (sync vs. `tokio`), CLI framework (`clap`), error handling
  (`anyhow`/`thiserror`), config loader, logging.
