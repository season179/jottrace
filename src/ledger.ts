import { Database, SQLiteError } from "bun:sqlite";
import type { Harness, Session } from "./models.ts";

export type TopicSourceKind =
	| "cwd"
	| "git_branch"
	| "repo"
	| "file_path"
	| "error"
	| "outcome"
	| "llm";

export type SynthesisStatus = "pending" | "done" | "retry" | "skipped";

export type SyncOutcome = "success" | "partial" | "error";

export interface SessionSummaryRow {
	harness: Harness;
	source_session_id: string;
	schema_version: number;
	summary_json: string;
	synthesized_at: string;
	provider: string;
	model: string;
}

export interface SyncRunRow {
	id: number;
	started_at: string;
	ended_at: string | null;
	outcome: SyncOutcome | null;
	sessions_seen: number;
	sessions_new: number;
	sessions_updated: number;
	synthesis_done: number;
	synthesis_failed: number;
	error_message: string | null;
}

export interface InsertSessionResult {
	short_id: string;
	collision_recovered: boolean;
}

export interface Digests {
	short: string;
	full: string;
}

export interface Ledger {
	readonly db: Database;
	close(): void;
	insertSession(session: Session, digests?: Digests): InsertSessionResult;
}

const SCHEMA_SQL = `
CREATE TABLE IF NOT EXISTS sessions (
	harness            TEXT NOT NULL,
	source_session_id  TEXT NOT NULL,
	parent_session_id  TEXT,
	source_path        TEXT NOT NULL,
	source_revision    TEXT NOT NULL,
	source_size        INTEGER NOT NULL,
	source_mtime       INTEGER NOT NULL,
	byte_offset        INTEGER NOT NULL DEFAULT 0,
	started_at         TEXT NOT NULL,
	ended_at           TEXT NOT NULL,
	cwd                TEXT,
	project            TEXT,
	git_branch         TEXT,
	model              TEXT,
	event_count        INTEGER NOT NULL,
	short_id           TEXT NOT NULL,
	note_path          TEXT,
	synthesis_status   TEXT NOT NULL DEFAULT 'pending',
	synthesis_error    TEXT,
	synthesis_attempts INTEGER NOT NULL DEFAULT 0,
	processed_at       TEXT NOT NULL,
	PRIMARY KEY (harness, source_session_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS sessions_short_id ON sessions (short_id);
CREATE INDEX IF NOT EXISTS sessions_started ON sessions (started_at);
CREATE INDEX IF NOT EXISTS sessions_project ON sessions (project);
CREATE INDEX IF NOT EXISTS sessions_synthesis_status ON sessions (synthesis_status) WHERE synthesis_status != 'done';
CREATE INDEX IF NOT EXISTS sessions_parent ON sessions (parent_session_id) WHERE parent_session_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS topic_candidates (
	harness            TEXT NOT NULL,
	source_session_id  TEXT NOT NULL,
	label              TEXT NOT NULL,
	source_kind        TEXT NOT NULL,
	weight             REAL NOT NULL DEFAULT 1.0,
	PRIMARY KEY (harness, source_session_id, label, source_kind),
	FOREIGN KEY (harness, source_session_id) REFERENCES sessions (harness, source_session_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS topic_candidates_label ON topic_candidates (label);

CREATE TABLE IF NOT EXISTS session_summaries (
	harness            TEXT NOT NULL,
	source_session_id  TEXT NOT NULL,
	schema_version     INTEGER NOT NULL DEFAULT 1,
	summary_json       TEXT NOT NULL,
	synthesized_at     TEXT NOT NULL,
	provider           TEXT NOT NULL,
	model              TEXT NOT NULL,
	PRIMARY KEY (harness, source_session_id),
	FOREIGN KEY (harness, source_session_id) REFERENCES sessions (harness, source_session_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS sync_runs (
	id              INTEGER PRIMARY KEY AUTOINCREMENT,
	started_at      TEXT NOT NULL,
	ended_at        TEXT,
	outcome         TEXT,
	sessions_seen   INTEGER NOT NULL DEFAULT 0,
	sessions_new    INTEGER NOT NULL DEFAULT 0,
	sessions_updated INTEGER NOT NULL DEFAULT 0,
	synthesis_done  INTEGER NOT NULL DEFAULT 0,
	synthesis_failed INTEGER NOT NULL DEFAULT 0,
	error_message   TEXT
);
`;

