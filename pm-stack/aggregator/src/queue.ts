// Read-only fleet-queue collector (#9920, split from #1239): the
// aggregator's projection of the queue-broker's read verbs — POST
// /health (per-queue counts) and POST /list (task rows, rich mode).
// READ-ONLY BY CONSTRUCTION: this module and its live source call no
// enqueue/claim/complete surface and hold no write credential; the
// broker token used is the read-capable agent token from its file,
// never the dispatch service-role credential and never a direct
// Turso credential.
//
// Issue-to-task join: a task's dedupe_key is
//   repo-health:v1:<kind>:<repo>:issue:<n>
// which parseDedupeIssueRef decodes to (repo, issue n). The row's
// payload.issue.number corroborates: when both are present and
// disagree, the payload wins and the projection records
// join="payload-mismatch" so the disagreement is visible rather
// than silently papered over. Tasks whose dedupe_key carries no
// issue (other queue families) project with issue=null.
//
// Freshness (#1239's rules): every snapshot carries fetchedAt (the
// source timestamp of the broker read). A snapshot is stale when
// the broker read failed (available=false) or its age exceeds
// QUEUE_STALE_MS — the aggregator's feed stale threshold. Panels
// must show stale data behind a warning, never as silently current.

export const QUEUE_STALE_MS = 5 * 60_000;

export interface BrokerHealthRow {
  queue?: string;
  queued?: number;
  at_cap_queued?: number;
  claimed?: number;
  parked?: number;
  handed_off?: number;
  blocked?: number;
  dep_blocked?: number;
  done?: number;
  cancelled?: number;
  failed?: number;
  total?: number;
  oldest_queued_at?: string | null;
  terminal_events_24h?: number;
  terminal_events_7d?: number;
}

export interface BrokerTaskRow {
  id?: number;
  queue?: string;
  title?: string;
  priority?: number;
  status?: string;
  dedupe_key?: string;
  created_at?: string;
  not_before?: string | null;
  worker_id?: string | null;
  claimed_at?: string | null;
  lease_until?: string | null;
  lane_id?: string | null;
  attempts?: number;
  max_attempts?: number;
  progress?: { stage?: string; next?: string; stage_at?: string; lane?: string } | null;
  payload?: {
    repo?: string;
    issue?: { number?: number; url?: string } | null;
  } | null;
}

export interface QueueCounts {
  queue: string;
  queued: number;
  claimed: number;
  parked: number;
  blocked: number;
  done: number;
  failed: number;
  cancelled: number;
  handedOff: number;
  depBlocked: number;
  total: number;
  /** Terminal outcomes (done/failed/cancelled events) the broker
   * counted in the last 24h / 7d — the sweep-outcome signal. */
  terminalEvents24h: number;
  terminalEvents7d: number;
  oldestQueuedAt: string | null;
  /** Age of the oldest queued task at projection time; null when
   * nothing is queued or the broker gave no timestamp. */
  oldestQueuedAgeMs: number | null;
}

export interface QueueTask {
  id: number;
  queue: string;
  title: string;
  priority: number;
  status: string;
  dedupeKey: string | null;
  /** Join result: the GitHub repo + issue this task serves. */
  repo: string | null;
  issue: number | null;
  /** How the join resolved: "dedupe" | "payload" |
   * "payload-mismatch" | "none". */
  join: "dedupe" | "payload" | "payload-mismatch" | "none";
  worker: string | null;
  lane: string | null;
  attempts: number | null;
  maxAttempts: number | null;
  createdAt: string | null;
  leaseUntil: string | null;
  progressStage: string | null;
  progressNext: string | null;
  progressAt: string | null;
}

