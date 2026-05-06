# Reader Fixture Corpus

This corpus started as the issue #21 reader-fixture seed set and now includes
the Claude local-agent, OpenCode SQLite, issue #64 Pi agent, and issue #67
Gemini CLI slices. It is
source-shaped from inspected local agent artifacts, but all content is
synthetic and safe to commit.

## Coverage

- `claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021.jsonl`
  is a normal Claude CLI session with permission, file snapshot, user,
  attachment, assistant thinking, tool use, tool result, system summary, queue,
  and final text events.
- `claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000021/subagents/`
  captures the observed subagent sidechain directory shape, including the JSONL
  transcript and a sibling `.meta.json` file.
- `codex-cli/sessions/2026/05/05/` captures the recursive Codex CLI
  `sessions/YYYY/MM/DD/` rollout shape.
- `codex-cli/archived_sessions/` captures the flat archived Codex rollout
  shape.
- `opencode/sqlite/opencode.sql` captures the OpenCode SQLite reader-relevant
  shape observed on 2026-05-06. The fixture is a SQL rebuild script with
  synthetic rows that preserve session, message, part, and parent-child
  relationship structure without committing the private source database.
- `pi-agent/sessions/--Users-fixture-Workspace-jottrace--/` captures the Pi
  agent `~/.pi/agent/sessions/<encoded-cwd>/` JSONL shape with session,
  message, model-change, and thinking-level-change events.
- `claude-local-agent/local-agent-mode-sessions/` captures the Claude Desktop
  local-agent metadata plus audit JSONL shape. The audit covers sanitized user,
  assistant, system, result, tool-summary, and rate-limit events. Browser
  session storage, app caches, uploads, outputs, and audit keys are
  intentionally not part of the fixture.
- `gemini-cli/tmp/fixture-project/chats/` captures the Gemini CLI chat JSON
  shape with user, gemini, and info messages plus thoughts, token metadata, and
  tool-call/result payloads.
- `factory/sessions/-Users-fixture-Workspace-jottrace/` captures the nested
  Factory / Droid-style JSONL shape, including `session_start`, `message`,
  `todo_state`, `compaction_state`, and sibling `.settings.json` metadata.
- `edge-cases/` contains partial-tail, corrupt-line, truncation, and same-size
  rewrite cases for the shared JSONL ingest core.

## Sanitization Contract

- Real prompts, tool outputs, reasoning, project names, local paths, usernames,
  repository owners, process ids, and session ids were replaced.
- Paths intentionally preserve source shape but use fixture-only values such as
  `/Users/fixture/Workspace/jottrace`.
- Encrypted or opaque model payloads are represented only by short fixture
  placeholders.
- No token-like values, private hostnames, email addresses, or real repository
  identifiers should appear here.

## Human Review

Review status: pending human approval for the Claude CLI, Claude local-agent,
Codex CLI, OpenCode SQLite, Pi agent, Gemini CLI, and Factory fixtures before
they become the baseline for new reader implementation.
Cursor fixture capture is still pending because no local Cursor `state.vscdb` or
`cursorDiskKV` source was available on this machine during the 2026-05-06
fixture pass.
