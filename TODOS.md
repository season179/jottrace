# TODOS

## Readers

### Capture Cursor and OpenCode Reader Fixtures

**What:** Capture sanitized real Cursor and OpenCode session fixtures before implementing those readers.

**Why:** The current design treats Cursor and OpenCode as designed stubs, but both have unresolved source-shape questions that should be settled from real artifacts before reader code is written.

**Status:** OpenCode SQLite reader-shape fixture added in `tests/fixtures/readers/opencode/sqlite/opencode.sql`; human fixture review still pending. Cursor capture is still pending because no local Cursor source DB was available during the 2026-05-06 fixture pass.

**Context:** Cursor may vary by version and storage table shape, while OpenCode needed real confirmation of message/part ordering and parent-child linkage. Start from the source notes in `docs/design.md`, capture sanitized fixtures, then update the reader contract before cutting implementation issues.

**Effort:** M
**Priority:** P2
**Depends on:** Claude and Codex preservation MVP

## Completed
