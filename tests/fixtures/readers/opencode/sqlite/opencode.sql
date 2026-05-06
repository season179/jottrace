-- Sanitized OpenCode SQLite fixture.
-- Reader-relevant shape from metadata-only inspection of
-- ~/.local/share/opencode/opencode.db on 2026-05-06.
-- All row values and payload text below are synthetic.

PRAGMA foreign_keys = ON;

CREATE TABLE `project` (
    `id` text PRIMARY KEY,
    `worktree` text NOT NULL,
    `vcs` text,
    `name` text,
    `icon_url` text,
    `icon_color` text,
    `time_created` integer NOT NULL,
    `time_updated` integer NOT NULL,
    `time_initialized` integer,
    `sandboxes` text NOT NULL,
    `commands` text
);

CREATE TABLE `session` (
    `id` text PRIMARY KEY,
    `project_id` text NOT NULL,
    `parent_id` text,
    `slug` text NOT NULL,
    `directory` text NOT NULL,
    `title` text NOT NULL,
    `version` text NOT NULL,
    `share_url` text,
    `summary_additions` integer,
    `summary_deletions` integer,
    `summary_files` integer,
    `summary_diffs` text,
    `revert` text,
    `permission` text,
    `time_created` integer NOT NULL,
    `time_updated` integer NOT NULL,
    `time_compacting` integer,
    `time_archived` integer,
    `workspace_id` text,
    CONSTRAINT `fk_session_project_id_project_id_fk` FOREIGN KEY (`project_id`) REFERENCES `project`(`id`) ON DELETE CASCADE
);

CREATE TABLE `message` (
    `id` text PRIMARY KEY,
    `session_id` text NOT NULL,
    `time_created` integer NOT NULL,
    `time_updated` integer NOT NULL,
    `data` text NOT NULL,
    CONSTRAINT `fk_message_session_id_session_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session`(`id`) ON DELETE CASCADE
);

CREATE TABLE `part` (
    `id` text PRIMARY KEY,
    `message_id` text NOT NULL,
    `session_id` text NOT NULL,
    `time_created` integer NOT NULL,
    `time_updated` integer NOT NULL,
    `data` text NOT NULL,
    CONSTRAINT `fk_part_message_id_message_id_fk` FOREIGN KEY (`message_id`) REFERENCES `message`(`id`) ON DELETE CASCADE
);

CREATE TABLE `session_entry` (
    `id` text PRIMARY KEY,
    `session_id` text NOT NULL,
    `type` text NOT NULL,
    `time_created` integer NOT NULL,
    `time_updated` integer NOT NULL,
    `data` text NOT NULL,
    CONSTRAINT `fk_session_entry_session_id_session_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session`(`id`) ON DELETE CASCADE
);

CREATE TABLE `event_sequence` (
    `aggregate_id` text PRIMARY KEY,
    `seq` integer NOT NULL
);

CREATE TABLE `event` (
    `id` text PRIMARY KEY,
    `aggregate_id` text NOT NULL,
    `seq` integer NOT NULL,
    `type` text NOT NULL,
    `data` text NOT NULL,
    CONSTRAINT `fk_event_aggregate_id_event_sequence_aggregate_id_fk` FOREIGN KEY (`aggregate_id`) REFERENCES `event_sequence`(`aggregate_id`) ON DELETE CASCADE
);

INSERT INTO `project` (
    `id`,
    `worktree`,
    `vcs`,
    `name`,
    `icon_url`,
    `icon_color`,
    `time_created`,
    `time_updated`,
    `time_initialized`,
    `sandboxes`,
    `commands`
) VALUES (
    '0000000000000000000000000000000000000001',
    '/Users/fixture/Workspace/jottrace',
    'git',
    'fixture-jottrace',
    NULL,
    NULL,
    1770000000000,
    1770000000000,
    1770000000000,
    '[]',
    '{}'
);

