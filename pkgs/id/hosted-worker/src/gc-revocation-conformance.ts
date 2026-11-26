// GC race + revocation conformance suite (component F slice 6,
// issue #12808; design record
// doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane,
// sections 5-6).
//
// Covers the design's six safety claims as their own slice so they
// run against any implementation of slices 1-5: (a) reference during
// grace clears the candidate; (b) post-re-verify reference is
// serialized by the epoch fence (re-upload path, never wrongful
// delete); (c) cross-partition resolution refused; (d) hash alone
// never serves bytes; (e) revocation stops new minting within the
// TTL bound; (f) sole-referencer erasure completes under the fence.
//
// Adapter-neutrality: cases speak ONLY the F adapter contract --
// ObjectPlaneAdapter (object-plane.ts), ReferenceStore + checkAccess
// (references.ts), ReferenceIndex, GcSweeper, PublicationService.
// No R2-specific consistency, URL, multipart, or lifecycle behavior
// is assumed, so the RustFS adapter (component I, #9087) runs this
// exact suite by supplying its own harness factory. Bytes are seeded
// through putStaged/publish (the contract itself), never through a
// backend.
//
// This module imports nothing Node- or worker-specific: the case
// runner returns plain { name, run } cases and the host test file
// (test/gc-revocation-conformance.test.ts here) executes them under
// its own harness. Failure reporting uses plain Errors; FailClosed
// codes are asserted through expectFailClosed.

import { checkAccess, type Partition, type ReferenceStore } from "./references.ts";
import type { ReferenceIndex } from "./reference-index.ts";
import type { GcSweeper } from "./gc-sweep.ts";
import type { PublicationService } from "./publication.ts";
import { verifyReadCapability, type ObjectPlaneAdapter } from "./object-plane.ts";
import { FailClosed } from "./routing.ts";

export const US_PRIVATE: Partition = { jurisdiction: "us", tier: "tenant-private" };
export const EU_PRIVATE: Partition = { jurisdiction: "eu", tier: "tenant-private" };
export const US_PUBLIC: Partition = { jurisdiction: "us", tier: "public" };
export const US_RESTRICTED: Partition = { jurisdiction: "us", tier: "tenant-restricted" };

export interface ConformanceCase {
  name: string;
  run(): Promise<void>;
}

/** One fresh harness per case, built by the host test file. The
 *  plane is the adapter under test (R2 here, RustFS under #9087);
 *  everything else is the real F contract wiring over it. For the
 *  public partition the harness also carries the publication
 *  service; for tenant partitions stores/index/gc suffice. */
export interface SliceHarness {
  partition: Partition;
  plane: ObjectPlaneAdapter;
  stores: ReferenceStore[];
  index: ReferenceIndex;
  gc: GcSweeper;
  publication?: PublicationService;
  publicDb?: ReferenceStore;
  capabilityKey: Uint8Array;
  locator: string;
}

export type HarnessFactory = (partition: Partition) => SliceHarness | Promise<SliceHarness>;

const T0 = "2026-11-26T00:00:00.000Z";
const DAY_MS = 24 * 60 * 60 * 1000;
const at = (ms: number) => new Date(Date.parse(T0) + ms).toISOString();
const atSec = (s: number) => new Date(Date.parse(T0) + s * 1000).toISOString();

const WRITER = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

function check(cond: unknown, message: string): asserts cond {
  if (!cond) throw new Error(`gc-revocation conformance: ${message}`);
}

export async function expectFailClosed(work: Promise<unknown>, code: string): Promise<void> {
  try {
    await work;
  } catch (e) {
    if (e instanceof FailClosed && e.code === code) return;
    const got = e instanceof FailClosed ? `FailClosed(${e.code})` : String(e);
    throw new Error(`gc-revocation conformance: expected FailClosed(${code}), got ${got}`);
  }
  throw new Error(`gc-revocation conformance: expected FailClosed(${code}), got success`);
}

/** Seed bytes through the contract and publish them. */
async function seed(plane: ObjectPlaneAdapter, bytes: Uint8Array): Promise<{ contentId: string; size: number }> {
  const receipt = await plane.putStaged(bytes, { now: T0 });
  await plane.publish(receipt.stagingId, receipt.contentId, { now: T0 });
  return { contentId: receipt.contentId, size: bytes.length };
}

const PAYLOAD = (s: string) => new TextEncoder().encode(`gc-revocation case payload ${s}`);

