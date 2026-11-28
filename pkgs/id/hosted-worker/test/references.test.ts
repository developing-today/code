// Issue #12803 (component F slice 1 of #9084): the reference record
// schema and the design section 5 access check, proven against the
// real ReferenceStore with in-memory SQLite (node:sqlite) behind its
// SQL interface -- the same harness shape as test/blobs.test.ts.
// Acceptance coverage: live/tombstoned references, cross-partition
// refusal, restricted-class minting, publication references, and the
// create/remove outbox the index (#12805) and GC (#12806) consume.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { ReferenceStore, checkAccess, REFERENCE_SCHEMA_STATEMENTS, type Partition } from "../src/references.ts";
import { FailClosed } from "../src/routing.ts";

const WRITER = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_WRITER = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CONTENT = "ab".repeat(32);
const OTHER_CONTENT = "cd".repeat(32);
const T0 = "2026-11-23T00:00:00.000Z";
const T1 = "2026-11-23T01:00:00.000Z";

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

function makeStore(dbId: string, partition: Partition, isPublicDb = false): ReferenceStore {
  let n = 0;
  const store = new ReferenceStore(fakeSql(), {
    dbId,
    partition,
    isPublicDb,
    newId: () => (++n).toString(16).padStart(32, "0"),
  });
  store.ensureSchema();
  return store;
}

const US_PRIVATE: Partition = { jurisdiction: "us", tier: "tenant-private" };
const EU_PRIVATE: Partition = { jurisdiction: "eu", tier: "tenant-private" };
const US_RESTRICTED: Partition = { jurisdiction: "us", tier: "tenant-restricted" };
const US_PUBLIC: Partition = { jurisdiction: "us", tier: "public" };

test("migration file and embedded schema statements cannot drift", () => {
  const url = new URL("../references/migrations/0001_reference_record.sql", import.meta.url);
  const fileSql = readFileSync(url, "utf8")
    .split("\n")
    .filter((line) => !line.trimStart().startsWith("--"))
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();
  for (const stmt of REFERENCE_SCHEMA_STATEMENTS) {
    const normalized = stmt.replace(/\s+/g, " ").trim();
    assert.ok(fileSql.includes(normalized), `migration file is missing statement: ${normalized.slice(0, 60)}...`);
  }
  const creates = fileSql.match(/CREATE /g) ?? [];
  assert.equal(creates.length, REFERENCE_SCHEMA_STATEMENTS.length);
});

test("a live reference grants read in its own partition via an authorized database", () => {
  const tenant = makeStore("tenant-a", US_PRIVATE);
  const { row, created } = tenant.createReference({
    contentId: CONTENT,
    refClass: "standard",
    createdBy: WRITER,
    now: T0,
  });
  assert.equal(created, true);
  assert.equal(row.tombstoned_at, null);
  const decision = checkAccess({ contentId: CONTENT, partition: US_PRIVATE, authorizedDbs: [tenant] });
  assert.equal(decision.allowed, true);
  assert.equal(decision.via, "reference");
  assert.equal(decision.referenceId, row.reference_id);
  assert.equal(decision.dbId, "tenant-a");
});

test("the hash alone grants nothing: no reference, no access", () => {
  const tenant = makeStore("tenant-a", US_PRIVATE);
  const decision = checkAccess({ contentId: CONTENT, partition: US_PRIVATE, authorizedDbs: [tenant] });
  assert.equal(decision.allowed, false);
  assert.equal(decision.via, null);
});

test("a tombstoned reference grants nothing; removal is revocation", () => {
  const tenant = makeStore("tenant-a", US_PRIVATE);
  const { row } = tenant.createReference({ contentId: CONTENT, refClass: "standard", createdBy: WRITER, now: T0 });
  assert.equal(checkAccess({ contentId: CONTENT, partition: US_PRIVATE, authorizedDbs: [tenant] }).allowed, true);
  assert.equal(tenant.removeReference(row.reference_id, { actor: WRITER, now: T1 }), true);
  assert.equal(checkAccess({ contentId: CONTENT, partition: US_PRIVATE, authorizedDbs: [tenant] }).allowed, false);
  // The row survives as a tombstone -- the state GC re-verifies against.
  const rows = tenant.referencesFor(CONTENT);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].tombstoned_at, T1);
});