const INSERT_SESSION_SQL = `
INSERT INTO sessions (
	harness, source_session_id, parent_session_id,
	source_path, source_revision, source_size, source_mtime, byte_offset,
	started_at, ended_at, cwd, project, git_branch, model,
	event_count, short_id, note_path,
	synthesis_status, synthesis_error, synthesis_attempts, processed_at
) VALUES (
	$harness, $source_session_id, $parent_session_id,
	$source_path, $source_revision, $source_size, $source_mtime, $byte_offset,
	$started_at, $ended_at, $cwd, $project, $git_branch, $model,
	$event_count, $short_id, $note_path,
	$synthesis_status, $synthesis_error, $synthesis_attempts, $processed_at
)
`;

export function computeDigests(
	harness: Harness,
	source_session_id: string,
): Digests {
	const hasher = new Bun.CryptoHasher("sha256");
	hasher.update(`${harness}:${source_session_id}`);
	const full = hasher.digest("hex");
	return { full, short: full.slice(0, 12) };
}

export function openLedger(path: string): Ledger {
	const db = new Database(path);
	db.run("PRAGMA journal_mode = WAL");
	db.run("PRAGMA busy_timeout = 5000");
	db.run("PRAGMA foreign_keys = ON");
	for (const stmt of splitStatements(SCHEMA_SQL)) {
		db.run(stmt);
	}

	const insertStmt = db.prepare<never, [SessionInsertParams]>(
		INSERT_SESSION_SQL,
	);

	const ledger: Ledger = {
		db,
		close() {
			db.close();
		},
		insertSession(session, digests) {
			const computed =
				digests ?? computeDigests(session.harness, session.source_session_id);
			const params = sessionParams(session, computed.short);
			try {
				insertStmt.run(params);
				return { short_id: computed.short, collision_recovered: false };
			} catch (err) {
				if (!isUniqueShortIdError(err)) throw err;
				insertStmt.run({ ...params, $short_id: computed.full });
				return { short_id: computed.full, collision_recovered: true };
			}
		},
	};

	return ledger;
}

export interface UpsertSessionSkeletonArgs {
	session: Session;
	note_path: string;
	processed_at: string;
	digests?: Digests;
	short_id?: string;
}

export function resolveSessionShortId(
	ledger: Ledger,
	session: Session,
	digests = computeDigests(session.harness, session.source_session_id),
): string {
	const existing = ledger.db
		.query<
			{ harness: Harness; source_session_id: string; short_id: string },
			[string]
		>(
			"SELECT harness, source_session_id, short_id FROM sessions WHERE short_id = ?",
		)
		.get(digests.short);

	if (!existing) return digests.short;
	if (
		existing.harness === session.harness &&
		existing.source_session_id === session.source_session_id
	) {
		return existing.short_id;
	}
	return digests.full;
}

export function upsertSessionSkeleton(
	ledger: Ledger,
	args: UpsertSessionSkeletonArgs,
): InsertSessionResult {
	const existing = ledger.db
		.query<SessionSkeletonRow, [Harness, string]>(
			`SELECT
				short_id, parent_session_id, source_path, source_revision, source_size,
				source_mtime, byte_offset, started_at, ended_at, cwd, project,
				git_branch, model, event_count, note_path, synthesis_status,
				synthesis_error, synthesis_attempts
			FROM sessions WHERE harness = ? AND source_session_id = ?`,
		)
		.get(args.session.harness, args.session.source_session_id);

	if (existing) {
		if (!sessionSkeletonMatches(existing, args)) {
			updateSessionSkeleton(ledger, args, existing.short_id);
		}
		return { short_id: existing.short_id, collision_recovered: false };
	}

	const digests =
		args.digests ??
		computeDigests(args.session.harness, args.session.source_session_id);
	const short_id =
		args.short_id ?? resolveSessionShortId(ledger, args.session, digests);
	const result = insertSessionSkeleton(ledger, args, short_id);
	if (result) {
		return {
			...result,
			collision_recovered:
				short_id === digests.full && short_id !== digests.short,
		};
	}

	insertSessionSkeleton(ledger, args, digests.full);
	return { short_id: digests.full, collision_recovered: true };
}

export interface UpsertTopicCandidateArgs {
	harness: Harness;
	source_session_id: string;
	label: string;
	source_kind: TopicSourceKind;
	weight?: number;
}

