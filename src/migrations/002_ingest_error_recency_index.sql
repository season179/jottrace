UPDATE ingest_errors
SET line_number = line_number + 1
WHERE line_number IS NOT NULL;

CREATE INDEX idx_ingest_errors_unresolved_last_seen
    ON ingest_errors (last_seen_at DESC, id DESC)
    WHERE resolved_at IS NULL;
