---
title: "Processor design"
status: DRAFT
updated: 2026-05-08
parent_doc: docs/design.md
repo: github.com/season179/jottrace
---

# Processor

This doc fills the processor and writer open seams from `docs/design.md`.
Reader stage is settled there; processor and writer were intentionally
deferred. This doc commits to a shape for both, scoped to a first
end-to-end product slice — extracting lessons, decisions, dead ends, and
friction signals from preserved sessions — without precluding future
extraction kinds.

The writer in this doc is **the SQLite writer only**. It inserts processed
output into new tables in the same `~/.jottrace/db.sqlite`. Markdown vault
export, file watchers, and other writer variants are explicitly deferred.
The DB stays the contract between stages.

## Why this exists

The reader preserves bytes; that is enough to never lose a session, but it
is not enough to *learn* from one. Sessions accumulate faster than a
human can read them. The processor's job is to turn the preserved event
stream into a small set of structured artifacts — decisions made, lessons
worth keeping, dead ends with retry conditions, friction signals — that
are searchable, durable, and feedable into future agent context (CLAUDE.md,
AGENTS.md). The journal is the substrate; the extractions are the payoff.

The first product question this answers: "what did I learn from my last
30 days of sessions, and what should I try again?"

## Stages, restated

```
[reader] -> [processor input builder] -> [processor] -> [sqlite writer]
                                       |
                                       +-> [processor_errors]
              ^                                       ^
              |                                       |
            scheduler triggers reader and processor passes
```

`processor input builder` is a processor-time adapter, not a new
architectural stage. It fetches preserved event rows, decodes the storage
codec (`raw` or `zstd`), orders rows by `seq`, wraps each original payload
with stable evidence metadata, and applies deterministic omission or
truncation rules for known-noisy payloads before the model sees them. The
reader contract from `design.md` is unchanged — readers still preserve raw
bytes and never interpret content.

A cross-source normalized event structure is explicitly out of scope for
this plan. The processor sees the original source payloads inside stable
envelopes. It is the LLM's job to understand source-specific event shapes
well enough to extract decisions, lessons, dead ends, and friction.

The writer in this version is a thin SQLite insert layer. It runs in the
same process as the processor, in the same transaction as the
`processor_run` row commit. Calling it out as a separate stage keeps the
data-flow language consistent with `design.md`, but there is no
cross-process boundary.

`scheduler` and `recall` remain open seams. The processor must be
runnable on demand from the CLI without either of them.

### Relationship to taste extraction

Taste extraction (`jottrace taste`, see `docs/design.md` and
`notes/taste-extraction-plan.md`) is a **separate deterministic stage**
that already ships. It shares the Claude event parse layer (`src/taste/parse.rs`)
with the future processor input builder, but the outputs are distinct:

- **Processor** — semantic, narrative, journal-facing extractions
  (`decision`, `lesson`, `dead_end`, `friction`, …) produced by an LLM.
- **Taste** — pairwise, trainer-facing preference labels
  (`accepted`/`rejected`/`edited`) compiled from file timelines and tool
  proposals without model calls.

The processor input builder should call the same parse module rather than
re-implementing Claude event decoding.

## Processor input builder

The input builder is a deterministic function over preserved data:

```text
build_processor_input(session_id, generation, window) -> RenderedInput
```

The output is stable prompt input, not a canonical transcript model. Each
rendered event keeps the source payload intact enough for evidence review,
for example:

```text
[seq=42 source=codex_cli ts=2026-05-05T09:00:08.000Z codec=raw]
{"timestamp":"...","type":"event_msg","payload":{...}}
```

The input builder owns only mechanical concerns:

- Decode the storage codec (`raw` / `zstd`) back to source bytes.
- Preserve `source`, `source_session_id`, `generation`, `seq`, and `ts`
  as evidence anchors.
- Render source payloads in deterministic order.
- Apply deterministic truncation or omission policy for payload classes
  that are too large or too noisy for a model window.
- Produce the exact bytes used for `input_window_hash`.

It must not introduce a `NormalizedEvent` type, translate every source
into `role/content/tool_calls/tool_results`, or hide the original payload
from the processor. If a future source has an opaque or binary payload
that cannot be rendered safely, that event becomes an `input_build_failure`
processor error for the run; it does not require a normalized data model.

## Processor contract

Every processor pass, regardless of extraction kind, follows the same
shape.

1. **Select.** Pick a session and generation to process. The unit of
   work is one `(session_id, generation)` pair. Processor work is never
   sub-session in this version.