export interface QueueSnapshot {
  /** Set only by the raw broker read after checking explicit success, complete
   * row containers and all eight counters. Legacy UI projection is permissive
   * (missing values become zero); the private bridge must not treat those
   * defaults as evidence that the queue was observed idle. */
  bridgeVerified?: boolean;
  /** When the broker read that produced this snapshot completed
   * (the source timestamp every payload carries). */
  fetchedAt: number;
  /** False when the broker read failed; queues/tasks then hold the
   * last good read if one exists, or empty lists. */
  available: boolean;
  /** True when unavailable or older than QUEUE_STALE_MS at
   * projection time. Panels render stale data only behind a
   * warning. */
  stale: boolean;
  /** Non-fatal read problems (broker error text, token missing). */
  errors: string[];
  queues: QueueCounts[];
  tasks: QueueTask[];
}

const DEDUPE_ISSUE_RE = /^repo-health:v1:[^:]+:(.+):issue:(\d+)$/;

export function parseDedupeIssueRef(dedupeKey: string | null | undefined): { repo: string; issue: number } | null {
  if (!dedupeKey) return null;
  const m = DEDUPE_ISSUE_RE.exec(dedupeKey);
  if (!m) return null;
  return { repo: m[1], issue: Number(m[2]) };
}

function num(v: number | undefined): number {
  return typeof v === "number" && Number.isFinite(v) ? v : 0;
}

export function projectQueueTask(row: BrokerTaskRow): QueueTask {
  const dedupe = parseDedupeIssueRef(row.dedupe_key);
  const payloadIssue = typeof row.payload?.issue?.number === "number" ? row.payload.issue.number : null;
  const payloadRepo = typeof row.payload?.repo === "string" ? row.payload.repo : null;
  let repo: string | null = dedupe?.repo ?? payloadRepo;
  let issue: number | null = dedupe?.issue ?? payloadIssue;
  let join: QueueTask["join"] = dedupe ? "dedupe" : payloadIssue !== null ? "payload" : "none";
  if (dedupe && payloadIssue !== null && dedupe.issue !== payloadIssue) {
    issue = payloadIssue;
    repo = payloadRepo ?? dedupe.repo;
    join = "payload-mismatch";
  }
  return {
    id: row.id ?? 0,
    queue: row.queue ?? "",
    title: row.title ?? "",
    priority: row.priority ?? 0,
    status: row.status ?? "",
    dedupeKey: row.dedupe_key ?? null,
    repo,
    issue,
    join,
    worker: row.worker_id ?? null,
    lane: row.lane_id ?? row.progress?.lane ?? null,
    attempts: typeof row.attempts === "number" ? row.attempts : null,
    maxAttempts: typeof row.max_attempts === "number" ? row.max_attempts : null,
    createdAt: row.created_at ?? null,
    leaseUntil: row.lease_until ?? null,
    progressStage: row.progress?.stage ?? null,
    progressNext: row.progress?.next ?? null,
    progressAt: row.progress?.stage_at ?? null,
  };
}

export function projectQueueSnapshot(
  healthRows: BrokerHealthRow[],
  taskRows: BrokerTaskRow[],
  fetchedAt: number,
  now: number,
  errors: string[] = [],
): QueueSnapshot {
  const queues: QueueCounts[] = healthRows
    .filter((r) => typeof r.queue === "string" && r.queue !== "")
    .map((r) => {
      const oldest = r.oldest_queued_at ?? null;
      const oldestMs = oldest ? Date.parse(oldest) : NaN;
      return {
        queue: r.queue as string,
        queued: num(r.queued),
        claimed: num(r.claimed),
        parked: num(r.parked),
        blocked: num(r.blocked),
        done: num(r.done),
        failed: num(r.failed),
        cancelled: num(r.cancelled),
        handedOff: num(r.handed_off),
        depBlocked: num(r.dep_blocked),
        total: num(r.total),
        terminalEvents24h: num(r.terminal_events_24h),
        terminalEvents7d: num(r.terminal_events_7d),
        oldestQueuedAt: oldest,
        oldestQueuedAgeMs: Number.isNaN(oldestMs) ? null : Math.max(0, now - oldestMs),
      };
    })
    .sort((a, b) => a.queue.localeCompare(b.queue));
  const available = errors.length === 0;
  return {
    fetchedAt,
    available,
    stale: !available || now - fetchedAt > QUEUE_STALE_MS,
    errors,
    queues,
    tasks: taskRows.map(projectQueueTask),
  };
}

