// Issue #12806 (component F slice 4 of #9084): the GC mark-and-sweep
// worker. Proven against real ReferenceStores and the real derived
// index on in-memory SQLite (node:sqlite), the harness shape of
// test/reference-index.test.ts, with a recording fake for the deletion
// side of the object-plane adapter.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { ReferenceStore, type Partition } from "../src/references.ts";
import { ReferenceIndex } from "../src/reference-index.ts";
import { DEFAULT_GRACE_MS, GcSweeper, type GcObjectPlane } from "../src/gc-sweep.ts";
import type { DeleteFence } from "../src/object-plane.ts";
import { FailClosed } from "../src/routing.ts";

const WRITER = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_WRITER = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C1 = "ab".repeat(32);
const C2 = "cd".repeat(32);
const T0 = "2026-11-25T00:00:00.000Z";
const DAY = 24 * 60 * 60 * 1000;
const at = (ms: number) => new Date(Date.parse(T0) + ms).toISOString();

const US_PRIVATE: Partition = { jurisdiction: "us", tier: "tenant-private" };
const EU_PRIVATE: Partition = { jurisdiction: "eu", tier: "tenant-private" };
const US_RESTRICTED: Partition = { jurisdiction: "us", tier: "tenant-restricted" };

function fakeSql() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(query: string, ...params: unknown[]) {
      const stmt = db.prepare(query);
      if (/^\s*select/i.test(query)) {
        const rows = stmt.all(...(params as never[])) as Record<string, unknown>[];
        return { toArray: () => rows };
      }
      stmt.run(...(params as never[]));
      return { toArray: () => [] as Record<string, unknown>[] };
    },
  };
}

function makeStore(dbId: string, partition: Partition = US_PRIVATE) {
  let n = 0;
  const store = new ReferenceStore(fakeSql(), {
    dbId,
    partition,
    newId: () => `${dbId.slice(0, 4)}${(++n).toString(16).padStart(28, "0")}`,
  });
  store.ensureSchema();
  return store;
}

class FakePlane implements GcObjectPlane {
  readonly partition: Partition;
  readonly present = new Set<string>();
  readonly deletes: { contentId: string; fence: DeleteFence }[] = [];
  /** Runs inside deleteReplica, before it returns: a stand-in for a
   *  writer racing the delete. */
  during: (() => void) | null = null;
  constructor(partition: Partition) {
    this.partition = partition;
  }
  async deleteReplica(contentId: string, fence: DeleteFence): Promise<{ deleted: boolean }> {
    this.deletes.push({ contentId, fence });
    this.during?.();
    const existed = this.present.delete(contentId);
    return { deleted: existed };
  }
}

function setup(partition: Partition = US_PRIVATE, policy?: { graceMs?: number; soleReferencerErasure?: boolean }) {
  const index = new ReferenceIndex(fakeSql(), { partition });
  index.ensureSchema();
  const plane = new FakePlane(partition);
  const gc = new GcSweeper(fakeSql(), {
    partition,
    index,
    adapter: plane,
    locator: `r2/${partition.jurisdiction}`,
    policy,
  });
  gc.ensureSchema();
  return { index, plane, gc };
}

/** Publish content, reference it, then drop the reference: the index
 *  sees it as unreferenced with a fresh location. */
function orphan(store: ReferenceStore, index: ReferenceIndex, plane: FakePlane, contentId: string) {
  plane.present.add(contentId);
  index.notePublished(contentId, `r2/${plane.partition.jurisdiction}`, { now: T0 });
  const { row } = store.createReference({ contentId, refClass: "standard", createdBy: WRITER, now: T0 });
  store.removeReference(row.reference_id, { actor: WRITER, now: T0 });
  index.consume(store, { now: T0 });
}

function throwsCode(fn: () => unknown, code: string) {
  assert.throws(fn, (e: unknown) => e instanceof FailClosed && e.code === code);
}

async function rejectsCode(p: Promise<unknown>, code: string) {
  await assert.rejects(p, (e: unknown) => e instanceof FailClosed && e.code === code);
}