2. **Build input.** Stream events for that pair through the input builder,
   in `seq` order. Rendered events accumulate in memory or are chunked,
   depending on size (see Input granularity below).

3. **Extract.** Run the extraction prompt against the rendered source
   event stream and parse the model's structured output into typed
   extractions.

4. **Persist.** In one transaction: insert one `processor_runs` row,
   insert N `extractions` rows linked to it, and update any cached
   counters. On any failure inside the transaction, no extractions are
   written; the run row may be written as `failed` so the failure is
   visible.

5. **Stay safe.** The processor never mutates `events`, `sessions`, or
   any reader-owned table. Source files are still off-limits. Old
   extractions from prior runs are never deleted; new runs append.

6. **Be idempotent at run grain.** Re-running the processor with the
   same `(session_id, generation, processor_version, input_window_hash)`
   is a no-op. See Idempotency below.

## Input granularity

The processor operates per `(session_id, generation)`. Within that, the
input to one model call is a **window** of rendered source events. Three
strategies, picked by event count and rendered byte size:

- **Whole session.** Default when the rendered event stream fits within
  the configured token budget. Most Claude CLI and Codex CLI sessions
  fit; long-running multi-day sessions may not.
- **Sliding window with overlap.** Used when the whole session does not
  fit. Window size and overlap are config; the default starts at the
  last N events and walks back in chunks. Overlap exists so an
  extraction whose evidence spans a chunk boundary is not split.
- **Two-pass summarize-and-zoom.** Used when sliding windows still
  produce noisy output: first pass produces a per-chunk synopsis; second
  pass concatenates synopses and runs the full extraction prompt on
  that. Costs 2x; reserved for the worst sessions.

Strategy selection is deterministic given session size and config, so
`input_window_hash` (next section) stays stable across re-runs.

## Schema

Three new tables. All migrations follow the existing
`PRAGMA user_version` discipline from `design.md`.

### `processor_runs`

One row per processor invocation. Records the inputs that produced a set
of extractions, so the run can be replayed or compared.

Columns:

- `id` — internal DB key.
- `session_id` — FK to `sessions.id`.
- `generation` — the generation processed; matches `events.generation`.
- `processor_version` — string identifier including model id and prompt
  hash (e.g., `claude-3-5-sonnet@2026-04-01/extract-v1/sha256:...`).
- `input_window_hash` — sha256 of the exact rendered input bytes
  the model saw, in deterministic order. Lets us detect "same input,
  same processor → skip."
- `input_event_first_seq`, `input_event_last_seq` — the inclusive event
  range covered by this run. For sliding windows, multiple runs cover
  one generation across non-overlapping `[first, last]` ranges.
- `strategy` — `whole_session`, `sliding`, `summarize_zoom`.
- `status` — `succeeded`, `failed`, `partial`.
- `started_at`, `finished_at`.
- `model_input_tokens`, `model_output_tokens`, `cost_micro_usd` —
  observability; null when the model client cannot report them.
- `error_kind`, `error_message` — populated when status is `failed` or
  `partial`.

Unique index on
`(session_id, generation, processor_version, input_window_hash)` so
re-running the same processor on the same input is a no-op insert.

### `extractions`

One row per extracted artifact. Wide table with a `kind` discriminator;
content lives in a JSON column whose schema varies by kind.

Columns:

- `id` — internal DB key.
- `run_id` — FK to `processor_runs.id`.
- `session_id`, `generation` — denormalized from the run for cheap
  filtering. Same values as on the run row.
- `kind` — one of:
  - `decision` — a choice the user or agent made and why.
  - `lesson` — a generalizable rule of the form "if X, then Y."
  - `dead_end` — an approach that was tried and abandoned.
  - `friction` — a redirection, correction, or rejection by the user
    (the postmortem-prompt category from the methodology).
  - `habit` — something that worked unexpectedly well, worth keeping.
  - `karpathy_failure` — a tagged instance of one of the four
    canonical failure modes (see Extraction kinds below).
- `content` — JSON. Per-kind schema documented below.
- `evidence_event_seqs` — JSON array of `seq` values from
  `events.seq` for this generation. The events the extraction is grounded
  in. Empty array is allowed (synthesis extractions) but discouraged.
- `confidence` — model-reported 0.0–1.0, nullable.
- `created_at` — wall-clock at write.
- Optional per-kind columns are inlined here when they are queried
  often enough to justify SQL filtering. Today: none. Future candidates
  are listed in Open seams.

Indexes:

- `(session_id, kind)` for per-session-per-kind reads.
- `(kind, created_at)` for "all decisions in May."
- `(run_id)` for cascade-style reads after a run.

