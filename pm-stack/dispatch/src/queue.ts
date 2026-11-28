// Queue broker client (#9028) — ALL queue access goes through the
// fleet queue broker (HTTP on the laptop tailnet address, token auth
// via the X-Agent-Token header; the token lives in
// ~/.config/queue-broker/token, the same file the fleet queue CLI's
// laptop transport reads). The broker is backed by the live Turso
// queue store and validates every packet on /enqueue.
//
// No database credential lives in this process and there is no generic
// row PATCH: the broker's verbs are the whole surface —
//   enqueue    -> POST /enqueue    {packet, queue}   (validated)
//   claimTask  -> POST /claim-flow {task_id, worker, queue, session_id,
//                                   launched_by, lease_seconds}
//                 (claim-by-id + lane ledger + session bind, one trip)
//   getTask / findByDedupeKey / findTaskBySessionId
//              -> POST /list       {rich: true, ...filters}
//   noteTask   -> POST /checkpoint {task_id, worker, stage, progress}
//                 (progress merge — the broker-era task note; there is
//                 no last_error column write anymore)
//   cancelTask -> POST /cancel     {task_id, worker, reason}
//   assignLane -> POST /lane       {task_id, worker}
import type { Packet } from "./packet.ts";
import { REPO_QUEUES } from "./config.ts";
import { outboundTimeout } from "./fetchTimeout.ts";

export class QueueError extends Error {
  status = 502;
  constructor(
    message: string,
    readonly httpStatus?: number,
  ) {
    super(message);
  }
}

/** The broker claim found the row no longer claimable as queued
 * (#10737's compare-and-swap, now the broker's claim-by-id): another
 * worker claimed (or someone cancelled/completed) the task between
 * the enqueue and the assignment. The task belongs to whoever moved
 * it, so the launch must back out without touching it. */
export class ClaimLostError extends QueueError {
  constructor(
    readonly taskId: number,
    readonly requiredStatus: string,
  ) {
    super(`task ${taskId} is no longer ${requiredStatus}; another worker holds it`, 409);
  }
}

export interface TaskRow {
  id: number;
  queue: string;
  status: string;
  dedupe_key: string;
  [k: string]: unknown;
}

/** A parsed broker row is a task row only when it is a plain object
 * with a numeric id (#7299's shape discipline, carried over): broker
 * bodies are shape-untrusted at this boundary — a bare cast lets a
 * string body resolve as a phantom TaskRow. */
function isTaskRow(v: unknown): v is TaskRow {
  return typeof v === "object" && v !== null && !Array.isArray(v) && typeof (v as { id?: unknown }).id === "number";
}

interface BrokerEnvelope {
  ok?: unknown;
  row?: unknown;
  rows?: unknown;
  error?: unknown;
  claim_flow_partial?: { step?: unknown; error?: unknown } | null;
  [k: string]: unknown;
}

export const DISPATCH_WORKER = "dispatch-opencode";

export class QueueClient {
  constructor(
    private brokerUrl: string,
    private brokerToken: string,
  ) {}

