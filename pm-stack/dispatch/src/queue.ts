// Supabase queue client (PostgREST, agent schema) — the same access
// pattern as ~/tools/repo-queue.py: service-role key via apikey +
// Authorization headers, Accept-Profile/Content-Profile: agent.
//
// Enqueue uses the same RPC selection repo-queue.py uses:
// enqueue_repo_task for developing-today/code, enqueue_repo_task_fleet
// for the other fleet repos. The RPC returns the created row (incl. id);
// NOTE it dedupes on dedupe_key against ALL tasks incl. done ones, so a
// returned row whose dedupe_key was already consumed is the OLD task —
// callers get the row as-is. The create-an-issue canonical-key flow
// anchors to a fresh issue, which cannot collide; the issue_number
// ATTACH flow anchors to an existing issue whose canonical key may
// already be consumed (live or terminal), so attach callers MUST detect
// a pre-existing row and must never report it as newly queued — see
// enqueueDispatchTask in server.ts (issue #2677).
import type { Packet } from "./packet.ts";
import { REPO_QUEUES } from "./config.ts";

export class QueueError extends Error {
  status = 502;
  constructor(
    message: string,
    readonly httpStatus?: number,
  ) {
    super(message);
  }
}

export interface TaskRow {
  id: number;
  queue: string;
  status: string;
  dedupe_key: string;
  [k: string]: unknown;
}

/** A parsed PostgREST body element is a task row only when it is a
 * plain object with a numeric id (#7299). PostgREST 200 bodies are
 * shape-untrusted: a bare cast lets a string body resolve as a
 * phantom TaskRow and a null body escape as a raw TypeError. */
function isTaskRow(v: unknown): v is TaskRow {
  return (
    typeof v === "object" &&
    v !== null &&
    !Array.isArray(v) &&
    typeof (v as { id?: unknown }).id === "number"
  );
}

/** The first row of a parsed rows body, or null when the body is
 * not an array, is empty, or its first element is not a task row. */
function firstTaskRow(body: unknown): TaskRow | null {
  if (!Array.isArray(body) || body.length === 0) return null;
  return isTaskRow(body[0]) ? body[0] : null;
}

export class QueueClient {
  constructor(
    private baseUrl: string,
    private serviceRoleKey: string,
  ) {}

  private headers(write: boolean): Record<string, string> {
    const h: Record<string, string> = {
      apikey: this.serviceRoleKey,
      Authorization: `Bearer ${this.serviceRoleKey}`,
      "Accept-Profile": "agent",
    };
    if (write) {
      h["Content-Profile"] = "agent";
      h["Content-Type"] = "application/json";
    }
    return h;
  }

  private async rpc(name: string, args: Record<string, unknown>): Promise<unknown> {
    const res = await fetch(`${this.baseUrl}/rest/v1/rpc/${name}`, {
      method: "POST",
      headers: this.headers(true),
      body: JSON.stringify(args),
    });
    const text = await res.text();
    if (!res.ok) {
      throw new QueueError(`queue RPC ${name} failed (HTTP ${res.status}): ${text.slice(0, 300)}`, res.status);
    }
    if (!text.trim()) return null;
    try {
      return JSON.parse(text);
    } catch {
      return null;
    }
  }

  async enqueue(packet: Packet, repo: string): Promise<TaskRow> {
    const queue = REPO_QUEUES[repo];
    if (!queue) throw new QueueError(`no queue mapped for repo ${repo}`);
    const rpcName = repo === "developing-today/code" ? "enqueue_repo_task" : "enqueue_repo_task_fleet";
    const result = await this.rpc(rpcName, {
      p_queue: queue,
      p_title: packet.title,
      p_body: packet.body,
      p_payload: packet.payload,
      p_dedupe_key: packet.dedupe_key,
      p_priority: packet.priority,
      p_not_before: null,
      p_max_attempts: (packet.payload.max_attempts as number) ?? null,
    });
    const row = (Array.isArray(result) ? result[0] : result) as TaskRow | null;
    if (!row || typeof row.id !== "number") {
      throw new QueueError(`queue RPC ${rpcName} returned no task row`);
    }
    return row;
  }