test("mark opens an epoch and puts unreferenced content in candidate state with the tier grace", () => {
  const store = makeStore("tenant-a");
  const { index, plane, gc } = setup();
  orphan(store, index, plane, C1);
  const live = store.createReference({ contentId: C2, refClass: "standard", createdBy: WRITER, now: T0 });
  assert.ok(live.created);
  index.consume(store, { now: T0 });

  const res = gc.mark({ now: T0 });
  assert.equal(res.epoch, "us/tenant-private/1");
  assert.deepEqual(res.marked, [C1]);
  const c = gc.candidate(C1);
  assert.equal(c?.state, "candidate");
  assert.equal(c?.graceUntil, at(DEFAULT_GRACE_MS["tenant-private"]));
  assert.equal(gc.candidate(C2), null);

  // A second mark does not re-stamp a standing candidate.
  const again = gc.mark({ now: at(DAY) });
  assert.equal(again.epoch, "us/tenant-private/2");
  assert.deepEqual(again.marked, []);
  assert.equal(gc.candidate(C1)?.epoch, "us/tenant-private/1");
});

test("a new reference clears the candidate, through the next mark or noteReferenced", () => {
  const store = makeStore("tenant-a");
  const { index, plane, gc } = setup();
  orphan(store, index, plane, C1);
  orphan(store, index, plane, C2);
  gc.mark({ now: T0 });

  store.createReference({ contentId: C1, refClass: "standard", createdBy: OTHER_WRITER, now: at(DAY) });
  index.consume(store, { now: at(DAY) });
  const res = gc.mark({ now: at(DAY) });
  assert.deepEqual(res.cleared, [C1]);
  assert.equal(gc.candidate(C1)?.state, "live");

  assert.equal(gc.noteReferenced(C2, { now: at(DAY) }), true);
  assert.equal(gc.candidate(C2)?.state, "live");
  assert.equal(gc.noteReferenced(C2), false, "idempotent once live");
});

test("re-verify against authoritative rows rescues a candidate the stale index still lists", async () => {
  const store = makeStore("tenant-a");
  const { index, plane, gc } = setup();
  orphan(store, index, plane, C1);
  gc.mark({ now: T0 });

  // A reference lands in the tenant DB, but the index has not consumed
  // it: the index still calls C1 unreferenced.
  store.createReference({ contentId: C1, refClass: "standard", createdBy: OTHER_WRITER, now: at(DAY) });
  assert.deepEqual(index.unreferencedContent(), [C1]);

  const res = await gc.sweep([store], { now: at(8 * DAY) });
  assert.deepEqual(res.rescued, [C1]);
  assert.deepEqual(res.deleted, []);
  assert.equal(plane.deletes.length, 0);
  assert.equal(gc.candidate(C1)?.state, "live");
  assert.ok(plane.present.has(C1));
});

test("past grace with no live reference, the delete runs under the epoch fence and is audited", async () => {
  const a = makeStore("tenant-a");
  const b = makeStore("tenant-b");
  const { index, plane, gc } = setup();
  orphan(a, index, plane, C1);
  gc.mark({ now: T0 });

  const early = await gc.sweep([a, b], { now: at(6 * DAY) });
  assert.deepEqual(early.waiting, [C1]);
  assert.equal(plane.deletes.length, 0, "nothing is deleted inside grace");

  const sweptAt = at(7 * DAY);
  const res = await gc.sweep([a, b], { now: sweptAt });
  assert.deepEqual(res.deleted, [C1]);
  assert.deepEqual(plane.deletes, [{ contentId: C1, fence: { epoch: "us/tenant-private/1", reverifiedAt: sweptAt } }]);
  assert.ok(!plane.present.has(C1));

  const audit = gc.auditLog();
  assert.equal(audit.length, 1);
  assert.deepEqual(
    { ...audit[0], auditId: 0 },
    {
      auditId: 0,
      contentId: C1,
      partition: US_PRIVATE,
      epoch: "us/tenant-private/1",
      reverifiedAt: sweptAt,
      deletedAt: sweptAt,
      existed: true,
      fenceSeq: audit[0].fenceSeq,
      reason: "grace",
      dbsReverified: ["tenant-a", "tenant-b"],
    },
  );
  assert.equal(gc.candidate(C1)?.state, "deleted");
  const loc = index.lookup(C1).locations;
  assert.equal(loc.length, 1);
  assert.equal(loc[0].state, "deleted");

  // Deleted content is not marked again until it is published again.
  assert.deepEqual(gc.mark({ now: at(8 * DAY) }).marked, []);
  index.notePublished(C1, "r2/us", { now: at(9 * DAY) });
  assert.deepEqual(gc.mark({ now: at(9 * DAY) }).marked, [C1]);
});

