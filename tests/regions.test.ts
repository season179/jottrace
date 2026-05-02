import { afterEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { updateGeneratedRegions } from "../src/render/regions.ts";
import { renderRegion } from "../src/render/session_md.ts";
import { cleanupTempDirs, createTempDir } from "./helpers/test-files.ts";

const tempDirs: string[] = [];

afterEach(() => {
	cleanupTempDirs(tempDirs);
});

describe("generated region updates", () => {
	test("replaces marker content while preserving hand edits outside markers", async () => {
		const dir = createTempDir(tempDirs, "jottrace-regions-test-");
		const notePath = join(dir, "note.md");
		await Bun.write(
			notePath,
			[
				"---",
				"type: jottrace-session",
				"---",
				"",
				"# Jottrace Session",
				"",
				"personal note before",
				"",
				renderRegion("summary", "old summary"),
				"",
				"personal note between",
				"",
				renderRegion("timeline", "old timeline"),
				"",
				"personal note after",
				"",
			].join("\n"),
		);

		const result = await updateGeneratedRegions({
			notePath,
			template: "",
			regions: {
				summary: "new summary",
				timeline: "new timeline",
			},
		});

		const updated = await Bun.file(notePath).text();
		expect(result).toEqual({ wrote: true });
		expect(updated).toContain("personal note before");
		expect(updated).toContain("personal note between");
		expect(updated).toContain("personal note after");
		expect(updated).toContain(renderRegion("summary", "new summary"));
		expect(updated).toContain(renderRegion("timeline", "new timeline"));
		expect(updated).not.toContain("old summary");
		expect(updated).not.toContain("old timeline");
	});

	test("aborts without rewriting when a marker pair is malformed", async () => {
		const dir = createTempDir(tempDirs, "jottrace-regions-test-");
		const notePath = join(dir, "note.md");
		const original = [
			"# Jottrace Session",
			"",
			"personal note before",
			"",
			"<!-- jottrace:summary:start v=1 -->",
			"old summary",
			"",
			"personal note after",
			"",
		].join("\n");
		await Bun.write(notePath, original);

		const result = await updateGeneratedRegions({
			notePath,
			template: "",
			regions: { summary: "new summary" },
		});

		expect(result.wrote).toBe(false);
		expect(result.warning).toContain("summary");
		expect(await Bun.file(notePath).text()).toBe(original);
	});

	test("aborts when an otherwise valid region has an extra unmatched marker", async () => {
		const dir = createTempDir(tempDirs, "jottrace-regions-test-");
		const notePath = join(dir, "note.md");
		const original = [
			"# Jottrace Session",
			"",
			renderRegion("summary", "old summary"),
			"",
			"<!-- jottrace:summary:start v=1 -->",
			"dangling duplicate",
			"",
		].join("\n");
		await Bun.write(notePath, original);

		const result = await updateGeneratedRegions({
			notePath,
			template: "",
			regions: { summary: "new summary" },
		});

		expect(result.wrote).toBe(false);
		expect(result.warning).toContain("summary");
		expect(await Bun.file(notePath).text()).toBe(original);
	});
});
