// Filing a task in the fleet's queue, by whoever serves the app.
//
// The queue's own filer is the fleet binary's broker (`fleet queue
// enqueue`, pkgs/fleet/crates/fleet-queue/src/turso_queue/claim.rs,
// verb_enqueue). It is on a machine, so a task filed while no machine
// was up used to wait for one. This files it the same way, straight
// into the queue's store, from Cloudflare or from a machine: the same
// row, by the same rules, for the packets this app makes.
//
// What is the broker's and is done the same here:
//   - the row: task_from_packet (max_attempts and required budget
//     selected and mirrored into the payload, the priority from the
//     payload's labels when they carry `expedite` or `next-up`, the
//     stated priority otherwise), inserted with the broker's columns;
//   - dedupe on (queue, dedupe_key): a live task with the key is the
//     answer and nothing is filed; so is a finished one, when its newest
//     finished task is done and the packet states no override (#13091);
//     otherwise the newest finished one is the predecessor, and the new
//     task is linked to it and both get an event (revive-on-enqueue,
//     #6838), and a stated override gets its own event;
//   - the labels: a packet that carries its own `payload.labels` is
//     not enriched from the GitHub mirror (#12744). Every packet this
//     app makes carries the labels it put on the issue, so the mirror
//     is never read and the row does not depend on how fresh it is.
// What is left to the broker: a packet with no override whose issue
// another task already works on under another key (the work-identity
// rule, #10627, whose full test is the claim guard's). That is never
// the case for a task filed with its own new issue; when it is, this
// says so (NeedsBroker) and the caller hands the packet to a machine.
//
// Also here, and not the broker's at filing: a task can be filed
// already waiting on others (an issue to close, a pull request to
// merge, another task to finish). The broker blocks a task on others
// only for the lane that holds it; filed that way, the row is exactly
// what that block leaves (status blocked, the dependencies on the row,
// a `blocked` event), so the broker's own recheck releases it when they
// clear, or when the time it may wait for them runs out.
//
// The test that this stays the broker's row: test/filing.test.ts holds
// the row made here to what `fleet queue enqueue --dry-run` made for
// the same packets, and reads the broker's INSERT from its source.
import { OpError } from "./errors.ts";
import type { Ops } from "./ops.ts";

/** The queue's priority scale (validate.rs PRIORITY_SCALE): lower is
 * claimed sooner. */
export const SCALE = [10, 20, 30, 40, 50, 70, 90] as const;
const TASK_DEFAULT_MAX_ATTEMPTS = 1000;
const EXPEDITE_PRIORITY = 15;
const NEXT_UP_PRIORITY = 25;
const PAYLOAD_MAX_BYTES = 16384;
/** What this app keeps a packet's payload under, so the broker's
 * claim guard (which checks the ceiling again) never refuses one. */
export const PAYLOAD_LIMIT = 12000;
const LIVE = ["queued", "claimed", "parked", "handed_off", "blocked"];
const TERMINAL = ["done", "failed", "cancelled"];
/** Who the queue's events name for what this app does. */
export const FILER = "pm-console";
/** A blocker is looked at again this often (DEP_RECHECK_DEFAULT_SECONDS). */
const RECHECK_SECONDS = 900;

// ---- urgency, importance and the claim order ---------------------------

/** How soon it should be picked up. */
export const URGENCIES = [
  ["now", "Now", "picked up next"],
  ["today", "Today", "picked up today"],
  ["week", "This week", "picked up this week"],
  ["later", "Whenever", "picked up when there is room"],
] as const;
/** How much it matters. */
export const IMPORTANCES = [
  ["critical", "Critical"],
  ["high", "High"],
  ["normal", "Normal"],
  ["low", "Low"],
] as const;
export type Urgency = (typeof URGENCIES)[number][0];
export type Importance = (typeof IMPORTANCES)[number][0];

/** The queue has one number, its claim order. Urgency and importance
 * each move it a step down the scale: now and critical is the top
 * (10), whenever and low the bottom (90), and each of the seven steps
 * between is one of the scale's seven numbers. */
