---
title: "Reader source inventory"
status: DRAFT
updated: 2026-05-06
parent_issue: "#54"
issue: "#59"
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
| Claude Desktop / local agent mode | `~/Library/Application Support/Claude/local-agent-mode-sessions/` with session metadata JSON and `audit.jsonl` files | Metadata JSON plus ordered audit JSONL | Fixture must confirm whether `session_id` or metadata UUID is canonical | Audit JSONL line order, with timestamps as metadata | Unknown until sanitized fixture review | Confirm whether metadata and audit files update append-only or can rewrite in place |
| Codex CLI | `~/.codex/sessions/YYYY/MM/DD/*.jsonl`, `~/.codex/archived_sessions/*.jsonl`, and same shapes under alternate Codex roots | JSONL rollout files | `session_meta.payload.id`, not filename | JSONL line number per generation | No separate sidechain file shape observed | Existing JSONL generation logic applies; file moves from `sessions/` to `archived_sessions/` should update `file_path` without duplicating the session |
| Hermes native SessionDB | `~/.hermes/state.db` | SQLite | Session table primary key, exact column to confirm from fixture schema | Message table row id, created timestamp, or explicit sequence column if present | Schema includes sessions and messages; parent/thread linkage must be confirmed from fixtures | In-place SQLite updates need per-session content hash, message updated timestamp, or last-seen row strategy |
| Pi agent | `~/.pi/agent/sessions/*.jsonl` | JSONL session files | Session event id or filename, to confirm from fixture | JSONL line number, with parent ids available on message events | Message parent ids exist; decide whether they are event-chain metadata or first-class session linkage | Confirm append-only behavior and whether same-size rewrites occur |
| Factory / Droid-style agent sessions | `~/.factory/sessions/*.jsonl` plus matching `.settings.json` files | JSONL plus per-session settings JSON | Session start id or filename, to confirm from fixture | JSONL line number; message/todo/compaction events carry timestamps | Parent ids may exist inside message payloads; first-class linkage not yet proven | Confirm whether encrypted OpenAI payload fields are useful to preserve as opaque raw events and whether settings changes should affect change detection |
| OpenCode | `~/.local/share/opencode/opencode.db`, with a coexisting `~/.local/share/opencode/storage/{session,message,part,project,session_diff}/` JSON tree on this machine | SQLite appears to be the complete current store in the 2026-05-06 metadata check; JSON storage exists but needs fixture proof before reader scope | `session.id`, with `ses_*` ids visible in JSON storage paths to cross-check by fixture | `message.time_created` + `message.id`, `part.message_id` + `part.id`, or `event.seq` if populated by the source | `session.parent_id` exists; this machine had 14 parent-linked sessions in the metadata check | In-place SQLite updates need per-session content hash or reliable `time_updated`; JSON storage would need file metadata plus content fingerprint if a fixture proves it is authoritative |
| Gemini CLI | `~/.gemini/tmp/<project-or-hash>/chats/*.json` plus `logs.json` sidecars | JSON session files with ordered `messages[]` | `sessionId` inside chat JSON | Array order within `messages[]`, with timestamps as metadata | No first-class child session shape confirmed | Whole-file JSON change detection needs file metadata plus content fingerprint; sidecar logs are reference material unless fixture review proves they add transcript events |

This inventory is metadata-only until each source has a sanitized fixture set
reviewed by a human. The existing Claude CLI and Codex CLI fixtures are the
only committed fixture baseline today.

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
