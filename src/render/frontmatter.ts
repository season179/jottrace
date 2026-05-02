export type FrontmatterValue = string | number | boolean | null;

export function emitFrontmatter(
	values: Record<string, FrontmatterValue>,
): string {
	const lines = ["---"];
	for (const [key, value] of Object.entries(values)) {
		lines.push(`${key}: ${formatValue(value)}`);
	}
	lines.push("---", "");
	return lines.join("\n");
}

export function parseFrontmatter(
	markdown: string,
): Record<string, FrontmatterValue> | null {
	if (!markdown.startsWith("---\n")) return null;
	const end = markdown.indexOf("\n---\n", 4);
	if (end === -1) return null;

	const entries: Record<string, FrontmatterValue> = {};
	for (const line of markdown.slice(4, end).split("\n")) {
		const sep = line.indexOf(":");
		if (sep === -1) continue;
		entries[line.slice(0, sep).trim()] = parseValue(line.slice(sep + 1).trim());
	}
	return entries;
}

function formatValue(value: FrontmatterValue): string {
	if (value == null) return "null";
	if (typeof value === "number" || typeof value === "boolean") {
		return String(value);
	}
	if (value === "null" || value === "true" || value === "false") {
		return JSON.stringify(value);
	}
	if (/^-?\d+(\.\d+)?$/.test(value)) return JSON.stringify(value);
	if (/^[a-zA-Z0-9_./:-]+$/.test(value)) return value;
	return JSON.stringify(value);
}

function parseValue(value: string): FrontmatterValue {
	if (value === "null") return null;
	if (value === "true") return true;
	if (value === "false") return false;
	if (/^-?\d+(\.\d+)?$/.test(value)) return Number(value);
	if (value.startsWith('"') && value.endsWith('"')) {
		try {
			return JSON.parse(value) as string;
		} catch {
			return value;
		}
	}
	return value;
}
