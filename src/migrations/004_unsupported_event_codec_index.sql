CREATE INDEX idx_events_unsupported_codec
    ON events (session_id, generation, seq)
    WHERE codec != 'raw';
