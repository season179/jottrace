CREATE INDEX idx_sessions_source_file_path
    ON sessions (source, file_path)
    WHERE file_path IS NOT NULL;