### `processor_errors`

Visible per-run input-building and model failures, mirroring
`ingest_errors` in spirit.

Columns: `run_id`, `session_id`, `generation`, `event_seq` (nullable for
session-grain errors), `error_kind` (`input_build_failure`,
`model_api_error`, `model_invalid_json`, `model_refusal`, `timeout`,
`oversized_window`, `internal`), `message`, `first_seen_at`,
`last_seen_at`, `resolved_at` (nullable), `raw_response` (nullable BLOB,
zstd, for model output failures so they can be diagnosed offline).

`jottrace status` and `jottrace doctor` learn to surface unresolved
processor errors next to unresolved ingest errors.

## Idempotency and generations

The reader's `(session_id, generation, seq)` PK keeps preservation
idempotent. The processor mirrors that with two layers:

- **Run-grain idempotency.** Unique
  `(session_id, generation, processor_version, input_window_hash)` on
  `processor_runs`. Re-invoking the processor with the same inputs
  inserts nothing. Changing the model id, the prompt, or the input
  window — including new events appended to the session — produces a new
  hash and a new run.

- **Generation handling.** A new generation on the source means new
  events with overlapping `seq` values; the processor treats each
  generation independently. Old generations' extractions stay; new
  generations get fresh runs. Cross-generation deduplication is not
  attempted; it is a recall-time concern.

Re-runs are append-only. Extractions are never updated in place. The
"current view" of extractions for a session is the rows belonging to
the latest succeeded run for that `(session_id, generation,
processor_version)` triple. Recall queries enforce this with a window
function or a `latest_run` materialized view; the choice is deferred
until recall is designed.

## Extraction kinds

The six kinds above are the v1 taxonomy. Each has a small per-kind JSON
schema in `extractions.content`. Schemas are versioned via a top-level
`schema_version` field in the JSON, so the model's output shape can
evolve without a migration.

### `decision`

```json
{
  "schema_version": 1,
  "summary": "Chose rusqlite over sqlx for the storage layer.",
  "alternatives_considered": ["sqlx async"],
  "rationale": "Single static binary requirement; no async benefit at this scale.",
  "tags": ["storage", "rust"]
}
```

### `lesson`

```json
{
  "schema_version": 1,
  "rule": "Run cargo clippy --all-targets before committing.",
  "trigger": "Whenever a new module is added.",
  "rationale": "Catches dead_code and unused_imports the test suite misses.",
  "tags": ["workflow", "rust"]
}
```

### `dead_end`

```json
{
  "schema_version": 1,
  "approach": "Per-event vector embeddings for similarity search.",
  "blocked_by": "Embedding cost dominates ingest budget; signal-to-noise was poor.",
  "failure_kind": "transient",
  "retry_condition": "Local embedding models with sub-millisecond inference per event.",
  "tags": ["search"]
}
```

`failure_kind` is `transient` or `persistent`, borrowed from the brief.
`retry_condition` is required when `failure_kind=transient` and forbidden
when `persistent`. This makes the "RETRY list" a query, not a separate
table: `SELECT ... WHERE kind='dead_end' AND content->>'failure_kind' =
'transient'`.

### `friction`

```json
{
  "schema_version": 1,
  "category": "redirection|correction|rejection|repeat_question",
  "user_text": "Stop. That's not what I asked.",
  "agent_misstep": "Edited unrelated test fixtures while fixing a single test.",
  "tags": ["karpathy-3"]
}
```

`category` is the postmortem-prompt taxonomy. The most useful early
signal across a corpus is grouping these by `agent_misstep` or by
`tags`.

### `habit`

```json
{
  "schema_version": 1,
  "what": "Asked agent to print a one-line plan before editing files.",
  "why_it_worked": "Surfaced silent assumptions without a separate review pass.",
  "tags": ["prompt", "karpathy-1"]
}
```

### `karpathy_failure`

A tagged instance of one of the four canonical failure modes, kept
distinct from `friction` because the user did not necessarily push back
in the moment — the failure may be visible only on review.

```json
{
  "schema_version": 1,
  "rule_index": 1,
  "rule_name": "silent_assumption",
  "evidence_summary": "Picked HTTP/1.1 keep-alive without asking the user about long-poll vs. websocket.",
  "tags": []
}
```

`rule_index` is 1–4 matching the canonical taxonomy
(`silent_assumption`, `overcomplication`, `orthogonal_edit`,
`weak_success_criteria`).

## Prompt shape

One prompt per processor pass. Single structured output, all kinds at
once. The prompt is held in the binary as a versioned string and hashed
into `processor_version` so its evolution is auditable.

