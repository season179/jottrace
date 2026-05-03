---
title: "Jottrace architecture"
status: DRAFT
updated: 2026-05-03
repo: github.com/season179/jottrace
---

# Jottrace

Local tool that turns AI coding-session transcripts (Claude CLI, Codex, etc.)
into a queryable journal.

This document is the source of truth for the current architecture. Earlier
v3.x designs (office-hours / grill-with-docs era) were scrapped on 2026-05-03;
recoverable from git history if needed.

## Stages

The system is a four-stage pipeline plus an orchestrator. We use the word
**stage** because the units form a directional flow; "module" would describe
their code shape but not their role.

```
[reader] -> [processor] -> [writer]
              ^              ^
              |              |
            scheduler triggers all three
```

1. **Reader** — reads sessions from disk and stores them in SQLite in a
   deterministic way. There will be many reader variants (Claude CLI, Claude
   coworker, Codex desktop, Codex CLI, OpenCode, ...). Each reader handles its
   own source format and its own deduplication. Readers do **not** interpret
   content; they store the raw event stream plus cheap metadata.

2. **Processor** — an LLM agent that takes the unstructured event content the
   reader stored and produces structured output (summaries, topics, decisions,
   dead ends, etc.). The session shape is deterministic; the *content* isn't —
   that's the processor's job.

3. **Writer** — takes processed content and writes it to a configured path
   (Markdown files in a vault, etc.).

4. **Scheduler** — orchestrates reader, processor, and writer runs.

We are focusing on the **Claude CLI reader** first. Everything else is named
but not designed.

## Implementation

- **Language: Rust.** Chosen for a single static binary, predictable
  performance on large JSONL files, and a strong SQLite ecosystem.
- **Storage:** SQLite at `~/.jottrace/db.sqlite`. The DB is the contract
  between stages, not in-process function calls.
- Specific crate choices (rusqlite vs. sqlx, sync vs. async, CLI framework,
  config loader) are not yet decided — see Open seams.

## Reader (Claude CLI) — settled

### Source

- Location: `~/.claude/projects/<encoded-cwd>/<session-uuid>.jsonl`
- One JSONL file per session, append-only.
- Each line is one event (JSON object). Event shape varies by `type`
  (`user`, `assistant`, `system`, `file-history-snapshot`, ...).
- The session UUID in the filename is the canonical session identifier.
- The encoded-cwd directory name is *not* reliably round-trippable, so the
  reader extracts the real `cwd` from event payloads instead of decoding the
  directory name.

### Output (SQLite)

- One DB per user at `~/.jottrace/db.sqlite`.
- Two tables:

  - `sessions` — one row per session file. Columns include `id` (session
    UUID), `source` (`claude_cli`), `file_path`, `cwd`, `started_at`,
    `ended_at`, `file_mtime`, `file_size`, `event_count`, `last_read_at`.

  - `events` — one row per JSONL line. Primary key is `(session_id, seq)`
    where `seq` is the 0-based line number in the file. Stores the raw line
    plus an extracted `ts` for cheap ordering.

### Dedup grain

Event-level. PK = `(session_id, seq)`. Re-reads are idempotent at event
granularity, so the processor can cheaply ask "what's new since I last
processed this session?"

We rejected session-level dedup (replace-the-row-on-each-run) because it
forces the processor to redo work and loses event-level history.

### What the reader extracts vs. leaves raw

The reader extracts only **cheap deterministic** metadata:

- `session_id` (filename), `source` (`claude_cli`), `file_path`
- `cwd` (from the first event with a `cwd` field)
- `started_at` / `ended_at` (min/max of top-level `timestamp` across events;
  some events nest it under `snapshot.timestamp` — also accepted)
- `file_mtime`, `file_size`, `event_count`

Per event: the raw JSON line plus an extracted `ts`. Nothing else.

Anything that requires interpretation — what topic the session was about,
what decisions got made, what was tried and abandoned — is processor
territory. The reader stays dumb on purpose.

### Re-read behavior

For each `.jsonl` file the reader sees:

| File state vs. stored row    | Action                                        |
| ---------------------------- | --------------------------------------------- |
| No row                        | Full import.                                  |
| Same `file_size`              | Skip.                                         |
| Larger `file_size`            | Append new lines from previous offset.        |
| Smaller `file_size`           | Truncation/rotation: wipe events, re-import.  |

Append works because JSONL is append-only and line numbers are stable.
Truncation is treated as a rebuild signal rather than an error.

## Open seams (not yet designed)

Listed so we don't pretend they're decided.

- **Processor.** What model, what prompt shape, what input window
  (whole session? last N events? per-event?), what output schema, how does
  the writer consume it.
- **Writer.** Output format (Markdown shape), path layout, regenerate-vs-
  append semantics.
- **Scheduler.** Trigger model (cron, watch, manual, on-demand), per-stage
  vs. whole-pipeline runs, partial-failure recovery.
- **Multi-source.** Whether all readers run in one binary or separately;
  whether they share the SQLite DB or each has its own.
- **`recall`.** A query interface over the journal exists in the older
  design but is not yet placed in this architecture.
- **Rust crate stack.** SQLite driver (`rusqlite` vs. `sqlx`), runtime model
  (sync vs. `tokio`), CLI framework (`clap`), error handling
  (`anyhow`/`thiserror`), config loader, logging.
- **Distribution.** `cargo install`, prebuilt release binaries, Homebrew
  formula — pick when MVP works locally.