export function claimPriority(urgency: Urgency, importance: Importance): number {
  const u = URGENCIES.findIndex((x) => x[0] === urgency);
  const i = IMPORTANCES.findIndex((x) => x[0] === importance);
  return SCALE[Math.max(0, u) + Math.max(0, i)];
}

/** The queue's word for a priority (validate.rs category_for_priority),
 * and the issue label that goes with it (label_for_priority). */
export function category(priority: number): "critical" | "high" | "normal" | "low" {
  if (priority <= 10) return "critical";
  if (priority <= 30) return "high";
  if (priority <= 50) return "normal";
  return "low";
}

// ---- what the owner says about a task ----------------------------------

/** A task this one waits for, as the queue keeps it (parse_block_deps). */
export type Blocker = { kind: "issue" | "pr"; repo: string; number: number } | { kind: "task"; task_id: number };

export interface FilingOptions {
  urgency: Urgency;
  importance: Importance;
  /** The day it is wanted by (UTC), or null. */
  due: string | null;
  blockers: Blocker[];
  /** Words for whoever picks it up. */
  note: string;
  /** Anything else, as names and values. */
  metadata: Record<string, string>;
}

export const DEFAULT_OPTIONS: Readonly<FilingOptions> = Object.freeze({
  urgency: "now",
  importance: "critical",
  due: null,
  blockers: [],
  note: "",
  metadata: {},
});

const REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const NOTE_MAX = 2000;
const META_KEYS = 12;
const META_KEY = /^[A-Za-z0-9_.-]{1,40}$/;
const META_VALUE_MAX = 200;
const BLOCKERS_MAX = 8;

/** A blocker as the owner writes it: `#123`, `owner/repo#123`, `PR #45`,
 * `task 6789`, or an issue's or pull request's address. A bare number
 * with # is an issue of this repository (an issue closes when it is
 * closed, and a pull request's issue when it is merged or closed). */
