CREATE INDEX idx_events_raw_compaction
    ON events (session_id, generation, seq)
    WHERE codec = 'raw';
