CREATE TABLE taste_extractions (
    source TEXT NOT NULL,
    source_session_id TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    event_count INTEGER NOT NULL CHECK (event_count >= 0),
    PRIMARY KEY (source, source_session_id)
);
