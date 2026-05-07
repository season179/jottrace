-- Sanitized Hermes SQLite SessionDB fixture.
-- Reader-relevant shape from metadata-only inspection of
-- ~/.hermes/state.db on 2026-05-07.
-- All row values and payload text below are synthetic.

PRAGMA foreign_keys = ON;

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    user_id TEXT,
    model TEXT,
    model_config TEXT,
    system_prompt TEXT,
    parent_session_id TEXT,
    started_at REAL NOT NULL,
    ended_at REAL,
    end_reason TEXT,
    message_count INTEGER DEFAULT 0,
    tool_call_count INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0,
    cache_write_tokens INTEGER DEFAULT 0,
    reasoning_tokens INTEGER DEFAULT 0,
    billing_provider TEXT,
    billing_base_url TEXT,
    billing_mode TEXT,
    estimated_cost_usd REAL,
    actual_cost_usd REAL,
    cost_status TEXT,
    cost_source TEXT,
    pricing_version TEXT,
    title TEXT,
    api_call_count INTEGER DEFAULT 0,
    FOREIGN KEY (parent_session_id) REFERENCES sessions(id)
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    role TEXT NOT NULL,
    content TEXT,
    tool_call_id TEXT,
    tool_calls TEXT,
    tool_name TEXT,
    timestamp REAL NOT NULL,
    token_count INTEGER,
    finish_reason TEXT,
    reasoning TEXT,
    reasoning_details TEXT,
    codex_reasoning_items TEXT,
    reasoning_content TEXT,
    codex_message_items TEXT
);

CREATE VIRTUAL TABLE messages_fts USING fts5(content);
CREATE VIRTUAL TABLE messages_fts_trigram USING fts5(content, tokenize='trigram');

CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content) VALUES (
        new.id,
        COALESCE(new.content, '') || ' ' || COALESCE(new.tool_name, '') || ' ' || COALESCE(new.tool_calls, '')
    );
END;

CREATE TRIGGER messages_fts_trigram_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts_trigram(rowid, content) VALUES (
        new.id,
        COALESCE(new.content, '') || ' ' || COALESCE(new.tool_name, '') || ' ' || COALESCE(new.tool_calls, '')
    );
END;

CREATE TABLE schema_version (version INTEGER NOT NULL);
CREATE TABLE state_meta (key TEXT PRIMARY KEY, value TEXT);

INSERT INTO sessions (
    id,
    source,
    user_id,
    model,
    model_config,
    system_prompt,
    parent_session_id,
    started_at,
    ended_at,
    end_reason,
    message_count,
    tool_call_count,
    input_tokens,
    output_tokens,
    cache_read_tokens,
    cache_write_tokens,
    reasoning_tokens,
    billing_provider,
    billing_base_url,
    billing_mode,
    estimated_cost_usd,
    actual_cost_usd,
    cost_status,
    cost_source,
    pricing_version,
    title,
    api_call_count
) VALUES
(
    'hermes_fixture_parent_0000000000',
    'hermes-cli',
    'fixture-user',
    'fixture-model',
    '{"temperature":0}',
    'fixture system prompt placeholder',
    NULL,
    1770000000.0,
    1770000060.0,
    'stop',
    2,
    0,
    12,
    18,
    0,
    0,
    4,
    'fixture-provider',
    'https://fixture.invalid/v1',
    'metered',
    0.001,
    0.001,
    'estimated',
    'fixture-pricing',
    '2026-05-07',
    'Fixture parent session',
    1
),
(
    'hermes_fixture_child_00000000000',
    'hermes-cli',
    'fixture-user',
    'fixture-model',
    '{"temperature":0}',
    'fixture system prompt placeholder',
    'hermes_fixture_parent_0000000000',
    1770000100.0,
    1770000130.0,
    'stop',
    3,
    1,
    16,
    24,
    0,
    0,
    6,
    'fixture-provider',
    'https://fixture.invalid/v1',
    'metered',
    0.002,
    0.002,
    'estimated',
    'fixture-pricing',
    '2026-05-07',
    'Fixture child session',
    1
);

INSERT INTO messages (
    id,
    session_id,
    role,
    content,
    tool_call_id,
    tool_calls,
    tool_name,
    timestamp,
    token_count,
    finish_reason,
    reasoning,
    reasoning_details,
    codex_reasoning_items,
    reasoning_content,
    codex_message_items
) VALUES
(
    1,
    'hermes_fixture_parent_0000000000',
    'user',
    'Please inspect the sanitized Hermes fixture.',
    NULL,
    NULL,
    NULL,
    1770000005.0,
    12,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL
),
(
    2,
    'hermes_fixture_parent_0000000000',
    'assistant',
    'The fixture shape is safe to preserve.',
    NULL,
    NULL,
    NULL,
    1770000030.0,
    18,
    'stop',
    'fixture reasoning placeholder',
    '[{"type":"summary","text":"synthetic reasoning detail"}]',
    '[{"id":"reasoning-fixture-parent","summary":"synthetic"}]',
    'fixture reasoning content placeholder',
    '[{"type":"message","role":"assistant","content":"synthetic"}]'
),
(
    12,
    'hermes_fixture_child_00000000000',
    'user',
    'Please continue from the parent SessionDB row.',
    NULL,
    NULL,
    NULL,
    1770000101.0,
    16,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL
),
(
    10,
    'hermes_fixture_child_00000000000',
    'assistant',
    NULL,
    NULL,
    '[{"id":"call_fixture_hermes_001","type":"function","function":{"name":"read_fixture_metadata","arguments":"{}"}}]',
    'read_fixture_metadata',
    1770000110.0,
    20,
    NULL,
    'fixture tool planning placeholder',
    '[{"type":"summary","text":"synthetic tool planning detail"}]',
    '[{"id":"reasoning-fixture-child","summary":"synthetic"}]',
    'fixture tool reasoning content placeholder',
    '[{"type":"tool_call","name":"read_fixture_metadata"}]'
),
(
    11,
    'hermes_fixture_child_00000000000',
    'assistant',
    'Hermes fixture import completed.',
    'call_fixture_hermes_001',
    NULL,
    NULL,
    1770000110.0,
    24,
    'stop',
    NULL,
    NULL,
    NULL,
    NULL,
    NULL
);

INSERT INTO schema_version (version) VALUES (1);
INSERT INTO state_meta (key, value) VALUES ('fixture', 'sanitized');
