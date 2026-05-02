import { closeSync, fsyncSync, openSync } from "node:fs";
import { mkdir, readFile, rename, unlink, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { isNodeError } from "../errors.ts";
import { renderRegion } from "./markers.ts";

export interface UpdateGeneratedRegionsArgs {
	notePath: string;
	template: string;
	regions: ReadonlyMap<string, string> | Record<string, string>;
}

export interface UpdateGeneratedRegionsResult {
	wrote: boolean;
	warning?: string;
}

export async function updateGeneratedRegions(
	args: UpdateGeneratedRegionsArgs,
): Promise<UpdateGeneratedRegionsResult> {
	const existing = await readExisting(args.notePath);

	if (existing === null) {
		await writeAtomically(args.notePath, args.template);
		return { wrote: true };
	}

	const replacementRegions = new Map(entries(args.regions));
	const validation = validateRegions(existing, replacementRegions);
	if (validation.warning) return { wrote: false, warning: validation.warning };

	const proposed = replaceRegions(existing, replacementRegions);

	if (proposed === existing) return { wrote: false };
	await writeAtomically(args.notePath, proposed);
	return { wrote: true };
}

async function readExisting(path: string): Promise<string | null> {
	try {
		return await readFile(path, "utf8");
	} catch (err) {
		if (isNodeError(err) && err.code === "ENOENT") return null;
		throw err;
	}
}

async function writeAtomically(path: string, content: string): Promise<void> {
	await mkdir(dirname(path), { recursive: true });
	const tmpPath = `${path}.tmp`;
	await writeFile(tmpPath, content, "utf8");
	const fd = openSync(tmpPath, "r");
	try {
		fsyncSync(fd);
	} finally {
		closeSync(fd);
	}
	try {
		await rename(tmpPath, path);
	} catch (err) {
		await unlink(tmpPath).catch(() => {});
		throw err;
	}
}

function validateRegions(
	text: string,
	regions: Map<string, string>,
): { warning?: string } {
	const pairCounts = new Map<string, number>();
	for (const match of text.matchAll(regionPattern())) {
		const name = match[1];
		if (name) pairCounts.set(name, (pairCounts.get(name) ?? 0) + 1);
	}

	for (const name of regions.keys()) {
		const startCount = markerCount(text, markerStartPattern(name));
		const endCount = markerCount(text, markerEndPattern(name));
		if (startCount !== 1 || endCount !== 1 || pairCounts.get(name) !== 1) {
			return { warning: `Malformed or missing jottrace region: ${name}` };
		}
	}
	return {};
}

function replaceRegions(text: string, regions: Map<string, string>): string {
	return text.replace(regionPattern(), (match, name: string) => {
		const replacement = regions.get(name);
		return replacement == null ? match : renderRegion(name, replacement);
	});
}

function regionPattern(): RegExp {
	return /<!-- jottrace:([^:]+):start v=1 -->\n[\s\S]*?\n<!-- jottrace:\1:end -->/g;
}

function markerStartPattern(name: string): RegExp {
	return new RegExp(`<!-- jottrace:${escapeRegex(name)}:start v=1 -->`, "g");
}

function markerEndPattern(name: string): RegExp {
	return new RegExp(`<!-- jottrace:${escapeRegex(name)}:end -->`, "g");
}

function markerCount(text: string, pattern: RegExp): number {
	return text.match(pattern)?.length ?? 0;
}

function entries(
	regions: ReadonlyMap<string, string> | Record<string, string>,
): [string, string][] {
	return regions instanceof Map
		? [...regions.entries()]
		: Object.entries(regions);
}

function escapeRegex(value: string): string {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
