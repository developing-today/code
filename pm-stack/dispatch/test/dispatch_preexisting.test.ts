// Issue #2677: pm-dispatch must detect a pre-existing enqueue row.
// Issue #4661: that detection must NOT compare the DB clock to the
// dispatch-host clock — classification is by pre-enqueue dedupe-key
// lookup compared by row id (plus the clock-independent terminal-status
// check), so skew in either direction cannot misclassify.
//
// The enqueue RPC dedupes on dedupe_key against ALL rows incl. terminal
// ones and returns the old row as-is. These tests drive the server-level
// orchestration (enqueueDispatchTask) with a stubbed queue — no network,
// no Supabase.
import { describe, expect, test } from "bun:test";
import { enqueueDispatchTask, isPreExistingTaskRow } from "../src/server.ts";
import type { TaskRow } from "../src/queue.ts";

// BEFORE/AFTER stand in for the two skew directions: a DB-behind fresh
// row carries an old created_at, a DB-ahead pre-existing row a new one.
// Classification must ignore both — only row ids (and terminal status)
// decide.
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

/** Stubbed dispatch queue: findExisting answers from `existing` by key,
 * enqueue answers `first` for the canonical key and `second` otherwise. */
function stubDispatch(opts: { existing?: Record<string, TaskRow | null>; first: TaskRow; second?: TaskRow }) {
  const enqueueKeys: string[] = [];
  const lookupKeys: string[] = [];
  const existing = opts.existing ?? {};
  const enqueue = async (key: string): Promise<TaskRow> => {
    enqueueKeys.push(key);
    if (key === CANONICAL) return opts.first;
    if (opts.second) return opts.second;
    throw new Error(`unexpected enqueue key in test: ${key}`);
  };
  const findExisting = async (key: string): Promise<TaskRow | null> => {
    lookupKeys.push(key);
    return existing[key] ?? null;
  };
  return { enqueue, findExisting, enqueueKeys, lookupKeys };
}

/** Attach-flow args with a findExisting backed by `existing`. */
function attachArgs(
  stub: ReturnType<typeof stubDispatch>,
  dedupeKey = CANONICAL,
  extra: Partial<Parameters<typeof enqueueDispatchTask>[0]> = {},
) {
  return {
    enqueue: stub.enqueue,
    findExisting: stub.findExisting,
    dedupeKey,
    attach: true,
    callerSuppliedKey: false,
    ...extra,
  };
}

describe("isPreExistingTaskRow (#2677, clock-independent per #4661)", () => {
  test("the looked-up row returned as-is is pre-existing, whatever its created_at", () => {
    const live = row({ id: 111, status: "queued" });
    // DB-behind skew on a genuinely pre-existing row: old created_at.
    expect(isPreExistingTaskRow(row({ id: 111, status: "queued", created_at: BEFORE }), live)).toBe(true);
    // DB-ahead skew on a genuinely pre-existing row: new created_at.
    expect(isPreExistingTaskRow(row({ id: 111, status: "queued", created_at: AFTER }), live)).toBe(true);
  });

  test("a terminal row is pre-existing even with no looked-up row and a fresh created_at", () => {
    for (const status of ["done", "cancelled", "failed"]) {
      expect(isPreExistingTaskRow(row({ id: 999, status, created_at: AFTER }), null)).toBe(true);
    }
  });

  test("a row with an id the lookup never saw is newly queued, whatever its created_at", () => {
    // DB-behind skew on a fresh row: old created_at must NOT read pre-existing.
    expect(isPreExistingTaskRow(row({ id: 222, status: "queued", created_at: BEFORE }), null)).toBe(false);
    expect(isPreExistingTaskRow(row({ id: 222, status: "claimed", created_at: BEFORE }), null)).toBe(false);
    // A different id than the looked-up one is a distinct (fresh) task.
    const other = row({ id: 111, status: "queued" });
    expect(isPreExistingTaskRow(row({ id: 222, status: "queued", created_at: AFTER }), other)).toBe(false);
  });

  test("created_at is never consulted: skewed timestamps alone decide nothing", () => {
    // Fresh live row with an old stamp, no prior row: new.
    expect(isPreExistingTaskRow(row({ id: 5, status: "queued", created_at: BEFORE }), null)).toBe(false);
    // Live row with a new stamp matching the looked-up id: pre-existing.
    const known = row({ id: 5, status: "queued", created_at: BEFORE });
    expect(isPreExistingTaskRow(row({ id: 5, status: "queued", created_at: AFTER }), known)).toBe(true);
  });
});

