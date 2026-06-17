CREATE TABLE file_timelines (
    source TEXT NOT NULL,
    source_session_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    seq INTEGER NOT NULL CHECK (seq >= 0),
    event_seq INTEGER NOT NULL CHECK (event_seq >= 0),
    content TEXT,
    trigger_event_ref TEXT,
    source_kind TEXT NOT NULL CHECK (source_kind IN (
        'inline_snapshot',
        'sidecar_snapshot',
        'missing_sidecar'
    )),
    PRIMARY KEY (source, source_session_id, file_path, seq)
);

CREATE INDEX idx_file_timelines_session_file
    ON file_timelines (source, source_session_id, file_path);
