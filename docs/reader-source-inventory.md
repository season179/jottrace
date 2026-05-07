---
title: "Reader source inventory"
status: DRAFT
updated: 2026-05-06
parent_issue: "#54"
issues:
  - "#59"
  - "#69"
---

# Reader Source Inventory

This note is the gate before Jottrace adds another local coding-agent
reader. Source discovery is not enough. A source enters reader scope only
when sanitized fixture review proves it has an intelligible ordered
session/event stream that can be preserved without writing to the source.

## Reader Fixture Gate

A discovered file, database, cache, or app-state key is reader-scope only
when sanitized fixtures confirm all of the following:

- Stable `source_session_id`, or a deterministic equivalent that stays stable
  across repeated reads of the same source artifact.
- Ordered messages/events with enough timestamp, sequence, row id, line
  number, array order, or parent chain data to produce deterministic `seq`.
- Raw role/content/tool/reasoning/result payloads, or equivalent source events
  worth preserving losslessly.
- Enough metadata for read-only change detection, so the reader can avoid
  mutating source files while still detecting append, rewrite, or in-place
  update cases.

Sources that fail this gate stay out of implementation issues until fixture
review proves otherwise.

## Known Source Inventory

| Source | Path shape | Storage format | Stable session identity | Ordering strategy | Parent/child linkage | Change-detection questions |
| --- | --- | --- | --- | --- | --- | --- |
| Claude CLI / Claude Code | `~/.claude/projects/<encoded-cwd>/<session-uuid>.jsonl`, alternate Claude install roots with the same `projects/` shape, plus legacy flat UUID JSONL files | JSONL, one event per line | Filename UUID for normal sessions; parent-qualified `<parent-session-uuid>/subagents/agent-<id>` for sidechains | JSONL line number per generation | Subagent sidechains live under `<parent-session-uuid>/subagents/`; link to parent session when the parent row exists | Existing JSONL generation logic applies: file size, mtime, fingerprint, next read offset, truncation, and same-size rewrite handling |
| Claude Desktop / local agent mode | `~/Library/Application Support/Claude/local-agent-mode-sessions/**/local_*` directories with `audit.jsonl`; sibling `local_*.json` sidecar metadata; source=`claude_local_agent`; Browser session storage remains excluded | Ordered audit JSONL is the first-class transcript source; `local_*.json` sidecar metadata supplies session metadata such as cwd when present | Audit event `session_id` | Audit JSONL line order, with timestamps as metadata | No first-class child session shape confirmed | Shared JSONL generation logic applies to `audit.jsonl`: file size, mtime, fingerprint, next read offset, truncation, and same-size rewrite handling |
| Codex CLI | `~/.codex/sessions/YYYY/MM/DD/*.jsonl`, `~/.codex/archived_sessions/*.jsonl`, and same shapes under alternate Codex roots | JSONL rollout files | `session_meta.payload.id`, not filename | JSONL line number per generation | No separate sidechain file shape observed | Existing JSONL generation logic applies; file moves from `sessions/` to `archived_sessions/` should update `file_path` without duplicating the session |
| Hermes native SessionDB (#63 fixture-confirmed) | `~/.hermes/state.db`; `tests/fixtures/readers/hermes/sqlite/state.sql` preserves the reader-relevant SQL subset with synthetic rows | SQLite `sessions` and `messages` rows; FTS mirror tables are excluded | `sessions.id` | Session row first by `sessions.started_at`; messages by `messages.timestamp` + `messages.id` | `sessions.parent_session_id` links child sessions when the parent row exists | In-place SQLite updates use a per-session content fingerprint over the preserved session row and ordered messages, plus file metadata to refresh path/mtime without duplicating unchanged rows |
| Pi agent (#64 fixture-confirmed) | `~/.pi/agent/sessions/<encoded-cwd>/<timestamp>_<session-id>.jsonl` | JSONL session files with `session`, `message`, `model_change`, and `thinking_level_change` events | `session.id`; fixture review confirmed it matches the filename session-id suffix on observed files | JSONL line number, with top-level timestamps retained as event metadata | Event-chain `parentId` values exist on message, model-change, and thinking-level-change events; they are preserved inside raw payloads, not promoted to session parent rows | Existing JSONL generation logic applies: file size, mtime, fingerprint, and next read offset for append, rewrite, and unchanged-file detection |
| Factory / Droid-style agent sessions | `~/.factory/sessions/<encoded-cwd>/<session-uuid>.jsonl` plus matching `<session-uuid>.settings.json` files | JSONL plus per-session settings JSON | `session_start.id` from the first committed JSONL event | JSONL line number per generation; message/todo/compaction events carry timestamps | Message `parentId` values stay inside raw event payloads for now; no first-class child session shape is proven | Existing JSONL generation logic applies; sibling settings path, size, mtime, content fingerprint, and parsed settings JSON are stored in `source_metadata` so settings changes update session metadata without duplicating events |
| OpenCode | `~/.local/share/opencode/opencode.db`, with a coexisting `~/.local/share/opencode/storage/{session,message,part,project,session_diff}/` JSON tree on this machine | SQLite appears to be the complete current store in the 2026-05-06 source check; `tests/fixtures/readers/opencode/sqlite/opencode.sql` preserves the reader-relevant SQL subset with synthetic rows; JSON storage exists but needs separate fixture proof before reader scope | `session.id`, with `ses_*` ids visible in JSON storage paths to cross-check if JSON storage becomes reader scope | `message.time_created` + `message.id`, `part.message_id` + `part.id`, or `event.seq` if populated by the source | `session.parent_id` exists; this machine had 14 parent-linked sessions in the metadata check, and the SQL fixture includes a synthetic parent/child pair | In-place SQLite updates need per-session content hash or reliable `time_updated`; JSON storage would need file metadata plus content fingerprint if a fixture proves it is authoritative |
| Gemini CLI | `~/.gemini/tmp/<project-or-hash>/chats/*.json` plus `logs.json` sidecars | JSON session files with ordered `messages[]` | `sessionId` inside chat JSON | Array order within `messages[]`, with timestamps as metadata | No first-class child session shape confirmed | Whole-file JSON change detection needs file metadata plus content fingerprint; sidecar logs are reference material unless fixture review proves they add transcript events |

This inventory is metadata-only until each source has a sanitized fixture set
reviewed by a human. The existing Claude CLI, Codex CLI, Hermes SQLite,
OpenCode SQLite, and Pi agent fixtures are the committed fixture baseline today.

## Deferred And Ignored Sources

Ignore by default, or hold until a sanitized fixture proves intelligible
ordered session content:

- Browser and Electron `Session Storage`, browser profile data, app caches,
  extension state, opaque state, protobuf/app-state blobs, and MCP/plugin
  sidecar logs.
- Thin command histories, startup logs, auth/model selection histories, and
  other files that do not preserve meaningful coding-session content.
- VS Code, Copilot, ChatGPT extension, Windsurf, Antigravity, Claude app
  browser storage, Codex app browser storage, Claude CLI node caches, and
  skills-manager state until fixture review can distinguish user, assistant,
  tool, and result events from generic app state.
- Manual backup/reference copies such as `~/Downloads/CodexSessionBackup-*`;
  they may seed fixtures but are not automatic scan roots.

### Ignored By Default

| Source class | Examples | Why it is out of reader scope |
| --- | --- | --- |
| Aider home startup history | Aider home startup history, command history, model/auth/config traces | Thin histories and startup traces do not prove an ordered user/assistant/tool/result session stream. |
| Electron browser session storage | Browser or Electron `Session Storage`, `Local Storage`, `IndexedDB`, profile cache, and cookie-like state | Browser persistence is app/runtime state, not a transcript contract. It can contain opaque keys, partial UI state, credentials, or cache entries without deterministic session ordering. |
| skills-manager app state | Skills-manager app state, installed-skill cache, plugin metadata, and generated registry files | These files describe tooling configuration, not preserved coding-agent conversations. |
| sidecar/cache outputs | Sidecar/cache outputs, MCP/plugin logs, temporary payload mirrors, downloaded model/cache artifacts, and derived indexes | Sidecars may be useful for diagnostics, but they are not authoritative transcript stores and can duplicate or redact the real source. |

### Deferred Until Fixture Proof

These sources stay out of default ingest scope until a future reader issue
includes sanitized fixtures and the fixture proof required to reconsider them:

| Source class | Fixture proof required to reconsider |
| --- | --- |
| Windsurf | A stable session id, ordered event/message rows, role/content/tool/result payloads, and in-place update detection for its VS Code-derived state. |
| VS Code/Copilot/ChatGPT extension state | Proof that extension state contains intelligible coding-agent sessions rather than editor UI state, account cache, prompts without responses, or partial browser/app persistence. |
| Antigravity app/protobuf/browser state | A safe textual or decoded fixture showing stable session identity, deterministic ordering, and recoverable event payloads from app/protobuf/browser state without committing opaque private data. |

## Fixture Requirements And Privacy

Future source-reader issues must include sanitized fixtures before
implementation. Fixtures must:

- Preserve source shape, ordering cues, and metadata fields needed for stable
  ids, deterministic `seq`, parent/child linkage, and read-only change
  detection.
- Replace real prompts, tool outputs, reasoning, project names, usernames,
  repository owners, private paths, process ids, session ids, credentials, and
  token-like values.
- Use fixture-only paths such as `/Users/fixture/Workspace/jottrace`.
- Keep encrypted, opaque, or binary payloads as short placeholders unless the
  source exposes a safe textual shape that matters to preservation.
- Receive human review before they become the baseline for reader tests.

No reader issue should commit raw private transcripts, credentials,
proprietary project text, browser profile contents, or unsanitized local
paths.