  /** Direct row PATCH (service role). This is how a dispatch-launched task
   * is "assigned immediately": claim_repo_task claims the NEXT task in a
   * queue, not a specific one, so claiming the exact row the enqueue just
   * created has to be a PATCH on that row. */
  async patchTask(id: number, fields: Record<string, unknown>): Promise<TaskRow> {
    const res = await fetch(`${this.baseUrl}/rest/v1/tasks?id=eq.${id}`, {
      method: "PATCH",
      headers: { ...this.headers(true), Prefer: "return=representation" },
      body: JSON.stringify(fields),
    });
    const text = await res.text();
    if (!res.ok) {
      throw new QueueError(`task PATCH ${id} failed (HTTP ${res.status}): ${text.slice(0, 300)}`, res.status);
    }
    let parsed: unknown = null;
    if (text.trim()) {
      try {
        parsed = JSON.parse(text);
      } catch {
        throw new QueueError(`task PATCH ${id} returned a non-JSON body: ${text.slice(0, 300)}`);
      }
    }
    const rows: unknown[] = Array.isArray(parsed) ? parsed : [];
    if (text.trim() && !Array.isArray(parsed)) {
      throw new QueueError(`task PATCH ${id} returned an unexpected shape: expected an array of task rows`);
    }
    if (rows.length > 0 && !isTaskRow(rows[0])) {
      throw new QueueError(`task PATCH ${id} returned an unexpected shape: first element is not a task row`);
    }
    const row = firstTaskRow(rows);
    if (!row) throw new QueueError(`task PATCH ${id} matched no row`);
    return row;
  }

  async getTask(id: number): Promise<TaskRow | null> {
    const res = await fetch(`${this.baseUrl}/rest/v1/tasks?id=eq.${id}&select=*`, {
      headers: this.headers(false),
    });
    // A failed lookup is not a missing task: returning null here made
    // the read route report an existing task as 404 whenever PostgREST
    // answered 401/5xx. Null is reserved for a successful empty result.
    if (!res.ok) {
      throw new QueueError(`task GET ${id} failed (HTTP ${res.status})`, res.status);
    }
    // A malformed 200 body is a lookup failure, not a missing task
    // (#7299): null is reserved for a successful EMPTY array, as with
    // the session lookup below — anything else (non-JSON, non-array,
    // or an array whose first element is not a task row) throws
    // QueueError, so the read route reports a lookup failure instead
    // of a 404 for a task that may exist. Never a raw SyntaxError
    // from res.json(), never a phantom non-object row.
    const body: unknown = await res.json().catch(() => {
      throw new QueueError(`task GET ${id} returned a non-JSON body`);
    });
    if (!Array.isArray(body)) {
      throw new QueueError(`task GET ${id} returned an unexpected shape: expected an array of task rows`);
    }
    const row = firstTaskRow(body);
    if (body.length > 0 && !row) {
      throw new QueueError(`task GET ${id} returned an unexpected shape: first element is not a task row`);
    }
    return row;
  }

  /** Session -> task lookup (issue #5555): the dispatch-launched task
   * bound to sessionId, or null. Two bindings are checked, live first:
   * the current progress.session_id, and - when that binding has been
   * cleared - the durable progress.session_cleared.session_id record
   * the queue leaves on the task row when it clears a binding on park /
   * lease expiry (opencode-claimant migration-0023). The cleared record
   * is what keeps the session route repo gate attached to a session
   * after its task unbinds: without it, removing the repo from the
   * allowlist would silently un-gate a still-live session. Unlike the
   * dedupe pre-check below, a lookup failure throws QueueError - this
   * lookup backs the repo gate on GET /v1/sessions/:id, and a transient
   * PostgREST failure must fail closed (the route answers 502), never
   * read as no task, so no gate. */
  async findTaskBySessionId(sessionId: string): Promise<TaskRow | null> {
    const live = await this.findTaskByProgressField('progress->>session_id', sessionId);
    if (live) return live;
    return this.findTaskByProgressField('progress->session_cleared->>session_id', sessionId);
  }

