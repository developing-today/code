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
    const rows = text.trim() ? (JSON.parse(text) as TaskRow[]) : [];
    if (!rows[0]) throw new QueueError(`task PATCH ${id} matched no row`);
    return rows[0];
  }

  async getTask(id: number): Promise<TaskRow | null> {
    const res = await fetch(`${this.baseUrl}/rest/v1/tasks?id=eq.${id}&select=*`, {
      headers: this.headers(false),
    });
    if (!res.ok) return null;
    const rows = (await res.json()) as TaskRow[];
    return rows[0] ?? null;
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