export function gcRevocationConformanceCases(factory: HarnessFactory): ConformanceCase[] {
  return [
    {
      name: "(a) reference created during sweep grace clears the candidate",
      run: async () => {
        const h = await factory(US_PRIVATE);
        const { contentId } = await seed(h.plane, PAYLOAD("a"));
        // Orphan: reference, then remove it, so the index sees the
        // content as unreferenced with a fresh location.
        const store = h.stores[0];
        const { row } = store.createReference({ contentId, refClass: "standard", createdBy: WRITER, now: T0 });
        h.index.notePublished(contentId, h.locator, { now: T0 });
        store.removeReference(row.reference_id, { actor: WRITER, now: T0 });
        h.index.consume(store, { now: T0 });

        const marked = h.gc.mark({ now: T0 });
        check(marked.marked.includes(contentId), "orphaned content is marked candidate");

        // A new reference lands during grace and the index consumes
        // it: the next mark clears the candidate to live.
        store.createReference({ contentId, refClass: "standard", createdBy: OTHER, now: at(DAY_MS) });
        h.index.consume(store, { now: at(DAY_MS) });
        const again = h.gc.mark({ now: at(DAY_MS) });
        check(again.cleared.includes(contentId), "grace reference clears the candidate");
        check(h.gc.candidate(contentId)?.state === "live", "candidate is live after the grace reference");

        // Past the original grace the sweep rescues nothing and
        // deletes nothing: the content was never collectible.
        const res = await h.gc.sweep(h.stores, { now: at(8 * DAY_MS) });
        check(!res.deleted.includes(contentId), "rescued content is never deleted");
        check((await h.plane.head(contentId)) !== null, "replica survives the sweep");
      },
    },
    {
      name: "(b) post-re-verify reference is serialized by the epoch fence via re-upload",
      run: async () => {
        const h = await factory(US_PRIVATE);
        const bytes = PAYLOAD("b");
        const { contentId } = await seed(h.plane, bytes);
        const store = h.stores[0];
        const { row } = store.createReference({ contentId, refClass: "standard", createdBy: WRITER, now: T0 });
        h.index.notePublished(contentId, h.locator, { now: T0 });
        store.removeReference(row.reference_id, { actor: WRITER, now: T0 });
        h.index.consume(store, { now: T0 });
        h.gc.mark({ now: T0 });

        // A writer races the sweep's adapter call: admission arrives
        // after the fence was written, so it is told to re-stage
        // rather than proceeding against bytes that are gone.
        const racing: { action: string; seq: number }[] = [];
        const inner = h.plane.deleteReplica.bind(h.plane);
        h.plane.deleteReplica = (async (id: string, fence: never, opts?: { now?: string }) => {
          racing.push(h.gc.admitReference(id, { now: opts?.now ?? T0 }));
          return inner(id, fence, opts);
        }) as typeof h.plane.deleteReplica;
        const res = await h.gc.sweep(h.stores, { now: at(8 * DAY_MS) });
        check(res.deleted.includes(contentId), "unreferenced content is deleted under the fence");
        check(racing.length === 1 && racing[0].action === "restage", "racing admission is told to re-stage");
        const fenceSeq = h.gc.auditLog()[0]?.fenceSeq;
        check(fenceSeq !== undefined && racing[0].seq > fenceSeq, "admission is ordered after the fence");

        // The writer re-uploads through the contract and references
        // the fresh bytes: a later mark sees a new candidate, never
        // a wrongful delete of the re-uploaded replica.
        const receipt = await h.plane.putStaged(bytes, { now: at(9 * DAY_MS) });
        await h.plane.publish(receipt.stagingId, contentId, { now: at(9 * DAY_MS) });
        check((await h.plane.head(contentId)) !== null, "re-uploaded bytes are served");
        store.createReference({ contentId, refClass: "standard", createdBy: OTHER, now: at(9 * DAY_MS) });
        h.index.consume(store, { now: at(9 * DAY_MS) });
        h.index.notePublished(contentId, h.locator, { now: at(9 * DAY_MS) });
        check(checkAccess({ contentId, partition: h.partition, authorizedDbs: h.stores }).allowed, "re-upload is referenceable");
      },
    },
    {
      name: "(c) cross-partition reference resolution is refused",
      run: async () => {
        const us = await factory(US_PRIVATE);
        const usBytes = PAYLOAD("c");
        const { contentId } = await seed(us.plane, usBytes);
        const { row } = us.stores[0].createReference({ contentId, refClass: "standard", createdBy: WRITER, now: T0 });
        void row;
        us.index.notePublished(contentId, us.locator, { now: T0 });
        us.index.consume(us.stores[0], { now: T0 });

        // A US reference grants nothing in the EU partition.
        const denied = checkAccess({ contentId, partition: EU_PRIVATE, authorizedDbs: us.stores });
        check(!denied.allowed, "us reference does not authorize eu reads");

        // The EU sweep refuses a mixed-partition store set before
        // reading anything, and deletes nothing anywhere.
        const eu = await factory(EU_PRIVATE);
        await expectFailClosed(us.gc.sweep([us.stores[0], eu.stores[0]], { now: at(8 * DAY_MS) }), "gc_store_partition_mismatch");
        check((await eu.plane.head(contentId)) === null, "eu partition holds no replica of the us content");
      },
    },
    {
      name: "(d) hash presentation alone never serves bytes",
      run: async () => {
        const h = await factory(US_PRIVATE);
        const { contentId } = await seed(h.plane, PAYLOAD("d"));

        // No reference anywhere: the access check denies, and no
        // capability can be minted from the check (there is no
        // granting reference to mint from).
        const denied = checkAccess({ contentId, partition: h.partition, authorizedDbs: h.stores });
        check(!denied.allowed, "hash alone authorizes nothing");

        // A capability bound to a different object is refused before
        // any byte is touched.
        const { mintReadCapability } = await import("./object-plane.ts");
        const other = await mintReadCapability(h.capabilityKey, {
          contentId: "ff".repeat(32),
          partition: h.partition,
          expiresAt: at(8 * DAY_MS),
        });
        await expectFailClosed(h.plane.read(contentId, other, { now: T0 }), "capability_wrong_object");

        // Granting a reference then removing it revokes the grant:
        // the check denies again with no live row behind it.
        const { row } = h.stores[0].createReference({ contentId, refClass: "standard", createdBy: WRITER, now: T0 });
        check(checkAccess({ contentId, partition: h.partition, authorizedDbs: h.stores }).allowed, "live reference authorizes");
        h.stores[0].removeReference(row.reference_id, { actor: WRITER, now: at(1000) });
        check(!checkAccess({ contentId, partition: h.partition, authorizedDbs: h.stores }).allowed, "removed reference revokes");
      },
    },
    {
      name: "(e) revocation by reference removal stops new capability minting within the TTL bound",
      run: async () => {
        const h = await factory(US_PUBLIC);
        check(h.publication !== undefined && h.publicDb !== undefined, "public harness carries the publication service");
        const publication = h.publication;
        const publicDb = h.publicDb;
        const tenantDb = h.stores[0];
        void publicDb;

        const { contentId } = await seed(h.plane, PAYLOAD("e"));
        tenantDb.createReference({ contentId, refClass: "standard", createdBy: WRITER, now: T0 });
        const pub = await publication.publish({ contentId, publisher: WRITER, authorizedDbs: [tenantDb], now: T0 });
        check(pub.created, "publication is recorded");

        // While published, minting works.
        const minted = await publication.mintPublicCapability({ contentId, now: atSec(5) });
        await verifyReadCapability(h.capabilityKey, contentId, h.partition, minted, atSec(5));

        // Unpublish stops new minting at once (no TTL lag on the
        // mint path); the already-minted capability runs out on its
        // own TTL and then fails closed.
        const res = publication.unpublish(pub.publicationId, { actor: WRITER, now: atSec(6) });
        check(res.removed === true, "unpublish removes the reference");
        await expectFailClosed(publication.mintPublicCapability({ contentId, now: atSec(7) }), "not_published");
        await expectFailClosed(publication.readPublished(contentId, { now: atSec(7) }), "not_published");
        await expectFailClosed(
          verifyReadCapability(h.capabilityKey, contentId, h.partition, minted, atSec(5 + 300)),
          "capability_expired",
        );
      },
    },
    {
      name: "(f) sole-referencer restricted erasure completes under the fence",
      run: async () => {
        const h = await factory(US_RESTRICTED);
        const { contentId } = await seed(h.plane, PAYLOAD("f"));
        const store = h.stores[0];
        const { row } = store.createReference({
          contentId,
          refClass: "restricted",
          createdBy: WRITER,
          canMintRestricted: true,
          now: T0,
        });
        h.index.notePublished(contentId, h.locator, { now: T0 });
        h.index.consume(store, { now: T0 });

        // Erasure is refused while the requester's own reference is
        // still live: remove it first.
        await expectFailClosed(h.gc.requestErasure(contentId, WRITER, h.stores, { now: T0 }), "erasure_reference_still_live");
        store.removeReference(row.reference_id, { actor: WRITER, now: at(1000) });
        h.index.consume(store, { now: at(1000) });

        const res = await h.gc.requestErasure(contentId, WRITER, h.stores, { now: at(2000) });
        check(res.erased === true && res.outcome === "deleted", "sole-referencer erasure deletes");
        check(res.audit?.reason === "erasure", "erasure is audited with its reason");
        check((await h.plane.head(contentId)) === null, "replica is gone after erasure");
        check(h.gc.admitReference(contentId).action === "restage", "post-erasure admission re-stages");

        // Shared bytes are never erased: the requester removes their
        // own reference but another live reference remains, so the
        // call records a note and deletes nothing.
        const h2 = await factory(US_RESTRICTED);
        const { contentId: secondId } = await seed(h2.plane, PAYLOAD("f2"));
        const mine = h2.stores[0].createReference({
          contentId: secondId,
          refClass: "restricted",
          createdBy: WRITER,
          canMintRestricted: true,
          now: T0,
        });
        h2.stores[1].createReference({
          contentId: secondId,
          refClass: "restricted",
          createdBy: OTHER,
          canMintRestricted: true,
          now: T0,
        });
        h2.stores[0].removeReference(mine.row.reference_id, { actor: WRITER, now: at(1000) });
        const shared = await h2.gc.requestErasure(secondId, WRITER, h2.stores, { now: at(2000) });
        check(shared.erased === false && shared.outcome === "shared", "shared bytes are not erased");
        check((await h2.plane.head(secondId)) !== null, "shared replica survives");
        check(h2.gc.erasureNotes().length === 1, "shared erasure records a note");
      },
    },
  ];
}
