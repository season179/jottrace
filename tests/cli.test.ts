import { afterEach, describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { cleanupTempDirs, createTempDir, jsonl } from "./helpers/test-files.ts";

const tempDirs: string[] = [];

afterEach(() => {
	cleanupTempDirs(tempDirs);
});

describe("jottrace CLI", () => {
	test("sync reads a Claude Code fixture path and prints the note path", async () => {
		const dir = createTempDir(tempDirs, "jottrace-cli-test-");
		const sourcePath = join(dir, "cccccccc-dddd-eeee-ffff-000000000000.jsonl");
		const outputPath = join(dir, "journal");
		const ledgerPath = join(dir, "jottrace.db");
		const lockPath = join(dir, "jottrace.lock");
		await Bun.write(
			sourcePath,
			jsonl([
				{
					type: "user",
					message: { role: "user", content: "Make a note" },
					timestamp: "2026-05-01T11:00:00.000Z",
					cwd: "/Users/season/Personal/jottrace",
					sessionId: "cccccccc-dddd-eeee-ffff-000000000000",
				},
			]),
		);

		const result = await runCliSync(
			sourcePath,
			outputPath,
			ledgerPath,
			lockPath,
		);

		expect(result.exitCode).toBe(0);
		expect(result.stderr).toBe("");
		const notePath = result.stdout.trim();
		expect(notePath).toMatch(
			/sessions\/2026\/05\/claude-code\/[0-9a-f]{12}\.md$/,
		);
		expect(existsSync(notePath)).toBe(true);
		expect(existsSync(lockPath)).toBe(false);
	});

	test("sync exits nonzero when generated region update aborts", async () => {
		const dir = createTempDir(tempDirs, "jottrace-cli-test-");
		const sourcePath = join(dir, "dddddddd-eeee-ffff-0000-111111111111.jsonl");
		const outputPath = join(dir, "journal");
		const ledgerPath = join(dir, "jottrace.db");
		const lockPath = join(dir, "jottrace.lock");
		await Bun.write(
			sourcePath,
			jsonl([
				{
					type: "user",
					message: { role: "user", content: "Make a note" },
					timestamp: "2026-05-01T11:00:00.000Z",
					cwd: "/Users/season/Personal/jottrace",
					sessionId: "dddddddd-eeee-ffff-0000-111111111111",
				},
			]),
		);

		const first = await runCliSync(
			sourcePath,
			outputPath,
			ledgerPath,
			lockPath,
		);
		expect(first.exitCode).toBe(0);
		const notePath = first.stdout.trim();
		await Bun.write(
			notePath,
			`${await Bun.file(notePath).text()}\n<!-- jottrace:summary:start v=1 -->\n`,
		);

		const second = await runCliSync(
			sourcePath,
			outputPath,
			ledgerPath,
			lockPath,
		);

		expect(second.exitCode).toBe(1);
		expect(second.stdout).toBe("");
		expect(second.stderr).toContain("summary");
		expect(existsSync(lockPath)).toBe(false);
	});
});

async function runCliSync(
	sourcePath: string,
	outputPath: string,
	ledgerPath: string,
	lockPath: string,
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
	const child = Bun.spawn(
		[
			process.execPath,
			"src/cli.ts",
			"sync",
			"--fixture",
			sourcePath,
			"--output",
			outputPath,
			"--db",
			ledgerPath,
			"--lock",
			lockPath,
		],
		{
			cwd: process.cwd(),
			stderr: "pipe",
			stdout: "pipe",
		},
	);
	const [stdout, stderr, exitCode] = await Promise.all([
		new Response(child.stdout).text(),
		new Response(child.stderr).text(),
		child.exited,
	]);
	return { stdout, stderr, exitCode };
}
