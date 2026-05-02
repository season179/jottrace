import { acquireOrExit } from "../../src/lock.ts";

const [lockPath, holdMsArg] = process.argv.slice(2);

if (!lockPath) {
	throw new Error("Usage: lock-child.ts <lock-path> <hold-ms>");
}

const holdMs = Number(holdMsArg ?? 0);
const handle = acquireOrExit({
	lockPath,
	writeMessage: (message) => console.log(message),
});

console.log(`acquired pid=${process.pid}`);
await Bun.sleep(holdMs);
handle.release();
console.log(`released pid=${process.pid}`);