  private async findTaskByProgressField(field: string, sessionId: string): Promise<TaskRow | null> {
    const url = this.baseUrl + '/rest/v1/tasks?' + field + '=eq.' + encodeURIComponent(sessionId) + '&select=*&limit=1';
    const res = await fetch(url, { headers: this.headers(false) });
    if (!res.ok) {
      throw new QueueError('task lookup by session failed (HTTP ' + res.status + ')', res.status);
    }
    // This lookup backs the session route's repo gate, so a
    // malformed 200 body fails closed with QueueError (#7299),
    // exactly like a non-OK response — never a raw SyntaxError,
    // never a phantom row the gate would trust.
    const body: unknown = await res.json().catch(() => {
      throw new QueueError("task lookup by session returned a non-JSON body");
    });
    if (!Array.isArray(body)) {
      throw new QueueError("task lookup by session returned an unexpected shape: expected an array of task rows");
    }
    const row = firstTaskRow(body);
    if (body.length > 0 && !row) {
      throw new QueueError("task lookup by session returned an unexpected shape: first element is not a task row");
    }
    return row;
  }

  /** Pre-enqueue dedupe-key lookup (issue #4661): the row, if any, that
   * already consumes `dedupeKey`. Compared by row id after the enqueue
   * (see isPreExistingTaskRow in server.ts) — never by comparing the
   * DB-assigned created_at against the dispatch-host clock. Returns the
   * existing row or null; a lookup failure returns null so a transient
   * PostgREST blip degrades to "no known existing row" (the terminal-
   * status check still catches terminal pre-existing rows). */
  async findByDedupeKey(dedupeKey: string): Promise<TaskRow | null> {
    const res = await fetch(
      `${this.baseUrl}/rest/v1/tasks?dedupe_key=eq.${encodeURIComponent(dedupeKey)}&select=*&limit=1`,
      { headers: this.headers(false) },
    );
    if (!res.ok) return null;
    // Lookup failures resolve null by contract (see above), and a
    // malformed 200 body is a lookup failure (#7299): never a raw
    // SyntaxError, never a phantom non-object row.
    const body: unknown = await res.json().catch(() => null);
    return firstTaskRow(body);
  }

  async cancelTask(id: number, reason: string, worker = "dispatch"): Promise<void> {
    await this.rpc("cancel_repo_task", { p_task_id: id, p_worker: worker, p_reason: reason });
  }

  /** Record the execution-lane assignment for a claimed task
   * (assign_repo_task_lane, the lane-ledger RPC the queue workers call
   * after claiming). It reads the lane from the task's progress, which
   * the claim PATCH has just set — so it must run after that PATCH. */
  async assignLane(id: number, worker = "dispatch-opencode"): Promise<void> {
    await this.rpc("assign_repo_task_lane", {
      p_task_id: id,
      p_worker: worker,
      p_exclusion_reason: null,
    });
  }

  private workerEnsured: Promise<void> | null = null;

  /** agent.tasks.worker_id is an FK into agent.workers, so the dispatch
   * worker identity must exist as a row before a task can be claimed
   * under it. Idempotent upsert (ignore-duplicates on the id PK). */
  ensureWorker(): Promise<void> {
    this.workerEnsured ??= (async () => {
      const res = await fetch(`${this.baseUrl}/rest/v1/workers`, {
        method: "POST",
        headers: {
          ...this.headers(true),
          Prefer: "resolution=ignore-duplicates,return=minimal",
        },
        body: JSON.stringify({
          id: "dispatch-opencode",
          role: "build",
          display_name: "PM dispatch — OpenCode session launcher",
          status: "working",
        }),
      });
      if (!res.ok) {
        const text = await res.text();
        throw new QueueError(`worker registration failed (HTTP ${res.status}): ${text.slice(0, 200)}`, res.status);
      }
    })().catch((err: unknown) => {
      // A rejected registration must not be cached (#2902):
      // registration failures are overwhelmingly transient (a
      // PostgREST 5xx, a network blip), and memoizing the rejection
      // wedges every later launch for the life of the process.
      // Reset the memo so the next call retries the upsert; a
      // successful registration stays cached. The reset runs before
      // the rejection reaches any caller awaiting this promise, and
      // no newer attempt can have replaced the memo in between --
      // a new attempt is only assigned while the memo is null, and
      // this catch is the only place that nulls it.
      this.workerEnsured = null;
      throw err;
    });
    return this.workerEnsured;
  }
}
