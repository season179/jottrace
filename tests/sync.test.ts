import { Database } from "bun:sqlite";
import { afterEach, describe, expect, test } from "bun:test";
import { existsSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { SESSION_REGION_NAMES } from "../src/render/session_md.ts";
import { syncClaudeCodeFixture } from "../src/sync.ts";
import { cleanupTempDirs, createTempDir, jsonl } from "./helpers/test-files.ts";

const tempDirs: string[] = [];

afterEach(() => {
	cleanupTempDirs(tempDirs);
});

describe("Claude Code fixture sync", () => {
	test("writes one skeleton session note and a pending ledger row", async () => {
		const dir = createTempDir(tempDirs, "jottrace-sync-test-");
		const sourcePath = join(dir, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl");
		const outputPath = join(dir, "journal");
		const ledgerPath = join(dir, "jottrace.db");

		await Bun.write(
			sourcePath,
			jsonl([
				{
					type: "permission-mode",
					permissionMode: "auto",
					sessionId: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
				},
				{
					parentUuid: null,
					isSidechain: false,
					type: "user",
					message: { role: "user", content: "Summarize this project" },
					uuid: "user-1",
					timestamp: "2026-05-01T10:15:00.000Z",
					cwd: "/Users/season/Personal/jottrace",
					sessionId: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
					gitBranch: "codex/issue-4-tracer-bullet",
					version: "2.1.118",
				},
				{
					parentUuid: "user-1",
					isSidechain: false,
					type: "assistant",
					message: {
						role: "assistant",
						model: "claude-opus-4-7",
						content: [{ type: "text", text: "I will inspect the repo." }],
					},
					uuid: "assistant-1",
					timestamp: "2026-05-01T10:16:00.000Z",
					cwd: "/Users/season/Personal/jottrace",
					sessionId: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
					gitBranch: "codex/issue-4-tracer-bullet",
				},
			]),
		);

		const result = await syncClaudeCodeFixture({
			sourcePath,
			outputPath,
			ledgerPath,
			now: () => new Date("2026-05-02T08:00:00.000Z"),
		});

		expect(result.notePath).toMatch(
			/sessions\/2026\/05\/claude-code\/[0-9a-f]{12}\.md$/,
		);
		expect(result.wroteNote).toBe(true);
		expect(existsSync(result.notePath)).toBe(true);

		const note = await Bun.file(result.notePath).text();
		expect(note).toContain("type: jottrace-session");
		expect(note).toContain("harness: claude-code");
		expect(note).toContain(
			"source_session_id: aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
		);
		expect(note).toContain("# Jottrace Session");
		expect(markerCount(note)).toBe(SESSION_REGION_NAMES.length * 2);
		for (const name of SESSION_REGION_NAMES) {
			expect(note).toContain(`<!-- jottrace:${name}:start v=1 -->`);
			expect(note).toContain(`<!-- jottrace:${name}:end -->`);
		}
		expect(note).toContain("Source file:");
		expect(note).toContain(
			"_synthesis pending — run `jottrace synth` after restoring LLM connectivity_",
		);

		const db = new Database(ledgerPath, { readonly: true });
		const row = db
			.query<
				{
					synthesis_status: string;
					synthesis_attempts: number;
					processed_at: string;
					note_path: string;
				},
				[]
			>(
				"SELECT synthesis_status, synthesis_attempts, processed_at, note_path FROM sessions",
			)
			.get();
		db.close();

		expect(row).toEqual({
			synthesis_status: "pending",
			synthesis_attempts: 0,
			processed_at: "2026-05-02T08:00:00.000Z",
			note_path: relative(outputPath, result.notePath),
		});

		const before = statSync(result.notePath).mtimeMs;
		const second = await syncClaudeCodeFixture({
			sourcePath,
			outputPath,
			ledgerPath,
			now: () => new Date("2026-05-02T08:01:00.000Z"),
		});
		const after = statSync(result.notePath).mtimeMs;

		expect(second.wroteNote).toBe(false);
		expect(after).toBe(before);

		const dbAfterSecondRun = new Database(ledgerPath, { readonly: true });
		const afterSecondRun = dbAfterSecondRun
			.query<{ processed_at: string }, []>("SELECT processed_at FROM sessions")
			.get();
		dbAfterSecondRun.close();
		expect(afterSecondRun?.processed_at).toBe("2026-05-02T08:00:00.000Z");

		await Bun.write(
			result.notePath,
			`${await Bun.file(result.notePath).text()}\n<!-- jottrace:summary:start v=1 -->\n`,
		);

		const malformed = await syncClaudeCodeFixture({
			sourcePath,
			outputPath,
			ledgerPath,
			now: () => new Date("2026-05-02T08:02:00.000Z"),
		});

		expect(malformed.wroteNote).toBe(false);
		expect(malformed.warning).toContain("summary");
		const dbAfterMalformedRun = new Database(ledgerPath, { readonly: true });
		const afterMalformedRun = dbAfterMalformedRun
			.query<{ processed_at: string }, []>("SELECT processed_at FROM sessions")
			.get();
		dbAfterMalformedRun.close();
		expect(afterMalformedRun?.processed_at).toBe("2026-05-02T08:00:00.000Z");
	});
});

function markerCount(note: string): number {
	return note.match(/<!-- jottrace:[^>]+ -->/g)?.length ?? 0;
}
