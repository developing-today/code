// Integrated queue/PM view — machine face (#7078, #2072 slice 3, first piece).
//
// One read model over two existing sources, no new ingestion and no new
// source of truth:
//   1. fleet queue task records (the broker's task JSON, as emitted by
//      `fleet queue claim` / the queue-broker /list rich response), and
//   2. the slice-2 hot list (#5204, PR #5327): the projector's append-only
//      `pmp_hot_events` ledger, consumed through a consumer checkpoint
//      exactly as slice 2 designed it (pin the high-water, page inside it,
//      ack only after the view has been emitted).
//
// The view answers, for a given window:
//   - what is queued, in claim order (the broker's pick order:
//     (priority IS NULL), priority ASC, created_at ASC) — a current
//     snapshot, because "what is queued, in what order" is a now-question;
//   - what was picked up inside the window, and by which system
//     (worker_id / lane_id / platform carried on the task record);
//   - snapshot counts by status, which reconcile with `fleet queue
//     health` when the caller feeds the full task list;
//   - links (never copies) to the existing usage/quota/job trend stores.
//
// Join key: the repo-qualified issue ref `owner/repo#N`, taken from the
// task payload (`payload.repo` + `payload.issue.number`) or, failing
// that, parsed out of the dedupe key
// (`repo-health:v1:<kind>:<owner/repo>:issue:<N>`). Hot events carry the
// same ref shape, so the join needs no hand reconstruction.

import type { Client } from "@libsql/client";
import {
  ackConsumerCheckpoint,
  getConsumerCheckpoint,
  hotHighWater,
  listHotEvents,
  type HotEventRow,
} from "./store";

/** A fleet queue task record (subset of the broker's task JSON we read). */
export interface QueueTaskRecord {
  id: number;
  queue?: string | null;
  title?: string | null;
  status: string;
  priority?: number | null;
  created_at?: string | null;
  claimed_at?: string | null;
  worker_id?: string | null;
  lane_id?: string | null;
  platform?: string | null;
  dedupe_key?: string | null;
  progress?: {
    last_checkpoint?: { at?: string | null } | null;
  } | null;
  payload?: {
    repo?: string | null;
    kind?: string | null;
    issue?: { number?: number | null } | null;
  } | null;
}

export interface ViewHotEvent {
  seq: number;
  event: string;
  action: string | null;
  actor: string | null;
  received_at: string;
  source_url: string | null;
}

export interface QueuedEntry {
  task_id: number;
  queue: string | null;
  title: string | null;
  priority: number | null;
  created_at: string | null;
  ref: string | null;
  kind: string | null;
  hot_events: ViewHotEvent[];
}

export interface ClaimedEntry extends QueuedEntry {
  claimed_at: string | null;
  claimed_by: {
    worker_id: string | null;
    lane_id: string | null;
    platform: string | null;
  };
}

export interface TrendLink {
  label: string;
  store: string;
  ref: string;
}

export interface QueueView {
  generated_at: string;
  window: { since: string; until: string };
  counts: Record<string, number>;
  queued: QueuedEntry[];
  claimed: ClaimedEntry[];
  hot_list: {
    consumer: string | null;
    from_seq: number;
    high_water: number;
    events_consumed: number;
  };
  links: TrendLink[];
}

export interface QueueViewOptions {
  /** Window start (ISO-8601). Claimed entries and hot events are filtered to it. */
  since: string;
  /** Window end (ISO-8601). Defaults to `since + 24h` at the CLI layer. */
  until: string;
  consumer?: string | null;
  fromSeq?: number;
  highWater?: number;
  links?: TrendLink[];
  now?: string;
}

const DEDUPE_REF_RE = /:((?:[A-Za-z0-9_.-]+\/){1,2}[A-Za-z0-9_.-]+):issue:(\d+)$/;

/** The repo-qualified issue ref for a task, or null when it carries none. */
export function taskRef(task: QueueTaskRecord): string | null {
  const repo = task.payload?.repo;
  const num = task.payload?.issue?.number;
  if (repo && typeof num === "number") return `${repo}#${num}`;
  if (task.dedupe_key) {
    const m = DEDUPE_REF_RE.exec(task.dedupe_key);
    if (m) return `${m[1]}#${m[2]}`;
  }
  return null;
}

/** The broker's claim order: (priority IS NULL), priority ASC, created_at ASC. */
export function claimOrder(a: QueueTaskRecord, b: QueueTaskRecord): number {
  const an = a.priority == null ? 1 : 0;
  const bn = b.priority == null ? 1 : 0;
  if (an !== bn) return an - bn;
  if (an === 0 && a.priority !== b.priority) return (a.priority as number) - (b.priority as number);
  const ac = a.created_at ?? "";
  const bc = b.created_at ?? "";
  if (ac !== bc) return ac < bc ? -1 : 1;
  return a.id - b.id;
}

function inWindow(iso: string | null | undefined, since: string, until: string): boolean {
  if (!iso) return false;
  return iso >= since && iso <= until;
}

/**
 * When the task was picked up, as far as the record can say: ONLY the
 * record's own `claimed_at` is a claim time. The broker's rich /list
 * rows omit `claimed_at` and carry `progress.last_checkpoint.at`
 * instead, but that timestamp is overwritten by every checkpoint
 * (verb_checkpoint), so it is activity evidence, never a claim time:
 * substituting it mis-dates claims in both directions (a task claimed
 * before the window but checkpointed inside it reads as newly
 * claimed; a fresh claim carrying an old checkpoint reads as stale).
 * Rows without `claimed_at` report null (unknown); buildQueueView
 * lists such claimed rows as now-facts rather than dating them by a
 * checkpoint. (Codex finding on PR #8481, 2026-11-11.)
 */