export function parseBlocker(text: string, repo: string): Blocker | null {
  const s = text.trim();
  let m = /^task\s*#?\s*(\d{1,9})$/i.exec(s);
  if (m) return { kind: "task", task_id: Number(m[1]) };
  m = /^https:\/\/github\.com\/([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)\/(issues|pull)\/(\d{1,9})\/?(?:[?#].*)?$/.exec(s);
  if (m) return { kind: m[2] === "pull" ? "pr" : "issue", repo: m[1], number: Number(m[3]) };
  m = /^(?:(pr|pull request)\s*)?(?:([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+))?#(\d{1,9})$/i.exec(s);
  if (m) return { kind: m[1] ? "pr" : "issue", repo: m[2] ?? repo, number: Number(m[3]) };
  return null;
}

/** How a blocker is said. */
export function blockerSaid(b: Blocker, repo: string): string {
  if (b.kind === "task") return `task ${b.task_id}`;
  const where = b.repo === repo ? "" : b.repo;
  return b.kind === "pr" ? `pull request ${where}#${b.number}` : `${where}#${b.number}`;
}

const isDay = (s: string) =>
  /^\d{4}-\d{2}-\d{2}$/.test(s) && new Date(`${s}T00:00:00Z`).toISOString().slice(0, 10) === s;

/** The owner's choices from an action's arguments: anything left out is
 * the default (now, critical, nothing else); anything given that is not
 * one of them is refused, and says why. */
export function filingOptions(args: Record<string, unknown>, repo: string): FilingOptions {
  const out: FilingOptions = { ...DEFAULT_OPTIONS, blockers: [], metadata: {} };
  if (args.urgency !== undefined) {
    if (!URGENCIES.some((u) => u[0] === args.urgency)) {
      throw new OpError(400, `urgency: one of ${URGENCIES.map((u) => u[0]).join(", ")}`);
    }
    out.urgency = args.urgency as Urgency;
  }
  if (args.importance !== undefined) {
    if (!IMPORTANCES.some((u) => u[0] === args.importance)) {
      throw new OpError(400, `importance: one of ${IMPORTANCES.map((u) => u[0]).join(", ")}`);
    }
    out.importance = args.importance as Importance;
  }
  if (args.due !== undefined && args.due !== null && args.due !== "") {
    if (typeof args.due !== "string" || !isDay(args.due)) throw new OpError(400, "due: a day, as YYYY-MM-DD");
    out.due = args.due;
  }
  if (args.blockers !== undefined) {
    if (!Array.isArray(args.blockers)) throw new OpError(400, "blockers: a list");
    if (args.blockers.length > BLOCKERS_MAX) throw new OpError(400, `blockers: at most ${BLOCKERS_MAX}`);
    for (const raw of args.blockers) {
      const b = typeof raw === "string" ? parseBlocker(raw, repo) : null;
      if (!b) {
        throw new OpError(
          400,
          `blockers: "${String(raw).slice(0, 60)}" is not an issue, pull request or task (#123, PR #45, task 6789)`,
        );
      }
      if (b.kind !== "task" && !REPO.test(b.repo)) throw new OpError(400, `blockers: ${b.repo} is not owner/name`);
      const said = JSON.stringify(b);
      if (!out.blockers.some((x) => JSON.stringify(x) === said)) out.blockers.push(b);
    }
  }
  if (args.note !== undefined && args.note !== null) {
    if (typeof args.note !== "string" || args.note.length > NOTE_MAX) {
      throw new OpError(400, `note: at most ${NOTE_MAX} characters`);
    }
    out.note = args.note.trim();
  }
  if (args.metadata !== undefined && args.metadata !== null) {
    const m = args.metadata;
    if (typeof m !== "object" || Array.isArray(m)) throw new OpError(400, "metadata: names and values");
    const entries = Object.entries(m as Record<string, unknown>);
    if (entries.length > META_KEYS) throw new OpError(400, `metadata: at most ${META_KEYS} names`);
    for (const [k, v] of entries) {
      if (!META_KEY.test(k))
        throw new OpError(400, `metadata: "${k.slice(0, 50)}" is not a name (letters, digits, . _ -, up to 40)`);
      if (typeof v !== "string" || v.length > META_VALUE_MAX) {
        throw new OpError(400, `metadata: ${k} must be words of at most ${META_VALUE_MAX} characters`);
      }
      out.metadata[k] = v;
    }
  }
  return out;
}

/** The options that are not the defaults, as an action's arguments:
 * what the app sends, and what a machine is handed. */
export function optionArgs(o: FilingOptions, repo: string): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  if (o.urgency !== DEFAULT_OPTIONS.urgency) out.urgency = o.urgency;
  if (o.importance !== DEFAULT_OPTIONS.importance) out.importance = o.importance;
  if (o.due) out.due = o.due;
  if (o.blockers.length) out.blockers = o.blockers.map((b) => blockerArg(b, repo));
  if (o.note) out.note = o.note;
  if (Object.keys(o.metadata).length) out.metadata = o.metadata;
  return out;
}

/** A blocker as written in an action's arguments (read back by parseBlocker). */
function blockerArg(b: Blocker, repo: string): string {
  if (b.kind === "task") return `task ${b.task_id}`;
  return `${b.kind === "pr" ? "PR " : ""}${b.repo === repo ? "" : b.repo}#${b.number}`;
}

/** What the owner chose, for the payload: every choice, so a lane
 * reading the task sees them without the issue. */
export function ownerBlock(o: FilingOptions, priority: number) {
  return {
    urgency: o.urgency,
    importance: o.importance,
    priority,
    ...(o.due ? { due: o.due } : {}),
    ...(o.blockers.length ? { blockers: o.blockers } : {}),
    ...(o.note ? { note: o.note } : {}),
    ...(Object.keys(o.metadata).length ? { metadata: o.metadata } : {}),
  };
}

/** What the owner chose, as lines for the issue: only what is not the
 * default, so a plain "do this" reads as before. */
export function ownerLines(o: FilingOptions, repo: string): string[] {
  const lines: string[] = [];
  const u = URGENCIES.find((x) => x[0] === o.urgency);
  const i = IMPORTANCES.find((x) => x[0] === o.importance);
  const p = claimPriority(o.urgency, o.importance);
  if (o.urgency !== "now" || o.importance !== "critical") {
    lines.push(`**Urgency:** ${u?.[1]} · **Importance:** ${i?.[1]} · queue priority ${p}`);
  }
  if (o.due) lines.push(`**Wanted by:** ${o.due}`);
  if (o.blockers.length) lines.push(`**Waits for:** ${o.blockers.map((b) => blockerSaid(b, repo)).join(", ")}`);
  if (o.note) lines.push(`**Note:** ${o.note}`);
  const meta = Object.entries(o.metadata);
  if (meta.length) lines.push(`**Metadata:** ${meta.map(([k, v]) => `${k}=${v}`).join("; ")}`);
  return lines;
}

// ---- the row -------------------------------------------------------------

type Json = Record<string, unknown>;

/** A time as the queue writes it (core.rs iso): to the second, Z. */
export const queueTime = (d: Date) => d.toISOString().replace(/\.\d{3}Z$/, "Z");

/** The row a packet becomes (validate.rs task_from_packet). */
export function taskRow(packet: Json, queue: string) {
  const payload: Json = structuredClone((packet.payload as Json) ?? {});
  const given = (v: unknown) => v !== undefined && v !== null;
  const maxAttempts = given(packet.max_attempts)
    ? packet.max_attempts
    : given(payload.max_attempts)
      ? payload.max_attempts
      : TASK_DEFAULT_MAX_ATTEMPTS;
  const priorMirror = payload.max_attempts;
  const hadMirror = Object.hasOwn(payload, "max_attempts");
  payload.max_attempts = maxAttempts;
  if (JSON.stringify(payload).length > PAYLOAD_MAX_BYTES) {
    if (hadMirror) payload.max_attempts = priorMirror;
    else delete payload.max_attempts;
  }
  const budget = given(packet.required_budget_minutes)
    ? packet.required_budget_minutes
    : given(payload.required_budget_minutes)
      ? payload.required_budget_minutes
      : null;
  if (budget !== null) payload.required_budget_minutes = budget;
  const labels = Array.isArray(payload.labels) ? (payload.labels as unknown[]) : null;
  const band = labels?.includes("expedite") ? EXPEDITE_PRIORITY : labels?.includes("next-up") ? NEXT_UP_PRIORITY : null;
  return {
    queue,
    title: packet.title ?? null,
    body: packet.body ?? null,
    payload,
    dedupe_key: packet.dedupe_key ?? null,
    priority: band ?? packet.priority ?? null,
    not_before: packet.not_before ?? null,
    max_attempts: maxAttempts,
    required_budget_minutes: budget,
  };
}

/** The issue a packet is about (label_enrich.rs payload_issue_ref). */
function issueRef(payload: Json, dedupeKey: unknown): { repo: string; number: number } | null {
  const number = Number((payload.issue as Json | undefined)?.number);
  if (!Number.isSafeInteger(number) || number <= 0) return null;
  const fromKey = (k: unknown) => {
    if (typeof k !== "string" || !k.startsWith("repo-health:v1:")) return null;
    const rest = k.slice("repo-health:v1:".length);
    const at = rest.indexOf(":");
    if (at < 0) return null;
    const tail = rest.slice(at + 1);
    const repo = tail.split(":issue:")[0];
    return repo === "" || repo === tail ? null : repo;
  };
  const repo =
    (typeof payload.repo === "string" && payload.repo !== "" ? payload.repo : null) ??
    (typeof (payload.issue as Json).repo === "string" && (payload.issue as Json).repo !== ""
      ? ((payload.issue as Json).repo as string)
      : null) ??
    fromKey(dedupeKey);
  return repo?.includes("/") ? { repo, number } : null;
}

/** The packet with its issue's labels from the GitHub mirror, where it
 * carries none of its own (label_enrich.rs enrich_payload_labels). The
 * mirror never stops a filing: one that cannot be read is passed over,
 * as the broker passes over it. */
async function withMirrorLabels(ops: Ops, packet: Json): Promise<Json> {
  const payload = (packet.payload as Json | undefined) ?? null;
  if (!payload || Array.isArray(payload.labels)) return packet;
  const ref = issueRef(payload, packet.dedupe_key);
  if (!ref) return packet;
  try {
    const [rows] = await ops.reads([
      { sql: "SELECT labels FROM github_mirror_issues WHERE repo = ? AND number = ?", args: [ref.repo, ref.number] },
    ]);
    if (!rows[0]) return packet;
    const raw = rows[0].labels;
    let list: unknown = raw;
    if (typeof raw === "string") {
      try {
        list = JSON.parse(raw);
      } catch {
        list = [];
      }
    }
    const labels = Array.isArray(list) ? list.filter((l): l is string => typeof l === "string") : [];
    if (labels.length === 0) return packet;
    return { ...packet, payload: { ...payload, labels } };
  } catch {
    return packet;
  }
}

/** The packet's override reason (work_identity.rs dedupe_override). */
function overrideOf(packet: Json): string | null {
  const payload = (packet.payload as Json) ?? packet;
  const r = typeof payload.dedupe_override === "string" ? payload.dedupe_override.trim() : "";
  return r === "" ? null : r;
}

/** The broker's word that this packet is the broker's to file. */
export class NeedsBroker extends Error {
  constructor(why: string) {
    super(why);
    this.name = "NeedsBroker";
  }
}

export interface Filed {
  /** A new task was put in the queue. */
  filed: boolean;
  /** The task that has the work: the new one, or the one already there. */
  task: number;
  status: string;
  priority: number | null;
  /** What happened, said for the owner. */
  said: string;
}

/** The columns the broker's INSERT writes, in its order (claim.rs
 * verb_enqueue); the test reads them from the broker's source. */
export const BROKER_COLUMNS = [
  "queue",
  "title",
  "body",
  "payload",
  "dedupe_key",
  "priority",
  "status",
  "not_before",
  "max_attempts",
  "required_budget_minutes",
  "platform",
  "progress",
  "result",
  "created_at",
  "updated_at",
] as const;
/** And the ones a block writes (admin.rs verb_block). */
const BLOCK_COLUMNS = ["blocked_deps", "dep_next_recheck_at", "dep_blocked_until", "dep_recheck_seconds"] as const;

/** How long a task filed waiting may wait for what it waits for, at
 * most (DEP_TTL_MAX_SECONDS): then it is released all the same. */
const WAIT_AT_MOST_SECONDS = 604_800;

/**
 * File `packet` in the queue, as the broker would, straight into the
 * store. `waitsFor` files it blocked on those (see the top). Throws
 * NeedsBroker when only the broker can say what filing it means; an
 * error from the store is thrown as it is (the task may or may not be
 * in; filing it again is safe, because the key is checked first).
 */
export async function fileTask(ops: Ops, given: Json, now: Date, waitsFor: Blocker[] = []): Promise<Filed> {
  const queue = ops.cfg.queue;
  const packet = await withMirrorLabels(ops, given);
  const row = taskRow(packet, queue);
  const payload = row.payload;
  if (typeof row.title !== "string" || row.title.trim() === "") throw new OpError(400, "the task has no title");
  if (!SCALE.includes(Number(packet.priority) as (typeof SCALE)[number])) {
    throw new OpError(400, `the task's priority is not one of ${SCALE.join(", ")}`);
  }
  if (JSON.stringify(payload).length > PAYLOAD_LIMIT) {
    throw new OpError(400, "what was given is too long to file as one task: shorten the note or the metadata");
  }
  const dedupe = typeof row.dedupe_key === "string" ? row.dedupe_key : null;
  if (!dedupe) throw new NeedsBroker("a task with no dedupe key");
  const override = overrideOf(packet);
  const kind = typeof payload.kind === "string" ? payload.kind : "";
  const issue = Number((payload.issue as Json | undefined)?.number);
  // The family a kind belongs to (work_identity.rs kind_family): only
  // these have a work identity.
  const family = { small_fix: "fix", triage: "triage", pr_shepherd: "pr_shepherd" }[kind] ?? null;
  const identityChecked =
    family !== null && Number.isSafeInteger(issue) && issue > 0 && override === null && payload.dedupe_exempt !== true;
  const LIVE_OR_DONE = `('${[...LIVE, "done"].join("','")}')`;

  const [keyed, holders] = await ops.reads([
    {
      sql: "SELECT id, status, priority, completed_at FROM tasks WHERE queue = ? AND dedupe_key = ? ORDER BY id ASC",
      args: [queue, dedupe],
    },
    {
      sql: identityChecked
        ? `SELECT id, status, priority FROM tasks WHERE status IN ${LIVE_OR_DONE} AND json_extract(payload, '$.kind') = ? AND CAST(json_extract(payload, '$.issue.number') AS INTEGER) = ? ORDER BY id ASC LIMIT 5`
        : "SELECT 1 AS none WHERE 0",
      args: identityChecked ? [kind, issue] : [],
    },
  ]);
  const live = keyed.find((r) => LIVE.includes(String(r.status)));
  if (live) return hit(live, "is already in the queue");
  const terminal = keyed.filter((r) => TERMINAL.includes(String(r.status))).pop() ?? null;
  if (terminal && override === null && terminal.status === "done") return hit(terminal, "was done already");
  if (identityChecked && holders.length > 0) {
    throw new NeedsBroker(`task ${holders[0].id} already works on #${issue}`);
  }

  const at = queueTime(now);
  const blocked = waitsFor.length > 0;
  const deps = waitsFor.map((b) => ({
    kind: b.kind,
    state: "open",
    reason: "named by the owner when filing",
    recorded_by: FILER,
    recorded_at: at,
    last_checked_at: null,
    last_observed: null,
    ...(b.kind === "task" ? { task_id: b.task_id } : { repo: b.repo, number: b.number }),
  }));
  const until = queueTime(new Date(now.getTime() + WAIT_AT_MOST_SECONDS * 1000));
  const blocker = blocked
    ? `waits for ${waitsFor.map((b) => blockerSaid(b, String(payload.repo ?? ""))).join(", ")}`
    : "";
  const marks = blocked ? { blocker, reason_class: "dependency", blocked_via: "owner" } : {};
  const values: (string | number | null)[] = [
    queue,
    row.title as string,
    (row.body as string | null) ?? null,
    JSON.stringify(payload),
    dedupe,
    row.priority as number,
    blocked ? "blocked" : "queued",
    (row.not_before as string | null) ?? null,
    Number(row.max_attempts),
    row.required_budget_minutes === null ? null : Number(row.required_budget_minutes),
    null,
    JSON.stringify(marks),
    JSON.stringify(marks),
    at,
    at,
    blocked ? JSON.stringify(deps) : null,
    blocked ? at : null,
    blocked ? until : null,
    blocked ? RECHECK_SECONDS : null,
  ];
  const columns = [...BROKER_COLUMNS, ...BLOCK_COLUMNS];
  // One statement, so nothing can file the same work between the look
  // above and the write: it writes only while the key is still unused
  // (by a live or done task, as above) and, where it was looked for, no
  // other task works on the issue.
  const guard = [
    `NOT EXISTS (SELECT 1 FROM tasks WHERE queue = ? AND dedupe_key = ? AND status IN ('${LIVE.join("','")}'))`,
    terminal === null
      ? "NOT EXISTS (SELECT 1 FROM tasks WHERE queue = ? AND dedupe_key = ?)"
      : "(SELECT max(id) FROM tasks WHERE queue = ? AND dedupe_key = ?) = ?",
    ...(identityChecked
      ? [
          `NOT EXISTS (SELECT 1 FROM tasks WHERE status IN ${LIVE_OR_DONE} AND json_extract(payload, '$.kind') = ? AND CAST(json_extract(payload, '$.issue.number') AS INTEGER) = ?)`,
        ]
      : []),
  ];
  const guardArgs: (string | number)[] = [
    queue,
    dedupe,
    queue,
    dedupe,
    ...(terminal === null ? [] : [Number(terminal.id)]),
    ...(identityChecked ? [kind, issue] : []),
  ];
  const [inserted] = await ops.writes([
    {
      sql: `INSERT INTO tasks (${columns.join(", ")}) SELECT ${columns.map(() => "?").join(", ")} WHERE ${guard.join(" AND ")} RETURNING id`,
      args: [...values, ...guardArgs],
    },
  ]);
  const id = Number(inserted[0]?.id);
  if (!Number.isSafeInteger(id) || id <= 0) {
    // Something filed it between the look and the write: say what is there.
    const [again] = await ops.reads([
      {
        sql: "SELECT id, status, priority FROM tasks WHERE queue = ? AND dedupe_key = ? ORDER BY id DESC LIMIT 1",
        args: [queue, dedupe],
      },
    ]);
    if (again[0]) return hit(again[0], "was filed a moment ago");
    throw new NeedsBroker("the task could not be written and is not there");
  }

  // What the broker writes after its insert, in its order.
  const after: { sql: string; args: (string | number | null)[] }[] = [];
  const event = (task: number, kind: string, detail: unknown) =>
    after.push({
      sql: "INSERT INTO events (worker_id, task_id, kind, detail, created_at) VALUES (?, ?, ?, ?, ?)",
      args: [FILER, task, kind, JSON.stringify(detail), at],
    });
  if (terminal) {
    const link = {
      task_id: Number(terminal.id),
      status: terminal.status,
      completed_at: terminal.completed_at ?? null,
      revived_at: at,
      via: "enqueue_repo_task_turso",
    };
    after.push({
      sql: "UPDATE tasks SET progress = ? WHERE id = ?",
      args: [JSON.stringify({ ...marks, revived_from: link }), id],
    });
    event(id, "task_revived", { predecessor: link, dedupe_key: dedupe });
    event(Number(terminal.id), "revived_by", { successor_task_id: id, dedupe_key: dedupe });
  }
  if (override !== null) event(id, "dedupe_override", { reason: override, identity: identityKey(payload, queue) });
  if (blocked) {
    event(id, "blocked", {
      blocker,
      reason_class: "dependency",
      deps,
      dep_blocked_until: until,
      dep_recheck_seconds: RECHECK_SECONDS,
    });
  }
  if (after.length) await ops.writes(after);
  const priority = row.priority as number;
  return {
    filed: true,
    task: id,
    status: blocked ? "blocked" : "queued",
    priority,
    said: blocked
      ? `filed as task ${id}, waiting for ${waitsFor.length === 1 ? "one" : waitsFor.length} other${waitsFor.length === 1 ? "" : "s"}`
      : `filed as task ${id}`,
  };

  function hit(r: Record<string, unknown>, why: string): Filed {
    return {
      filed: false,
      task: Number(r.id),
      status: String(r.status),
      priority: r.priority === null || r.priority === undefined ? null : Number(r.priority),
      said: `task ${Number(r.id)} ${why}`,
    };
  }
}

/** The work identity's key (work_identity.rs WorkIdentity::key), for
 * the override event; null where the packet has none. */
function identityKey(payload: Json, queue: string): string | null {
  const family = ({ small_fix: "fix", triage: "triage", pr_shepherd: "pr_shepherd" } as Record<string, string>)[
    String(payload.kind)
  ];
  const issue = Number((payload.issue as Json | undefined)?.number);
  if (!family || !Number.isSafeInteger(issue) || issue <= 0 || payload.dedupe_exempt === true) return null;
  const repo =
    typeof payload.repo === "string" && payload.repo !== ""
      ? payload.repo
      : queue
          .replace(/^repo-health-/, "")
          .replace("-pilot", "")
          .replace("-shadow", "");
  if (!repo.includes("/")) return null;
  return `${repo}#${issue}:${family}`;
}
