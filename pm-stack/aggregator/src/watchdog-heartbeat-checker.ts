import { readFile } from "node:fs/promises";

// Intentionally duplicated rather than importing the watchdog writer: the
// checker is an independent reader of the journal, not part of the watchdog.
const HEARTBEAT_FACET = "watchdog-heartbeat";
const HEARTBEAT_INTERVAL_MS = 5 * 60_000;

export type WatchdogHeartbeatAlert = "unknown" | "none" | "needs-orchestrator" | "page-drew";

export interface WatchdogHeartbeatCheck {
  alert: WatchdogHeartbeatAlert;
  missedCycles: number;
  lastHeartbeatAtMs: number | null;
}

/** Delivery stays injected so this standalone reader has no watchdog or
 *  aggregator dependencies; the page-health-push runner supplies it. */
export interface WatchdogHeartbeatAlertSink {
  needsOrchestrator(check: WatchdogHeartbeatCheck): Promise<void> | void;
  pageDrew(check: WatchdogHeartbeatCheck): Promise<void> | void;
}

/**
 * Independently read the append-only JSONL journal and report the alert
 * level implied by its latest valid heartbeat. At two missed five-minute
 * cycles the orchestrator is alerted; at three, Drew is paged.
 */
export async function checkWatchdogHeartbeatJournal(
  file: string,
  nowMs: number,
  sink?: WatchdogHeartbeatAlertSink,
): Promise<WatchdogHeartbeatCheck> {
  const raw = await readFile(file, "utf8").catch((error: NodeJS.ErrnoException) => {
    if (error.code === "ENOENT") return "";
    throw error;
  });

  let latest: number | null = null;
  for (const line of raw.split("\n")) {
    if (!line) continue;
    try {
      const record: unknown = JSON.parse(line);
      if (
        typeof record === "object" &&
        record !== null &&
        "facet" in record &&
        record.facet === HEARTBEAT_FACET &&
        "atMs" in record &&
        typeof record.atMs === "number" &&
        Number.isFinite(record.atMs) &&
        (latest === null || record.atMs > latest)
      ) {
        latest = record.atMs;
      }
    } catch {
      // A torn or unrelated journal line cannot suppress the last good beat.
    }
  }

  if (latest === null) return { alert: "unknown", missedCycles: 0, lastHeartbeatAtMs: null };

  const elapsedMs = Math.max(0, nowMs - latest);
  const missedCycles = Math.floor(elapsedMs / HEARTBEAT_INTERVAL_MS);
  const alert: WatchdogHeartbeatAlert =
    missedCycles >= 3 ? "page-drew" : missedCycles >= 2 ? "needs-orchestrator" : "none";
  const check = { alert, missedCycles, lastHeartbeatAtMs: latest };
  if (alert === "needs-orchestrator") await sink?.needsOrchestrator(check);
  if (alert === "page-drew") await sink?.pageDrew(check);
  return check;
}