export function claimTimestamp(task: QueueTaskRecord): string | null {
  return task.claimed_at ?? null;
}

function toViewEvents(events: HotEventRow[]): ViewHotEvent[] {
  return events.map((e) => ({
    seq: e.seq,
    event: e.event,
    action: e.action,
    actor: e.actor,
    received_at: e.received_at,
    source_url: e.source_url,
  }));
}

/**
 * Build the integrated view from task records + already-consumed hot
 * events. Pure: the same inputs always produce the same view, so the
 * join is unit-testable without a database or a live broker.
 */
export function buildQueueView(
  tasks: QueueTaskRecord[],
  hotEvents: HotEventRow[],
  opts: QueueViewOptions,
): QueueView {
  const counts: Record<string, number> = {};
  for (const t of tasks) counts[t.status] = (counts[t.status] ?? 0) + 1;

  const byRef = new Map<string, HotEventRow[]>();
  for (const e of hotEvents) {
    if (!e.ref) continue;
    if (!inWindow(e.received_at, opts.since, opts.until)) continue;
    const list = byRef.get(e.ref) ?? [];
    list.push(e);
    byRef.set(e.ref, list);
  }

  const entry = (t: QueueTaskRecord): QueuedEntry => {
    const ref = taskRef(t);
    return {
      task_id: t.id,
      queue: t.queue ?? null,
      title: t.title ?? null,
      priority: t.priority ?? null,
      created_at: t.created_at ?? null,
      ref,
      kind: t.payload?.kind ?? null,
      hot_events: ref ? toViewEvents(byRef.get(ref) ?? []) : [],
    };
  };

  const queued = tasks
    .filter((t) => t.status === "queued")
    .sort(claimOrder)
    .map(entry);

  const claimed = tasks
    .filter((t) => {
      if (t.status !== "claimed") return false;
      const at = claimTimestamp(t);
      // A claimed record with no claim timestamp cannot be proven out
      // of window; its claimed state is a now-fact, so it is listed.
      return at === null || inWindow(at, opts.since, opts.until);
    })
    .sort((a, b) => (claimTimestamp(a) ?? "9999").localeCompare(claimTimestamp(b) ?? "9999") || a.id - b.id)
    .map((t) => ({
      ...entry(t),
      claimed_at: claimTimestamp(t),
      claimed_by: {
        worker_id: t.worker_id ?? null,
        lane_id: t.lane_id ?? null,
        platform: t.platform ?? null,
      },
    }));

  const seqs = hotEvents.map((e) => e.seq);
  return {
    generated_at: opts.now ?? new Date().toISOString(),
    window: { since: opts.since, until: opts.until },
    counts,
    queued,
    claimed,
    hot_list: {
      consumer: opts.consumer ?? null,
      from_seq: opts.fromSeq ?? 0,
      high_water: opts.highWater ?? (seqs.length ? Math.max(...seqs) : 0),
      events_consumed: hotEvents.length,
    },
    links: opts.links ?? defaultTrendLinks(),
  };
}

/**
 * Links to the existing trend stores — references, never copies. Bases
 * are env-overridable at the CLI layer; the defaults are the projector's
 * documented local API (127.0.0.1:7801) and the aggregator's repo paths.
 */
export function defaultTrendLinks(env: Record<string, string | undefined> = {}): TrendLink[] {
  const projector = env.PM_PROJECTOR_URL ?? "http://127.0.0.1:7801";
  const aggregator = env.PM_AGGREGATOR_URL ?? "pm-stack/aggregator";
  return [
    { label: "Hot list (slice-2 event stream)", store: "projector", ref: `${projector}/hot` },
    { label: "Joined items + comment activity", store: "projector", ref: `${projector}/items` },
    { label: "Usage & billing trends", store: "aggregator", ref: `${aggregator}/src/burn-history.ts` },
    { label: "Plan quota snapshots", store: "aggregator", ref: `${aggregator}/scripts/gather-quota.ts` },
    { label: "Scheduled-job activity stream", store: "aggregator", ref: `${aggregator}/src/activity.ts` },
  ];
}

export interface ConsumedHotList {
  events: HotEventRow[];
  fromSeq: number;
  highWater: number;
  /** Ack the consumer checkpoint at the pinned high-water. Call ONLY
   *  after the view built from `events` has been emitted successfully;
   *  without the ack the same events are re-consumed next run. */
  ack: () => Promise<number>;
}

/**
 * Consume the hot list for `consumer` per the slice-2 protocol: read the
 * acked checkpoint, pin the server's current high-water, page ascending
 * strictly after the checkpoint and never past the pin.
 */
export async function consumeHotList(
  db: Client,
  consumer: string,
  opts: { pageSize?: number } = {},
): Promise<ConsumedHotList> {
  const pageSize = opts.pageSize ?? 500;
  const fromSeq = await getConsumerCheckpoint(db, consumer);
  const highWater = await hotHighWater(db);
  const events: HotEventRow[] = [];
  let after = fromSeq;
  for (;;) {
    const page = await listHotEvents(db, { after, limit: pageSize, highWater });
    events.push(...page);
    if (page.length < pageSize) break;
    after = page[page.length - 1].seq;
  }
  return {
    events,
    fromSeq,
    highWater,
    ack: () => ackConsumerCheckpoint(db, consumer, highWater, highWater),
  };
}
