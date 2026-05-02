---
title: "Jottrace grilling-pass refinements"
date: 2026-05-02
status: rationale-archive (folded into design doc v3.1)
applies_to: season-unknown-design-20260501-224222.md
mode: Builder
---

# Jottrace grilling-pass refinements (2026-05-02)

> **Status: folded.** All twelve decisions below are folded into v3.1 of the design doc (`../season-unknown-design-20260501-224222.md`). This file is preserved as a rationale archive — it explains *why* each seam was decided, which the design doc itself does not. Don't update jottrace based on this file alone; the design doc is the source of truth.

A `/grill-with-docs` pass on the approved v3 design doc. Each Q below was a seam the original plan did not pin down; each carries a single locked decision plus a short rationale and any schema/CLI consequence.

The four MVP scope items in the original doc that are *unchanged* by this pass: SQLite ledger schema (premises 1, 17), canonical Session/Event model, LLMProvider interface, EvidencePacket budget rules, ExtractedSummary Zod shape. Everything below is downstream of those.

---

## Q1 — Daily-notes regression

**Decision.** Minimal deterministic daily rollup is in MVP scope.

**Shape.** One file per day at `<output>/daily/YYYY-MM-DD.md`. Pure deterministic — no LLM call. Contains a flat list of session-note links grouped by hour bucket of `started_at`. Generated at the end of every `sync` for every distinct `started_at`-day touched by sessions added or updated in that run.

**Why.** Original plan deferred all daily/topic notes to post-MVP; that is a real Day-1 regression for Season's existing daily-reading habit. Deterministic-only keeps the LLM scope bounded.

---

## Q2 — Daily rollup path collision with v2 vault

**Decision.** Moot. The v2 vault `~/obsidian-vault/work-journal/` was archived (zipped backup) and deleted on 2026-05-02. No coexistence required. `output_path` is picked at setup time; default `~/jottrace-journal/`.

---

## Q3 — In-progress sessions

**Decision.** Skip entirely until quiet. Discovery filter excludes any source file where `(now - source_mtime) ≤ quiet_minutes`. Active sessions are not ledger'd, no skeleton is written, they don't appear in any daily rollup. They become visible to jottrace only after they go quiet.

**Plan amendment.** Premise 13 ("Deterministic skeleton always written first") is qualified: "for every new or updated source file *that has gone quiet*."

**Why.** Avoids re-LLM'ing the same session multiple times during the same day; avoids forcing the LLM to pick an `outcome.status` for a session that hasn't ended; avoids misleading mid-stream summaries. Cost: today's currently-running session does not appear in today's daily rollup. Acceptable.

---

## Q4 — Quiet threshold + cross-day rollup placement

**Decision (a) — quiet threshold.** `[sync].quiet_minutes = 30` (default; configurable).

**Decision (b) — cross-day placement.** Daily rollup placement uses `started_at` day. A session 23:50 → 01:30 lands in the *previous* day's rollup at the 23 bucket.

**Decision (c) — past-day rewrites.** Daily rollups for past days can be rewritten on later syncs when previously-deferred sessions close. The renderer regenerates rollups for every distinct `started_at`-day touched by sessions added or updated in the current run. The idempotency contract (premise 20) still holds — no-op writes are short-circuited.

---

## Q5 — Marker-region ownership inside session notes

**Decision.** Strict marker discipline plus two structural exceptions.

