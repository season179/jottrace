DROP INDEX IF EXISTS idx_events_unsupported_codec;

CREATE INDEX idx_events_unsupported_codec
    ON events (session_id, generation, seq)
    WHERE codec NOT IN ('raw', 'zstd');
