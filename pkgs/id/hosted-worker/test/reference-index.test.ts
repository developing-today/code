// Issue #12805 (component F slice 3 of #9084): the derived
// per-partition reference/location index, built from the
// ReferenceStore outbox (#12803), and its reconciliation job with
// drift measurement. Proven against real ReferenceStores and the
// index on in-memory SQLite (node:sqlite), the harness shape of
// test/references.test.ts.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { ReferenceStore, type Partition } from "../src/references.ts";
import { ReferenceIndex, parseLocator } from "../src/reference-index.ts";
import { FailClosed } from "../src/routing.ts";

const WRITER = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_WRITER = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C1 = "ab".repeat(32);
const C2 = "cd".repeat(32);
const C3 = "ef".repeat(32);
const T0 = "2026-11-25T00:00:00.000Z";
const T1 = "2026-11-25T01:00:00.000Z";
const T2 = "2026-11-25T02:00:00.000Z";

const US_PRIVATE: Partition = { jurisdiction: "us", tier: "tenant-private" };
const EU_PRIVATE: Partition = { jurisdiction: "eu", tier: "tenant-private" };

function fakeSql() {
  const db = new DatabaseSync(":memory:");
  return {
    db,
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
  const sql = fakeSql();
  const store = new ReferenceStore(sql, {
    dbId,
    partition,
    newId: () => `${dbId.slice(0, 4)}${(++n).toString(16).padStart(28, "0")}`,
  });
  store.ensureSchema();
  return { store, sql };
}

function makeIndex(partition: Partition = US_PRIVATE) {
  const sql = fakeSql();
  const index = new ReferenceIndex(sql, { partition });
  index.ensureSchema();
  return { index, sql };
}

function expectFail(fn: () => unknown, code: string) {
  assert.throws(fn, (e: unknown) => e instanceof FailClosed && e.code === code);
}

test("consume builds content -> referencing databases from the outbox", () => {
  const a = makeStore("tenant-a").store;
  const b = makeStore("tenant-b").store;
  const { index } = makeIndex();
  a.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 });
  a.createReference({ contentId: C2, refClass: "standard", createdBy: WRITER, now: T0 });
  b.createReference({ contentId: C1, refClass: "standard", createdBy: OTHER_WRITER, now: T0 });

  assert.deepEqual(index.consume(a, { now: T1 }), { dbId: "tenant-a", read: 2, applied: 2, replayed: 0, cursor: 2 });
  assert.equal(index.consume(b, { now: T1 }).applied, 1);

  const l1 = index.lookup(C1);
  assert.deepEqual(l1.referencingDbs, ["tenant-a", "tenant-b"]);
  assert.equal(l1.references.length, 2);
  assert.deepEqual(index.lookup(C2).referencingDbs, ["tenant-a"]);
  assert.deepEqual(index.lookup(C3).referencingDbs, []);
  assert.deepEqual(index.unreferencedContent(), []);
});

test("removal events drop a database from the set; content with no live reference becomes a candidate", () => {
  const a = makeStore("tenant-a").store;
  const { index } = makeIndex();
  const { row } = a.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 });
  index.consume(a, { now: T1 });
  a.removeReference(row.reference_id, { actor: WRITER, now: T1 });
  const r = index.consume(a, { now: T2 });
  assert.equal(r.read, 1);
  assert.equal(r.cursor, 2);
  assert.deepEqual(index.lookup(C1).referencingDbs, []);
  assert.deepEqual(index.unreferencedContent(), [C1]);
});

test("consume is incremental and resumes from its cursor across small batches", () => {
  const a = makeStore("tenant-a").store;
  const { index } = makeIndex();
  for (const c of [C1, C2, C3]) a.createReference({ contentId: c, refClass: "standard", createdBy: WRITER, now: T0 });
  const first = index.consume(a, { batch: 2, now: T1 });
  assert.equal(first.read, 3);
  assert.equal(index.cursor("tenant-a"), 3);
  assert.equal(index.consume(a, { now: T1 }).read, 0);
  expectFail(() => index.consume(a, { batch: 0 }), "invalid_batch");
});