| Region                                          | Owner                                      |
|-------------------------------------------------|--------------------------------------------|
| YAML frontmatter                                | renderer (re-emitted as structured data)   |
| H1 title (`# ...`)                              | renderer (replaced when LLM emits new one) |
| `<!-- jottrace:source-trace:start v=1 --> ... :end` | renderer                              |
| `<!-- jottrace:summary:start v=1 --> ... :end`  | renderer                                   |
| `<!-- jottrace:timeline:start v=1 --> ... :end` | renderer                                   |
| `<!-- jottrace:wins:start v=1 --> ... :end`     | renderer                                   |
| `<!-- jottrace:dead-ends:start v=1 --> ... :end`| renderer                                   |
| `<!-- jottrace:decisions:start v=1 --> ... :end`| renderer                                   |
| `<!-- jottrace:outcome:start v=1 --> ... :end`  | renderer                                   |
| `<!-- jottrace:artifacts:start v=1 --> ... :end`| renderer                                   |
| `<!-- jottrace:followups:start v=1 --> ... :end`| renderer                                   |
| Anything else (between markers, after markers, sub-headings the user adds) | user — preserved verbatim |

**Why.** Original plan said only "Content OUTSIDE marker pairs is preserved verbatim." That left the H1 title and frontmatter ambiguous. Splitting renderer-owned blocks into many narrowly-scoped marker pairs lets the renderer regenerate just the sections that changed without rewriting the whole body. Frontmatter and H1 are renderer-owned because both are structured data the recall query and daily rollup index against.

**Plan amendment.** Premise 4 ("YAML frontmatter is the canonical metadata layer") is now also explicit about ownership: frontmatter is renderer-owned and re-emitted on every sync. Hand-edits to frontmatter will be lost on the next sync.

---

## Q6 — Recall ranking

**Decision.** Default order = `started_at DESC` (most recent first). When `<topic>` is given, filter to sessions with a matching `topic_candidates.label` AND order by `MAX(topic_candidates.weight)` DESC for that label, tiebreaker `started_at DESC`. `--limit` default 5, hard max 25.

**Output addition.** When more sessions match than `--limit`, append a footer line: `… and N more (raise --limit to see them)`. Original plan's recall output mockup had no such note.

---

## Q7 — First-sync UX + resumability

**Decisions.**

