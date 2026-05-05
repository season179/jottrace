CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    source_session_id TEXT NOT NULL,
    file_path TEXT,
    cwd TEXT,
    parent_session_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
    started_at TEXT,
    ended_at TEXT,
    current_generation INTEGER NOT NULL DEFAULT 0 CHECK (current_generation >= 0),
    file_mtime INTEGER,
    file_size INTEGER,
    content_fingerprint TEXT,
    next_read_offset INTEGER NOT NULL DEFAULT 0 CHECK (next_read_offset >= 0),
    event_count INTEGER NOT NULL DEFAULT 0 CHECK (event_count >= 0),
    last_read_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX idx_sessions_source_session_id
    ON sessions (source, source_session_id);

CREATE INDEX idx_sessions_source_started_at
    ON sessions (source, started_at);

CREATE INDEX idx_sessions_parent_session_id
    ON sessions (parent_session_id)
    WHERE parent_session_id IS NOT NULL;

CREATE TABLE events (
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK (generation >= 0),
    seq INTEGER NOT NULL CHECK (seq >= 0),
    ts TEXT,
    payload BLOB NOT NULL,
    codec TEXT NOT NULL CHECK (codec IN ('raw', 'zstd')),
    payload_size INTEGER NOT NULL CHECK (payload_size >= 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (session_id, generation, seq)
) WITHOUT ROWID;

CREATE INDEX idx_events_session_ts
    ON events (session_id, ts)
    WHERE ts IS NOT NULL;

CREATE TABLE ingest_errors (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    source_session_id TEXT,
    session_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
    file_path TEXT NOT NULL,
    generation INTEGER CHECK (generation IS NULL OR generation >= 0),
    byte_offset INTEGER CHECK (byte_offset IS NULL OR byte_offset >= 0),
    line_number INTEGER CHECK (line_number IS NULL OR line_number >= 0),
    error_kind TEXT NOT NULL,
    message TEXT NOT NULL,
    first_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    occurrence_count INTEGER NOT NULL DEFAULT 1 CHECK (occurrence_count >= 1),
    resolved_at TEXT,
    resolution_note TEXT
);

CREATE INDEX idx_ingest_errors_unresolved
    ON ingest_errors (source, source_session_id, file_path)
    WHERE resolved_at IS NULL;

CREATE INDEX idx_ingest_errors_session_unresolved
    ON ingest_errors (session_id)
    WHERE resolved_at IS NULL AND session_id IS NOT NULL;
