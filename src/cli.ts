#!/usr/bin/env bun
import { acquireOrExit } from "./lock.ts";
import { syncClaudeCodeFixture } from "./sync.ts";

const [, , command, ...args] = Bun.argv;

if (command === "sync") {
	const fixture = readFlag(args, "--fixture");
	const output = readFlag(args, "--output");
	const db = readFlag(args, "--db");
	const lockPath = readFlag(args, "--lock") ?? process.env.JOTTRACE_LOCK_PATH;

	if (!fixture || !output || !db) {
		console.error(
			"Usage: jottrace sync --fixture <jsonl> --output <dir> --db <db>",
		);
		process.exit(1);
	}

	const lock = acquireOrExit({ lockPath });
	try {
		const result = await syncClaudeCodeFixture({
			sourcePath: fixture,
			outputPath: output,
			ledgerPath: db,
		});
		if (result.warning) {
			console.error(result.warning);
			process.exitCode = 1;
		} else {
			console.log(result.notePath);
		}
	} finally {
		lock.release();
	}
} else {
	console.log(
		"Usage: jottrace sync --fixture <jsonl> --output <dir> --db <db>",
	);
}

function readFlag(args: string[], flag: string): string | null {
	const index = args.indexOf(flag);
	if (index === -1) return null;
	return args[index + 1] ?? null;
}
