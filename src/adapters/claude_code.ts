import { readdir } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, extname, join } from "node:path";
import { isNodeError } from "../errors.ts";
import type { Event, Session } from "../models.ts";
import type { Adapter } from "./base.ts";

const MAX_TOOL_OUTPUT_BYTES = 4096;
const ERRORED_HEAD_BYTES = 1024;
const ERRORED_TAIL_BYTES = 3072;
const FALLBACK_TS = "__jottrace_fallback_ts__";

export const claudeCodeAdapter: Adapter = {
	harness: "claude-code",
	async discover(root = join(homedir(), ".claude", "projects")) {
		const paths = await collectJsonl(root);
		return { count: paths.length, samples: paths.slice(0, 5) };
	},
	async parse(sourcePath) {
		return parseClaudeCodeSession(sourcePath);
	},
};

export async function parseClaudeCodeSession(
	sourcePath: string,
): Promise<Session> {
	const file = Bun.file(sourcePath);
	const stat = await file.stat();
	const parsed = await parseRecords(sourcePath, file);
	const fallbackTs =
		parsed.firstTimestamp ?? new Date(stat.mtimeMs).toISOString();
	const events = parsed.events.map((event) =>
		event.ts === FALLBACK_TS ? { ...event, ts: fallbackTs } : event,
	);
	const started_at = minTimestamp(events) ?? fallbackTs;
	const ended_at = maxTimestamp(events) ?? started_at;
	const source_session_id = parsed.sourceSessionId;

	return {
		harness: "claude-code",
		source_session_id,
		parent_session_id: parsed.parentSessionId,
		source_path: sourcePath,
		source_revision: parsed.sourceRevision,
		source_size: stat.size,
		source_mtime: Math.floor(stat.mtimeMs / 1000),
		byte_offset: stat.size,
		started_at,
		ended_at,
		cwd: parsed.cwd,
		project: parsed.cwd ? basename(parsed.cwd) : null,
		git_branch: parsed.gitBranch,
		model: parsed.model,
		events,
	};
}

export function truncateToolOutput(
	output: string,
	options: { exit_code?: number | null; error?: string | null } = {},
): string {
	const bytes = new TextEncoder().encode(output);
	if (bytes.byteLength <= MAX_TOOL_OUTPUT_BYTES) return output;

	const isErrored =
		(options.exit_code != null && options.exit_code !== 0) ||
		options.error != null;
	if (!isErrored) {
		const tail = decodeBytes(
			bytes.slice(bytes.byteLength - MAX_TOOL_OUTPUT_BYTES),
		);
		return `${elision(bytes.byteLength - MAX_TOOL_OUTPUT_BYTES)}${tail}`;
	}

	const head = decodeBytes(bytes.slice(0, ERRORED_HEAD_BYTES));
	const tail = decodeBytes(bytes.slice(bytes.byteLength - ERRORED_TAIL_BYTES));
	return `${head}${elision(
		bytes.byteLength - ERRORED_HEAD_BYTES - ERRORED_TAIL_BYTES,
	)}${tail}`;
}

async function collectJsonl(root: string): Promise<string[]> {
	try {
		const entries = await readdir(root, { withFileTypes: true });
		const paths: string[] = [];
		for (const entry of entries) {
			const path = join(root, entry.name);
			if (entry.isDirectory()) {
				paths.push(...(await collectJsonl(path)));
			} else if (entry.isFile() && extname(entry.name) === ".jsonl") {
				paths.push(path);
			}
		}
		return paths.sort();
	} catch (err) {
		if (isNodeError(err) && err.code === "ENOENT") return [];
		throw err;
	}
}

async function parseRecords(
	sourcePath: string,
	file: Bun.BunFile,
): Promise<ParsedRecords> {
	const hasher = new Bun.CryptoHasher("sha256");
	const decoder = new TextDecoder();
	const reader = file.stream().getReader();
	const parsed: MutableParsedRecords = {
		events: [],
		sourceSessionId: basename(sourcePath, ".jsonl"),
		parentSessionId: null,
		cwd: null,
		gitBranch: null,
		model: null,
		firstTimestamp: null,
	};
	let pending = "";

	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		hasher.update(value);
		pending += decoder.decode(value, { stream: true });
		pending = processCompleteLines(pending, parsed);
	}

	pending += decoder.decode();
	if (pending.trim().length > 0) {
		applyRecord(JSON.parse(pending) as ClaudeRecord, parsed);
	}

	return {
		...parsed,
		sourceRevision: hasher.digest("hex"),
	};
}

function processCompleteLines(
	text: string,
	parsed: MutableParsedRecords,
): string {
	const lines = text.split(/\r?\n/);
	const pending = lines.pop() ?? "";
	for (const line of lines) {
		if (line.trim().length === 0) continue;
		applyRecord(JSON.parse(line) as ClaudeRecord, parsed);
	}
	return pending;
}

