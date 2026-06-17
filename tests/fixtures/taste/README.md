# Taste Extraction Fixture Corpus

Sanitized Claude CLI sessions for taste-extraction development. All content is
synthetic and safe to commit. Paths use fixture-only values such as
`/Users/fixture/Workspace/jottrace`.

See `notes/taste-extraction-plan.md` for the extraction contract.

## Coverage

- `claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000031.jsonl`
  — parent session with inline and `backupFileName` snapshots, `Edit`, `Write` (with sidecar snapshot),
  `Bash` file edits, `NotebookEdit`, MCP file edit, a partial-accept edit pair, a permission denial, a revert,
  and timeline bumps.
- `claude-cli/file-history/00000000-0000-4000-8000-000000000031/`
  — snapshot sidecar blobs referenced by `backupFileName` in the session.
- `claude-cli/projects/-Users-fixture-Workspace-jottrace/00000000-0000-4000-8000-000000000031/subagents/`
  — subagent sidechain that performs an `Edit` on a file.

## Sanitization Contract

- Real prompts, tool outputs, paths, usernames, and session ids were replaced.
- Sidecar blob names are fixture-only hashes (`fixture-a1b2c3d4@vN`).
- No token-like values, private hostnames, or real repository identifiers.
