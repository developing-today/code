// Orchestrator-feed publication contract for the independent watchdog
// heartbeat checker (#14410, follow-up to #14005 item 2).
//
// Contract
// --------
// - The feed is an append-only JSONL file written by the INDEPENDENT
//   checker path (scripts/check-watchdog-heartbeat.ts). It is not the
//   aggregator's alerts ledger (post-alert.ts calls back into the
//   aggregator, so it cannot deliver when the aggregator is down) and
//   it is not the aggregator-derived /attention tier (derive.ts, which
//   also requires the aggregator). The checker's external runner / the
//   orchestrator consumes this file directly.
// - Location: `--orchestrator-feed <path>` on the checker, else env
//   PM_WATCHDOG_ORCHESTRATOR_FEED_FILE, else `<journal>.orchestrator-feed.jsonl`
//   beside the SLO journal (same convention as the checker state file).
// - One item is published per outage, only when the checker reports
//   `needs-orchestrator` — i.e. after two consecutive missed five-minute
//   heartbeat cycles (missedCycles >= 2, see watchdog-heartbeat-checker.ts).
//   A single missed cycle (alert "none") publishes nothing. There is no
//   separate pending-state file: the pending state is the journal-derived
//   missed-cycle count itself, so a recovery heartbeat (alert "none")
//   clears it without publishing, and a later outage gets a new item id
//   (keyed by its outage reference) and can publish again.
// - Dedupe / exactly-one: the item id is stable for one outage
//   (`watchdog-heartbeat-<outageSinceMs>-needs-orchestrator`, the same
//   key post-alert.ts uses for the ledger incident). Republishing a
//   check for the same outage appends nothing.
// - Item shape: one JSON object per line, `OrchestratorFeedItem` below.
//   It names the watchdog (`watchdog`), the condition (`condition` /
//   `alert`), and the cycles observed (`cyclesObserved`).
//
// This module is pure file I/O on the feed file: no aggregator, network,
// or secrets dependency, so publication works with the aggregator down.
import { appendFile, readFile } from "node:fs/promises";
import type { WatchdogHeartbeatCheck } from "./watchdog-heartbeat-checker.ts";

export const WATCHDOG_ORCHESTRATOR_FEED_KIND = "watchdog-escalation";
export const PM_STACK_WATCHDOG_NAME = "pm-stack-watchdog";
export const NEEDS_ORCHESTRATOR_CONDITION = "NEEDS_ORCHESTRATOR: watchdog heartbeat missing";

export interface OrchestratorFeedItem {
  kind: typeof WATCHDOG_ORCHESTRATOR_FEED_KIND;
  /** Stable per outage; republishing the same outage is a no-op. */
  id: string;
  watchdog: typeof PM_STACK_WATCHDOG_NAME;
  alert: "needs-orchestrator";
  condition: typeof NEEDS_ORCHESTRATOR_CONDITION;
  /** Missed five-minute heartbeat cycles observed when published. */
  cyclesObserved: number;
  outageSinceMs: number | null;
  lastHeartbeatAtMs: number | null;
  publishedAtMs: number;
  title: string;
  body: string;
}

/** The feed item for a check, or null when the check publishes nothing
 *  (single missed cycle, healthy, page-drew, or unknown). */
export function orchestratorFeedItemFor(
  check: WatchdogHeartbeatCheck,
  publishedAtMs: number,
): OrchestratorFeedItem | null {
  if (check.alert !== "needs-orchestrator" || check.outageSinceMs === null) return null;
  const title = "NEEDS ORCHESTRATOR: PM-stack watchdog missed two heartbeat cycles";
  const body =
    check.lastHeartbeatAtMs === null
      ? `No valid watchdog heartbeat has ever been recorded; the outage reference is ${new Date(check.outageSinceMs).toISOString()}. ${check.missedCycles} five-minute cycles have been missed.`
      : `The last valid watchdog heartbeat was ${new Date(check.lastHeartbeatAtMs).toISOString()}. ${check.missedCycles} five-minute cycles have been missed.`;
  return {
    kind: WATCHDOG_ORCHESTRATOR_FEED_KIND,
    id: `watchdog-heartbeat-${check.outageSinceMs}-needs-orchestrator`,
    watchdog: PM_STACK_WATCHDOG_NAME,
    alert: "needs-orchestrator",
    condition: NEEDS_ORCHESTRATOR_CONDITION,
    cyclesObserved: check.missedCycles,
    outageSinceMs: check.outageSinceMs,
    lastHeartbeatAtMs: check.lastHeartbeatAtMs,
    publishedAtMs,
    title,
    body,
  };
}

function isOrchestratorFeedItem(value: unknown): value is OrchestratorFeedItem {
  if (typeof value !== "object" || value === null) return false;
  const item = value as Record<string, unknown>;
  return (
    item.kind === WATCHDOG_ORCHESTRATOR_FEED_KIND &&
    typeof item.id === "string" &&
    item.watchdog === PM_STACK_WATCHDOG_NAME &&
    item.alert === "needs-orchestrator" &&
    item.condition === NEEDS_ORCHESTRATOR_CONDITION &&
    typeof item.cyclesObserved === "number" &&
    typeof item.title === "string" &&
    typeof item.body === "string"
  );
}

/** Parse a feed file's text, dropping malformed lines at the boundary
 *  (one torn line must not hide the rest of the feed). */
export function parseOrchestratorFeed(text: string): OrchestratorFeedItem[] {
  const items: OrchestratorFeedItem[] = [];
  for (const line of text.split("\n")) {
    if (!line) continue;
    try {
      const parsed: unknown = JSON.parse(line);
      if (isOrchestratorFeedItem(parsed)) items.push(parsed);
    } catch {
      // Torn or unrelated line: skip it, keep the valid items.
    }
  }
  return items;
}

export async function readOrchestratorFeed(feedFile: string): Promise<OrchestratorFeedItem[]> {
  const text = await readFile(feedFile, "utf8").catch((error: NodeJS.ErrnoException) => {
    if (error.code === "ENOENT") return "";
    throw error;
  });
  return parseOrchestratorFeed(text);
}

export interface OrchestratorFeedPublication {
  published: boolean;
  item: OrchestratorFeedItem | null;
}

/**
 * Publish a check to the orchestrator feed. Exactly one item per
 * outage: a repeat check for an already-published outage returns
 * `{ published: false }` with the would-be item and appends nothing.
 */
export async function publishToOrchestratorFeed(
  check: WatchdogHeartbeatCheck,
  feedFile: string,
  publishedAtMs: number,
): Promise<OrchestratorFeedPublication> {
  const item = orchestratorFeedItemFor(check, publishedAtMs);
  if (item === null) return { published: false, item: null };
  const existing = await readOrchestratorFeed(feedFile);
  if (existing.some((entry) => entry.id === item.id)) return { published: false, item };
  await appendFile(feedFile, `${JSON.stringify(item)}\n`, "utf8");
  return { published: true, item };
}
