# Work Journal Automation Setup

## What is installed
- Trigger: `launchd`
- Runner: `~/.local/bin/hermes-work-journal`
- Collector: `~/.hermes/scripts/ai-work-journal/collector.py`
- Hermes instructions: `~/.hermes/scripts/ai-work-journal/INSTRUCTIONS.md`
- Scheduler plist: `~/Library/LaunchAgents/com.season.ai-work-journal.plist`

## Schedule
- 12:00 PM
- 6:00 PM
- `RunAtLoad` is enabled, so loading the agent also triggers a catch-up run

## Mental model
- `launchd` triggers the job
- the wrapper script prepares pending context from local on-disk session sources
- Hermes reads the context, reasons about what is worth preserving, and writes append-only notes into Obsidian
- the wrapper commits state only after a successful Hermes run

## Sources currently ingested
- Hermes: `~/.hermes/.hermes_history`
- Codex legacy history: `~/.codex/history.jsonl`
- Codex app session transcripts: `~/.codex/sessions/**/*.jsonl`
- Codex app archived transcripts: `~/.codex/archived_sessions/*.jsonl`
- Claude local agent: `~/Library/Application Support/Claude/local-agent-mode-sessions/**/audit.jsonl`

## Journal outputs
- Daily notes: `work-journal/daily/YYYY-MM-DD.md`
- Topic notes: `work-journal/topics/<slug>.md`
- State: `work-journal/state/state.json`
- Seen hashes: `work-journal/state/seen_hashes.txt`
- Run log: `work-journal/state/run-log.md`
- launchd logs:
  - `work-journal/state/launchd.stdout.log`
  - `work-journal/state/launchd.stderr.log`

## Commands
Manual incremental run:
```bash
~/.local/bin/hermes-work-journal run
```

Bootstrap today again:
```bash
~/.local/bin/hermes-work-journal bootstrap-today
```

Backfill one older day without moving live source cursors:
```bash
~/.local/bin/hermes-work-journal backfill-date 2026-04-05
```

Backfill an older date range without moving live source cursors:
```bash
~/.local/bin/hermes-work-journal backfill-range 2026-04-01 2026-04-05
```

Reload the launchd job:
```bash
launchctl unload ~/Library/LaunchAgents/com.season.ai-work-journal.plist >/dev/null 2>&1 || true
launchctl load ~/Library/LaunchAgents/com.season.ai-work-journal.plist
```

Check whether the job is loaded:
```bash
launchctl list | grep com.season.ai-work-journal
```

## Important behavior
- Append-only journal updates
- Daily-note-first organization
- Topic notes created immediately
- Catch-up window is from the last successful run to the current execution time
- Historical backfill can target explicit older dates or ranges
- Backfill updates seen-hash dedup state but does not advance the live incremental source cursors
- Duplicate prevention uses source cursors plus content-hash deduplication

## Current limitation
Version 1 reasons from local source snippets, not full semantic reconstruction of every tool result. When the local evidence is incomplete, journal entries explicitly say the outcome is unclear.
