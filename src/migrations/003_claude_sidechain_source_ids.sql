UPDATE sessions
SET source_session_id = (
    SELECT parent.source_session_id || '/subagents/' || sessions.source_session_id
    FROM sessions AS parent
    WHERE parent.id = sessions.parent_session_id
)
WHERE source = 'claude_cli'
  AND source_session_id LIKE 'agent-%'
  AND parent_session_id IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM sessions AS parent
      WHERE parent.id = sessions.parent_session_id
  );

UPDATE sessions
SET source_session_id =
    substr(
        substr(
            file_path,
            1,
            length(file_path) - length('/subagents/' || source_session_id || '.jsonl')
        ),
        -36
    ) || '/subagents/' || source_session_id
WHERE source = 'claude_cli'
  AND source_session_id LIKE 'agent-%'
  AND parent_session_id IS NULL
  AND file_path LIKE '%/subagents/' || source_session_id || '.jsonl'
  AND substr(
      substr(
          file_path,
          1,
          length(file_path) - length('/subagents/' || source_session_id || '.jsonl')
      ),
      -36
  ) GLOB '????????-????-????-????-????????????';

UPDATE ingest_errors AS errors
SET source_session_id = sessions.source_session_id
FROM sessions
WHERE sessions.id = errors.session_id
  AND errors.source = 'claude_cli'
  AND errors.source_session_id LIKE 'agent-%'
  AND sessions.source = 'claude_cli'
  AND sessions.source_session_id LIKE '%/subagents/%';
