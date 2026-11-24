// The queue door's store (#12589): the second front door's view of
// the one fleet queue (design §6). The native broker's atomicity
// never lived in the broker — it is the conditional UPDATE in the
// queue backend under single-writer serialization; the doors are
// stateless peers over that backend. This interface is that backend
// contract for the hosted door: every mutation that must be atomic
// is a single conditional operation here (conditionalUpdateTask),
// so two doors claiming concurrently for one item cannot both win
// — the losing update matches zero rows and the claim re-picks,
// exactly as the native door's conditional UPDATE does.
//
// FixtureQueueStore is the in-memory implementation for tests and
// local development; the production implementation is a libSQL
// store over the existing fleet-queue Turso DB (#6741) behind this
// same interface — no second queue, no schema fork. Its methods are
// synchronous by design: in this runtime a synchronous
// check-and-set cannot interleave, which is the same guarantee the
// production store gets from one conditional UPDATE statement.
//
// Row shapes are the native wire rows (turso_queue TASK_COLS /
// LEDGER_COLS / events): JSON columns (payload, progress, result,
// ready_condition, blocked_deps, ledger platform fields stay text)
// travel as parsed values, everything else as stored scalars.

import type { Json, JsonObject } from "./queue-validate.ts";

export interface TaskRow {
  id: number;
  queue: string;
  title: string | null;
  body: string | null;
  payload: Json;
  dedupe_key: string | null;
  priority: number | null;
  status: string;
  not_before: string | null;
  worker_id: string | null;
  claimed_at: string | null;
  lease_until: string | null;
  lane_id: string | null;
  platform: string | null;
  attempts: number;
  max_attempts: number;
  claim_provider: string | null;
  claim_model: string | null;
  claim_variant: string | null;
  claim_session_id: string | null;
  progress: Json;
  result: Json;
  last_error: string | null;
  park_kind: string | null;
  wait_kind: string | null;
  ready_condition: Json;
  required_budget_minutes: number | null;
  last_checked_at: string | null;
  check_outcome: string | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  blocked_deps: Json;
  dep_next_recheck_at: string | null;
  dep_blocked_until: string | null;
  dep_recheck_seconds: number | null;
}

export interface LedgerRow {
  id: number;
  queue: string;
  task_id: number;
  assigned_lane: string | null;
  executed_lane: string | null;
  executed_lane_id: string | null;
  exclusion_reason: string | null;
  opencode_reason: string | null;
  platform_limit: string | null;
  platform_limit_reason: string | null;
  assigned_by: string | null;
  session_id: string | null;
  fallback_reason: string | null;
  assigned_at: string | null;
  executed_at: string | null;
  updated_at: string | null;
}

export interface EventRow {
  id: number;
  worker_id: string | null;
  task_id: number | null;
  kind: string;
  detail: Json;
  created_at: string;
}

export interface LaneRow {
  lane_id: string;
  worker_id: string | null;
  queue: string | null;
  enabled: number;
}

export interface LeaseRow {
  task_id: number;
  worker_id: string;
  lease_until: string;
  heartbeat_at: string;
}

/** A dep observation (dep-recheck): the native DepObs. */
export type DepObs =
  | { tag: "open"; evidence: string }
  | { tag: "cleared"; evidence: string }
  | { tag: "broken"; evidence: string }
  | { tag: "failed"; evidence: string };

/**
 * The GitHub dep probe the dep-recheck machinery consults for
 * pr/issue/ci deps (task deps resolve from the store itself). The
 * native broker runs with no probe configured in exactly the same
 * way: every GitHub dep then reports Failed("no github probe
 * configured on this broker") and stays blocked until its TTL.
 */
export interface QueueDepProbe {
  probeIssue(repo: string, number: number): DepObs;
  probePr(repo: string, number: number): DepObs;
  probeCi(repo: string, runId: number): DepObs;
}

export const NO_DEP_PROBE: QueueDepProbe = {
  probeIssue: () => ({ tag: "failed", evidence: "no github probe configured on this broker" }),
  probePr: () => ({ tag: "failed", evidence: "no github probe configured on this broker" }),
  probeCi: () => ({ tag: "failed", evidence: "no github probe configured on this broker" }),
};