test("a replayed outbox prefix converges: an old create never resurrects a removed reference", () => {
  const a = makeStore("tenant-a").store;
  const { index, sql: ixSql } = makeIndex();
  const { row } = a.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 });
  a.removeReference(row.reference_id, { actor: WRITER, now: T1 });
  index.consume(a, { now: T1 });
  assert.deepEqual(index.lookup(C1).referencingDbs, []);
  // Simulate a crash before the cursor write: rewind the cursor and
  // replay both events.
  ixSql.exec("UPDATE reference_index_cursor SET last_seq = 0 WHERE db_id = ?", "tenant-a");
  const replay = index.consume(a, { now: T2 });
  assert.equal(replay.read, 2);
  assert.equal(replay.applied, 0);
  assert.equal(replay.replayed, 2);
  assert.deepEqual(index.lookup(C1).referencingDbs, []);
});

test("rebuild from the outboxes equals incremental folding (#9115 projection property)", () => {
  const a = makeStore("tenant-a").store;
  const b = makeStore("tenant-b").store;
  const incremental = makeIndex().index;
  const r1 = a.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 }).row;
  incremental.consume(a, { now: T1 });
  b.createReference({ contentId: C2, refClass: "restricted", createdBy: WRITER, canMintRestricted: true, now: T0 });
  incremental.consume(b, { now: T1 });
  a.removeReference(r1.reference_id, { actor: WRITER, now: T1 });
  a.createReference({ contentId: C3, refClass: "standard", createdBy: WRITER, now: T1 });
  incremental.consume(a, { now: T2 });

  const rebuilt = makeIndex().index;
  rebuilt.consume(a, { now: T0 });
  rebuilt.rebuild([a, b], { now: T2 });
  for (const c of [C1, C2, C3]) {
    assert.deepEqual(rebuilt.lookup(c).references, incremental.lookup(c).references, c);
  }
  assert.deepEqual(rebuilt.unreferencedContent(), incremental.unreferencedContent());
  assert.equal(rebuilt.cursor("tenant-a"), incremental.cursor("tenant-a"));
});

test("the index refuses a database or audit from another partition", () => {
  const eu = makeStore("tenant-eu", EU_PRIVATE).store;
  const { index } = makeIndex(US_PRIVATE);
  eu.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 });
  expectFail(() => index.consume(eu), "index_partition_mismatch");
  expectFail(() => index.rebuild([eu]), "index_partition_mismatch");
  expectFail(
    () =>
      index.noteDeleted(
        { contentId: C1, partition: EU_PRIVATE, epoch: "e1", reverifiedAt: T1, deletedAt: T2, existed: true },
        "r2/acct/bucket/key",
      ),
    "index_audit_partition_mismatch",
  );
});

test("locations: publish marks fresh, a GC delete audit marks deleted with its epoch", () => {
  const { index } = makeIndex();
  index.notePublished(C1, "r2/acct/media-us/ab", { now: T0 });
  index.notePublished(C1, "laptop/amd/srv/media/ab", { now: T0 });
  assert.deepEqual(index.unreferencedContent(), [C1], "a published object with no reference is a candidate");
  index.noteDeleted(
    { contentId: C1, partition: US_PRIVATE, epoch: "e7", reverifiedAt: T1, deletedAt: T2, existed: true },
    "r2/acct/media-us/ab",
  );
  const locs = index.lookup(C1).locations;
  assert.deepEqual(
    locs.map((l) => [l.locator, l.state]),
    [
      ["laptop/amd/srv/media/ab", "fresh"],
      ["r2/acct/media-us/ab", "deleted"],
    ],
  );
  assert.match(locs[1]!.provenance, /gc epoch e7/);
  assert.equal(locs[1]!.updatedAt, T2);
});

test("locators must be tagged r2/, laptop/ or drive/ locations", () => {
  assert.equal(parseLocator("drive/me/file123/https://x"), "drive/me/file123/https://x");
  for (const bad of ["", "r2", "r2/", "s3/acct/b/k", "/r2/a", "r2/a b/c"]) {
    expectFail(() => parseLocator(bad), "invalid_locator");
  }
  const { index } = makeIndex();
  expectFail(() => index.notePublished("not-a-hash", "r2/a/b/c"), "invalid_content_id");
});

test("reconcile on a consistent index reports zero drift and records the run", () => {
  const a = makeStore("tenant-a").store;
  const { index } = makeIndex();
  a.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 });
  index.consume(a, { now: T1 });
  const report = index.reconcile([a], { now: T2 });
  assert.equal(report.authoritative, 1);
  assert.equal(report.indexed, 1);
  assert.deepEqual([report.missing, report.stale, report.mismatched], [[], [], []]);
  assert.equal(report.driftRatio, 0);
  assert.equal(report.repaired, false);
  const history = index.driftHistory();
  assert.equal(history.length, 1);
  assert.equal(history[0]!.reconciledAt, T2);
});

