# AI Session Source Inventory

Verified on disk:

1. Hermes
- Path: `~/.hermes/.hermes_history`
- Status: present
- Notes: plain text history with timestamped prompts

2. Codex
- Path: `~/.codex/history.jsonl`
- Status: present
- Notes: JSONL with `session_id`, timestamp, and prompt text

3. Claude local agent mode
- Path: `~/Library/Application Support/Claude/local-agent-mode-sessions/`
- Status: present
- Notes: contains many `audit.jsonl` files with user messages, assistant messages, tool calls, timestamps, and session IDs
- Current rough count: about 159 `audit.jsonl` files

4. Cursor / Cursor Agent
- Status: retired / disabled from collector as of 2026-04-29
- Reason: Season no longer plans to use Cursor and intends to delete Cursor plus leftover local data to reclaim disk space.
- Historical note: Cursor transcripts were previously verified at `~/.cursor/projects/**/agent-transcripts/**/*.jsonl`, and `~/.local/share/cursor-agent` appeared to contain binaries only, not history.

## Journaling rule
A useful work journal should not only record wins. It should also keep:
- failed attempts
- dead ends
- constraints discovered
- later successes linked back to earlier failed attempts

## Next build step
Generate normalized markdown entries from these sources into `work-journal/entries/YYYY/`.