export interface QueueStore {
  /** Insert a task row; returns the stored row (id assigned). */
  insertTask(row: Omit<TaskRow, "id">): TaskRow;
  getTask(id: number): TaskRow | null;
  /** Rows in one queue with this dedupe key, id ASC (enqueue dedupe). */
  tasksByDedupe(queue: string, dedupeKey: string): TaskRow[];
  /** Every task row, id ASC (claim scans, health, identity checks). */
  allTasks(): TaskRow[];
  /**
   * THE conditional update: apply `set` to task `id` only if every
   * `guard` field still equals its expected value; returns whether
   * a row was updated. This is the claim atomicity primitive.
   */
  conditionalUpdateTask(id: number, guard: Partial<TaskRow>, set: Partial<TaskRow>): boolean;
  /** Unconditional field update (post-claim fixups, lifecycle verbs). */
  updateTask(id: number, set: Partial<TaskRow>): void;

  getLedger(taskId: number): LedgerRow | null;
  insertLedger(row: Omit<LedgerRow, "id">): LedgerRow;
  updateLedger(taskId: number, set: Partial<LedgerRow>): void;

  upsertLease(row: LeaseRow, opts: { keepWorker: boolean }): void;
  deleteLease(taskId: number): void;
  deleteExpiredLeases(nowIso: string): void;

  getLane(laneId: string): LaneRow | null;
  insertLane(row: LaneRow): void;

  /** workers upsert (touch_worker): status working/idle + heartbeat. */
  touchWorker(workerId: string, working: boolean, nowIso: string): void;
  /** checkpoints upsert keyed (worker_id, key). */
  upsertCheckpoint(workerId: string, key: string, value: Json): void;

  appendEvent(workerId: string | null, taskId: number | null, kind: string, detail: Json, createdAt: string): EventRow;
  allEvents(): EventRow[];
  eventsForTask(taskId: number): EventRow[];
  deleteEventsBefore(cutoffIso: string): number;

  /**
   * The GitHub mirror's labels for an issue (#12744 enrichment):
   * the stored label names, or null when the mirror has no row.
   * Throws on lookup failure — the caller turns that into a
   * warning, never a gate.
   */
  mirrorLabels(repo: string, issueNumber: number): string[] | null;

  /** What the health verb reports about this backend. */
  describe(): { backend: string; projectSource: string; netloc: string | null };
}

function clone<T>(v: T): T {
  return v === undefined ? v : (JSON.parse(JSON.stringify(v)) as T);
}

export class FixtureQueueStore implements QueueStore {
  private tasks = new Map<number, TaskRow>();
  private ledgers = new Map<number, LedgerRow>();
  private leases = new Map<number, LeaseRow>();
  private lanes = new Map<string, LaneRow>();
  private workers = new Map<string, JsonObject>();
  private checkpoints = new Map<string, Json>();
  private events: EventRow[] = [];
  private mirror = new Map<string, string[]>();
  private nextTaskId = 1;
  private nextLedgerId = 1;
  private nextEventId = 1;

  // --- test seeding helpers (not part of the door contract) ---

  seedMirrorLabels(repo: string, issueNumber: number, labels: string[]): void {
    this.mirror.set(`${repo}#${issueNumber}`, [...labels]);
  }

  seedLane(row: LaneRow): void {
    this.lanes.set(row.lane_id, clone(row));
  }

  workerRow(workerId: string): JsonObject | null {
    const w = this.workers.get(workerId);
    return w ? clone(w) : null;
  }

  checkpointValue(workerId: string, key: string): Json | undefined {
    const v = this.checkpoints.get(`${workerId} ${key}`);
    return v === undefined ? undefined : clone(v);
  }

  leaseRow(taskId: number): LeaseRow | null {
    const l = this.leases.get(taskId);
    return l ? clone(l) : null;
  }

  // --- QueueStore ---

  insertTask(row: Omit<TaskRow, "id">): TaskRow {
    const stored: TaskRow = { ...clone(row), id: this.nextTaskId++ };
    this.tasks.set(stored.id, stored);
    return clone(stored);
  }

  getTask(id: number): TaskRow | null {
    const t = this.tasks.get(id);
    return t ? clone(t) : null;
  }

