# Taste Extraction — Plan

*Collaboratively designed with the `agent` (Cursor CLI) as a second reviewer.
Two passes: (1) open critique of an emerging plan, (2) validation of the
locked synthesis. See the end of this file for the documented corrections.*

Status: **IMPLEMENTED** (2026-06-17). R3 async Task transcripts are
explicitly excluded; see the section below.
Created: 2026-06-17. Repo: `github.com/season179/jottrace`.

## What we are building

Jottrace stores full, lossless AI coding-session transcripts as raw event
blobs in SQLite. This feature makes jottrace able to **extract taste** —
structured preference signals about a developer's coding choices — from
those stored sessions, and export them as a labeled preference dataset.

The Command Code "Taste" objective in `notes/command-code-taste-formula.md`
is the InstructGPT PPO-ptx loss verbatim. We are **not training a model**.
The honest, high-value, tractable scope is the **data-extraction layer**:
produce the labeled `(context, proposal, outcome)` triples — the reward-model
training signal, i.e. `D_RL` in the formula — that an external trainer would
consume. The formula samples `x` (context) and `y` (proposal) and scores
`RM_NS(x,y)`; our job is to produce correctly-labeled `(x, y, outcome)`
rows. Anything short of that is not done.

## Locked decisions (from the human)

These are non-negotiable design inputs, settled in the planning conversation.

1. **Source scope — Claude only.** Other sources (Codex, Gemini, OpenCode,
   Factory, Pi, Hermes, Claude local-agent) do not store per-file content
   snapshots in their preserved transcripts, so the accept/reject signal is
   not recoverable from them. This is forced by the data, not a scope cut.