test("reconcile measures missing and stale drift against authoritative rows", () => {
  const a = makeStore("tenant-a").store;
  const b = makeStore("tenant-b").store;
  const eu = makeStore("tenant-eu", EU_PRIVATE).store;
  const { index } = makeIndex();
  const r1 = a.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 }).row;
  index.consume(a, { now: T0 });
  // Not yet consumed: the index lags the authoritative rows.
  a.removeReference(r1.reference_id, { actor: WRITER, now: T1 });
  b.createReference({ contentId: C2, refClass: "standard", createdBy: WRITER, now: T1 });

  const report = index.reconcile([a, b, eu], { now: T2 });
  assert.deepEqual(report.dbsScanned, ["tenant-a", "tenant-b"]);
  assert.deepEqual(report.dbsSkipped, ["tenant-eu"]);
  assert.equal(report.authoritative, 1);
  assert.equal(report.indexed, 1);
  assert.deepEqual(
    report.missing.map((e) => [e.dbId, e.contentId]),
    [["tenant-b", C2]],
  );
  assert.deepEqual(
    report.stale.map((e) => [e.dbId, e.contentId]),
    [["tenant-a", C1]],
  );
  assert.equal(report.driftRatio, 1);
  assert.deepEqual(report.affectedContent, [C1, C2].sort());
  // Measuring does not change the index.
  assert.deepEqual(index.lookup(C1).referencingDbs, ["tenant-a"]);
});

test("reconcile with repair makes the index match, and later outbox events still apply on top", () => {
  const a = makeStore("tenant-a").store;
  const { index } = makeIndex();
  const r1 = a.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 }).row;
  const r2 = a.createReference({ contentId: C2, refClass: "standard", createdBy: WRITER, now: T0 }).row;
  index.consume(a, { now: T0 }); // cursor 2
  a.removeReference(r1.reference_id, { actor: WRITER, now: T1 }); // seq 3, unconsumed

  const repaired = index.reconcile([a], { repair: true, now: T1 });
  assert.equal(repaired.stale.length, 1);
  assert.equal(repaired.repaired, true);
  assert.deepEqual(index.lookup(C1).referencingDbs, []);
  assert.equal(index.reconcile([a], { now: T1 }).driftRatio, 0);

  // The unconsumed removal (seq 3 > repair seq 2) applies cleanly, and a
  // later removal of C2 still reaches the index.
  a.removeReference(r2.reference_id, { actor: WRITER, now: T2 });
  const r = index.consume(a, { now: T2 });
  assert.equal(r.read, 2);
  assert.deepEqual(index.lookup(C2).referencingDbs, []);
  assert.equal(index.reconcile([a], { now: T2 }).driftRatio, 0);
  assert.deepEqual(
    index.driftHistory().map((h) => [h.stale, h.repaired]),
    [
      [1, true],
      [0, false],
      [0, false],
    ],
  );
});

test("reconcile flags an index entry whose content or class disagrees with the row", () => {
  const a = makeStore("tenant-a").store;
  const { index, sql } = makeIndex();
  const { row } = a.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 });
  index.consume(a, { now: T0 });
  sql.exec("UPDATE reference_index_entry SET content_id = ? WHERE reference_id = ?", C3, row.reference_id);
  const report = index.reconcile([a], { repair: true, now: T1 });
  assert.equal(report.mismatched.length, 1);
  assert.deepEqual(report.mismatched[0]!.indexed, { contentId: C3, class: "standard" });
  assert.deepEqual(report.affectedContent, [C1, C3].sort());
  assert.deepEqual(index.lookup(C1).referencingDbs, ["tenant-a"]);
  assert.deepEqual(index.lookup(C3).referencingDbs, []);
});

test("an event that claims another partition is refused, not indexed", () => {
  const { store: a, sql: aSql } = makeStore("tenant-a");
  const { index } = makeIndex();
  a.createReference({ contentId: C1, refClass: "standard", createdBy: WRITER, now: T0 });
  aSql.exec("UPDATE reference_outbox SET jurisdiction = 'eu'");
  expectFail(() => index.consume(a, { now: T1 }), "index_event_partition_mismatch");
  assert.deepEqual(index.lookup(C1).referencingDbs, []);
  assert.equal(index.cursor("tenant-a"), 0);
});
