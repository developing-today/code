import type { FleetFeed } from "./feed.ts";

/** Inputs assembled by scripts/collect-fleet-feed.ts immediately before
 * it writes the published feed. Keeping the optional-section projection
 * here lets the contract suite exercise the collector's real output
 * assembly without importing its credentialed/networked entry point. */
export type CollectorFeedInput = Pick<FleetFeed, "generatedAt" | "quota" | "attention"> &
  Partial<Omit<FleetFeed, "version" | "generatedAt" | "quota" | "attention">>;

/** Build the versioned fleet feed written by the collector. */
export function assembleCollectorFeed(input: CollectorFeedInput): FleetFeed {
  return { version: 1, ...input };
}