test("a reference admitted before the fence clears the candidate; one after it is told to re-stage", async () => {
  const store = makeStore("tenant-a");
  const { index, plane, gc } = setup();
  orphan(store, index, plane, C1);
  orphan(store, index, plane, C2);
  gc.mark({ now: T0 });

  const before = gc.admitReference(C1, { now: at(7 * DAY) });
  assert.equal(before.action, "proceed");
  assert.equal(gc.candidate(C1)?.state, "live");

  // A writer races the delete of C2: its admission arrives while the
  // adapter call is in flight, after the fence was written.
  const racing: { action: string; seq: number }[] = [];
  plane.during = () => {
    racing.push(gc.admitReference(C2, { now: at(7 * DAY) }));
  };
  const res = await gc.sweep([store], { now: at(7 * DAY) });
  assert.deepEqual(res.deleted, [C2]);
  assert.deepEqual(res.rescued, [], "C1 left candidate state at admission, so the sweep never considers it");
  assert.equal(plane.deletes.length, 1);
  assert.equal(racing.length, 1);
  assert.equal(racing[0].action, "restage");
  assert.ok(racing[0].seq > before.seq);
  assert.ok(racing[0].seq > gc.auditLog()[0].fenceSeq, "the admission is ordered after the fence");
  assert.equal(gc.admitReference(C2).action, "restage");
});

test("a sweep never sees or deletes another partition's replicas", async () => {
  const usStore = makeStore("tenant-us");
  const euStore = makeStore("tenant-eu", EU_PRIVATE);
  const us = setup(US_PRIVATE);
  const eu = setup(EU_PRIVATE);
  orphan(usStore, us.index, us.plane, C1);
  orphan(euStore, eu.index, eu.plane, C1);
  us.gc.mark({ now: T0 });

  await rejectsCode(us.gc.sweep([usStore, euStore], { now: at(8 * DAY) }), "gc_store_partition_mismatch");
  assert.equal(us.plane.deletes.length, 0, "a refused sweep deletes nothing");

  await us.gc.sweep([usStore], { now: at(8 * DAY) });
  assert.equal(us.plane.deletes.length, 1);
  assert.equal(eu.plane.deletes.length, 0);
  assert.ok(eu.plane.present.has(C1), "the EU replica of the same content is untouched");
  assert.equal(eu.gc.candidate(C1), null);

  const index = new ReferenceIndex(fakeSql(), { partition: EU_PRIVATE });
  throwsCode(
    () =>
      new GcSweeper(fakeSql(), { partition: US_PRIVATE, index, adapter: new FakePlane(US_PRIVATE), locator: "r2/us" }),
    "gc_index_partition_mismatch",
  );
  throwsCode(
    () =>
      new GcSweeper(fakeSql(), {
        partition: US_PRIVATE,
        index: us.index,
        adapter: new FakePlane(EU_PRIVATE),
        locator: "r2/us",
      }),
    "gc_adapter_partition_mismatch",
  );
  const staging: Partition = { jurisdiction: "us", tier: "staging" };
  throwsCode(
    () =>
      new GcSweeper(fakeSql(), {
        partition: staging,
        index: new ReferenceIndex(fakeSql(), { partition: staging }),
        adapter: new FakePlane(staging),
        locator: "r2/us",
      }),
    "gc_staging_partition",
  );
  await rejectsCode(us.gc.sweep([], { now: T0 }), "gc_no_authoritative_dbs");
});

