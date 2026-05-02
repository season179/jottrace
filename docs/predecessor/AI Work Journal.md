# AI Work Journal

This vault section tracks work done through local AI agent sessions so past work is recoverable, searchable, and reusable.

## Sources currently available on disk
- Hermes: `~/.hermes/.hermes_history`
- Codex: `~/.codex/history.jsonl`
- Claude local agent mode: `~/Library/Application Support/Claude/local-agent-mode-sessions/**/audit.jsonl`

## What to record for each entry
- Date/time
- Source (`hermes`, `codex`, `claude-local`)
- Session ID
- Project / repo / folder mentioned
- Short task summary
- Outcome tag: `success`, `failed`, `partial`, `research`, `blocked`
- Important files, commands, or decisions
- Follow-up notes

## Recommended organization
- `work-journal/index/` for source inventories and generated indexes
- `work-journal/entries/YYYY/` for journal entries
- Keep entries append-only. If a failed attempt later succeeds, add a new entry and link the old one rather than rewriting history.

## Why this exists
To make past work easy to find later, including failed attempts that may become useful with better context, tools, or timing.

## Seed notes
- [[AI Session Source Inventory]]
- [[Automation Setup]]
- [[../plan/AI Work Journal Automation Plan]]
