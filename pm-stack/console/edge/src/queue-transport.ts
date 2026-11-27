// The console's own view of the queue transport's health (#13503 slice 1).
//
// The queue-transport-alert Worker remains the probe: every ten minutes it
// asks the broker's public /healthz through Cloudflare Access and keeps
// the verdict under its /status. The console reads that verdict over the
// existing QUEUE_TRANSPORT_ALERT service binding and tells the owner
// itself, so one place — the console's notifications — decides what
// reaches his phone. Retiring the Worker is a later slice, after this
// notification path is deployed and seen working.
//
// Oracle: the alert Worker's publicStatus shape
// (pkgs/queue-transport-alert/worker/src/status.js) answered by a
// stand-in fetcher; the notification decision is compared with what
// that shape says.

/** The transport verdict the console keeps, as the alert Worker stated it. */
export interface QueueTransport {
  /** up | degraded (failing below threshold) | down (alerted) | stale | unknown */
  status: string;
  consecutiveFailures: number;
  failingSince: string | null;
  lastFailure: string | null;
  lastCheck: string | null;
  lastOk: string | null;
  incident: { number: number; url: string } | null;
  checkedAt: number;
}

/** The storage key the object keeps the last verdict under. */
export const QUEUE_TRANSPORT_KEY = "queue-transport";
/** The storage key for the once-per-outage notification flag. */
export const QUEUE_TRANSPORT_TOLD_KEY = "queue-transport-told";

/** Read a stored verdict back, or null when what is stored is not one. */
export function parseQueueTransport(raw: unknown): QueueTransport | null {
  if (typeof raw !== "object" || raw === null) return null;
  const record = raw as Record<string, unknown>;
  if (typeof record.status !== "string") return null;
  if (typeof record.consecutiveFailures !== "number") return null;
  if (typeof record.checkedAt !== "number") return null;
  const strOrNull = (value: unknown): string | null =>
    value === null || value === undefined ? null : typeof value === "string" ? value : null;
  let incident: QueueTransport["incident"] = null;
  if (record.incident !== null && record.incident !== undefined) {
    if (typeof record.incident !== "object") return null;
    const issue = record.incident as Record<string, unknown>;
    if (typeof issue.number !== "number" || typeof issue.url !== "string") return null;
    incident = { number: issue.number, url: issue.url };
  }
  return {
    status: record.status,
    consecutiveFailures: record.consecutiveFailures,
    failingSince: strOrNull(record.failingSince),
    lastFailure: strOrNull(record.lastFailure),
    lastCheck: strOrNull(record.lastCheck),
    lastOk: strOrNull(record.lastOk),
    incident,
    checkedAt: record.checkedAt,
  };
}

/** Fetch the alert Worker's /status over a service binding and normalize it. */
export async function fetchQueueTransport(
  fetcher: (request: Request) => Promise<Response>,
  checkedAt: number,
): Promise<QueueTransport | null> {
  let res: Response;
  try {
    res = await fetcher(new Request("https://queue-transport.internal/status"));
  } catch {
    return null;
  }
  let body: unknown;
  try {
    body = await res.json();
  } catch {
    return null;
  }
  if (typeof body !== "object" || body === null) return null;
  const record = body as Record<string, unknown>;
  if (typeof record.status !== "string") return null;
  const num = (value: unknown): number => (typeof value === "number" ? value : 0);
  const str = (value: unknown): string | null => (typeof value === "string" ? value : null);
  let incident: QueueTransport["incident"] = null;
  if (record.incident !== null && record.incident !== undefined) {
    if (typeof record.incident !== "object") return null;
    const issue = record.incident as Record<string, unknown>;
    if (typeof issue.number !== "number" || typeof issue.url !== "string") return null;
    incident = { number: issue.number, url: issue.url };
  }
  return {
    status: record.status,
    consecutiveFailures: num(record.consecutive_failures),
    failingSince: str(record.failing_since),
    lastFailure: str(record.last_failure),
    lastCheck: str(record.last_check),
    lastOk: str(record.last_ok),
    incident,
    checkedAt,
  };
}

/** Whether this verdict is worth waking the owner for. */
export function transportNeedsAttention(transport: QueueTransport | null): boolean {
  if (!transport) return false;
  return transport.status === "down" || transport.status === "degraded" || transport.status === "stale";
}

/**
 * Whether the owner should be told now: attention-worthy and not told
 * for this outage yet (told names the failing-since it was told for),
 * or recovered after having been told. Pure, so the tests hold it
 * without devices.
 */
export function transportNotifyDecision(
  transport: QueueTransport | null,
  told: string | null,
): "notify" | "recovered" | "quiet" {
  if (!transport || !transportNeedsAttention(transport)) {
    return told ? "recovered" : "quiet";
  }
  const marker = transport.failingSince ?? transport.status;
  return told === marker ? "quiet" : "notify";
}

/** The notification said when the transport needs attention. */
export function transportMessage(transport: QueueTransport): { title: string; body: string; tag: string } {
  const incident = transport.incident ? ` Incident #${transport.incident.number}.` : "";
  const failing = transport.consecutiveFailures
    ? ` (${transport.consecutiveFailures} failed checks${transport.failingSince ? ` since ${transport.failingSince}` : ""})`
    : "";
  const last = transport.lastFailure ? ` Last failure: ${transport.lastFailure}` : "";
  if (transport.status === "stale") {
    return {
      title: "Queue broker status is stale",
      body: `The queue transport alert has not checked in.${failing}${incident}`,
      tag: "queue-transport",
    };
  }
  if (transport.status === "degraded") {
    return {
      title: "Queue broker is struggling",
      body: `The broker is failing checks but below the alert threshold.${failing}${last}${incident}`,
      tag: "queue-transport",
    };
  }
  return {
    title: "Queue broker is down",
    body: `The repo-queue broker is unreachable.${failing}${last}${incident}`,
    tag: "queue-transport",
  };
}

/** The marker stored once the owner has been told of this outage. */
export function transportToldMarker(transport: QueueTransport): string {
  return transport.failingSince ?? transport.status;
}
