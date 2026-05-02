import {
	closeSync,
	mkdirSync,
	openSync,
	readFileSync,
	unlinkSync,
	writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { isNodeError } from "./errors.ts";

export interface LockHandle {
	readonly lockPath: string;
	release(): void;
}

export interface LockInfo {
	pid: number;
	startedAt: string;
}

export interface AcquireOptions {
	lockPath?: string;
	now?: () => Date;
	writeMessage?: (message: string) => void;
}

export class LockHeldError extends Error {
	readonly exitCode = 0;
	readonly holder: LockInfo;

	constructor(holder: LockInfo) {
		super(
			`jottrace is already running (pid ${holder.pid}, started at ${holder.startedAt}); exiting without starting another run.`,
		);
		this.name = "LockHeldError";
		this.holder = holder;
	}
}

const DEFAULT_LOCK_PATH = join(
	homedir(),
	".config",
	"jottrace",
	"jottrace.lock",
);

export function acquire(options: AcquireOptions = {}): LockHandle {
	const lockPath = options.lockPath ?? DEFAULT_LOCK_PATH;
	const startedAt = (options.now ?? (() => new Date()))().toISOString();

	mkdirSync(dirname(lockPath), { recursive: true });

	for (;;) {
		const fd = tryCreateLock(lockPath);
		if (fd === null) {
			const holder = readLock(lockPath);
			if (isProcessAlive(holder.pid)) {
				throw new LockHeldError(holder);
			}
			removeLock(lockPath);
			continue;
		}

		return writeHeldLock(lockPath, fd, startedAt);
	}
}

export function acquireOrExit(options: AcquireOptions = {}): LockHandle {
	try {
		return acquire(options);
	} catch (err) {
		if (err instanceof LockHeldError) {
			const writeMessage =
				options.writeMessage ?? ((message) => console.log(message));
			writeMessage(err.message);
			process.exit(err.exitCode);
		}
		throw err;
	}
}

function tryCreateLock(lockPath: string): number | null {
	let fd: number;
	try {
		fd = openSync(lockPath, "wx", 0o600);
	} catch (err) {
		if (isNodeError(err) && err.code === "EEXIST") {
			return null;
		}
		throw err;
	}

	return fd;
}

function writeHeldLock(
	lockPath: string,
	fd: number,
	startedAt: string,
): LockHandle {
	try {
		writeFileSync(fd, `pid=${process.pid}\nstarted_at=${startedAt}\n`, "utf8");
	} finally {
		closeSync(fd);
	}

	let released = false;
	const heldLock = { pid: process.pid, startedAt };
	return {
		lockPath,
		release() {
			if (released) return;
			released = true;
			removeHeldLock(lockPath, heldLock);
		},
	};
}

function isProcessAlive(pid: number): boolean {
	try {
		process.kill(pid, 0);
		return true;
	} catch (err) {
		if (isNodeError(err) && (err.code === "ESRCH" || err.code === "ENOENT")) {
			return false;
		}
		return true;
	}
}

function removeLock(lockPath: string): void {
	try {
		unlinkSync(lockPath);
	} catch (err) {
		if (!isNodeError(err) || err.code !== "ENOENT") {
			throw err;
		}
	}
}

function removeHeldLock(lockPath: string, heldLock: LockInfo): void {
	let current: LockInfo;
	try {
		current = readLock(lockPath);
	} catch (err) {
		if (isNodeError(err) && err.code === "ENOENT") {
			return;
		}
		throw err;
	}

	if (
		current.pid === heldLock.pid &&
		current.startedAt === heldLock.startedAt
	) {
		removeLock(lockPath);
	}
}

function readLock(lockPath: string): LockInfo {
	const text = readFileSync(lockPath, "utf8");
	const lines = text.split(/\r?\n/);
	const pidLine = lines.find((line) => line.startsWith("pid="));
	const startedAtLine = lines.find((line) => line.startsWith("started_at="));
	const pid = Number(pidLine?.slice("pid=".length));
	const startedAt = startedAtLine?.slice("started_at=".length);

	if (!Number.isInteger(pid) || !startedAt) {
		throw new Error(`Invalid jottrace lock file at ${lockPath}`);
	}

	return { pid, startedAt };
}