INSERT INTO `session` (
    `id`,
    `project_id`,
    `parent_id`,
    `slug`,
    `directory`,
    `title`,
    `version`,
    `share_url`,
    `summary_additions`,
    `summary_deletions`,
    `summary_files`,
    `summary_diffs`,
    `revert`,
    `permission`,
    `time_created`,
    `time_updated`,
    `time_compacting`,
    `time_archived`,
    `workspace_id`
) VALUES
(
    'ses_fixture_parent_00000000000',
    '0000000000000000000000000000000000000001',
    NULL,
    'fixture-parent-session',
    '/Users/fixture/Workspace/jottrace',
    'Fixture parent session',
    '1.4.10',
    NULL,
    2,
    1,
    1,
    '{"files":["src/lib.rs"],"note":"synthetic summary diff"}',
    NULL,
    '{"mode":"ask"}',
    1770000000000,
    1770000003000,
    NULL,
    NULL,
    NULL
),
(
    'ses_fixture_child_000000000000',
    '0000000000000000000000000000000000000001',
    'ses_fixture_parent_00000000000',
    'fixture-child-session',
    '/Users/fixture/Workspace/jottrace',
    'Fixture child session',
    '1.4.10',
    NULL,
    1,
    0,
    1,
    '{"files":["tests/fixture_corpus.rs"],"note":"synthetic child summary"}',
    NULL,
    '{"mode":"ask"}',
    1770000004000,
    1770000007000,
    NULL,
    NULL,
    NULL
);

INSERT INTO `message` (
    `id`,
    `session_id`,
    `time_created`,
    `time_updated`,
    `data`
) VALUES
(
    'msg_fixture_parent_00000000000',
    'ses_fixture_parent_00000000000',
    1770000000000,
    1770000000000,
    '{"role":"user","time":{"created":1770000000000},"path":{"cwd":"/Users/fixture/Workspace/jottrace"}}'
),
(
    'msg_fixture_parent_reply_000000',
    'ses_fixture_parent_00000000000',
    1770000001000,
    1770000003000,
    '{"role":"assistant","providerID":"fixture","modelID":"fixture-model","mode":"build","time":{"created":1770000001000,"completed":1770000003000},"tokens":{"input":12,"output":18},"cost":0}'
),
(
    'msg_fixture_child_000000000000',
    'ses_fixture_child_000000000000',
    1770000004000,
    1770000004000,
    '{"role":"user","parentID":"msg_fixture_parent_reply_000000","time":{"created":1770000004000},"path":{"cwd":"/Users/fixture/Workspace/jottrace"}}'
),
(
    'msg_fixture_child_reply_0000000',
    'ses_fixture_child_000000000000',
    1770000005000,
    1770000007000,
    '{"role":"assistant","providerID":"fixture","modelID":"fixture-model","mode":"build","time":{"created":1770000005000,"completed":1770000007000},"tokens":{"input":16,"output":24},"tools":{"bash":1},"cost":0}'
);

INSERT INTO `part` (
    `id`,
    `message_id`,
    `session_id`,
    `time_created`,
    `time_updated`,
    `data`
) VALUES
(
    'prt_fixture_parent_text_000000',
    'msg_fixture_parent_00000000000',
    'ses_fixture_parent_00000000000',
    1770000000000,
    1770000000000,
    '{"type":"text","text":"Please inspect the sanitized fixture project.","time":{"start":1770000000000,"end":1770000000000}}'
),
(
    'prt_fixture_parent_reason_0000',
    'msg_fixture_parent_reply_000000',
    'ses_fixture_parent_00000000000',
    1770000001000,
    1770000001000,
    '{"type":"reasoning","text":"Synthetic reasoning placeholder.","time":{"start":1770000001000,"end":1770000001000}}'
),
(
    'prt_fixture_parent_tool_00000',
    'msg_fixture_parent_reply_000000',
    'ses_fixture_parent_00000000000',
    1770000002000,
    1770000003000,
    '{"type":"tool","tool":"bash","callID":"call_fixture_parent","state":{"status":"completed","input":{"command":"cargo test"},"output":"synthetic test output"},"time":{"start":1770000002000,"end":1770000003000}}'
),
(
    'prt_fixture_child_text_00000000',
    'msg_fixture_child_000000000000',
    'ses_fixture_child_000000000000',
    1770000004000,
    1770000004000,
    '{"type":"text","text":"Please continue from the parent session.","time":{"start":1770000004000,"end":1770000004000}}'
),
(
    'prt_fixture_child_reply_0000000',
    'msg_fixture_child_reply_0000000',
    'ses_fixture_child_000000000000',
    1770000005000,
    1770000007000,
    '{"type":"text","text":"Synthetic child-session response.","time":{"start":1770000005000,"end":1770000007000}}'
);
