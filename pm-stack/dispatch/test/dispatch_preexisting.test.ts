// Issue #2677: pm-dispatch must detect a pre-existing enqueue row.
//
// The enqueue RPC dedupes on dedupe_key against ALL rows incl. terminal
// ones and returns the old row as-is. handleTasks used to report that row
// as the newly queued task (201 + "Status: queued"), silently discarding
// the new dispatch. These tests drive the server-level orchestration
// (enqueueDispatchTask) with a stubbed queue — no network, no Supabase.
import { describe, expect, test } from "bun:test";
import { enqueueDispatchTask, isPreExistingTaskRow } from "../src/server.ts";
import type { TaskRow } from "../src/queue.ts";

const START = new Date("2026-10-31T12:00:00.000Z");
const BEFORE = "2026-10-30T09:00:00.000Z";
const AFTER = "2026-10-31T12:00:01.000Z";
const CANONICAL = "repo-health:v1:fix:developing-today/code:issue:2677";

function row(partial: Partial<TaskRow>): TaskRow {
  return {
    id: 1,
    queue: "repo-health-code-pilot",
    status: "queued",
    dedupe_key: CANONICAL,
    created_at: AFTER,
    ...partial,
  };
}

/** Stubbed queue: canonical key returns `first`, any other key `second`. */
function stubQueue(first: TaskRow, second?: TaskRow) {
  const keys: string[] = [];
  const enqueue = async (key: string): Promise<TaskRow> => {
    keys.push(key);
    if (key === CANONICAL) return first;
    if (second) return second;
    throw new Error(`unexpected enqueue key in test: ${key}`);
  };
  return { enqueue, keys };
}

describe("isPreExistingTaskRow (#2677)", () => {
  test("a row created before the request started is pre-existing", () => {
    expect(isPreExistingTaskRow(row({ created_at: BEFORE }), START)).toBe(true);
  });

  test("a terminal row is pre-existing even with a fresh created_at", () => {
    for (const status of ["done", "cancelled", "failed"]) {
      expect(isPreExistingTaskRow(row({ status, created_at: AFTER }), START)).toBe(true);
    }
  });

  test("a fresh live row is newly queued", () => {
    expect(isPreExistingTaskRow(row({ status: "queued", created_at: AFTER }), START)).toBe(false);
    expect(isPreExistingTaskRow(row({ status: "claimed", created_at: AFTER }), START)).toBe(false);
  });
});

describe("enqueueDispatchTask attach flow (#2677)", () => {
  test("second attach against a TERMINAL canonical-key task gets its own task", async () => {
    const old = row({ id: 111, status: "done", created_at: BEFORE });
    const fresh = row({ id: 222, status: "queued", created_at: AFTER });
    const { enqueue, keys } = stubQueue(old, fresh);

    const out = await enqueueDispatchTask({
      enqueue,
      dedupeKey: CANONICAL,
      attach: true,
      requestStartedAt: START,
    });

    expect(keys.length).toBe(2);
    expect(keys[1]).toStartWith(`${CANONICAL}:dispatch:`);
    expect(out.task.id).toBe(222);
    expect(out.preExisting).toBe(false);
    expect(out.retried).toBe(true);
    expect(out.warnings.join(" ")).toContain("#111");
    expect(out.warnings.join(" ")).toContain('"done"');
  });

  test("second attach against a LIVE canonical-key task gets its own task", async () => {
    const old = row({ id: 111, status: "queued", created_at: BEFORE });
    const fresh = row({ id: 333, status: "queued", created_at: AFTER });
    const { enqueue, keys } = stubQueue(old, fresh);

    const out = await enqueueDispatchTask({
      enqueue,
      dedupeKey: CANONICAL,
      attach: true,
      requestStartedAt: START,
    });

    expect(keys.length).toBe(2);
    expect(keys[1]).toStartWith(`${CANONICAL}:dispatch:`);
    expect(out.task.id).toBe(333);
    expect(out.preExisting).toBe(false);
    expect(out.retried).toBe(true);
    expect(out.warnings.join(" ")).toContain("#111");
  });

  test("attach retry that also returns pre-existing warns explicitly, never silent-new", async () => {
    const old = row({ id: 111, status: "done", created_at: BEFORE });
    const alsoOld = row({ id: 111, status: "done", created_at: BEFORE });
    const { enqueue, keys } = stubQueue(old, alsoOld);

    const out = await enqueueDispatchTask({
      enqueue,
      dedupeKey: CANONICAL,
      attach: true,
      requestStartedAt: START,
    });

    expect(keys.length).toBe(2);
    expect(out.preExisting).toBe(true);
    expect(out.retried).toBe(true);
    expect(out.warnings.join(" ")).toContain("no new task was queued");
  });
});

describe("enqueueDispatchTask non-attach flows (#2677)", () => {
  test("caller-supplied dedupe_key repeat returns the existing task WITH a warning naming its status", async () => {
    const old = row({ id: 444, status: "claimed", created_at: BEFORE });
    const { enqueue, keys } = stubQueue(old);

    const out = await enqueueDispatchTask({
      enqueue,
      dedupeKey: CANONICAL,
      attach: false,
      requestStartedAt: START,
    });

    expect(keys.length).toBe(1); // no retry for non-attach repeats
    expect(out.task.id).toBe(444);
    expect(out.preExisting).toBe(true);
    expect(out.retried).toBe(false);
    expect(out.warnings.join(" ")).toContain('"claimed"');
    expect(out.warnings.join(" ")).toContain("no new task was queued");
  });

  test("a fresh enqueue passes through untouched", async () => {
    const fresh = row({ id: 555, status: "queued", created_at: AFTER });
    const { enqueue, keys } = stubQueue(fresh);

    const out = await enqueueDispatchTask({
      enqueue,
      dedupeKey: CANONICAL,
      attach: true,
      requestStartedAt: START,
    });

    expect(keys.length).toBe(1);
    expect(out.task.id).toBe(555);
    expect(out.preExisting).toBe(false);
    expect(out.retried).toBe(false);
    expect(out.warnings).toEqual([]);
  });
});
