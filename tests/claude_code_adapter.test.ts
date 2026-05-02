import { afterEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import {
	parseClaudeCodeSession,
	truncateToolOutput,
} from "../src/adapters/claude_code.ts";
import { cleanupTempDirs, createTempDir, jsonl } from "./helpers/test-files.ts";

const tempDirs: string[] = [];

afterEach(() => {
	cleanupTempDirs(tempDirs);
});

describe("Claude Code adapter", () => {
	test("maps Claude Code records to canonical session events", async () => {
		const dir = createTempDir(tempDirs, "jottrace-cc-adapter-test-");
		const sourcePath = join(dir, "bbbbbbbb-cccc-dddd-eeee-ffffffffffff.jsonl");
		await Bun.write(
			sourcePath,
			jsonl([
				{
					type: "permission-mode",
					permissionMode: "auto",
					sessionId: "bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
				},
				{
					type: "file-history-snapshot",
					sessionId: "bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
					timestamp: "2026-05-01T09:59:00.000Z",
				},
				{
					type: "user",
					message: { role: "user", content: "Check the project" },
					timestamp: "2026-05-01T10:00:00.000Z",
					cwd: "/Users/season/Personal/jottrace",
					sessionId: "bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
					gitBranch: "main",
				},
				{
					type: "assistant",
					message: {
						role: "assistant",
						model: "claude-opus-4-7",
						content: [
							{ type: "text", text: "I will inspect files." },
							{
								type: "tool_use",
								id: "toolu_123",
								name: "Bash",
								input: { command: "ls" },
							},
						],
					},
					timestamp: "2026-05-01T10:01:00.000Z",
					cwd: "/Users/season/Personal/jottrace",
					sessionId: "bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
					gitBranch: "main",
				},
				{
					type: "user",
					message: {
						role: "user",
						content: [
							{
								type: "tool_result",
								tool_use_id: "toolu_123",
								content: "src\nREADME.md",
								is_error: false,
							},
						],
					},
					timestamp: "2026-05-01T10:02:00.000Z",
					cwd: "/Users/season/Personal/jottrace",
					sessionId: "bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
					gitBranch: "main",
				},
				{
					type: "last-prompt",
					lastPrompt: "Check the project",
					sessionId: "bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
				},
			]),
		);

		const session = await parseClaudeCodeSession(sourcePath);

		expect(session.source_session_id).toBe(
			"bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
		);
		expect(session.project).toBe("jottrace");
		expect(session.git_branch).toBe("main");
		expect(session.model).toBe("claude-opus-4-7");
		expect(session.started_at).toBe("2026-05-01T09:59:00.000Z");
		expect(session.ended_at).toBe("2026-05-01T10:02:00.000Z");
		expect(session.events.map((event) => event.kind)).toEqual([
			"system",
			"system",
			"user",
			"assistant",
			"tool_call",
			"tool_result",
			"system",
		]);
		expect(session.events[0]?.extra).toEqual({ subtype: "permission-mode" });
		expect(session.events[1]?.extra).toEqual({
			subtype: "file-history-snapshot",
		});
		expect(session.events[4]).toMatchObject({
			kind: "tool_call",
			tool: "Bash",
			tool_call_id: "toolu_123",
			tool_input: { command: "ls" },
		});
		expect(session.events[5]).toMatchObject({
			kind: "tool_result",
			tool_call_id: "toolu_123",
			tool_output: "src\nREADME.md",
		});
		expect(session.events[6]?.extra).toEqual({ subtype: "last-prompt" });
	});

	test("uses the JSONL filename as source id and links sidechains to parent session id", async () => {
		const dir = createTempDir(tempDirs, "jottrace-cc-adapter-test-");
		const sourcePath = join(dir, "agent-a0fa7c4.jsonl");
		await Bun.write(
			sourcePath,
			jsonl([
				{
					parentUuid: null,
					isSidechain: true,
					type: "user",
					message: { role: "user", content: "Inspect schema files" },
					timestamp: "2026-05-01T10:00:00.000Z",
					cwd: "/Users/season/Personal/jottrace",
					sessionId: "bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
					agentId: "a0fa7c4",
				},
			]),
		);

		const session = await parseClaudeCodeSession(sourcePath);

		expect(session.source_session_id).toBe("agent-a0fa7c4");
		expect(session.parent_session_id).toBe(
			"bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
		);
	});

	test("does not treat a main-session record sessionId as a parent link", async () => {
		const dir = createTempDir(tempDirs, "jottrace-cc-adapter-test-");
		const sourcePath = join(dir, "bbbbbbbb-cccc-dddd-eeee-ffffffffffff.jsonl");
		await Bun.write(
			sourcePath,
			jsonl([
				{
					parentUuid: null,
					isSidechain: false,
					type: "user",
					message: { role: "user", content: "Inspect schema files" },
					timestamp: "2026-05-01T10:00:00.000Z",
					cwd: "/Users/season/Personal/jottrace",
					sessionId: "bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
				},
			]),
		);

		const session = await parseClaudeCodeSession(sourcePath);

		expect(session.source_session_id).toBe(
			"bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
		);
		expect(session.parent_session_id).toBeNull();
	});

	test("truncates normal tool outputs to the last 4 KB", () => {
		const output = `${"a".repeat(1000)}${"b".repeat(4096)}`;

		const truncated = truncateToolOutput(output);

		expect(truncated).toBe(`[…1000 bytes elided…]${"b".repeat(4096)}`);
	});

	test("truncates errored tool outputs to the first 1 KB and last 3 KB", () => {
		const output = `${"a".repeat(2000)}${"b".repeat(4000)}`;

		const truncated = truncateToolOutput(output, { exit_code: 1 });

		expect(truncated).toBe(
			`${"a".repeat(1024)}[…1904 bytes elided…]${"b".repeat(3072)}`,
		);
	});
});