  /** One broker round trip. Throws QueueError on a transport failure,
   * a non-OK HTTP status, a non-JSON body, or an {ok: false} envelope
   * (the broker's verb refusals carry their message in ). */
  private async call(path: string, body: Record<string, unknown>): Promise<BrokerEnvelope> {
    let res: Response;
    try {
      res = await fetch(`${this.brokerUrl}${path}`, {
        method: "POST",
        headers: {
          "X-Agent-Token": this.brokerToken,
          "Content-Type": "application/json",
        },
        body: JSON.stringify(body),
        signal: outboundTimeout(),
      });
    } catch (e) {
      throw new QueueError(`queue broker ${path} unreachable: ${e instanceof Error ? e.message : String(e)}`);
    }
    const text = await res.text();
    let parsed: unknown = null;
    if (text.trim()) {
      try {
        parsed = JSON.parse(text);
      } catch {
        throw new QueueError(
          `queue broker ${path} returned a non-JSON body (HTTP ${res.status}): ${text.slice(0, 300)}`,
          res.status,
        );
      }
    }
    if (!res.ok) {
      throw new QueueError(`queue broker ${path} failed (HTTP ${res.status}): ${text.slice(0, 300)}`, res.status);
    }
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new QueueError(`queue broker ${path} returned an unexpected shape: expected an envelope object`);
    }
    const env = parsed as BrokerEnvelope;
    if (env.ok === false) {
      const msg = typeof env.error === "string" ? env.error : `queue broker ${path} refused the request`;
      throw new QueueError(msg);
    }
    return env;
  }

  /** The row an envelope carries, shape-checked; null when the
   * envelope's row is explicitly null (the broker's no-row answer). */
  private envelopeRow(env: BrokerEnvelope, path: string): TaskRow | null {
    if (env.row === null || env.row === undefined) return null;
    if (!isTaskRow(env.row)) {
      throw new QueueError(`queue broker ${path} returned an unexpected shape: row is not a task row`);
    }
    return env.row;
  }

  private envelopeRows(env: BrokerEnvelope, path: string): TaskRow[] {
    if (!Array.isArray(env.rows)) {
      throw new QueueError(`queue broker ${path} returned an unexpected shape: expected a rows array`);
    }
    for (const r of env.rows) {
      if (!isTaskRow(r)) {
        throw new QueueError(`queue broker ${path} returned an unexpected shape: a row is not a task row`);
      }
    }
    return env.rows as TaskRow[];
  }

  /** Enqueue via the broker: the broker runs the packet validator
   * before the row exists, and dedupes on dedupe_key against ALL
   * tasks incl. done ones, so a returned row whose dedupe_key was
   * already consumed is the OLD task — callers get the row as-is.
   * The create-an-issue canonical-key flow anchors to a fresh issue,
   * which cannot collide; the issue_number ATTACH flow anchors to an
   * existing issue whose canonical key may already be consumed (live
   * or terminal), so attach callers MUST detect a pre-existing row
   * and must never report it as newly queued — see
   * enqueueDispatchTask in server.ts (issue #2677). */
  async enqueue(packet: Packet, repo: string): Promise<TaskRow> {
    const queue = REPO_QUEUES[repo];
    if (!queue) throw new QueueError(`no queue mapped for repo ${repo}`);
    const env = await this.call("/enqueue", { packet, queue });
    const row = this.envelopeRow(env, "/enqueue");
    if (!row) throw new QueueError("queue broker /enqueue returned no task row");
    return row;
  }

  /** Assign-immediately, the broker way: claim-flow with an explicit
   * task_id claims exactly the row the enqueue created (claim-by-id
   * is the broker's special case for naming a task), records the
   * lane ledger, and binds the launched session — one round trip,
   * replacing the old worker-upsert + status-CAS PATCH + lane RPC.
   * The claim applies only while the row is still queued: a fleet
   * worker that claimed it in the meantime keeps it, the broker
   * answers with a null row, and this launch backs out
   * (ClaimLostError, #10737). A lane-step partial is noted on the
   * task and the launch proceeds (the claim and the bind are real);
   * a bind-step partial throws — the session binding is the point of
   * the claim, and a launch whose session is not bound must back out
   * through the caller's failure compensation. */
  async claimTask(task: TaskRow, sessionId: string, launchedBy: string, leaseSeconds: number): Promise<TaskRow> {
    const env = await this.call("/claim-flow", {
      task_id: task.id,
      worker: DISPATCH_WORKER,
      queue: task.queue,
      session_id: sessionId,
      launched_by: launchedBy,
      lease_seconds: leaseSeconds,
    });
    const row = this.envelopeRow(env, "/claim-flow");
    if (!row) throw new ClaimLostError(task.id, "queued");
    const partial = env.claim_flow_partial;
    if (partial) {
      if (partial.step === "lane") {
        const msg = typeof partial.error === "string" ? partial.error : "unknown lane error";
        await this.noteTask(task.id, `lane assignment failed (launch unaffected): ${msg}`).catch(() => {});
      } else {
        throw new QueueError(
          `queue broker /claim-flow bind failed for task ${task.id}: ${typeof partial.error === "string" ? partial.error : "unknown bind error"}`,
        );
      }
    }
    return row;
  }

  /** One task by id via the broker's rich list. A failed lookup is
   * not a missing task: null is reserved for a successful empty
   * result, so a transient broker failure throws QueueError instead
   * of making the read route report an existing task as 404. */
  async getTask(id: number): Promise<TaskRow | null> {
    const env = await this.call("/list", { task: id, rich: true });
    const rows = this.envelopeRows(env, "/list");
    return rows[0] ?? null;
  }

  /** Session -> task lookup (issue #5555): the dispatch-launched task
   * bound to sessionId, or null. Two bindings are checked, live
   * first: the current progress.session_id, and — when that binding
   * has been cleared — the durable progress.session_cleared.session_id
   * record the queue leaves on the task row when it clears a binding
   * on park / lease expiry. The cleared record is what keeps the
   * session route repo gate attached to a session after its task
   * unbinds. The broker has no by-session filter, so this scans the
   * rich list by status (live statuses first); a lookup failure
   * throws QueueError — this lookup backs the repo gate on
   * GET /v1/sessions/:id, and a transient broker failure must fail
   * closed (the route answers 502), never read as no task, so no
   * gate. */
  async findTaskBySessionId(sessionId: string): Promise<TaskRow | null> {
    const live = await this.scanBySessionField("session_id", sessionId);
    if (live) return live;
    return this.scanBySessionField("session_cleared", sessionId);
  }

  private async scanBySessionField(
    field: "session_id" | "session_cleared",
    sessionId: string,
  ): Promise<TaskRow | null> {
    const STATUSES = ["claimed", "parked", "handed_off", "queued", "blocked", "done", "cancelled", "failed"];
    const PAGE = 1000;
    const MAX_PAGES = 20;
    for (const status of STATUSES) {
      for (let page = 0; page < MAX_PAGES; page++) {
        const env = await this.call("/list", { status, rich: true, limit: PAGE, offset: page * PAGE });
        const rows = this.envelopeRows(env, "/list");
        for (const row of rows) {
          const progress = row.progress as Record<string, unknown> | undefined;
          const holder =
            field === "session_id" ? progress : (progress?.session_cleared as Record<string, unknown> | undefined);
          if (holder && holder.session_id === sessionId) return row;
        }
        if (rows.length < PAGE) break;
      }
    }
    return null;
  }

  /** Pre-enqueue dedupe-key lookup (issue #4661): the row, if any,
   * that already consumes . Compared by row id after the
   * enqueue (see isPreExistingTaskRow in server.ts) — never by
   * comparing the DB-assigned created_at against the dispatch-host
   * clock. Returns the existing row or null; a lookup failure returns
   * null so a transient broker blip degrades to "no known existing
   * row" (the terminal-status check still catches terminal
   * pre-existing rows). */
  async findByDedupeKey(dedupeKey: string): Promise<TaskRow | null> {
    try {
      const env = await this.call("/list", { dedupe_key: dedupeKey, rich: true });
      const rows = this.envelopeRows(env, "/list");
      return rows[0] ?? null;
    } catch {
      return null;
    }
  }

  /** Record a note on a task this dispatch worker claims: a broker
   * checkpoint, whose progress merge carries the note (dispatch_note,
   * plus last_error inside progress for the annotation-prefix reader).
   * This is the broker-era replacement for the old last_error PATCH —
   * the broker exposes no generic row write. Checkpoints require the
   * task to be claimed by this worker, which is exactly the
   * post-claim window every caller notes in. */
  async noteTask(id: number, note: string): Promise<TaskRow> {
    const env = await this.call("/checkpoint", {
      task_id: id,
      worker: DISPATCH_WORKER,
      stage: "dispatch_note",
      progress: { dispatch_note: note, last_error: note },
    });
    const row = this.envelopeRow(env, "/checkpoint");
    if (!row) throw new QueueError(`queue broker /checkpoint for task ${id} returned no task row`);
    return row;
  }

  async cancelTask(id: number, reason: string, worker = "dispatch"): Promise<void> {
    await this.call("/cancel", { task_id: id, worker, reason });
  }

  /** Record the execution-lane assignment for a claimed task (the
   * broker's /lane verb, the same ledger the queue workers record
   * after claiming). claimTask's claim-flow already records it; this
   * stays for callers that claimed by another path. */
  async assignLane(id: number, worker = DISPATCH_WORKER): Promise<void> {
    await this.call("/lane", { task_id: id, worker });
  }

  /** The broker registers/touches the claiming worker itself inside
   * the claim (claim preamble), so there is no workers-table upsert
   * left to do — the FK-era ensureWorker is a no-op kept so call
   * sites read the same. */
  ensureWorker(): Promise<void> {
    return Promise.resolve();
  }
}