/** The snapshot for a broker read that failed with no prior good
 * read: unavailable + stale, empty data, the error named. */
export function unavailableQueueSnapshot(now: number, error: string): QueueSnapshot {
  return {
    fetchedAt: now,
    available: false,
    stale: true,
    errors: [error],
    queues: [],
    tasks: [],
  };
}

type FetchLike = (
  url: string,
  init?: { method?: string; headers?: Record<string, string>; body?: string },
) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;

/** Read the broker token from its file; null when unreadable. The
 * token is used only as the X-Agent-Token header on read verbs. */
export async function readBrokerToken(
  tokenFile: string,
  readFileImpl: (path: string) => Promise<string> = (p) =>
    import("node:fs/promises").then((fs) => fs.readFile(p, "utf8")),
): Promise<string | null> {
  try {
    const token = (await readFileImpl(tokenFile)).trim();
    return token === "" ? null : token;
  } catch {
    return null;
  }
}

async function brokerPost(
  fetchImpl: FetchLike,
  brokerUrl: string,
  token: string,
  path: string,
  body: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const res = await fetchImpl(`${brokerUrl}${path}`, {
    method: "POST",
    headers: { "X-Agent-Token": token, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const json = (await res.json()) as Record<string, unknown>;
  if (!res.ok || json.ok === false || typeof json.error === "string") {
    throw new Error(`queue-broker ${path} failed: ${String(json.error ?? res.status)}`);
  }
  return json;
}

/**
 * One live collector pass: POST /health for the per-queue counts,
 * then POST /list (rich) per queue named by the health rows for the
 * task rows. Any failure throws; the caller decides the fail-soft
 * policy (last-good snapshot re-projected stale, or the unavailable
 * snapshot). Read verbs only — /health and /list are the whole
 * surface this collector will ever call.
 */
export async function fetchQueueSnapshot(
  brokerUrl: string,
  token: string,
  fetchImpl: FetchLike = fetch as unknown as FetchLike,
  now: () => number = () => Date.now(),
  listLimit = 200,
): Promise<QueueSnapshot> {
  const health = await brokerPost(fetchImpl, brokerUrl, token, "/health", {});
  const states = ["queued", "claimed", "parked", "handed_off", "blocked", "done", "cancelled", "failed"];
  const object = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === "object" && !Array.isArray(v);
  const count = (n: unknown): n is number => Number.isSafeInteger(n) && (n as number) >= 0;
  let bridgeVerified = health.ok === true && Array.isArray(health.rows)
    && health.rows.every((row: unknown) => object(row) && typeof row.queue === "string" && row.queue.length > 0
      && states.every(state => count(row[state])));
  const healthRows = (Array.isArray(health.rows) ? health.rows : []) as BrokerHealthRow[];
  bridgeVerified &&= new Set(healthRows.map(row => row.queue)).size === healthRows.length;
  const taskRows: BrokerTaskRow[] = [];
  for (const row of healthRows) {
    if (!object(row)) { bridgeVerified = false; continue; }
    if (typeof row.queue !== "string" || row.queue === "") continue;
    const list = await brokerPost(fetchImpl, brokerUrl, token, "/list", {
      queue: row.queue,
      rich: true,
      limit: listLimit,
    });
    bridgeVerified &&= list.ok === true && Array.isArray(list.rows)
      && list.rows.every((task: unknown) => object(task) && count(task.id) && task.id > 0
        && states.includes(task.status as string)
        && (task.priority === null || (typeof task.priority === "number" && Number.isFinite(task.priority))));
    if (Array.isArray(list.rows)) taskRows.push(...(list.rows as BrokerTaskRow[]));
  }
  const fetchedAt = now();
  return { ...projectQueueSnapshot(healthRows, taskRows, fetchedAt, fetchedAt), bridgeVerified };
}