test("a reference in a database the caller is not authorized for grants nothing", () => {
  const mine = makeStore("tenant-a", US_PRIVATE);
  const theirs = makeStore("tenant-b", US_PRIVATE);
  theirs.createReference({ contentId: CONTENT, refClass: "standard", createdBy: OTHER_WRITER, now: T0 });
  const decision = checkAccess({ contentId: CONTENT, partition: US_PRIVATE, authorizedDbs: [mine] });
  assert.equal(decision.allowed, false);
});

test("cross-partition refusal: a reference never grants outside its own partition", () => {
  const eu = makeStore("tenant-eu", EU_PRIVATE);
  eu.createReference({ contentId: CONTENT, refClass: "standard", createdBy: WRITER, now: T0 });
  // Same content, same tier, different jurisdiction: refused.
  assert.equal(checkAccess({ contentId: CONTENT, partition: US_PRIVATE, authorizedDbs: [eu] }).allowed, false);
  // Same jurisdiction, different tier: refused.
  const us = makeStore("tenant-us", US_PRIVATE);
  us.createReference({ contentId: CONTENT, refClass: "standard", createdBy: WRITER, now: T0 });
  assert.equal(checkAccess({ contentId: CONTENT, partition: US_PUBLIC, authorizedDbs: [us] }).allowed, false);
  // And the in-partition check still passes, so the refusals above
  // are the partition rule, not a broken store.
  assert.equal(checkAccess({ contentId: CONTENT, partition: EU_PRIVATE, authorizedDbs: [eu] }).allowed, true);
});

test("publication: a live publication reference in the public DB grants any caller, unpublish revokes", () => {
  const publicDb = makeStore("public", US_PUBLIC, true);
  const { row } = publicDb.createReference({
    contentId: CONTENT,
    refClass: "publication",
    createdBy: WRITER,
    now: T0,
  });
  // No authorized tenant DBs at all -- the publication reference is
  // the grant (design section 5, rule 2).
  const decision = checkAccess({ contentId: CONTENT, partition: US_PUBLIC, authorizedDbs: [], publicDb });
  assert.equal(decision.allowed, true);
  assert.equal(decision.via, "publication");
  assert.equal(decision.dbId, "public");
  // Publication never leaks into a tenant partition.
  assert.equal(checkAccess({ contentId: CONTENT, partition: US_PRIVATE, authorizedDbs: [], publicDb }).allowed, false);
  // Unpublishing is reference removal.
  publicDb.removeReference(row.reference_id, { actor: WRITER, now: T1 });
  assert.equal(checkAccess({ contentId: CONTENT, partition: US_PUBLIC, authorizedDbs: [], publicDb }).allowed, false);
});

test("a publication reference cannot be minted outside the public database", () => {
  const tenant = makeStore("tenant-a", US_PRIVATE);
  assert.throws(
    () => tenant.createReference({ contentId: CONTENT, refClass: "publication", createdBy: WRITER, now: T0 }),
    (e: unknown) => e instanceof FailClosed && e.code === "publication_reference_outside_public_db",
  );
});

test("restricted-class minting requires an authorized writer of the source content", () => {
  const tenant = makeStore("tenant-r", US_RESTRICTED);
  assert.throws(
    () => tenant.createReference({ contentId: CONTENT, refClass: "restricted", createdBy: WRITER, now: T0 }),
    (e: unknown) => e instanceof FailClosed && e.code === "restricted_class_not_authorized",
  );
  const { created } = tenant.createReference({
    contentId: CONTENT,
    refClass: "restricted",
    createdBy: WRITER,
    canMintRestricted: true,
    now: T0,
  });
  assert.equal(created, true);
});

test("tenant-restricted access requires the restricted class on the reference", () => {
  const tenant = makeStore("tenant-r", US_RESTRICTED);
  // A standard-class reference in the same partition is not enough.
  tenant.createReference({ contentId: CONTENT, refClass: "standard", createdBy: WRITER, now: T0 });
  assert.equal(checkAccess({ contentId: CONTENT, partition: US_RESTRICTED, authorizedDbs: [tenant] }).allowed, false);
  // The restricted class is.
  tenant.createReference({
    contentId: CONTENT,
    refClass: "restricted",
    createdBy: WRITER,
    canMintRestricted: true,
    now: T0,
  });
  const decision = checkAccess({ contentId: CONTENT, partition: US_RESTRICTED, authorizedDbs: [tenant] });
  assert.equal(decision.allowed, true);
  assert.equal(decision.via, "reference");
});

