import type { Harness, Session } from "../models.ts";

export interface DiscoveryResult {
	count: number;
	samples: string[];
}

export interface Adapter {
	readonly harness: Harness;
	discover(root?: string): Promise<DiscoveryResult>;
	parse(sourcePath: string): Promise<Session>;
}