function applyRecord(record: ClaudeRecord, parsed: MutableParsedRecords): void {
	parsed.parentSessionId ??=
		stringOrNull(record.parentSessionId) ??
		(record.isSidechain === true ? stringOrNull(record.sessionId) : null);
	parsed.cwd ??= stringOrNull(record.cwd);
	parsed.gitBranch ??= stringOrNull(record.gitBranch);
	parsed.model ??= stringOrNull(record.message?.model);
	parsed.firstTimestamp ??= stringOrNull(record.timestamp);
	parsed.events.push(...eventsFromRecord(record, FALLBACK_TS));
}

function eventsFromRecord(record: ClaudeRecord, fallbackTs: string): Event[] {
	const ts = record.timestamp ?? fallbackTs;
	if (record.type === "user") return userEvents(record, ts);
	if (record.type === "assistant") return assistantEvents(record, ts);

	if (isSystemSubtype(record.type)) {
		return [
			{
				ts,
				kind: "system",
				extra: { subtype: record.type },
			},
		];
	}

	return [];
}

function userEvents(record: ClaudeRecord, ts: string): Event[] {
	const content = normalizeContent(record.message?.content);
	const events: Event[] = [];
	const text = content
		.filter((block) => block.type === "text")
		.map((block) => block.text)
		.filter(isString)
		.join("\n\n");

	if (text.length > 0 || typeof record.message?.content === "string") {
		events.push({ ts, kind: "user", text });
	}

	for (const block of content.filter((item) => item.type === "tool_result")) {
		const exit_code = numberValue(record.toolUseResult?.exit_code);
		const error = stringValue(record.toolUseResult?.error);
		events.push({
			ts,
			kind: "tool_result",
			tool_call_id: stringValue(block.tool_use_id),
			tool_output: truncateToolOutput(stringifyContent(block.content), {
				exit_code,
				error: error ?? (block.is_error === true ? "tool result error" : null),
			}),
			exit_code: exit_code ?? undefined,
			error: error ?? undefined,
		});
	}

	return events;
}

function assistantEvents(record: ClaudeRecord, ts: string): Event[] {
	const content = normalizeContent(record.message?.content);
	const events: Event[] = [];
	const text = content
		.filter((block) => block.type === "text")
		.map((block) => block.text)
		.filter(isString)
		.join("\n\n");

	if (text.length > 0 || typeof record.message?.content === "string") {
		events.push({ ts, kind: "assistant", text });
	}

	for (const block of content.filter((item) => item.type === "tool_use")) {
		events.push({
			ts,
			kind: "tool_call",
			tool: stringValue(block.name),
			tool_call_id: stringValue(block.id),
			tool_input: block.input,
		});
	}

	return events;
}

function normalizeContent(content: unknown): ContentBlock[] {
	if (typeof content === "string") return [{ type: "text", text: content }];
	if (Array.isArray(content)) return content as ContentBlock[];
	return [];
}

function stringifyContent(content: unknown): string {
	if (typeof content === "string") return content;
	if (Array.isArray(content)) {
		return content
			.map((item) =>
				typeof item === "string" ? item : JSON.stringify(item, null, 2),
			)
			.join("\n");
	}
	if (content == null) return "";
	return JSON.stringify(content, null, 2);
}

function minTimestamp(events: Event[]): string | null {
	const first = events[0];
	if (!first) return null;
	return events.reduce(
		(min, event) => (event.ts < min ? event.ts : min),
		first.ts,
	);
}

function maxTimestamp(events: Event[]): string | null {
	const first = events[0];
	if (!first) return null;
	return events.reduce(
		(max, event) => (event.ts > max ? event.ts : max),
		first.ts,
	);
}

function isSystemSubtype(type: string | undefined): type is string {
	return (
		type === "system" ||
		type === "permission-mode" ||
		type === "last-prompt" ||
		type === "attachment" ||
		type === "file-history-snapshot"
	);
}

function decodeBytes(bytes: Uint8Array): string {
	return new TextDecoder().decode(bytes);
}

function elision(bytes: number): string {
	return `[…${bytes} bytes elided…]`;
}

function stringValue(value: unknown): string | undefined {
	return typeof value === "string" ? value : undefined;
}

function numberValue(value: unknown): number | undefined {
	return typeof value === "number" ? value : undefined;
}

function stringOrNull(value: unknown): string | null {
	return typeof value === "string" && value.length > 0 ? value : null;
}

function isString(value: unknown): value is string {
	return typeof value === "string";
}

interface ClaudeRecord {
	type?: string;
	timestamp?: string;
	sessionId?: string;
	isSidechain?: boolean;
	parentSessionId?: string | null;
	cwd?: string;
	gitBranch?: string;
	message?: {
		role?: string;
		model?: string;
		content?: unknown;
	};
	toolUseResult?: {
		exit_code?: number;
		error?: string | null;
	};
}

interface ContentBlock {
	type?: string;
	text?: unknown;
	id?: unknown;
	name?: unknown;
	input?: unknown;
	tool_use_id?: unknown;
	content?: unknown;
	is_error?: boolean;
}

interface ParsedRecords {
	events: Event[];
	sourceSessionId: string;
	parentSessionId: string | null;
	cwd: string | null;
	gitBranch: string | null;
	model: string | null;
	firstTimestamp: string | null;
	sourceRevision: string;
}

type MutableParsedRecords = Omit<ParsedRecords, "sourceRevision">;
