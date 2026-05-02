import { join, relative } from "node:path";
import { claudeCodeAdapter } from "./adapters/claude_code.ts";
import {
	computeDigests,
	openLedger,
	resolveSessionShortId,
	upsertSessionSkeleton,
} from "./ledger.ts";
import { updateGeneratedRegions } from "./render/regions.ts";
import {
	renderSkeletonSessionNote,
	skeletonRegions,
} from "./render/session_md.ts";

export interface SyncClaudeCodeFixtureOptions {
	sourcePath: string;
	outputPath: string;
	ledgerPath: string;
	now?: () => Date;
}

export interface SyncClaudeCodeFixtureResult {
	sourceSessionId: string;
	shortId: string;
	notePath: string;
	wroteNote: boolean;
	warning?: string;
}

export async function syncClaudeCodeFixture(
	options: SyncClaudeCodeFixtureOptions,
): Promise<SyncClaudeCodeFixtureResult> {
	const session = await claudeCodeAdapter.parse(options.sourcePath);
	const digests = computeDigests(session.harness, session.source_session_id);
	const processedAt = (options.now ?? (() => new Date()))().toISOString();
	const ledger = openLedger(options.ledgerPath);

	try {
		const shortId = resolveSessionShortId(ledger, session, digests);
		const notePath = sessionNotePath(
			options.outputPath,
			session.started_at,
			session.harness,
			shortId,
		);
		const noteRelativePath = relative(options.outputPath, notePath);

		const update = await updateGeneratedRegions({
			notePath,
			template: renderSkeletonSessionNote(session),
			regions: skeletonRegions(session),
		});

		if (!update.warning) {
			upsertSessionSkeleton(ledger, {
				session,
				note_path: noteRelativePath,
				processed_at: processedAt,
				digests,
				short_id: shortId,
			});
		}

		return {
			sourceSessionId: session.source_session_id,
			shortId,
			notePath,
			wroteNote: update.wrote,
			warning: update.warning,
		};
	} finally {
		ledger.close();
	}
}

function sessionNotePath(
	outputPath: string,
	startedAt: string,
	harness: string,
	shortId: string,
): string {
	const date = new Date(startedAt);
	const year = String(date.getUTCFullYear());
	const month = String(date.getUTCMonth() + 1).padStart(2, "0");
	return join(outputPath, "sessions", year, month, harness, `${shortId}.md`);
}
