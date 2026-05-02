import { afterEach, describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { acquire, LockHeldError } from "../src/lock.ts";

const testDir = dirname(fileURLToPath(import.meta.url));
const lockChildPath = join(testDir, "helpers", "lock-child.ts");

const tempDirs: string[] = [];

afterEach(() => {
	for (const dir of tempDirs.splice(0)) {
		rmSync(dir, { force: true, recursive: true });
	}
});

function tempLockPath(): string {
	const dir = mkdtempSync(join(tmpdir(), "jottrace-lock-test-"));
	tempDirs.push(dir);
	return join(dir, "jottrace.lock");
}

describe("single-instance lock", () => {
	test("acquire writes pid and started_at lines, then release removes the lock", async () => {
		const lockPath = tempLockPath();
		const handle = acquire({
			lockPath,
			now: () => new Date("2026-05-02T07:45:00.000Z"),
		});

		expect(await Bun.file(lockPath).text()).toBe(
			`pid=${process.pid}\nstarted_at=2026-05-02T07:45:00.000Z\n`,
		);

		handle.release();

		expect(existsSync(lockPath)).toBe(false);
	});

	test("release can be called more than once", () => {
		const lockPath = tempLockPath();
		const handle = acquire({ lockPath });

		handle.release();
		handle.release();

		expect(existsSync(lockPath)).toBe(false);
	});

	test("release does not remove a newer lock written after this handle", async () => {
		const lockPath = tempLockPath();
		const handle = acquire({
			lockPath,
			now: () => new Date("2026-05-02T07:46:00.000Z"),
		});
		const replacement = `pid=${process.pid}\nstarted_at=2026-05-02T07:47:00.000Z\n`;
		rmSync(lockPath);
		await Bun.write(lockPath, replacement);

		handle.release();

		expect(await Bun.file(lockPath).text()).toBe(replacement);
	});

	test("release ignores a lock already removed elsewhere", () => {
		const lockPath = tempLockPath();
		const handle = acquire({ lockPath });
		rmSync(lockPath);

		handle.release();

		expect(existsSync(lockPath)).toBe(false);
	});

	test("acquire reports a live holder as a clean exit condition", async () => {
		const lockPath = tempLockPath();
		const startedAt = "2026-05-02T07:50:00.000Z";
		await Bun.write(lockPath, `pid=${process.pid}\nstarted_at=${startedAt}\n`);

		let caught: unknown;
		try {
			acquire({ lockPath });
		} catch (err) {
			caught = err;
		}

		expect(caught).toBeInstanceOf(LockHeldError);
		expect((caught as LockHeldError).exitCode).toBe(0);
		expect((caught as LockHeldError).holder).toEqual({
			pid: process.pid,
			startedAt,
		});
		expect((caught as Error).message).toContain(String(process.pid));
		expect((caught as Error).message).toContain(startedAt);
	});

	test("acquire silently reclaims a stale lock from a dead process", async () => {
		const lockPath = tempLockPath();
		const deadPid = await exitedChildPid();
		const now = "2026-05-02T07:55:00.000Z";
		await Bun.write(
			lockPath,
			`pid=${deadPid}\nstarted_at=2026-05-02T07:00:00.000Z\n`,
		);

		const handle = acquire({
			lockPath,
			now: () => new Date(now),
		});

		expect(await Bun.file(lockPath).text()).toBe(
			`pid=${process.pid}\nstarted_at=${now}\n`,
		);

		handle.release();
	});

	test("two child processes contend cleanly, and a later third acquire succeeds", async () => {
		const lockPath = tempLockPath();
		const first = spawnLockChild(lockPath, 250);
		await waitForLock(lockPath);

		const second = spawnLockChild(lockPath, 0);
		const secondOutput = await readProcessOutput(second);
		expect(await second.exited).toBe(0);
		expect(secondOutput.stdout).toContain("jottrace is already running");
		expect(secondOutput.stdout).toContain("pid ");
		expect(secondOutput.stdout).toContain("started at ");

		expect(await first.exited).toBe(0);

		const third = spawnLockChild(lockPath, 0);
		const thirdOutput = await readProcessOutput(third);
		expect(await third.exited).toBe(0);
		expect(thirdOutput.stdout).toContain("acquired");
		expect(thirdOutput.stdout).toContain("released");
	});
});

async function exitedChildPid(): Promise<number> {
	const child = Bun.spawn([process.execPath, "-e", ""], {
		stderr: "ignore",
		stdout: "ignore",
	});
	await child.exited;
	return child.pid;
}

function spawnLockChild(lockPath: string, holdMs: number) {
	return Bun.spawn(
		[process.execPath, lockChildPath, lockPath, String(holdMs)],
		{
			stderr: "pipe",
			stdout: "pipe",
		},
	);
}

async function waitForLock(lockPath: string): Promise<void> {
	for (let attempt = 0; attempt < 50; attempt += 1) {
		if (existsSync(lockPath)) return;
		await Bun.sleep(10);
	}
	throw new Error(`Timed out waiting for ${lockPath}`);
}

async function readProcessOutput(
	process: ReturnType<typeof spawnLockChild>,
): Promise<{ stdout: string; stderr: string }> {
	const [stdout, stderr] = await Promise.all([
		new Response(process.stdout).text(),
		new Response(process.stderr).text(),
	]);
	return { stdout, stderr };
}
