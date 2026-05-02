# AI Work Journal Automation Plan

## Goal
Build an append-only work journal in Obsidian that ingests local AI session history from Hermes, Codex, and Claude local agent mode twice a day, preserves backlinks across related attempts, and stays easy for Hermes to query later.

## Constraints
- Append-only history
- Local on-disk sources only
- Human-readable Markdown in Obsidian
- Automatic twice-daily runs
- Backlinks between retries, failures, and later successes
- Safe re-runs with no duplicate notes

## Verified source inputs
- Hermes: `~/.hermes/.hermes_history`
- Codex: `~/.codex/history.jsonl`
- Claude local agent: `~/Library/Application Support/Claude/local-agent-mode-sessions/**/audit.jsonl`

## Proposed vault structure
- `work-journal/daily/YYYY-MM-DD.md` — primary journal note for the day
- `work-journal/topics/<slug>.md` — topic index notes generated immediately for reusable backlinks
- `work-journal/index/` — inventories and generated indexes
- `work-journal/state/state.json` — ingestion cursor state
- `work-journal/state/run-log.md` — append-only run log for visibility
- `work-journal/archive/YYYY/` — optional long-term archive if daily notes become too large

## Proposed daily-note schema
Each daily note should contain append-only work items grouped by date, time, and topic rather than by session.

Daily note frontmatter:
- date
- sources_seen
- generated_at
- last_covered_at
- journal_version

Each appended work item inside the daily note should contain:
- time or time range
- topic slug(s)
- source(s)
- content_hash
- repo / path references
- status: `success | failed | partial | research | blocked`
- related topic links
- related prior attempt links
- summary
- actions
- outcome
- artifacts
- follow-up

Important design choice:
- sessions are input material, not the primary output unit
- the primary output unit is a dated work item within a daily note

## Backlink strategy
1. Daily structure
- the daily note is the primary container
- each appended work item links to topic notes and prior related attempts

2. Topic backlinks
- extract stable topics from file paths, repo names, feature names, package names, and repeated error strings
- generate topic notes immediately on first appearance using `[[topic-slug]]`
- topic notes maintain reverse chronological lists of related daily-note blocks

3. Continuation backlinks
- link new work items to earlier related attempts based on overlap in repo names, file paths, branch names, and repeated error strings
- especially preserve failure -> later success chains across days
- continuation links should point directly to the daily note plus a local heading/block reference when practical

## Idempotency / deduplication
- maintain per-source cursors in `work-journal/state/state.json`
- compute a normalized `content_hash` per candidate work item
- skip writing if an entry with that hash already exists
- if a source file rotates or truncates, reset cursor and rely on hash dedup to prevent duplication

### What “cursor + content-hash deduplication” means
This is a two-layer safety system.

1. Cursor
- A cursor is a saved checkpoint for each source.
- Example:
  - Hermes: last byte offset read from `~/.hermes/.hermes_history`
  - Codex: last line number processed in `~/.codex/history.jsonl`
  - Claude: last line number processed for each `audit.jsonl`
- On each run, the importer reads only the new material after the saved checkpoint.
- This makes normal runs fast and ensures the journal covers the time from the last successful run until now, even if the machine was off at the scheduled time.

2. Content hash
- A content hash is a fingerprint of the normalized work item after extraction.
- Example inputs to the hash:
  - source
  - timestamp window
  - topic slug(s)
  - normalized summary text
  - referenced repo/file paths
- If the same work item is seen again, it produces the same hash.
- Before writing a new daily entry block, the importer checks whether that hash was already recorded. If yes, it skips it.

Why both are needed:
- Cursor handles the common case efficiently.
- Content hash protects against duplicate writes caused by:
  - manual re-runs
  - job retries
  - source-file truncation/rotation
  - scheduler overlap

Practical result:
- if the noon run is missed because the MacBook is off, the 6 PM run will ingest everything since the last successful run
- if that same range gets scanned again later, duplicate journal items will still be skipped safely

## Scheduling strategy
Primary recommendation:
- use macOS `launchd` as the trigger layer
- use Hermes as the ingestion/reasoning/writing layer

Execution model:
- `launchd` wakes the job at the scheduled times
- `launchd` invokes a local Hermes command/script
- Hermes reads the on-disk source histories since the last successful cursor(s)
- Hermes reasons over what is worth recording
- Hermes writes or appends the normalized journal content into the Obsidian vault
- Hermes updates journal state and run logs

Why:
- `launchd` is more reliable on this Mac for triggering a local file-ingestion task
- Hermes is better suited for the extraction, judgment, normalization, backlinking, and note-writing steps
- this cleanly separates reliable scheduling from higher-level reasoning
- best fit when the job reads local files and writes into a local Obsidian vault

Fallback option:
- Hermes `cronjob` only if we later want chat delivery/reporting as the primary execution layer rather than local-native reliability

Scheduled times:
- 12:00 PM
- 6:00 PM

Coverage rule:
- each run processes everything from the last successful run up to the current execution time
- it does not assume the machine was on at the scheduled time
- if the 12 PM run is missed, the 6 PM run catches up automatically

Reason:
- keeps notes fresh
- reduces giant end-of-week dumps
- makes failures visible quickly

## Hermes integration plan
Hermes should treat the Obsidian work journal as a first-class recall source.

Operational rule:
- before starting work on a topic that smells like ongoing/project history, Hermes should search `~/obsidian-vault/work-journal/` for related notes
- after completing meaningful work, Hermes should update or append relevant journal notes via the importer flow, not by manually editing old entries

Practical access pattern:
- search notes by repo, feature, package, issue keyword, or status
- read the most recent related entries first
- prefer append-only follow-up notes over rewriting prior notes

## Loud-failure design
Each automated run should append to `work-journal/state/run-log.md`:
- run start time
- sources scanned
- entries created
- entries skipped as duplicates
- warnings
- hard failures

If automation fails, the failure should be visible in the log and in the scheduler output/logging path. If we later add Hermes delivery on top, failures should also be reported there.

## Phased implementation
### Phase 1: foundation
- define note format
- define folder layout
- build source inventory and state files
- create one end-to-end ingestion path for one source

### Phase 2: ingestion
- add Hermes parser
- add Codex parser
- add Claude audit parser
- normalize extracted work items into shared schema

### Phase 3: linking
- generate daily notes
- generate topic notes
- generate continuation backlinks across related attempts

### Phase 4: automation
- schedule twice-daily runs
- append run results to run log
- ensure failures report loudly

### Phase 5: Hermes recall habit
- update Hermes working practice so journal search happens proactively when relevant

## Claude Code brainstorm takeaways
- summarize sessions instead of dumping raw transcripts
- use cursor + content hash for two-layer deduplication
- use topic notes and daily rollups as the main backlink scaffolding
- keep raw histories in original locations; keep Obsidian notes readable

## Finalized decisions before implementation
- granularity: daily notes are the primary output; work items inside them are grouped by date, time, and topic rather than by session
- automation: macOS `launchd` is the primary scheduler because it is the more reliable option for local on-disk ingestion on this machine
- schedule times: 12:00 PM and 6:00 PM
- coverage window: from the last successful run to the current execution time
- topic notes: generate on first appearance
- keep all history append-only
