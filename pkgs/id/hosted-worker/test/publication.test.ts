// Issue #12807 (component F slice 5 of #9084): publication semantics.
// Proven against real ReferenceStores on in-memory SQLite (node:sqlite),
// the harness shape of test/gc-sweep.test.ts, with a fake object-plane
// adapter that verifies capabilities with the real verifier and counts
// every write-path call, so "no byte copy at publish" is observable.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { ReferenceStore, type Partition } from "../src/references.ts";
import {
  verifyReadCapability,
  type AdapterCapabilityMatrix,
  type ObjectHead,
  type ObjectPlaneAdapter,
  type ObjectReadCapability,
  type ReadResult,
  type StagingReceipt,
} from "../src/object-plane.ts";
import { PublicationService } from "../src/publication.ts";
import { FailClosed } from "../src/routing.ts";

const PUBLISHER = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const STRANGER = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const ADMIN = "cccccccccccccccccccccccccccccccc";
const C1 = "ab".repeat(32);
const C2 = "cd".repeat(32);
const T0 = "2026-11-26T00:00:00.000Z";
const at = (s: number) => new Date(Date.parse(T0) + s * 1000).toISOString();
const KEY = new TextEncoder().encode("publication-test-capability-key!");

const US_PUBLIC: Partition = { jurisdiction: "us", tier: "public" };
const US_PRIVATE: Partition = { jurisdiction: "us", tier: "tenant-private" };
const EU_PUBLIC: Partition = { jurisdiction: "eu", tier: "public" };

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

function makeStore(dbId: string, partition: Partition, isPublicDb = false) {
  let n = 0;
  const store = new ReferenceStore(fakeSql(), {
    dbId,
    partition,
    isPublicDb,
    newId: () => `${dbId.slice(0, 4)}${(++n).toString(16).padStart(28, "0")}`,
  });
  store.ensureSchema();
  return store;
}

class FakeAdapter implements ObjectPlaneAdapter {
  readonly partition: Partition;
  readonly objects = new Map<string, Uint8Array>();
  cdnEligible = true;
  writeCalls = 0;
  reads = 0;
  constructor(partition: Partition) {
    this.partition = partition;
  }
  capabilities(): AdapterCapabilityMatrix {
    return {
      provider: "fake",
      conditionalWrite: "if-none-match",
      rangeReads: "client-slice",
      multipart: { supported: false },
      replicaClasses: this.cdnEligible ? ["standard", "cdn-edge"] : ["standard"],
      cdnEligible: this.cdnEligible,
      readAfterWrite: true,
      stagingTtlSeconds: 86400,
      notes: [],
    };
  }
  async putStaged(): Promise<StagingReceipt> {
    this.writeCalls++;
    throw new Error("publication must never stage bytes");
  }
  async verify(): Promise<StagingReceipt> {
    this.writeCalls++;
    throw new Error("publication must never verify staged bytes");
  }
  async publish(): Promise<ObjectHead> {
    this.writeCalls++;
    throw new Error("publication must never copy bytes");
  }
  async head(contentId: string): Promise<ObjectHead | null> {
    const bytes = this.objects.get(contentId);
    return bytes ? { contentId, sizeBytes: bytes.length, servedFromReplica: 0 } : null;
  }
  async read(contentId: string, capability: ObjectReadCapability, opts?: { now?: string }): Promise<ReadResult> {
    await verifyReadCapability(KEY, contentId, this.partition, capability, opts?.now ?? new Date().toISOString());
    const bytes = this.objects.get(contentId);
    if (!bytes) throw new FailClosed(404, "not_found", "no replica");
    this.reads++;
    return { bytes, head: { contentId, sizeBytes: bytes.length, servedFromReplica: 0 } };
  }
  async deleteReplica(): Promise<{ deleted: boolean }> {
    this.writeCalls++;
    return { deleted: false };
  }
  async reapStaging(): Promise<{ reaped: number }> {
    return { reaped: 0 };
  }
}

