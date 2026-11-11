// Integrated queue/PM view (#7078, #2072 slice 3 first piece): the join
// of fleet queue task records with the slice-2 hot list, claim-order
// sorting, claimed-by attribution, count reconciliation, and consumer-
// checkpoint consumption. Oracles are stated per test: the broker's
// documented claim order, the input record set itself (counts), and the
// slice-2 checkpoint protocol from test/hot.test.ts.
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  migrate,
  appendHotEvents,
  getConsumerCheckpoint,
  type HotEventInput,
} from "../src/store";
import {
  buildQueueView,
  claimOrder,
  consumeHotList,
  taskRef,
  type QueueTaskRecord,
} from "../src/queue-view";
import type { HotEventRow } from "../src/store";

const SINCE = "2026-11-10T00:00:00Z";
const UNTIL = "2026-11-11T00:00:00Z";

const task = (over: Partial<QueueTaskRecord>): QueueTaskRecord => ({
  id: 1,
  queue: "repo-health-code-pilot",
  title: "task",
  status: "queued",
  priority: 30,
  created_at: "2026-11-09T00:00:00Z",
  claimed_at: null,
  worker_id: null,
  lane_id: null,
  platform: null,
  dedupe_key: null,
  payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 100 } },
  ...over,
});

const hotRow = (over: Partial<HotEventRow>): HotEventRow => ({
  seq: 1,
  event_id: "d1:issue:100",
  delivery_id: "d1",
  repo: "developing-today/code",
  event: "issues",
  action: "opened",
  ref: "developing-today/code#100",
  actor: "dezren39",
  source_url: "https://github.com/developing-today/code/issues/100",
  source_timestamp: "2026-11-10T01:00:00Z",
  received_at: "2026-11-10T01:00:00Z",
  ...over,
});

describe("queue-view: refs and claim order", () => {
  test("taskRef prefers payload repo+issue, falls back to the dedupe key", () => {
    expect(taskRef(task({}))).toBe("developing-today/code#100");
    expect(
      taskRef(
        task({
          payload: null,
          dedupe_key: "repo-health:v1:fix:developing-today/code:issue:5497",
        }),
      ),
    ).toBe("developing-today/code#5497");
    expect(taskRef(task({ payload: null, dedupe_key: null }))).toBeNull();
  });

  test("claimOrder is the broker order: non-null priority first, then priority ASC, then created_at ASC", () => {
    const rows = [
      task({ id: 1, priority: null, created_at: "2026-11-01T00:00:00Z" }),
      task({ id: 2, priority: 30, created_at: "2026-11-09T00:00:00Z" }),
      task({ id: 3, priority: 10, created_at: "2026-11-08T00:00:00Z" }),
      task({ id: 4, priority: 30, created_at: "2026-11-08T00:00:00Z" }),
    ];
    expect([...rows].sort(claimOrder).map((t) => t.id)).toEqual([3, 4, 2, 1]);
  });
});