describe("enqueueDispatchTask attach flow (#2677)", () => {
  test("second attach against a TERMINAL canonical-key task gets its own task", async () => {
    const old = row({ id: 111, status: "done", created_at: BEFORE });
    const fresh = row({ id: 222, status: "queued", created_at: AFTER });
    const stub = stubDispatch({ existing: { [CANONICAL]: old }, first: old, second: fresh });

    const out = await enqueueDispatchTask(attachArgs(stub));

    expect(stub.enqueueKeys.length).toBe(2);
    expect(stub.enqueueKeys[1]).toStartWith(`${CANONICAL}:dispatch:`);
    expect(out.task.id).toBe(222);
    expect(out.preExisting).toBe(false);
    expect(out.retried).toBe(true);
    expect(out.warnings.join(" ")).toContain("#111");
    expect(out.warnings.join(" ")).toContain('"done"');
  });

  test("second attach against a LIVE canonical-key task gets its own task", async () => {
    const old = row({ id: 111, status: "queued", created_at: BEFORE });
    const fresh = row({ id: 333, status: "queued", created_at: AFTER });
    const stub = stubDispatch({ existing: { [CANONICAL]: old }, first: old, second: fresh });

    const out = await enqueueDispatchTask(attachArgs(stub));

    expect(stub.enqueueKeys.length).toBe(2);
    expect(stub.enqueueKeys[1]).toStartWith(`${CANONICAL}:dispatch:`);
    expect(out.task.id).toBe(333);
    expect(out.preExisting).toBe(false);
    expect(out.retried).toBe(true);
    expect(out.warnings.join(" ")).toContain("#111");
  });

  test("attach retry that also returns pre-existing warns explicitly, never silent-new", async () => {
    const old = row({ id: 111, status: "done", created_at: BEFORE });
    const alsoOld = row({ id: 111, status: "done", created_at: BEFORE });
    const stub = stubDispatch({ existing: { [CANONICAL]: old }, first: old, second: alsoOld });

    const out = await enqueueDispatchTask(attachArgs(stub));

    expect(stub.enqueueKeys.length).toBe(2);
    expect(out.preExisting).toBe(true);
    expect(out.retried).toBe(true);
    expect(out.warnings.join(" ")).toContain("no new task was queued");
  });
});

describe("enqueueDispatchTask clock-skew directions (#4661)", () => {
  test("DB-behind fresh row (created_at long before the request) is newly queued with no :dispatch: retry", async () => {
    // No row consumed the canonical key before this request; the DB
    // clock is behind the dispatch host, so the fresh row's stamp
    // predates the request — the old timestamp comparison read this as
    // pre-existing and queued a spurious duplicate. Row-id comparison
    // reports it newly queued with a single enqueue.
    const fresh = row({ id: 222, status: "queued", created_at: BEFORE });
    const stub = stubDispatch({ existing: {}, first: fresh, second: fresh });

    const out = await enqueueDispatchTask(attachArgs(stub));

    expect(stub.lookupKeys).toContain(CANONICAL);
    expect(stub.enqueueKeys.length).toBe(1);
    expect(out.task.id).toBe(222);
    expect(out.preExisting).toBe(false);
    expect(out.retried).toBe(false);
    expect(out.warnings).toEqual([]);
  });

  test("DB-ahead live pre-existing row (created_at after the request) is still detected, with retry", async () => {
    // The canonical key was already consumed by live task #111; the DB
    // clock runs ahead, so the old row's stamp postdates the request —
    // the old timestamp comparison read this as fresh (the #2677
    // silent-vanish in miniature). Row-id comparison still detects it
    // and the attach flow gets its own task via the retry key.
    const old = row({ id: 111, status: "queued", created_at: AFTER });
    const fresh = row({ id: 333, status: "queued", created_at: AFTER });
    const stub = stubDispatch({ existing: { [CANONICAL]: old }, first: old, second: fresh });

    const out = await enqueueDispatchTask(attachArgs(stub));

    expect(stub.enqueueKeys.length).toBe(2);
    expect(stub.enqueueKeys[1]).toStartWith(`${CANONICAL}:dispatch:`);
    expect(out.task.id).toBe(333);
    expect(out.preExisting).toBe(false);
    expect(out.retried).toBe(true);
    expect(out.warnings.join(" ")).toContain("#111");
  });
});

describe("enqueueDispatchTask non-attach flows (#2677)", () => {
  test("caller-supplied dedupe_key repeat returns the existing task WITH a warning naming its status", async () => {
    const old = row({ id: 444, status: "claimed", created_at: BEFORE });
    const stub = stubDispatch({ existing: { [CANONICAL]: old }, first: old });

    const out = await enqueueDispatchTask({
      enqueue: stub.enqueue,
      findExisting: stub.findExisting,
      dedupeKey: CANONICAL,
      attach: false,
      callerSuppliedKey: true,
    });

    expect(stub.enqueueKeys.length).toBe(1); // no retry for non-attach repeats
    expect(out.task.id).toBe(444);
    expect(out.preExisting).toBe(true);
    expect(out.retried).toBe(false);
    expect(out.warnings.join(" ")).toContain('"claimed"');
    expect(out.warnings.join(" ")).toContain("no new task was queued");
  });

  test("a fresh enqueue passes through untouched", async () => {
    const fresh = row({ id: 555, status: "queued", created_at: AFTER });
    const stub = stubDispatch({ existing: {}, first: fresh });

    const out = await enqueueDispatchTask(attachArgs(stub));

    expect(stub.enqueueKeys.length).toBe(1);
    expect(out.task.id).toBe(555);
    expect(out.preExisting).toBe(false);
    expect(out.retried).toBe(false);
    expect(out.warnings).toEqual([]);
  });
});

describe("enqueueDispatchTask caller-supplied key on an attach request (#2677, Codex P2)", () => {
  test("issue_number + explicit dedupe_key repeat returns the existing task — no dispatch-scoped retry", async () => {
    const old = row({ id: 777, status: "queued", created_at: BEFORE });
    const stub = stubDispatch({ existing: { [CANONICAL]: old }, first: old });

    const out = await enqueueDispatchTask({
      enqueue: stub.enqueue,
      findExisting: stub.findExisting,
      dedupeKey: CANONICAL,
      attach: true,
      callerSuppliedKey: true,
    });

    expect(stub.enqueueKeys.length).toBe(1); // the caller chose the key: a repeat is an intentional dedupe
    expect(out.task.id).toBe(777);
    expect(out.preExisting).toBe(true);
    expect(out.retried).toBe(false);
    expect(out.warnings.join(" ")).toContain("no new task was queued");
  });
});