test("referencing is idempotent: a duplicate live create returns the row and emits no second event", () => {
  const tenant = makeStore("tenant-a", US_PRIVATE);
  const first = tenant.createReference({ contentId: CONTENT, refClass: "standard", createdBy: WRITER, now: T0 });
  const second = tenant.createReference({ contentId: CONTENT, refClass: "standard", createdBy: WRITER, now: T1 });
  assert.equal(second.created, false);
  assert.equal(second.row.reference_id, first.row.reference_id);
  assert.equal(tenant.outboxAfter(0).filter((e) => e.event_type === "reference.created").length, 1);
});

test("create and remove emit outbox events the index and GC consume, pageable by seq", () => {
  const tenant = makeStore("tenant-a", US_PRIVATE);
  const { row } = tenant.createReference({ contentId: CONTENT, refClass: "standard", createdBy: WRITER, now: T0 });
  tenant.createReference({ contentId: OTHER_CONTENT, refClass: "standard", createdBy: WRITER, now: T0 });
  tenant.removeReference(row.reference_id, { actor: WRITER, now: T1 });

  const events = tenant.outboxAfter(0);
  assert.equal(events.length, 3);
  assert.deepEqual(
    events.map((e) => e.event_type),
    ["reference.created", "reference.created", "reference.removed"],
  );
  const removed = events[2];
  assert.equal(removed.reference_id, row.reference_id);
  assert.equal(removed.content_id, CONTENT);
  assert.equal(removed.jurisdiction, "us");
  assert.equal(removed.sensitivity_tier, "tenant-private");
  assert.equal(removed.class, "standard");
  assert.equal(removed.actor, WRITER);
  assert.equal(removed.occurred_at, T1);
  // A consumer resuming from its cursor sees only what is new.
  assert.deepEqual(
    tenant.outboxAfter(events[1].seq).map((e) => e.event_type),
    ["reference.removed"],
  );
});

test("removing an unknown reference fails closed; a second remove is a no-op with no second event", () => {
  const tenant = makeStore("tenant-a", US_PRIVATE);
  const missing = "f".repeat(32);
  assert.throws(
    () => tenant.removeReference(missing, { actor: WRITER, now: T0 }),
    (e: unknown) => e instanceof FailClosed && e.code === "reference_not_found",
  );
  const { row } = tenant.createReference({ contentId: CONTENT, refClass: "standard", createdBy: WRITER, now: T0 });
  assert.equal(tenant.removeReference(row.reference_id, { actor: WRITER, now: T1 }), true);
  assert.equal(tenant.removeReference(row.reference_id, { actor: WRITER, now: T1 }), false);
  assert.equal(tenant.outboxAfter(0).filter((e) => e.event_type === "reference.removed").length, 1);
});

test("staged content is not referenceable", () => {
  const staging = makeStore("staging", { jurisdiction: "us", tier: "staging" });
  assert.throws(
    () => staging.createReference({ contentId: CONTENT, refClass: "standard", createdBy: WRITER, now: T0 }),
    (e: unknown) => e instanceof FailClosed && e.code === "staging_not_referenceable",
  );
});

test("malformed content ids and binding ids fail closed", () => {
  const tenant = makeStore("tenant-a", US_PRIVATE);
  assert.throws(
    () => tenant.createReference({ contentId: "not-a-hash", refClass: "standard", createdBy: WRITER, now: T0 }),
    (e: unknown) => e instanceof FailClosed && e.code === "invalid_content_id",
  );
  assert.throws(
    () => tenant.createReference({ contentId: CONTENT, refClass: "standard", createdBy: "nope", now: T0 }),
    (e: unknown) => e instanceof FailClosed,
  );
  assert.throws(
    () => checkAccess({ contentId: "nope", partition: US_PRIVATE, authorizedDbs: [tenant] }),
    (e: unknown) => e instanceof FailClosed && e.code === "invalid_content_id",
  );
});