describe("queue-view: buildQueueView", () => {
  const tasks = [
    task({ id: 11, priority: 20, created_at: "2026-11-09T02:00:00Z" }),
    task({ id: 12, priority: 10, created_at: "2026-11-09T01:00:00Z", payload: { repo: "developing-today/code", kind: "triage", issue: { number: 200 } } }),
    task({
      id: 13,
      status: "claimed",
      claimed_at: "2026-11-10T05:00:00Z",
      worker_id: "repo-worker-code",
      lane_id: "code-d",
      payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 300 } },
    }),
    task({
      id: 14,
      status: "claimed",
      claimed_at: "2026-11-01T05:00:00Z", // claimed before the window
      worker_id: "repo-worker-code",
      lane_id: "code-a",
      payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 400 } },
    }),
    task({ id: 15, status: "done", payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 500 } } }),
  ];
  const events = [
    hotRow({ seq: 7, ref: "developing-today/code#100" }),
    hotRow({ seq: 8, ref: "developing-today/code#999", event_id: "d2:issue:999" }), // no task carries this ref
    hotRow({ seq: 9, ref: "developing-today/code#100", received_at: "2026-11-12T01:00:00Z", event_id: "d3:issue:100" }), // outside window
  ];

  test("queued lists only queued tasks, in claim order, with their in-window hot events joined by ref", () => {
    const view = buildQueueView(tasks, events, { since: SINCE, until: UNTIL, now: UNTIL });
    expect(view.queued.map((q) => q.task_id)).toEqual([12, 11]);
    const first = view.queued.find((q) => q.task_id === 11);
    expect(first?.ref).toBe("developing-today/code#100");
    expect(first?.hot_events.map((e) => e.seq)).toEqual([7]); // seq 9 is outside the window
  });

  test("claimed lists only claims inside the window, with claiming system/lane", () => {
    const view = buildQueueView(tasks, events, { since: SINCE, until: UNTIL, now: UNTIL });
    expect(view.claimed.map((c) => c.task_id)).toEqual([13]);
    expect(view.claimed[0].claimed_by).toEqual({
      worker_id: "repo-worker-code",
      lane_id: "code-d",
      platform: null,
    });
  });

  test("claimed falls back to the progress checkpoint timestamp when claimed_at is absent (rich /list rows)", () => {
    const rich = [
      task({
        id: 21,
        status: "claimed",
        claimed_at: null,
        progress: { last_checkpoint: { at: "2026-11-10T06:00:00Z" } },
        worker_id: "repo-worker-code",
        lane_id: "code-l",
        payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 600 } },
      }),
      task({
        id: 22,
        status: "claimed",
        claimed_at: null,
        progress: { last_checkpoint: { at: "2026-11-01T06:00:00Z" } }, // stale: before the window
        worker_id: "repo-worker-code",
        lane_id: "code-a",
        payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 700 } },
      }),
      task({
        id: 23,
        status: "claimed",
        claimed_at: null,
        progress: null, // no timestamp anywhere: listed, claimed_at null
        worker_id: "repo-worker-code",
        lane_id: null,
        payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 800 } },
      }),
    ];
    const view = buildQueueView(rich, [], { since: SINCE, until: UNTIL, now: UNTIL });
    expect(view.claimed.map((c) => c.task_id)).toEqual([21, 23]);
    expect(view.claimed[0].claimed_at).toBe("2026-11-10T06:00:00Z");
    expect(view.claimed[1].claimed_at).toBeNull();
  });

  test("counts reconcile with the input snapshot: every record counted once, by status", () => {
    const view = buildQueueView(tasks, events, { since: SINCE, until: UNTIL, now: UNTIL });
    expect(view.counts).toEqual({ queued: 2, claimed: 2, done: 1 });
    expect(Object.values(view.counts).reduce((a, b) => a + b, 0)).toBe(tasks.length);
  });
});

describe("queue-view: consumeHotList checkpoint protocol", () => {
  let db: Client;
  let dir: string;

  const ev = (over: Partial<HotEventInput>): HotEventInput => ({
    eventId: "e1:issue:100",
    deliveryId: "e1",
    repo: "developing-today/code",
    event: "issues",
    action: "opened",
    ref: "developing-today/code#100",
    actor: "dezren39",
    sourceUrl: "https://github.com/developing-today/code/issues/100",
    sourceTimestamp: "2026-11-10T01:00:00Z",
    ...over,
  });

  beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), "pm-projector-qv-"));
    db = createClient({ url: `file:${join(dir, "test.db")}` });
    await migrate(db);
    await appendHotEvents(db, [
      ev({}),
      ev({ eventId: "e2:issue:200", deliveryId: "e2", ref: "developing-today/code#200" }),
      ev({ eventId: "e3:issue:300", deliveryId: "e3", ref: "developing-today/code#300" }),
    ]);
  });

  afterAll(() => {
    db.close();
    rmSync(dir, { recursive: true, force: true });
  });

  test("consumption without ack re-reads the same events; ack advances the checkpoint monotonically", async () => {
    const first = await consumeHotList(db, "queue-view-test");
    expect(first.fromSeq).toBe(0);
    expect(first.events.map((e) => e.seq)).toEqual([1, 2, 3]);
    expect(first.highWater).toBe(3);
    // No ack yet: the checkpoint has not moved.
    expect(await getConsumerCheckpoint(db, "queue-view-test")).toBe(0);

    const again = await consumeHotList(db, "queue-view-test");
    expect(again.events.map((e) => e.seq)).toEqual([1, 2, 3]);

    expect(await first.ack()).toBe(3);
    const after = await consumeHotList(db, "queue-view-test");
    expect(after.fromSeq).toBe(3);
    expect(after.events).toEqual([]);
  });
});
