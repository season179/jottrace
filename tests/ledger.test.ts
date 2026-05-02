import { beforeEach, describe, expect, test } from "bun:test";
import {
	computeDigests,
	type Ledger,
	openLedger,
	upsertTopicCandidate,
} from "../src/ledger.ts";
import type { Session } from "../src/models.ts";

let ledger: Ledger;

beforeEach(() => {
	ledger = openLedger(":memory:");
});

function makeSession(overrides: Partial<Session> = {}): Session {
	return {
		harness: "claude-code",
		source_session_id: "session-fixture",
		parent_session_id: null,
		source_path: "/tmp/fixture.jsonl",
		source_revision: "0".repeat(64),
		source_size: 0,
		source_mtime: 0,
		byte_offset: 0,
		started_at: "2026-05-02T00:00:00Z",
		ended_at: "2026-05-02T00:01:00Z",
		cwd: null,
		project: null,
		git_branch: null,
		model: null,
		events: [],
		...overrides,
	};
}

describe("schema and pragmas", () => {
	test("opening :memory: applies the three required PRAGMAs", () => {
		const journal = ledger.db.query("PRAGMA journal_mode").get() as {
			journal_mode: string;
		};
		const busy = ledger.db.query("PRAGMA busy_timeout").get() as {
			timeout: number;
		};
		const fkeys = ledger.db.query("PRAGMA foreign_keys").get() as {
			foreign_keys: number;
		};
		// :memory: SQLite reports 'memory' for journal_mode regardless of WAL request,
		// but a filesystem DB would report 'wal'. We assert WAL was attempted by setting
		// it (no error) and the other two PRAGMAs that DO stick on :memory:.
		expect(["wal", "memory"]).toContain(journal.journal_mode);
		expect(busy.timeout).toBe(5000);
		expect(fkeys.foreign_keys).toBe(1);
	});

	test("schema-init creates all four tables and required indexes", () => {
		const tables = ledger.db
			.query<{ name: string }, []>(
				"SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
			)
			.all()
			.map((row) => row.name);
		expect(tables).toEqual([
			"session_summaries",
			"sessions",
			"sync_runs",
			"topic_candidates",
		]);

		const indexes = ledger.db
			.query<{ name: string }, []>(
				"SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name",
			)
			.all()
			.map((row) => row.name);
		expect(indexes).toEqual([
			"sessions_parent",
			"sessions_project",
			"sessions_short_id",
			"sessions_started",
			"sessions_synthesis_status",
			"topic_candidates_label",
		]);
	});

	test("schema-init is idempotent — opening again does not error", () => {
		ledger.close();
		// Re-open the same in-memory handle would be a different DB, so use a tmp file.
		const tmp = `/tmp/jottrace-test-${Date.now()}-${Math.random()}.db`;
		const a = openLedger(tmp);
		a.close();
		const b = openLedger(tmp);
		b.close();
	});
});

describe("insertSession with short_id collision", () => {
	test("12-hex collision falls back to full 64-hex digest", () => {
		const sessionA = makeSession({ source_session_id: "session-A" });
		const sessionB = makeSession({ source_session_id: "session-B" });

		const fullA = `${"a".repeat(12)}${"1".repeat(52)}`;
		const fullB = `${"a".repeat(12)}${"2".repeat(52)}`;

		const a = ledger.insertSession(sessionA, {
			short: "a".repeat(12),
			full: fullA,
		});
		expect(a.short_id).toBe("a".repeat(12));
		expect(a.collision_recovered).toBe(false);

		const b = ledger.insertSession(sessionB, {
			short: "a".repeat(12),
			full: fullB,
		});
		expect(b.collision_recovered).toBe(true);
		expect(b.short_id).toBe(fullB);
		expect(b.short_id.length).toBe(64);

		const persistedA = ledger.db
			.query<{ short_id: string }, [string, string]>(
				"SELECT short_id FROM sessions WHERE harness = ? AND source_session_id = ?",
			)
			.get("claude-code", "session-A");
		expect(persistedA?.short_id).toBe("a".repeat(12));

		const persistedB = ledger.db
			.query<{ short_id: string }, [string, string]>(
				"SELECT short_id FROM sessions WHERE harness = ? AND source_session_id = ?",
			)
			.get("claude-code", "session-B");
		expect(persistedB?.short_id).toBe(fullB);
	});

	test("computeDigests returns 12-hex short and 64-hex full from sha256(harness:id)", () => {
		const d = computeDigests("claude-code", "abc");
		expect(d.full.length).toBe(64);
		expect(d.short).toBe(d.full.slice(0, 12));
		expect(/^[0-9a-f]+$/.test(d.full)).toBe(true);
	});
});

describe("topic_candidates UPSERT", () => {
	test("re-seen (harness, sid, label, source_kind) tuple increments weight", () => {
		const session = makeSession({ source_session_id: "topic-fixture" });
		ledger.insertSession(session);

		upsertTopicCandidate(ledger, {
			harness: "claude-code",
			source_session_id: "topic-fixture",
			label: "rust",
			source_kind: "cwd",
		});
		upsertTopicCandidate(ledger, {
			harness: "claude-code",
			source_session_id: "topic-fixture",
			label: "rust",
			source_kind: "cwd",
		});
		upsertTopicCandidate(ledger, {
			harness: "claude-code",
			source_session_id: "topic-fixture",
			label: "rust",
			source_kind: "git_branch",
		});

		const rows = ledger.db
			.query<
				{ label: string; source_kind: string; weight: number },
				[string, string]
			>(
				"SELECT label, source_kind, weight FROM topic_candidates WHERE harness = ? AND source_session_id = ? ORDER BY source_kind",
			)
			.all("claude-code", "topic-fixture");

		expect(rows).toEqual([
			{ label: "rust", source_kind: "cwd", weight: 2 },
			{ label: "rust", source_kind: "git_branch", weight: 1 },
		]);
	});
});
