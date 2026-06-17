CREATE TABLE preference_examples_new (
    source TEXT NOT NULL,
    source_session_id TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 0),
    proposal_event_seq INTEGER NOT NULL CHECK (proposal_event_seq >= 0),
    tool_use_id TEXT NOT NULL,
    file_path TEXT,
    tool_name TEXT NOT NULL,
    proposal_content TEXT,
    context TEXT,
    outcome TEXT NOT NULL CHECK (outcome IN ('accepted', 'rejected', 'edited')),
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    evidence_kind TEXT NOT NULL CHECK (evidence_kind IN (
        'direct_edit',
        'direct_write',
        'bash_correlation',
        'mcp_correlation',
        'permission_denial',
        'missing_final_state'
    )),
    extractor_version TEXT NOT NULL,
    PRIMARY KEY (source, source_session_id, tool_use_id)
);

INSERT INTO preference_examples_new
SELECT * FROM preference_examples;

DROP TABLE preference_examples;

ALTER TABLE preference_examples_new RENAME TO preference_examples;

CREATE INDEX idx_preference_examples_session
    ON preference_examples (source, source_session_id);