test("public content uses the 24-hour grace", async () => {
  const PUBLIC: Partition = { jurisdiction: "us", tier: "public" };
  const store = makeStore("public-db", PUBLIC);
  const { index, plane, gc } = setup(PUBLIC);
  orphan(store, index, plane, C1);
  gc.mark({ now: T0 });
  assert.equal(gc.candidate(C1)?.graceUntil, at(DAY));
  assert.deepEqual((await gc.sweep([store], { now: at(DAY - 1) })).waiting, [C1]);
  assert.deepEqual((await gc.sweep([store], { now: at(DAY) })).deleted, [C1]);
});

test("sole-referencer erasure in tenant-restricted deletes at zero grace under the fence", async () => {
  const store = makeStore("tenant-r", US_RESTRICTED);
  const { index, plane, gc } = setup(US_RESTRICTED, { soleReferencerErasure: true });
  plane.present.add(C1);
  index.notePublished(C1, "r2/us", { now: T0 });
  const { row } = store.createReference({
    contentId: C1,
    refClass: "restricted",
    createdBy: WRITER,
    canMintRestricted: true,
    now: T0,
  });

  await rejectsCode(gc.requestErasure(C1, WRITER, [store], { now: T0 }), "erasure_reference_still_live");
  await rejectsCode(gc.requestErasure(C1, OTHER_WRITER, [store], { now: T0 }), "erasure_not_a_referencer");

  store.removeReference(row.reference_id, { actor: WRITER, now: at(1000) });
  const res = await gc.requestErasure(C1, WRITER, [store], { now: at(2000) });
  assert.equal(res.outcome, "deleted");
  assert.equal(res.audit?.reason, "erasure");
  assert.equal(res.audit?.existed, true);
  assert.equal(plane.deletes.length, 1);
  assert.equal(plane.deletes[0].fence.reverifiedAt, at(2000));
  assert.equal(gc.candidate(C1)?.state, "deleted");
  assert.equal(gc.admitReference(C1).action, "restage");
});

test("erasure of shared bytes deletes nothing and records an erasure note", async () => {
  const a = makeStore("tenant-a", US_RESTRICTED);
  const b = makeStore("tenant-b", US_RESTRICTED);
  const { plane, gc } = setup(US_RESTRICTED, { soleReferencerErasure: true });
  plane.present.add(C1);
  const mine = a.createReference({
    contentId: C1,
    refClass: "restricted",
    createdBy: WRITER,
    canMintRestricted: true,
    now: T0,
  });
  b.createReference({
    contentId: C1,
    refClass: "restricted",
    createdBy: OTHER_WRITER,
    canMintRestricted: true,
    now: T0,
  });
  a.removeReference(mine.row.reference_id, { actor: WRITER, now: at(1000) });

  const res = await gc.requestErasure(C1, WRITER, [a, b], { now: at(2000) });
  assert.deepEqual(res, { erased: false, outcome: "shared", audit: null });
  assert.equal(plane.deletes.length, 0);
  assert.deepEqual(gc.erasureNotes(), [
    { contentId: C1, requester: WRITER, notedAt: at(2000), otherLiveReferences: 1 },
  ]);
});

test("erasure is refused outside tenant-restricted and when the policy does not allow it", async () => {
  const store = makeStore("tenant-a");
  const priv = setup(US_PRIVATE, { soleReferencerErasure: true });
  await rejectsCode(priv.gc.requestErasure(C1, WRITER, [store], { now: T0 }), "erasure_wrong_tier");
  const restricted = makeStore("tenant-r", US_RESTRICTED);
  const off = setup(US_RESTRICTED);
  await rejectsCode(off.gc.requestErasure(C1, WRITER, [restricted], { now: T0 }), "erasure_not_enabled");
});