  tasksByDedupe(queue: string, dedupeKey: string): TaskRow[] {
    return this.allTasks().filter((t) => t.queue === queue && t.dedupe_key === dedupeKey);
  }

  allTasks(): TaskRow[] {
    return [...this.tasks.values()].sort((a, b) => a.id - b.id).map((t) => clone(t));
  }

  conditionalUpdateTask(id: number, guard: Partial<TaskRow>, set: Partial<TaskRow>): boolean {
    const t = this.tasks.get(id);
    if (!t) return false;
    for (const [k, v] of Object.entries(guard)) {
      if (JSON.stringify(t[k as keyof TaskRow] ?? null) !== JSON.stringify(v ?? null)) {
        return false;
      }
    }
    Object.assign(t, clone(set));
    return true;
  }

  updateTask(id: number, set: Partial<TaskRow>): void {
    const t = this.tasks.get(id);
    if (t) Object.assign(t, clone(set));
  }

  getLedger(taskId: number): LedgerRow | null {
    const l = this.ledgers.get(taskId);
    return l ? clone(l) : null;
  }

  insertLedger(row: Omit<LedgerRow, "id">): LedgerRow {
    const stored: LedgerRow = { ...clone(row), id: this.nextLedgerId++ };
    this.ledgers.set(stored.task_id, stored);
    return clone(stored);
  }

  updateLedger(taskId: number, set: Partial<LedgerRow>): void {
    const l = this.ledgers.get(taskId);
    if (l) Object.assign(l, clone(set));
  }

  upsertLease(row: LeaseRow, opts: { keepWorker: boolean }): void {
    const existing = this.leases.get(row.task_id);
    if (existing && opts.keepWorker) {
      existing.lease_until = row.lease_until;
      existing.heartbeat_at = row.heartbeat_at;
      return;
    }
    this.leases.set(row.task_id, clone(row));
  }

  deleteLease(taskId: number): void {
    this.leases.delete(taskId);
  }

  deleteExpiredLeases(nowIso: string): void {
    for (const [id, l] of this.leases) {
      if (l.lease_until < nowIso) this.leases.delete(id);
    }
  }

  getLane(laneId: string): LaneRow | null {
    const l = this.lanes.get(laneId);
    return l ? clone(l) : null;
  }

  insertLane(row: LaneRow): void {
    this.lanes.set(row.lane_id, clone(row));
  }

  touchWorker(workerId: string, working: boolean, nowIso: string): void {
    const existing = this.workers.get(workerId);
    this.workers.set(workerId, {
      worker_id: workerId,
      role: existing?.["role"] ?? null,
      display_name: existing?.["display_name"] ?? null,
      status: working ? "working" : "idle",
      last_heartbeat_at: nowIso,
      created_at: existing?.["created_at"] ?? nowIso,
    });
  }

  upsertCheckpoint(workerId: string, key: string, value: Json): void {
    this.checkpoints.set(`${workerId} ${key}`, clone(value));
  }

  appendEvent(workerId: string | null, taskId: number | null, kind: string, detail: Json, createdAt: string): EventRow {
    const row: EventRow = {
      id: this.nextEventId++,
      worker_id: workerId,
      task_id: taskId,
      kind,
      detail: clone(detail),
      created_at: createdAt,
    };
    this.events.push(row);
    return clone(row);
  }

  allEvents(): EventRow[] {
    return this.events.map((e) => clone(e));
  }

  eventsForTask(taskId: number): EventRow[] {
    return this.events.filter((e) => e.task_id === taskId).map((e) => clone(e));
  }

  deleteEventsBefore(cutoffIso: string): number {
    const before = this.events.length;
    this.events = this.events.filter((e) => !(e.created_at < cutoffIso));
    return before - this.events.length;
  }

  mirrorLabels(repo: string, issueNumber: number): string[] | null {
    const labels = this.mirror.get(`${repo}#${issueNumber}`);
    return labels ? [...labels] : null;
  }

  describe(): { backend: string; projectSource: string; netloc: string | null } {
    return { backend: "hosted-fixture", projectSource: "hosted-fixture", netloc: null };
  }
}