export function upsertTopicCandidate(
	ledger: Ledger,
	args: UpsertTopicCandidateArgs,
): void {
	const weight = args.weight ?? 1.0;
	ledger.db
		.prepare(
			`INSERT INTO topic_candidates (harness, source_session_id, label, source_kind, weight)
			 VALUES ($harness, $sid, $label, $kind, $weight)
			 ON CONFLICT (harness, source_session_id, label, source_kind)
			 DO UPDATE SET weight = weight + excluded.weight`,
		)
		.run({
			$harness: args.harness,
			$sid: args.source_session_id,
			$label: args.label,
			$kind: args.source_kind,
			$weight: weight,
		});
}

export interface InsertSessionSummaryArgs {
	harness: Harness;
	source_session_id: string;
	summary_json: string;
	synthesized_at: string;
	provider: string;
	model: string;
	schema_version?: number;
}

export function insertSessionSummary(
	ledger: Ledger,
	args: InsertSessionSummaryArgs,
): void {
	ledger.db
		.prepare(
			`INSERT INTO session_summaries (harness, source_session_id, schema_version, summary_json, synthesized_at, provider, model)
			 VALUES ($harness, $sid, $schema_version, $summary_json, $synthesized_at, $provider, $model)
			 ON CONFLICT (harness, source_session_id) DO UPDATE SET
				schema_version = excluded.schema_version,
				summary_json = excluded.summary_json,
				synthesized_at = excluded.synthesized_at,
				provider = excluded.provider,
				model = excluded.model`,
		)
		.run({
			$harness: args.harness,
			$sid: args.source_session_id,
			$schema_version: args.schema_version ?? 1,
			$summary_json: args.summary_json,
			$synthesized_at: args.synthesized_at,
			$provider: args.provider,
			$model: args.model,
		});
}

export function getSessionSummary(
	ledger: Ledger,
	harness: Harness,
	source_session_id: string,
): SessionSummaryRow | null {
	const row = ledger.db
		.query<SessionSummaryRow, [string, string]>(
			"SELECT * FROM session_summaries WHERE harness = ? AND source_session_id = ?",
		)
		.get(harness, source_session_id);
	return row ?? null;
}

export interface StartSyncRunArgs {
	started_at: string;
}

export function startSyncRun(ledger: Ledger, args: StartSyncRunArgs): number {
	const result = ledger.db
		.prepare("INSERT INTO sync_runs (started_at) VALUES ($started_at)")
		.run({ $started_at: args.started_at });
	return Number(result.lastInsertRowid);
}

export interface FinishSyncRunArgs {
	id: number;
	ended_at: string;
	outcome: SyncOutcome;
	sessions_seen?: number;
	sessions_new?: number;
	sessions_updated?: number;
	synthesis_done?: number;
	synthesis_failed?: number;
	error_message?: string | null;
}

export function finishSyncRun(ledger: Ledger, args: FinishSyncRunArgs): void {
	ledger.db
		.prepare(
			`UPDATE sync_runs SET
				ended_at = $ended_at,
				outcome = $outcome,
				sessions_seen = $sessions_seen,
				sessions_new = $sessions_new,
				sessions_updated = $sessions_updated,
				synthesis_done = $synthesis_done,
				synthesis_failed = $synthesis_failed,
				error_message = $error_message
			WHERE id = $id`,
		)
		.run({
			$id: args.id,
			$ended_at: args.ended_at,
			$outcome: args.outcome,
			$sessions_seen: args.sessions_seen ?? 0,
			$sessions_new: args.sessions_new ?? 0,
			$sessions_updated: args.sessions_updated ?? 0,
			$synthesis_done: args.synthesis_done ?? 0,
			$synthesis_failed: args.synthesis_failed ?? 0,
			$error_message: args.error_message ?? null,
		});
}

interface SessionInsertParams {
	[key: string]: string | number | null;
	$harness: Harness;
	$source_session_id: string;
	$parent_session_id: string | null;
	$source_path: string;
	$source_revision: string;
	$source_size: number;
	$source_mtime: number;
	$byte_offset: number;
	$started_at: string;
	$ended_at: string;
	$cwd: string | null;
	$project: string | null;
	$git_branch: string | null;
	$model: string | null;
	$event_count: number;
	$short_id: string;
	$note_path: string | null;
	$synthesis_status: SynthesisStatus;
	$synthesis_error: string | null;
	$synthesis_attempts: number;
	$processed_at: string;
}