- **Phase A bounded by `[sync].max_per_run`.** Already in config; default 200. First sync handles up to 200 source files; subsequent syncs catch up.
- **Phase B sequential in MVP.** New config: `[synth].max_concurrent = 1` (default). codex-cli is sequential anyway; anthropic-api could safely go to 3, but MVP keeps sequential to make `sync_runs` accounting simple.
- **Progress reporting.** `jottrace sync` emits one human-readable line per session — `✓ 12/47 codex/abc123 — 2m23s` — so a long first sync is not a black box. `--json` flag emits one NDJSON event per session for piping into other tools.
- **Ctrl-C safety.** SIGINT handler releases the lockfile and writes `sync_runs.outcome='partial'`. Phase A is per-file transactional (UPSERT + atomic rename); Phase B is per-session transactional (single ledger update at end of each session's synthesis). Re-running resumes naturally — pending/retry sessions stay queued.

**Why.** Backfilling 60 days of CC + Codex sessions on a heavy user is hundreds of LLM calls. A black-box CLI that runs for an hour is a Day-1 trust killer. Progress lines and Ctrl-C-safety make it operable.

---

## Q8 — LLM-extracted topics → topic_candidates

**Decision.** LLM-extracted `topics[]` are folded into `topic_candidates` with `source_kind='llm'` and `weight=1.0` per topic. `recall <project> <topic>` matches both deterministic labels (cwd / git_branch / repo / file_path / error / outcome) and LLM-named labels.

**Plan amendment.** `topic_candidates.source_kind` enum gains `'llm'` as an additional value.

**Why.** Without this, the LLM's curated topic list — the most semantically useful one — is invisible to recall. Premise 10 says topic *notes* are not auto-materialised in MVP, but the topic *candidate ledger* is built from day 1; LLM-named topics belong in it.

---

## Q9 — Long tool output truncation

**Decision.** Adapter truncation policy by tool-result kind:
- Normal tool result: keep last 4 KB. (As planned.)
- Errored tool result (non-zero exit OR `error` field present): keep first 1 KB + last 3 KB.
- Below 4 KB total: no truncation.
- Elision marker: `[…N bytes elided…]`.

**Why.** The actual error message often lives at the top of a long traceback while the tail is unrelated retry/cleanup noise. Tail-only truncation can drop the diagnostic bit. Cheap fix.

---

## Q10 — Short-id collision detection

**Decision.** Add `short_id TEXT NOT NULL` column to `sessions` plus a unique index `sessions_short_id ON sessions (short_id)`. At write time, the renderer attempts the 12-hex short id first; on uniqueness violation it falls back to the full 64-hex digest for that single session and records that on the row.

**Why.** Premise 18 mentioned "theoretical collision the adapter falls back to the full 64-hex digest" but didn't specify detection. Indexed uniqueness check is cheap and produces a deterministic guarantee.

---

## Q11 — Daily rollup template

**Decision.** Concrete shape of `<output>/daily/YYYY-MM-DD.md`:

```md
---
type: jottrace-daily-rollup
date: 2026-05-01
session_count: 7
projects:
  - jottrace
  - agent-bappeda
  - admin-v3
generated_at: 2026-05-02T09:14:08+08:00
---

# 2026-05-01 — 7 sessions, 3 projects

## 23:00

- 23:50 · `claude-code` · `agent-bappeda` · [Tanstack rebuild — sidebar refactor](sessions/2026/05/claude-code/abc123def456.md) · partial

## 21:00

- 21:14 · `codex` · `agent-bappeda` · [Investigate eval drift](sessions/2026/05/codex/789012abcdef.md) · success
- 21:02 · `claude-code` · `jottrace` · [Adapter discovery wiring](sessions/2026/05/claude-code/cafe5678abcd.md) · success

…
```

Hour buckets `## HH:00` are listed newest-to-oldest top-down. Within a bucket, lines are ordered by `started_at` DESC. Session links are relative to `<output>` root.

---

## Q12 — JSONL truncation / rotation

**Decision.** If on re-discovery `current_size < byte_offset`, treat as a rotation/truncation: reset `byte_offset = 0`, force a full re-parse, set `synthesis_status='pending'`. The size+sha mismatch already detects this case implicitly; making it explicit prevents an off-by-one bug where the incremental parser reads from a stale offset into a smaller file.

---

## Settled, not reopened

The following were considered for grilling and intentionally not reopened:

- **Naming.** "Jottrace" is locked per the design doc; the directory is named, the memory is named, no churn.
- **Provider matrix.** codex-cli (default) + anthropic-api in MVP, others post-MVP. As planned.
- **No scheduler in MVP.** Manual `jottrace sync`. As planned.
- **No MCP, no live-tail daemon.** Approach A. As planned.
- **Output path layout.** `<output>/sessions/YYYY/MM/<harness>/<short-id>.md` and `<output>/daily/YYYY-MM-DD.md` (newly added). User picks `<output>` at setup.
- **Topic notes (materialised Markdown).** Still post-MVP. The candidate ledger is the only topic surface in MVP.

---

## Schema deltas in one place

Folding Q5, Q8, Q10 into the SQLite schema in the original plan:

```sql
ALTER TABLE sessions ADD COLUMN short_id TEXT NOT NULL;        -- Q10
CREATE UNIQUE INDEX sessions_short_id ON sessions (short_id);  -- Q10
-- topic_candidates.source_kind enum is widened to include 'llm' (Q8); no DDL change since the column is TEXT.
```

## Config deltas in one place

Folding Q4, Q7 into `config.toml`:

```toml
[sync]
quiet_minutes = 30          # Q4 — discovery filter for in-progress sessions
backfill_days = 60          # unchanged
max_per_run = 200           # unchanged

[synth]
max_concurrent = 1          # Q7 — phase B concurrency cap (1 in MVP)
target_input_tokens = 8000  # unchanged
hard_input_tokens = 16000   # unchanged
```