2. **Tool scope — complete coverage, not incremental.** Capture every
   file-modifying action: `Edit`, `Write`, `NotebookEdit`, `Bash` (sed, cat,
   tee, heredocs, spawned editors), MCP tools that write files, and
   subagent edits. The human rejects incremental shipping ("it is either
   done or not done"). Design for complete coverage in the first working
   version, even though this is more upfront work.
3. **Subagents — merged into the parent timeline.** A subagent edit is still
   an accepted/rejected proposal. Merge child sidechain sessions into the
   parent and attribute edits to the parent session.
4. **Accept definition — present-at-session-end.** A proposal is `accepted`
   iff its effect is still present in the final file state of the session;
   `rejected` if reverted, overwritten, or corrected at any later point. No
   tunable look-ahead window.
5. **Privacy — not a concern for this user.** Export includes full file
   content verbatim. (See correction C1: "full content" requires resolving
   snapshot sidecars, which the plan now does.)
6. **Execution model — separate command, materialized.** `jottrace taste
   extract` writes results into its own tables, is re-runnable, and is
   versioned via `extractor_version`. **Not** at ingest (ingest is
   byte-preserving per `docs/design.md`; interpretation belongs to a later
   stage). **Not** an on-demand view (re-scans everything, slow on large
   journals).

## Architecture

Timeline-first, with an inspectable intermediate layer.

```
events (raw, already stored losslessly by the reader stage)
  │
  ├─ [Claude event parser]  ← shared module; the future processor input
  │                          builder will call the same parse layer
  │
  ▼
file_timelines  (session_id incl. merged subagents, file_path, seq,
                 content, trigger_event_ref, source_kind)
  │
  ├─ [preference compiler: joins timelines + tool proposals +
  │    outcome detection + snapshot sidecar resolution]
  │
  ▼
preference_examples  (export-ready labeled dataset rows)
```

Two new data tables (migrations `010`–`011`; the `taste_extractions`
idempotency metadata table is added by `013`):

- **`file_timelines`** — one row per `(session_id, file_path, seq)`: the
  reconstructed content of that file at that point in the session, what
  triggered the change, and whether the content came from an inline
  snapshot, a `backupFileName` sidecar, or a tool `input`. Inspectable via
  `jottrace taste show timeline`. Reusable beyond taste (session replay).
- **`preference_examples`** — one row per detected proposal: `source`,
  `session_id`, `generation`, `proposal_event_seq`, `tool_use_id`,
  `file_path`, `tool_name`, `proposal_content`, `context` (N prior events +
  file excerpt at the before-state), `outcome` (`accepted`/`rejected`/
  `edited`), `confidence`, `evidence_kind`, `extractor_version`.

`edited` is a **low-confidence subclass** with explicit provenance, not a
peer outcome — see correction C2.

## CLI surface

A `taste` subcommand group (mirrors the existing `ingest`/`status`/`web`
shape):

- `jottrace taste extract [--session <id>]` — runs the pipeline above,
  writes/refreshes `file_timelines` and `preference_examples`. Idempotent
  per `extractor_version`: re-running replaces only rows for sessions whose
  raw events changed or whose prior extraction used an older version.
- `jottrace taste status` — counts: sessions processed, proposals detected,
  outcomes by class, coverage (% of file-modifying events resolved).
- `jottrace taste show timeline --session <id> --file <path>` — inspect the
  reconstructed per-file timeline (debuggability).
- `jottrace taste show example <id>` — inspect one preference example with
  full context.
- `jottrace taste export --format jsonl [--out <path>]` — emit
  `(context, chosen, rejected)` triples. Writes to a path the user chooses;
  never auto-transmits.

## Open implementation risks (from the validation pass)

See "Corrections" below for provenance. These shape the parser/compiler
design and must be handled, not deferred:

- **R1 — Snapshot sidecars.** Real Claude sessions store snapshots as
  `backupFileName` references into `~/.claude/file-history/<session>/`, not
  inline `content`. The sanitized test fixture uses inline content, which
  masks this. The extract pipeline needs a **snapshot sidecar resolver**
  that reads those blobs. If the sidecar files are gone, final state is not
  reconstructable and affected proposals must be flagged low-confidence or
  dropped (not silently emitted).
- **R2 — Bash attribution is best-effort.** `Bash` records `command` only;
  `file-history-snapshot` bumps carry no `tool_use_id`. Per-file linkage is
  temporal-ordering correlation, not payload truth. This is structural, not
  a bug to fix — the design treats Bash attribution as best-effort
  snapshot-diff correlation with a lowered `confidence` and a distinct
  `evidence_kind`.
- **R3 — Async Task transcripts are not ingested (excluded).** Claude's
  async Task agents write transcripts to
  `/private/tmp/claude-501/.../tasks/*.output`, referenced by parent events
  but **not preserved** by jottrace today. **Decision (2026-06-17): no** —
  async Task transcript edits are out of scope for taste-extraction
  completeness; this feature ships without a `tasks/*.output` reader.
- **R4 — Manual human edits and untracked paths.** A snapshot may change
  with no corresponding `tool_use` (the human edited in the IDE), and some
  files never enter `trackedFileBackups`. These produce timeline entries
  with no attributable proposal — they are context, not preference
  examples.

## Implementation sequence

The work is sequenced for *completeness*, not for incremental shipping —
each step is part of one done feature, not a releasable subset.

1. **Capture real fixtures.** Add sanitized real Claude sessions to
   `tests/fixtures/` covering: inline-content snapshots AND
   `backupFileName`-referenced snapshots; `Edit`, `Write`, `Bash` file
   edits; a permission denial; a revert; a subagent sidechain that edits a
   file. The existing fixture is too clean.
2. **Shared Claude parse layer.** A Rust module (`src/taste/parse.rs` or
   similar) that walks decoded event payloads for a session (+ merged
   children) and emits a normalized `(seq, kind, file_path, content_or_ref,
   tool_ref)` stream. One trait; source-specific today (Claude only).
3. **Snapshot sidecar resolver.** Resolves `backupFileName` → content from
   `~/.claude/file-history/<session>/`, with graceful degradation when
   blobs are missing.
4. **`file_timelines` materialization.** Migration `010` + the extract
   logic that writes the per-file content timeline.
5. **Preference compiler.** Joins timelines with tool proposals and
   computes outcomes via present-at-session-end comparison. Handles the
   confidence/evidence_kind matrix across `Edit`/`Write` (high),
   `Bash`/MCP (best-effort), `edited` (low).
6. **`preference_examples` materialization + CLI.** The `taste extract`,
   `status`, `show`, and `export` commands.
7. **Coverage report.** `taste status` must report the fraction of
   file-modifying events resolved with high confidence, so gaps are
   visible rather than silent.

## Deferred: async Task transcripts (R3)

**Status: excluded (2026-06-17).** Claude's async Task agents write to
`/private/tmp/claude-501/.../tasks/*.output`, which jottrace does not
ingest today. **Decision: no** — async Task transcript edits are out of
scope for taste-extraction completeness; this feature ships without a
`tasks/*.output` reader.

If scope changes later:

- If **yes** → this plan grows a new reader for `tasks/*.output` before
  taste extraction can be called done for that expanded scope.

---

## Corrections (changes the validation pass forced)

These are the places the open-design round got it wrong, and what changed.

- **C1 — Snapshots are not always inline.** I assumed `file-history-snapshot`
  carried full inline `content`. The fixture does; real sessions often use
  `backupFileName` pointers to external blobs. Added the snapshot sidecar
  resolver (R1, step 3). This also refines locked decision #5: "full
  content verbatim" is the intent, but it requires resolving sidecars, and
  is best-effort when blobs are missing.
- **C2 — `edited` is not a peer outcome.** My original plan treated
  accepted/rejected/edited as three equal labels. Partial-accept detection
  is noisy and often indistinguishable from "agent made a second Edit."
  Now a low-confidence subclass with explicit provenance.
- **C3 — `tool_result` success ≠ accept.** Claude returns `tool_result`
  when an Edit *executed*, not when the user liked it. True rejection
  surfaces downstream (revert, denial, correction). The present-at-
  session-end definition (decision #4) sidesteps this by looking at final
  state, not the immediate tool_result.
- **C4 — Bash/MCP attribution is structurally lossy.** I implied complete
  tool coverage (decision #2) was cleanly achievable. It is achievable, but
  Bash/MCP attribution is best-effort correlation (R2), not strict
  tool↔file linkage. The design reflects this with lowered confidence and a
  distinct `evidence_kind` rather than pretending to certainty.

## Reference

- Formula decoding: `notes/command-code-taste-formula.md`
- Architecture contract: `docs/design.md` (stages, reader = byte-preserving)
- Processor overlap: `docs/processor-design.md` (`friction` is semantic/
  narrative/journal-facing; taste is deterministic/pairwise/trainer-facing —
  distinct outputs, shared parse layer)
- Schema: `src/migrations/001_initial_schema.sql`
