import type { Session } from "../models.ts";
import { emitFrontmatter } from "./frontmatter.ts";
import { renderRegion } from "./markers.ts";

export const PLACEHOLDER =
	"_synthesis pending — run `jottrace synth` after restoring LLM connectivity_";

export const SESSION_REGION_NAMES = [
	"source-trace",
	"summary",
	"timeline",
	"wins",
	"dead-ends",
	"decisions",
	"outcome",
	"artifacts",
	"followups",
] as const;

export type SessionRegionName = (typeof SESSION_REGION_NAMES)[number];

export function renderSkeletonSessionNote(session: Session): string {
	const regions = skeletonRegions(session);
	return `${emitFrontmatter({
		type: "jottrace-session",
		harness: session.harness,
		source_session_id: session.source_session_id,
		parent_session_id: session.parent_session_id,
		started_at: session.started_at,
		ended_at: session.ended_at,
		cwd: session.cwd,
		project: session.project,
		git_branch: session.git_branch,
		model: session.model,
		synthesis_status: "pending",
	})}# Jottrace Session

${SESSION_REGION_NAMES.map((name) => renderRegion(name, regions[name])).join("\n\n")}
`;
}

export function skeletonRegions(
	session: Session,
): Record<SessionRegionName, string> {
	return {
		"source-trace": [
			`- Source file: \`${session.source_path}\``,
			`- Harness: ${session.harness}`,
			`- Source session: \`${session.source_session_id}\``,
			`- Events: ${session.events.length}`,
			`- Source revision: \`${session.source_revision}\``,
		].join("\n"),
		summary: PLACEHOLDER,
		timeline: PLACEHOLDER,
		wins: PLACEHOLDER,
		"dead-ends": PLACEHOLDER,
		decisions: PLACEHOLDER,
		outcome: PLACEHOLDER,
		artifacts: PLACEHOLDER,
		followups: PLACEHOLDER,
	};
}

export { renderRegion };
