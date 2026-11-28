import type { SloJournalLine } from "./slo-journal.ts";

/** The heartbeat is written on the same append-only journal as SLO samples. */
export const WATCHDOG_HEARTBEAT_FACET = "watchdog-heartbeat";
export const WATCHDOG_HEARTBEAT_INTERVAL_MS = 5 * 60_000;

/** Build the journal record for one successful watchdog run. */
export function watchdogHeartbeatLine(atMs: number): SloJournalLine {
  return { facet: WATCHDOG_HEARTBEAT_FACET, atMs, bad: false, value: null };
}