interface SessionSkeletonRow {
	short_id: string;
	parent_session_id: string | null;
	source_path: string;
	source_revision: string;
	source_size: number;
	source_mtime: number;
	byte_offset: number;
	started_at: string;
	ended_at: string;
	cwd: string | null;
	project: string | null;
	git_branch: string | null;
	model: string | null;
	event_count: number;
	note_path: string | null;
	synthesis_status: SynthesisStatus;
	synthesis_error: string | null;
	synthesis_attempts: number;
}

function sessionParams(
	session: Session,
	short_id: string,
): SessionInsertParams {
	return {
		$harness: session.harness,
		$source_session_id: session.source_session_id,
		$parent_session_id: session.parent_session_id,
		$source_path: session.source_path,
		$source_revision: session.source_revision,
		$source_size: session.source_size,
		$source_mtime: session.source_mtime,
		$byte_offset: session.byte_offset,
		$started_at: session.started_at,
		$ended_at: session.ended_at,
		$cwd: session.cwd,
		$project: session.project,
		$git_branch: session.git_branch,
		$model: session.model,
		$event_count: session.events.length,
		$short_id: short_id,
		$note_path: null,
		$synthesis_status: "pending",
		$synthesis_error: null,
		$synthesis_attempts: 0,
		$processed_at: new Date().toISOString(),
	};
}

function skeletonParams(
	args: UpsertSessionSkeletonArgs,
	short_id: string,
): SessionInsertParams {
	return {
		...sessionParams(args.session, short_id),
		$note_path: args.note_path,
		$processed_at: args.processed_at,
	};
}

function insertSessionSkeleton(
	ledger: Ledger,
	args: UpsertSessionSkeletonArgs,
	short_id: string,
): InsertSessionResult | null {
	try {
		ledger.db.prepare(INSERT_SESSION_SQL).run(skeletonParams(args, short_id));
		return { short_id, collision_recovered: false };
	} catch (err) {
		if (!isUniqueShortIdError(err)) throw err;
		return null;
	}
}

function updateSessionSkeleton(
	ledger: Ledger,
	args: UpsertSessionSkeletonArgs,
	short_id: string,
): void {
	const params = skeletonParams(args, short_id);
	ledger.db
		.prepare(
			`UPDATE sessions SET
				parent_session_id = $parent_session_id,
				source_path = $source_path,
				source_revision = $source_revision,
				source_size = $source_size,
				source_mtime = $source_mtime,
				byte_offset = $byte_offset,
				started_at = $started_at,
				ended_at = $ended_at,
				cwd = $cwd,
				project = $project,
				git_branch = $git_branch,
				model = $model,
				event_count = $event_count,
				note_path = $note_path,
				synthesis_status = $synthesis_status,
				synthesis_error = $synthesis_error,
				synthesis_attempts = $synthesis_attempts,
				processed_at = $processed_at
			WHERE harness = $harness AND source_session_id = $source_session_id`,
		)
		.run(params);
}

function sessionSkeletonMatches(
	row: SessionSkeletonRow,
	args: UpsertSessionSkeletonArgs,
): boolean {
	const session = args.session;
	return (
		row.parent_session_id === session.parent_session_id &&
		row.source_path === session.source_path &&
		row.source_revision === session.source_revision &&
		row.source_size === session.source_size &&
		row.source_mtime === session.source_mtime &&
		row.byte_offset === session.byte_offset &&
		row.started_at === session.started_at &&
		row.ended_at === session.ended_at &&
		row.cwd === session.cwd &&
		row.project === session.project &&
		row.git_branch === session.git_branch &&
		row.model === session.model &&
		row.event_count === session.events.length &&
		row.note_path === args.note_path &&
		row.synthesis_status === "pending" &&
		row.synthesis_error === null &&
		row.synthesis_attempts === 0
	);
}

function isUniqueShortIdError(err: unknown): boolean {
	if (!(err instanceof SQLiteError)) return false;
	const msg = err.message.toLowerCase();
	return msg.includes("unique") && msg.includes("sessions.short_id");
}

function splitStatements(sql: string): string[] {
	return sql
		.split(";")
		.map((s) => s.trim())
		.filter((s) => s.length > 0);
}