function setup(opts: { cdnEligible?: boolean } = {}) {
  const publicDb = makeStore("pub0", US_PUBLIC, true);
  const tenantDb = makeStore("ten0", US_PUBLIC);
  const adapter = new FakeAdapter(US_PUBLIC);
  adapter.cdnEligible = opts.cdnEligible ?? true;
  adapter.objects.set(C1, new TextEncoder().encode("published bytes"));
  const svc = new PublicationService(fakeSql(), { publicDb, adapter, capabilityKey: KEY, capabilityTtlSeconds: 300 });
  svc.ensureSchema();
  tenantDb.createReference({ contentId: C1, refClass: "standard", createdBy: PUBLISHER, now: T0 });
  return { publicDb, tenantDb, adapter, svc };
}

async function rejects(p: Promise<unknown> | (() => unknown), code: string) {
  await assert.rejects(
    async () => (typeof p === "function" ? p() : p),
    (e: unknown) => e instanceof FailClosed && (e as FailClosed & { code: string }).code === code,
  );
}

test("published content is readable via the publication reference alone", async () => {
  const { publicDb, tenantDb, svc } = setup();
  const pub = await svc.publish({ contentId: C1, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: T0 });
  assert.equal(pub.created, true);
  // A reader with no authorized databases at all.
  const res = await svc.readPublished(C1, { now: at(10) });
  assert.equal(new TextDecoder().decode(res.bytes), "published bytes");
  const rows = publicDb.referencesFor(C1);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].class, "publication");
  assert.equal(rows[0].reference_id, pub.publicationId);
});

test("publish creates no byte copy and is idempotent", async () => {
  const { publicDb, tenantDb, adapter, svc } = setup();
  const a = await svc.publish({ contentId: C1, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: T0 });
  const b = await svc.publish({ contentId: C1, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: at(1) });
  assert.equal(adapter.writeCalls, 0);
  assert.equal(adapter.objects.size, 1);
  assert.equal(b.created, false);
  assert.equal(b.publicationId, a.publicationId);
  assert.equal(publicDb.outboxAfter(0).length, 1);
});

test("a content hash alone cannot be published", async () => {
  const { svc } = setup();
  await rejects(svc.publish({ contentId: C1, publisher: STRANGER, authorizedDbs: [], now: T0 }), "publication_requires_reference");
  const otherDb = makeStore("oth0", US_PRIVATE);
  otherDb.createReference({ contentId: C1, refClass: "standard", createdBy: STRANGER, now: T0 });
  // A reference in another partition grants nothing in the public one.
  await rejects(
    svc.publish({ contentId: C1, publisher: STRANGER, authorizedDbs: [otherDb], now: T0 }),
    "publication_requires_reference",
  );
});

test("publish refuses content with no verified public replica", async () => {
  const { tenantDb, svc } = setup();
  tenantDb.createReference({ contentId: C2, refClass: "standard", createdBy: PUBLISHER, now: T0 });
  await rejects(
    svc.publish({ contentId: C2, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: T0 }),
    "publication_requires_verified_replica",
  );
});

test("unpublished content is refused new capabilities immediately", async () => {
  const { tenantDb, svc } = setup();
  const pub = await svc.publish({ contentId: C1, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: T0 });
  const minted = await svc.mintPublicCapability({ contentId: C1, now: at(5) });
  const res = svc.unpublish(pub.publicationId, { actor: PUBLISHER, now: at(6) });
  assert.equal(res.removed, true);
  await rejects(svc.mintPublicCapability({ contentId: C1, now: at(7) }), "not_published");
  await rejects(svc.readPublished(C1, { now: at(7) }), "not_published");
  // Already-minted capabilities run out on their own TTL (documented
  // revocation lag), then fail closed.
  await rejects(
    verifyReadCapability(KEY, C1, US_PUBLIC, minted, at(5 + 300)),
    "capability_expired",
  );
});

test("never-published content is refused capabilities", async () => {
  const { svc } = setup();
  await rejects(svc.mintPublicCapability({ contentId: C1, now: T0 }), "not_published");
});

test("unpublish is limited to the publisher or an admin", async () => {
  const { tenantDb, svc } = setup();
  const pub = await svc.publish({ contentId: C1, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: T0 });
  await rejects(() => svc.unpublish(pub.publicationId, { actor: STRANGER, now: at(1) }), "unpublish_not_authorized");
  assert.equal(svc.unpublish(pub.publicationId, { actor: ADMIN, admin: true, now: at(2) }).removed, true);
  // Second removal is a no-op, unknown ids fail closed.
  assert.equal(svc.unpublish(pub.publicationId, { actor: PUBLISHER, now: at(3) }).removed, false);
  await rejects(() => svc.unpublish("f".repeat(32), { actor: PUBLISHER, now: at(3) }), "reference_not_found");
});