The prompt's high-level shape:

1. System instructions: role (extraction analyst), output format
   (strict JSON object with named arrays per kind), determinism rules
   (no speculation beyond evidence, evidence required, confidence
   reported per item).
2. Schemas: the per-kind JSON schemas above, inlined.
3. Input: the rendered source event stream. Each event is wrapped with
   stable metadata such as `[seq=N source=<source> ts=<ts>]`, followed by
   the original source payload or a deterministic truncation marker.
4. Output instructions: "emit one JSON object with `decisions`,
   `lessons`, `dead_ends`, `friction`, `habits`, `karpathy_failures`
   arrays. Each item must include `evidence_event_seqs`. Confidence
   per item. Skip kinds with no instances."

Two failures the prompt must explicitly guard against, both observed in
practice when running similar prompts:

- **Hallucinated evidence.** Items emitted with `evidence_event_seqs` that
  do not exist in the input. The processor validates every emitted seq
  against the input window's seq set and downgrades violators to a
  `processor_error` of kind `model_invalid_json`.
- **Overproduction.** The model emits a "lesson" for every minor edit.
  The prompt sets a per-kind ceiling (configurable; default 5
  extractions per kind per session) and asks the model to pick the
  highest-signal items if it would otherwise exceed it.

A second prompt — a cross-session "compile" pass that deduplicates and
synthesizes across sessions — is **not in this scope**. It is a recall-
or scheduler-time concern. The current scope is per-session extraction
only.

## Subagent and multi-source handling

Subagent sidechain sessions are processed independently as their own
`(session_id, generation)` pairs. They already have their own row in
`sessions` with `parent_session_id` set; the processor does not fold
them into the parent. Cross-session synthesis (does this subagent's
failure tell us something about the parent task?) is deferred.

Multi-source sessions go through the same processor without
specialization. The prompt is allowed to see source-specific payload
shape, but the input envelope is consistent, so every extraction can cite
the same `evidence_event_seqs` regardless of source.

OpenCode and Hermes (SQLite sources) have a generation model that does
not match JSONL append semantics. The reader has already projected those
rows into ordered `events` rows; the input builder only relies on
`seq`, and that ordering is the contract.

## CLI surface additions

Three new commands plus extensions to existing ones.

- `jottrace process` — run the processor over all sessions with at least
  one event and no succeeded run for the current `processor_version`.
  Prints per-source counts of processed sessions, total extractions
  inserted, and unresolved processor errors. Compact stdout by default,
  matching the existing `ingest`/`status`/`doctor` convention; use
  `--details` for model token totals, cost estimates, and per-kind
  counts.

- `jottrace process --session <source_session_id>` — process one
  session.

- `jottrace process --reprocess` — force a new run even when a run for
  the current `processor_version` already exists. Useful when a prompt
  is being iterated on locally.

