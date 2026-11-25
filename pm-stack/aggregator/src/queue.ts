// Read-only fleet-queue collector (#9920, split from #1239): the
// aggregator's projection of the queue-broker's read verbs — POST
// /health (per-queue counts), POST /list (task rows, rich mode), and
// POST /history-since (incremental task_history pages).
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

/** One immutable transition from the broker's task_history table. */
export interface BrokerHistoryRow {
  id?: number;
  task_id?: number;
  kind?: string;
  from_status?: string | null;
  to_status?: string | null;
  actor?: string | null;
  created_at?: string;
}

export interface QueueHistoryEvent {
  id: number;
  taskId: number;
  kind: string;
  fromStatus: string | null;
  toStatus: string | null;
  actor: string | null;
  createdAt: string;
}

export interface QueueHistoryPage {
  /** Highest history id observed; the next read resumes strictly after it. */
  cursor: number;
  rows: QueueHistoryEvent[];
  hasMore: boolean;
  available: boolean;
  error: string | null;
}

export const QUEUE_HISTORY_PAGE_SIZE = 5_000;

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
  /** Incremental transition feed. Optional for snapshots built from
   * fixtures or older cache records that predate Q8. */
  history?: QueueHistoryPage;
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
 * One live collector pass: POST /health for per-queue counts, POST
 * /list (rich) per queue named by health, then one POST /history-since
 * for the shared monotonic history cursor. Health/list failures throw;
 * a history failure is recorded on its own page so the last good queue
 * counts can still render. Read verbs only.
 */
export async function fetchQueueSnapshot(
  brokerUrl: string,
  token: string,
  fetchImpl: FetchLike = fetch as unknown as FetchLike,
  now: () => number = () => Date.now(),
  listLimit = 200,
  historyCursor = 0,
): Promise<QueueSnapshot> {
  const health = await brokerPost(fetchImpl, brokerUrl, token, "/health", {});
  const healthRows = (Array.isArray(health.rows) ? health.rows : []) as BrokerHealthRow[];
  const taskRows: BrokerTaskRow[] = [];
  for (const row of healthRows) {
    if (typeof row.queue !== "string" || row.queue === "") continue;
    const list = await brokerPost(fetchImpl, brokerUrl, token, "/list", {
      queue: row.queue,
      rich: true,
      limit: listLimit,
    });
    if (Array.isArray(list.rows)) taskRows.push(...(list.rows as BrokerTaskRow[]));
  }
  const fetchedAt = now();
  let history: QueueHistoryPage;
  try {
    const response = await brokerPost(fetchImpl, brokerUrl, token, "/history-since", {
      after_id: historyCursor,
      limit: QUEUE_HISTORY_PAGE_SIZE,
    });
    const sourceRows = Array.isArray(response.rows) ? (response.rows as BrokerHistoryRow[]) : [];
    const rows = sourceRows
      .map(projectQueueHistoryEvent)
      .filter((row): row is QueueHistoryEvent => row !== null && row.id > historyCursor)
      .sort((a, b) => a.id - b.id);
    const reportedCursor =
      typeof response.next_cursor === "number" && Number.isSafeInteger(response.next_cursor)
        ? response.next_cursor
        : historyCursor;
    const cursor = Math.max(historyCursor, reportedCursor, rows.at(-1)?.id ?? historyCursor);
    history = {
      cursor,
      rows,
      hasMore:
        typeof response.has_more === "boolean" ? response.has_more : sourceRows.length >= QUEUE_HISTORY_PAGE_SIZE,
      available: true,
      error: null,
    };
  } catch (error) {
    // History is additive to the existing queue snapshot. A new reader
    // must not take the queue counts offline while brokers roll forward.
    history = {
      cursor: historyCursor,
      rows: [],
      hasMore: false,
      available: false,
      error: error instanceof Error ? error.message : String(error),
    };
  }
  return { ...projectQueueSnapshot(healthRows, taskRows, fetchedAt, fetchedAt), history };
}

function projectQueueHistoryEvent(row: BrokerHistoryRow): QueueHistoryEvent | null {
  if (
    !Number.isSafeInteger(row.id) ||
    !Number.isSafeInteger(row.task_id) ||
    typeof row.kind !== "string" ||
    typeof row.created_at !== "string"
  ) {
    return null;
  }
  return {
    id: row.id as number,
    taskId: row.task_id as number,
    kind: row.kind,
    fromStatus: typeof row.from_status === "string" ? row.from_status : null,
    toStatus: typeof row.to_status === "string" ? row.to_status : null,
    actor: typeof row.actor === "string" ? row.actor : null,
    createdAt: row.created_at,
  };
}
