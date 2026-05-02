---
title: AI Work Journal Redesign Plan
created: 2026-05-01
updated: 2026-05-01
status: discussing
tags:
  - work-journal
  - ai-agents
  - obsidian
  - design
journal_version: 3
---

# AI Work Journal Redesign Plan

This note tracks the redesign discussion for the AI coding-agent work journal.

## Product Thesis

The product is automatic temporal documentation for AI-assisted engineering work.

The core insight comes from Simon Willison's "Coping strategies for the serial project hoarder": the most useful engineering memory is often not polished documentation, but timestamped context about what was being attempted, what designs were considered, what failed, and what decisions were made. Simon describes GitHub issue threads as a practical lab notebook for engineering work: they preserve background, links, code snippets, false starts, screenshots, decisions, and eventual outcomes in the context of the work at the time.

AI coding sessions already contain that material:

- prompts explain intent and motivation
- tool calls show investigation paths
- failed commands and rejected patches show dead ends
- review comments and follow-up prompts show decisions
- final summaries often capture outcome and remaining uncertainty

The opportunity is to turn those raw transcripts into the same kind of work journal automatically.

One-line pitch:

> The journal writes itself, dead ends and all.

Source: [Simon Willison - Coping strategies for the serial project hoarder](https://simonwillison.net/2022/Nov/26/productivity/)

## Current Agreement

- Keep Obsidian as the human-readable journal destination.
- Replace the old Hermes-dependent writer flow.
- Use a SQLite state and ledger so processed sessions are tracked explicitly.
- A processed session should not be processed again unless its source session changes.
- Preserve useful lessons from the current `work-journal` output, but do not rebuild around the old collector shape.

## Current Situation

- Existing journal root: `~/obsidian-vault/work-journal`
- Existing old collector: `~/.hermes/scripts/ai-work-journal/collector.py`
- Existing old wrapper: `~/.local/bin/hermes-work-journal`
- Existing output shape:
  - daily notes in `work-journal/daily/YYYY-MM-DD.md`
  - topic notes in `work-journal/topics/<topic>.md`
  - state files under `work-journal/state/`
- Known old weak spots:
  - state is cursor/hash based instead of a durable per-session ledger
  - daily notes are treated as the main generated artifact, which makes updated sessions awkward
  - the writer shells out to Hermes and then verifies afterward
  - backfill required several ad hoc scripts and reseeding passes

## Online Research Snapshot

Research date: 2026-05-01.

- Obsidian Properties are YAML frontmatter fields with typed values such as text, numbers, dates, date-times, lists, booleans, tags, and internal links. This supports machine-readable metadata while keeping notes plain Markdown. Source: [Obsidian Properties](https://obsidian.md/help/properties)
- Obsidian supports both Wikilinks and standard Markdown links. Wikilinks are compact and default in Obsidian; Markdown links are more interoperable. Source: [Obsidian Internal Links](https://obsidian.md/help/links)
- Obsidian-flavored Markdown supports block IDs, comments, task lists, callouts, tables, and links. Block IDs are useful for stable references inside daily notes, but they are Obsidian-specific. Source: [Obsidian Flavored Markdown](https://obsidian.md/help/obsidian-flavored-markdown)
- Dataview can query YAML frontmatter automatically. Inline fields are useful for item-level metadata, but canonical metadata should live in frontmatter where possible. Source: [Dataview - Adding Metadata](https://blacksmithgu.github.io/obsidian-dataview/annotation/add-metadata/)
- Obsidian Bases can query note properties and file properties, which makes typed frontmatter a good future-facing choice even without Dataview. Source: [Obsidian Bases Syntax](https://obsidian.md/help/bases/syntax)
- ADR/MADR formats are useful when the journal needs to preserve decisions, alternatives, context, and consequences. They should inform the "Decisions" section, not dominate every session note. Sources: [MIT ADR guide](https://mitlibraries.github.io/guides/misc/adr.html), [MADR](https://adr.github.io/madr/)
- Bullet Journal rapid logging is useful for daily notes because it separates tasks, events, and notes into concise entries. This maps well to human daily rollups, but not to canonical session storage. Source: [Bullet Journal Rapid Logging](https://bulletjournal.com/blogs/faq/what-is-rapid-logging-understand-rapid-logging-bullets-and-signifiers)
- CommonMark is the portability baseline. Prefer ordinary headings, lists, fenced code blocks, and links unless an Obsidian feature provides a clear value. Source: [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/)

## Format Recommendation

Use a three-layer Markdown architecture.

## Normalization Pipeline

The source sessions can stay different. The program should not try to make Codex, Claude CLI, OpenCode, Pi, Factory, and Hermes store the same raw transcript shape.

Instead, each source gets an adapter that translates its native transcript into one shared internal model:

```text
source file -> source adapter -> canonical session record -> Markdown renderer
```

The adapter layer absorbs source-specific details:

- where the session ID lives
- how timestamps are represented
- how user messages, assistant messages, tool calls, command output, and final answers are encoded
- where cwd, repo, branch, model, and title metadata live
- whether the source is append-only, SQLite-backed, JSONL, JSON, or plain text

After adapter parsing, every source becomes the same kind of object:

```text
Session
- source
- source_session_id
- source_path
- source_signature
- started_at
- last_event_at
- cwd / repo / branch
- participants or agent names
- turns
- tool actions
- files and commands mentioned
- detected outcomes
- raw evidence pointers
```

The Markdown writer only sees that canonical object. It does not care whether the source was Codex JSONL, Claude CLI JSONL, OpenCode SQLite, or something else.

This keeps the generated note human-readable because the note is not a schema dump. The schema is used internally to produce a small narrative:

- what was being attempted
- what changed or was investigated
- what worked
- what failed or remains unclear
- what artifacts matter later
- which source session proves it

The rule of thumb: preserve raw transcripts where they already live, store normalized evidence in SQLite, and render only the useful human memory into Obsidian.

## LLM Writing Boundary

Yes, the system should use an LLM, but the LLM should not be the component that freely edits Obsidian files.

Preferred pipeline:

```text
Canonical session
-> deterministic evidence packet
-> LLM structured summary
-> validation
-> deterministic Markdown renderer
-> Obsidian files
```

The LLM's job:

- infer what the work was about
- distinguish motivation, actions, outcome, failures, uncertainty, and follow-ups
- choose a concise human title
- suggest topics and related sessions
- write short narrative fields
- identify questions when the source evidence is incomplete

The program's job:

- decide which sessions need processing
- build the evidence packet
- enforce the output schema
- validate file paths, timestamps, source IDs, and hashes
- render frontmatter and Markdown templates
- update SQLite ledger rows transactionally
- write Obsidian notes safely

The LLM should return structured data, not directly edit Markdown:

```json
{
  "title": "Flow Orchestrator index review",
  "summary": "...",
  "timeline": [
    {"time": "09:34", "text": "..."}
  ],
  "decisions": [],
  "outcome": {
    "status": "partial",
    "worked": ["..."],
    "did_not_work": [],
    "unclear": ["..."]
  },
  "artifacts": [
    "/Users/season/Repositories/flow-orchestrator"
  ],
  "followups": [],
  "topics": ["flow-orchestrator", "admin-v3", "postgres"],
  "confidence": "medium",
  "questions": []
}
```

Then a deterministic renderer turns that JSON into Markdown. This avoids the old failure mode where an agent rewrites daily notes, drops block IDs, invents links, or edits state files.

An agent may still be useful for exceptional cases:

- backfill review when many sessions need clustering
- difficult cross-session linking
- manual repair of malformed historical notes
- answering "what happened with this project?" from the journal

But the regular ingestion path should be a program with LLM calls, not a general-purpose agent mutating files.

### 1. Session Notes

Session notes should be the canonical generated artifact.

Proposed path:

```text
work-journal/sessions/YYYY/MM/<source>/<session-key>.md
```

Why:

- The SQLite ledger naturally tracks sessions and session revisions.
- If a source session changes, the program can update one session note instead of trying to surgically rewrite old daily blocks.
- Session notes can contain rich metadata without making daily notes noisy.
- The raw transcript can stay in its original location; the session note stores the useful human summary plus source trace.

Session notes should be allowed to update inside clearly marked generated regions when the source session changes.

### 2. Daily Notes

Daily notes should remain the main human reading surface.

Existing path:

```text
work-journal/daily/YYYY-MM-DD.md
```

Daily notes should summarize work by time/topic, link back to canonical session notes, and preserve stable block IDs for recall.

Daily notes should feel like a work journal, not like a dump of sessions.

### 3. Topic And Project Notes

Topic notes should be indexes, not full duplicated summaries.

Existing path:

```text
work-journal/topics/<topic>.md
```

They should link to the best daily entries and session notes, with less topic sprawl than the current system.

## Proposed Session Note Template

```md
---
type: ai-session
journal_version: 3
source: codex
source_session_id: "019dd950-..."
source_path: "/Users/season/.codex/sessions/..."
source_signature: "sha256-or-size-mtime"
ledger_revision: 3
status: complete
started_at: 2026-04-29T20:57:00+08:00
last_event_at: 2026-04-29T22:14:08+08:00
processed_at: 2026-05-01T12:00:00+08:00
cwd: "/Users/season/Personal/smartknowledge/agent-bappeda"
repo: agent-bappeda
branch: revamp-tanstack
sources:
  - codex
models:
  - gpt-5.4
topics:
  - agent-bappeda
  - tanstack
  - evals
outcome: partial
confidence: medium
daily: "[[work-journal/daily/2026-04-29]]"
related_topics:
  - "[[work-journal/topics/agent-bappeda]]"
related_sessions: []
---

# Short Human Title

## Summary

Two to five sentences covering why the session happened, how the work moved, and what outcome is actually visible.

## Timeline

- 20:57 - Session started around ...
- 21:14 - Investigated ...
- 22:03 - Verified ...

## Decisions

- Decided to ...
- Rejected ... because ...

## Outcome

- Worked: ...
- Did not work: ...
- Unclear: ...

## Artifacts

- `/absolute/path/to/file`
- `command or test name`
- external URL if relevant

## Follow-ups

- [ ] Concrete next action if one is visible.

## Source Trace

- Source: codex
- Session ID: `019dd950-...`
- Source path: `/Users/season/.codex/sessions/...`
- Source signature: `...`

<!-- generated:session-summary:start -->
Generated content owned by the journal program.
<!-- generated:session-summary:end -->
```

## Proposed Daily Note Entry Template

```md
## HH:MM - Short topic title

- Type: event | note | decision | follow-up
- Sessions: [[work-journal/sessions/2026/04/codex/<session-key>]]
- Topics: [[work-journal/topics/agent-bappeda]], [[work-journal/topics/tanstack]]
- Outcome: partial
- Summary: One compact paragraph focused on what Season would want to remember later.
- Follow-ups:
  - [ ] Optional concrete next action.

<!-- session_keys: codex:<session-id>, claude-cli:<session-id> -->
<!-- content_hash: <hash-of-generated-daily-entry> -->
^wj-<stable-id>
```

## Recommended Rules

- Keep raw transcripts out of Obsidian unless there is a specific reason to preserve an excerpt.
- Put canonical machine-readable metadata in YAML frontmatter.
- Use generated-region comments for content the program may update.
- Use stable block IDs in daily notes for Obsidian recall.
- Use task lists only for real follow-ups, not for every action observed in a transcript.
- Use ADR-style subsections only when a real decision or tradeoff occurred.
- Prefer fewer, better topic notes. Topics should represent reusable project memory, not every keyword.
- Keep body prose concise and truth-grounded. If outcome is unclear, say so explicitly.

## Project Naming Research

Research date: 2026-05-01.

Current KIV from first batch:

- `Trylog`: still viable; small collisions, but no strong wrong association found yet.
- `Aftertrace`: still viable; distinctive, but slightly forensic/surveillance-flavored.

Second batch:

| Name | Signal | Collision notes | Verdict |
| --- | --- | --- | --- |
| `Diffiary` | diff + diary | Search mostly found generic/unbranded watch listings, not software. Example: [Allegro listing](https://allegro.pl/oferta/diffiary-watch-waterproof-electronic-sports-mountainering-outdoor-rd-17874148884). | Strong KIV if the pun feels acceptable. |
| `Jottrace` | jot + trace | No meaningful exact-match software/product collision found in web, GitHub, npm, or PyPI searches. | Strong KIV; clearer than `TraceJot`. |
| `TraceJot` | trace + jot | No meaningful exact-match software/product collision found, but it sounds slightly less natural than `Jottrace`. | Maybe. |
| `Forenote` | a preceding note or preface | Existing English word meaning a preliminary note/preface. Source: [Wiktionary](https://en.wiktionary.org/wiki/forenote). | Strong KIV, though it suggests pre-work more than after-work. |
| `Buildwake` | wake left by building work | No meaningful exact-match product collision found; weaker semantic clarity. | Maybe. |
| `Labwake` | lab notebook aftermath | No meaningful exact-match product collision found; may read like a Wake Forest lab. | Maybe. |
| `Patchtrail` | patch history trail | Product collision with outdoor shoes. Example: [PatchTrail sneaker](https://artoaku.com/products/stridez-patchtrail-men-s-multi-color-outdoor-street-sneakers). | Weak. |
| `Workwake` | wake of work | Too close visually/aurally to WorkWave, a real business software company. Source: [WorkWave](https://www.workwave.com/company/). | Avoid. |
| `Threadwake` | wake of conversation threads | Collides with a sci-fi novella and parked domain. Sources: [Threadwake book](https://www.walmart.com/ip/17370606785), [threadwake.com](https://www.threadwake.com/). | Avoid. |
| `Loglore` | lore from logs | Too fantasy/gaming flavored; not worth carrying forward. | Avoid. |
| `Prompttrail` | prompt history trail | Direct collision with LLM/dev tools. Sources: [PromptTrail PyPI](https://pypi.org/project/prompttrail/), [PromptTrail early access](https://prompttrail.observantconvo.com/). | Avoid. |
| `Runledger` | ledger of runs | Direct collision with an agent CI/eval package. Source: [RunLedger PyPI](https://pypi.org/project/runledger/). | Avoid. |
| `Tracewright` | writer of traces | Direct collision with an AI Playwright regression testing tool. Source: [Tracewright](https://tracewright.com/). | Avoid. |
| `Threadscribe` | scribe for threads | Direct collision with Slack/thread summarization tools. Source: [ThreadScribe.ai](https://pitchwall.co/product/threadscribeai). | Avoid. |
| `Tryscribe` | scribe for attempts | Direct collision with an AI productivity suite. Source: [TryScribe](https://tryscribe.in/). | Avoid. |

Current naming shortlist:

- `Trylog`
- `Aftertrace`
- `Diffiary`
- `Jottrace`
- `Forenote`

## Open Decisions

1. Should session notes be visible in normal Obsidian browsing, or kept in a generated subfolder that is mostly machine-facing?
2. Should daily notes be append-only forever, or should the program be allowed to update generated daily entries when a session changes?
3. Should body links use existing Obsidian Wikilinks, or should the new program prefer standard Markdown links for portability?
4. What sources are in scope for the first version: Codex, Claude CLI, Claude local, Hermes sessions, OpenCode, Pi, Factory, agent-orchestrator?
5. How much summarization should be deterministic extraction versus LLM narrative writing?
6. Should the journal have a monthly index or dashboard generated from SQLite/frontmatter?

## Current Leaning

The cleanest design is:

- SQLite is the authority for source discovery, source signatures, session revisions, and processing status.
- Session notes are canonical generated records.
- Daily notes are human-friendly rollups linked to session notes.
- Topic notes are sparse indexes.
- The writer should be a standalone program, not a Hermes prompt wrapper.

## Discussion Log

### 2026-05-01

- Season asked to redo the AI work journal without Hermes-agent while still writing to Obsidian.
- We agreed on SQLite state/ledger.
- We started discussing the Markdown note format before implementation.
- Initial recommendation after online research: use typed YAML frontmatter plus canonical session notes, with daily notes as the readable work journal layer.
- Season connected the product direction to Simon Willison's temporal documentation idea: AI coding sessions already contain the lab-notebook material developers rarely write by hand, including false starts, decisions, and failures.
- We started naming research. `Trylog` and `Aftertrace` are KIV from the first batch. Second-batch shortlist: `Diffiary`, `Jottrace`, and `Forenote`.

## Related Existing Notes

- [[work-journal/plan/AI Work Journal Automation Plan]]
- [[work-journal/index/AI Session Source Inventory]]
- [[work-journal/index/Automation Setup]]