- `jottrace extractions` — read-only listing.
  - `--kind <decision|lesson|dead_end|friction|habit|karpathy_failure>`
  - `--session <id>` / `--source <name>`
  - `--since <date>` / `--until <date>`
  - `--limit N` / `--all` (matches `events`' bounded-output contract)
  - `--retry-only` — sugar for `kind=dead_end AND failure_kind=transient`.

- `jottrace status` — extended with processor counts: total runs, runs
  by status, total extractions by kind, unresolved processor errors.

- `jottrace web` — extended with a per-session "Extractions" panel
  rendering each kind's content. Read-only, no new mutations. The
  existing search box gains an "extraction text" facet.

`jottrace doctor` reports unresolved processor errors and missing
configuration (no API key, no model id) the same way it reports unresolved
ingest errors today.

## Configuration

Processor needs configuration the reader doesn't:

```json
{
  "processor": {
    "provider": "anthropic",
    "model": "claude-3-5-sonnet-20240620",
    "api_key_env": "ANTHROPIC_API_KEY",
    "max_input_tokens": 150000,
    "per_kind_ceiling": 5,
    "concurrency": 2,
    "cost_cap_micro_usd_per_run": 50000
  }
}
```

Lives in the existing `~/.jottrace/config.json` (mode `0600`) alongside
`auto_update`. The API key is read from the environment, never from the
config file or DB. `cost_cap_micro_usd_per_run` is a hard stop: a run
that would exceed it logs a `processor_error` of kind `oversized_window`
and exits the run early without inserting extractions.

## Test strategy

Mirrors `design.md`'s test strategy. Three tiers.

**Input-builder unit tests.** Each source has fixture event payloads
already present for the implemented readers. Tests assert deterministic
rendering, evidence envelopes, codec decoding, omission/truncation
markers, and stable `input_window_hash` bytes. They do not assert a
normalized cross-source event structure.

**Processor unit tests with mocked LLM.** A `MockProcessor` that takes
canned model responses lets us test:

- The full insert path (run + extractions + error rows) under all
  status outcomes.
- `evidence_event_seqs` validation rejecting hallucinated seqs.
- Idempotency: same input + same processor_version → no new row.
- Generation handling: new generation produces a new run; old run rows
  remain.
- `cost_cap_micro_usd_per_run` enforcement.
- JSON parse failures landing in `processor_errors`.

**Integration tests against a real model, opt-in.** Gated by an env var
so they only run when the developer wants to spend money. They assert
schema validity (each emitted JSON object validates against the per-kind
schema), not exact content, because LLM output is non-deterministic.
Run on at least one fixture per source.

The `extractions.evidence_event_seqs` validation runs in the processor
pipeline itself, not just in tests, because it catches a real failure
mode in production.

## Phasing

Three slices, each shippable on its own.

**v0 — single source, single kind, end-to-end.**

- Claude CLI input builder path only.
- Friction-only extraction kind. The cheapest first move from the
  methodology brief; gives the user a ranked list of recurring agent
  misstep categories per session.
- New tables: `processor_runs`, `extractions`, `processor_errors`.
- New CLI: `jottrace process` and `jottrace extractions`.
- Web UI: an "Extractions" tab on session detail.
- One opt-in integration test against the real model.

This proves the loop end-to-end. The user gets value (recurring failure
modes per session) before a single other extraction kind exists.

**v1 — full taxonomy, second source.**

- Add `decision`, `dead_end` (with retry condition), `habit`,
  `karpathy_failure`, `lesson` extraction kinds.
- Add Codex CLI input rendering coverage.
- `jottrace extractions --retry-only`.
- Cost reporting in `--details`.

**v2 — remaining sources, recall handoff.**

- Input rendering coverage for Factory, Gemini, OpenCode, Hermes, Pi,
  and Claude local-agent.
- Per-source input-builder fixtures.
- Define the recall query interface (separate doc).
- Cross-session compile pass (separate doc).

## Open seams

Listed so we don't pretend they're decided. Each will get its own design
note when the work begins.

- **Rust LLM client choice.** Direct HTTP via `reqwest` with hand-rolled
  Anthropic / OpenAI clients vs. one of the third-party crates
  (`async-openai`, `anthropic-sdk-rust`, `genai`, etc.). Affects
  binary size, async vs. sync, retry/backoff strategy.
- **Async runtime.** Processor wants concurrency across sessions;
  reader doesn't. Whether to introduce `tokio` for the processor only,
  whether to keep everything sync and parallelize across processes,
  whether to defer concurrency entirely in v0.
- **Token counting.** Pre-flight token estimation determines window
  strategy; the source-of-truth is the model API. Whether to ship a
  model-side tokenizer (`tiktoken`-equivalent) or budget conservatively
  by character count and let the model API reject oversized inputs.
- **Cross-session compile pass.** The "Karpathy wiki" / "compound loop"
  pattern from the brief lives here. Out of scope for this doc;
  belongs in a future `docs/compile-design.md`.
- **Vault writer.** Markdown output to a configured vault path is
  deferred. When designed, it becomes a second writer variant that reads
  the `extractions` table and produces files. Schema does not need to
  change for that to be added.
- **Recall query CLI.** `jottrace extractions` is a thin listing today.
  A real recall surface (filter by tag, search text, group by theme,
  ranked results) is its own design doc. Schema choices in this doc
  must not foreclose it; the indexes above and the JSON-tag convention
  on `content.tags` are picked with that in mind.
- **Cost guardrails.** Per-run cap is a starting point. Daily caps, per-
  source caps, and dry-run estimation modes are all reasonable; pick
  after observing real cost on the user's corpus.
- **Privacy of model traffic.** Sessions contain private code and
  prompts; the processor sends them to a third-party API. Whether to
  support local models (via Ollama or llama.cpp) is a real product
  question, not a future-proofing nice-to-have. Picked when v1 ships.
- **Extraction supersession.** Today re-runs append; recall picks the
  latest. Whether to mark older extractions as superseded with a
  `superseded_by` column, or keep them strictly historical, is a
  recall-time decision.
- **Per-kind columns.** All non-evidence content lives inside the
  `content` JSON for now. If a specific field (e.g. `content.tags`)
  becomes a hot filter, promote it to its own indexed column via
  migration.
