import { describe, expect, test } from "bun:test";
import {
	emitFrontmatter,
	parseFrontmatter,
} from "../src/render/frontmatter.ts";

describe("frontmatter rendering", () => {
	test("emits and parses renderer-owned YAML metadata", () => {
		const markdown = `${emitFrontmatter({
			type: "jottrace-session",
			harness: "claude-code",
			source_session_id: "session with spaces",
			model: "123",
			project: "true",
			synthesis_attempts: 0,
			parent_session_id: null,
		})}# Jottrace Session
`;

		expect(markdown).toContain('source_session_id: "session with spaces"');
		expect(markdown).toContain('model: "123"');
		expect(markdown).toContain('project: "true"');
		expect(parseFrontmatter(markdown)).toEqual({
			type: "jottrace-session",
			harness: "claude-code",
			source_session_id: "session with spaces",
			model: "123",
			project: "true",
			synthesis_attempts: 0,
			parent_session_id: null,
		});
	});
});
