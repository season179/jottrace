export type Harness = "claude-code" | "codex";

export type EventKind =
	| "user"
	| "assistant"
	| "tool_call"
	| "tool_result"
	| "system"
	| "error";

export interface Event {
	ts: string;
	kind: EventKind;
	text?: string;
	tool?: string;
	tool_call_id?: string;
	tool_input?: unknown;
	tool_output?: string;
	exit_code?: number;
	error?: string;
	extra?: Record<string, unknown>;
}

export interface Session {
	harness: Harness;
	source_session_id: string;
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
	events: Event[];
}