test("unpublish refuses a reference that is not a publication", async () => {
  const { publicDb, svc } = setup();
  const { row } = publicDb.createReference({ contentId: C1, refClass: "standard", createdBy: PUBLISHER, now: T0 });
  await rejects(() => svc.unpublish(row.reference_id, { actor: PUBLISHER, now: at(1) }), "publication_not_found");
  assert.equal(publicDb.liveReferences().length, 1);
});

test("CDN path verifies while published and in the current epoch", async () => {
  const { tenantDb, svc } = setup();
  const pub = await svc.publish({ contentId: C1, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: T0 });
  const path = await svc.cdnPath(C1, { now: at(1) });
  assert.match(path.url, new RegExp(`^/cdn/v1/us/${C1}\\?`));
  assert.equal(path.epoch, 0);
  assert.deepEqual(await svc.verifyCdnPath(path.url, { now: at(2) }), { contentId: C1, publicationId: pub.publicationId });
  // Tampering with the epoch or content breaks the signature.
  await rejects(svc.verifyCdnPath(path.url.replace("epoch=0", "epoch=1"), { now: at(2) }), "cdn_path_bad_signature");
  await rejects(svc.verifyCdnPath(path.url, { now: at(3601) }), "cdn_path_expired");
});

test("unpublish invalidates the CDN path at the next epoch, not instantly", async () => {
  const { tenantDb, svc } = setup();
  const pub = await svc.publish({ contentId: C1, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: T0 });
  const path = await svc.cdnPath(C1, { now: at(1) });
  const res = svc.unpublish(pub.publicationId, { actor: PUBLISHER, now: at(2) });
  assert.equal(res.cdnInvalidAfterEpoch, 0);
  // No new CDN paths once unpublished.
  await rejects(svc.cdnPath(C1, { now: at(3) }), "not_published");
  // The edge check also consults the live reference, so an edge that
  // re-verifies refuses at once; cached responses age out by epoch.
  await rejects(svc.verifyCdnPath(path.url, { now: at(3) }), "cdn_path_unpublished");
  svc.advanceCdnEpoch({ now: at(4) });
  await rejects(svc.verifyCdnPath(path.url, { now: at(5) }), "cdn_path_stale_epoch");
});

test("an epoch advance retires CDN paths even while still published", async () => {
  const { tenantDb, svc } = setup();
  await svc.publish({ contentId: C1, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: T0 });
  const old = await svc.cdnPath(C1, { now: at(1) });
  assert.equal(svc.advanceCdnEpoch({ now: at(2) }), 1);
  await rejects(svc.verifyCdnPath(old.url, { now: at(3) }), "cdn_path_stale_epoch");
  const fresh = await svc.cdnPath(C1, { now: at(3) });
  assert.equal(fresh.epoch, 1);
  await svc.verifyCdnPath(fresh.url, { now: at(4) });
});

test("non-CDN-eligible adapters serve under a capability only", async () => {
  const { tenantDb, svc } = setup({ cdnEligible: false });
  await svc.publish({ contentId: C1, publisher: PUBLISHER, authorizedDbs: [tenantDb], now: T0 });
  await rejects(svc.cdnPath(C1, { now: at(1) }), "cdn_not_eligible");
  const res = await svc.readPublished(C1, { now: at(1) });
  assert.equal(res.bytes.length, 15);
});

test("the service refuses a non-public database or a mismatched adapter", () => {
  const tenantDb = makeStore("ten1", US_PUBLIC);
  assert.throws(
    () => new PublicationService(fakeSql(), { publicDb: tenantDb, adapter: new FakeAdapter(US_PUBLIC), capabilityKey: KEY }),
    (e: unknown) => e instanceof FailClosed && (e as FailClosed & { code: string }).code === "publication_requires_public_db",
  );
  const publicDb = makeStore("pub1", US_PUBLIC, true);
  assert.throws(
    () => new PublicationService(fakeSql(), { publicDb, adapter: new FakeAdapter(EU_PUBLIC), capabilityKey: KEY }),
    (e: unknown) => e instanceof FailClosed && (e as FailClosed & { code: string }).code === "publication_partition_mismatch",
  );
});
