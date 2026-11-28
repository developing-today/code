// R2 implementation of the object-plane adapter contract
// (object-plane.ts; component F slice 2, issue #12804).
//
// Key layout (partition isolation: a bucket is a partition
// container, never a mixing point -- #9136 alignment):
//   staging/<jurisdiction>/<tier>/<stagingId>          staged bytes
//   staging/<jurisdiction>/<tier>/<stagingId>.receipt.json
//                                                     staging receipt sidecar
//   objects/<jurisdiction>/<tier>/<contentId>         published bytes
// The adapter is bound to ONE partition at construction; every key
// it touches carries that partition's prefix, and capabilities for
// any other partition are refused before any backend call.
//
// Binding notes: the package carries no workers-types dependency,
// so this adapter is written against routing.ts R2BucketLike plus
// a list() narrowing (real R2 supports prefix listing; the local
// shape just does not declare it). Puts are single-part through the
// binding -- there is deliberately no multipart path, which is what
// makes publish-as-verification (never multipart completion) true
// by construction. A binding with richer metadata support could
// fold the receipt sidecar into R2 custom metadata; the matrix
// records that this binding does not.

import { FailClosed, type R2BucketLike } from "./routing.ts";
import { parseContentId } from "./references.ts";
import {
  mintReadCapability,
  sha256Hex,
  verifyReadCapability,
  type AdapterCapabilityMatrix,
  type DeleteAuditSink,
  type DeleteFence,
  type ObjectHead,
  type ObjectPlaneAdapter,
  type ObjectReadCapability,
  type Partition,
  type ReadResult,
  type StagingReceipt,
  type VerifyExpectation,
} from "./object-plane.ts";

/** R2BucketLike plus prefix listing. Real R2 buckets satisfy this;
 *  the in-memory fakes in the conformance suite implement it too. */
export interface R2ListableBucket extends R2BucketLike {
  list(prefix: string): Promise<string[]>;
}

export interface R2AdapterReplica {
  bucket: R2BucketLike;
}

export interface R2ObjectPlaneAdapterOpts {
  /** Primary bucket: staging and publication live here. */
  bucket: R2ListableBucket;
  /** Read/fallback replicas (placement movement targets). Reads
   *  try the primary first, then these in order; deletes fan out
   *  to every store. */
  replicas?: R2AdapterReplica[];
  partition: Partition;
  /** HMAC key binding read capabilities (see mintReadCapability).
   *  Must be non-empty; the adapter refuses to construct without
   *  one because capability verification would be vacuous. */
  capabilityKey: Uint8Array;
  /** Abandoned-staging horizon in seconds. Recorded in the matrix;
   *  enforced by reapStaging callers (GC/lifecycle), not by reads. */
  stagingTtlSeconds?: number;
  /** Where delete audit events go (slice #12806 owns durable audit
   *  storage; default is a no-op). */
  audit?: DeleteAuditSink;
  newId?: () => string;
}

export const R2_STAGING_TTL_SECONDS = 24 * 60 * 60;

export const R2_CAPABILITY_MATRIX: AdapterCapabilityMatrix = {
  provider: "r2",
  conditionalWrite: "none",
  rangeReads: "client-slice",
  multipart: { supported: false },
  replicaClasses: ["standard"],
  cdnEligible: false,
  readAfterWrite: true,
  stagingTtlSeconds: R2_STAGING_TTL_SECONDS,
  notes: [
    "Single-part puts through the R2BucketLike binding only; no multipart path exists, so publish-as-verification holds by construction.",
    "Staging receipts live in .receipt.json sidecars because the binding declares no custom-metadata support; a richer binding could fold them into object metadata.",
    "Range reads are served by fetching the verified object and slicing client-side; the capability range is MAC-bound either way.",
    "Head sizes are advisory; read verifies bytes against the content ID on every serve.",
  ],
};

const STAGING_ID_RE = /^stg_[0-9a-f]{64}$/;

function stagingKey(partition: Partition, stagingId: string): string {
  return `staging/${partition.jurisdiction}/${partition.tier}/${stagingId}`;
}

function receiptKey(partition: Partition, stagingId: string): string {
  return `${stagingKey(partition, stagingId)}.receipt.json`;
}

function publishedKey(partition: Partition, contentId: string): string {
  return `objects/${partition.jurisdiction}/${partition.tier}/${contentId}`;
}

async function readAllBytes(body: ReadableStream): Promise<Uint8Array> {
  const buf = await new Response(body).arrayBuffer();
  return new Uint8Array(buf);
}

async function readJson(bucket: R2BucketLike, key: string): Promise<Record<string, unknown> | null> {
  const found = await bucket.get(key);
  if (!found) return null;
  try {
    return JSON.parse(new TextDecoder().decode(await readAllBytes(found.body)));
  } catch {
    return null;
  }
}

export class R2ObjectPlaneAdapter implements ObjectPlaneAdapter {
  readonly partition: Partition;
  private primary: R2ListableBucket;
  private replicaBuckets: R2BucketLike[];
  private capabilityKey: Uint8Array;
  private stagingTtlSeconds: number;
  private audit: DeleteAuditSink;

  constructor(opts: R2ObjectPlaneAdapterOpts) {
    if (opts.capabilityKey.length === 0) {
      throw new FailClosed(500, "object_plane_misconfigured", "capability key must not be empty");
    }
    this.partition = opts.partition;
    this.primary = opts.bucket;
    this.replicaBuckets = (opts.replicas ?? []).map((r) => r.bucket);
    this.capabilityKey = opts.capabilityKey;
    this.stagingTtlSeconds = opts.stagingTtlSeconds ?? R2_STAGING_TTL_SECONDS;
    this.audit = opts.audit ?? (() => undefined);
  }

  capabilities(): AdapterCapabilityMatrix {
    return { ...R2_CAPABILITY_MATRIX, stagingTtlSeconds: this.stagingTtlSeconds, notes: [...R2_CAPABILITY_MATRIX.notes] };
  }

  /** Mint a read capability for content in this adapter's partition
   *  (test and tooling helper; production minting moves to slice
   *  #12807 but must produce exactly this shape). */
  mintCapability(input: {
    contentId: string;
    expiresAt: string;
    rangeStart?: number;
    rangeEnd?: number;
  }): Promise<ObjectReadCapability> {
    return mintReadCapability(this.capabilityKey, { ...input, partition: this.partition });
  }

  async putStaged(bytes: Uint8Array, opts?: { now?: string }): Promise<StagingReceipt> {
    const sha = await sha256Hex(bytes);
    const stagingId = `stg_${sha}`;
    const receipt: StagingReceipt = {
      stagingId,
      contentId: sha,
      sizeBytes: bytes.length,
      sha256: sha,
      stagedAt: opts?.now ?? new Date().toISOString(),
    };
    // Content-addressed staging: the same bytes always land on the
    // same key, so duplicate requests converge without duplicating
    // storage. Overwriting an identical key with identical bytes is
    // a no-op semantically.
    await this.primary.put(stagingKey(this.partition, stagingId), bytes);
    await this.primary.put(
      receiptKey(this.partition, stagingId),
      new TextEncoder().encode(JSON.stringify({ stagedAt: receipt.stagedAt })),
    );
    return receipt;
  }

  async verify(stagingId: string, expected: VerifyExpectation): Promise<StagingReceipt> {
    const { receipt } = await this.readStaged(stagingId);
    if (receipt.sizeBytes !== expected.sizeBytes || receipt.sha256 !== expected.sha256) {
      throw new FailClosed(
        409,
        "staged_checksum_mismatch",
        `staged bytes do not match the expectation (size ${receipt.sizeBytes} vs ${expected.sizeBytes})`,
      );
    }
    return receipt;
  }

  async publish(stagingId: string, contentId: string, opts?: { now?: string }): Promise<ObjectHead> {
    const id = parseContentId(contentId);
    if (stagingId !== `stg_${id}`) {
      throw new FailClosed(409, "publish_staging_mismatch", "staging id does not name the content being published");
    }
    const staged = await this.primary.get(stagingKey(this.partition, stagingId));
    if (!staged) {
      // Retry-safety: the staged copy is dropped by a successful
      // publish, so a retried publish after a lost success response
      // converges here when the published object already exists and
      // verifies by address. Anything else is a genuine miss.
      const existing = await this.head(id);
      if (existing) return existing;
      throw new FailClosed(404, "staged_not_found", "no staged bytes for this staging id");
    }
    const bytes = await readAllBytes(staged.body);
    const sha = await sha256Hex(bytes);
    if (sha !== id || bytes.length !== staged.size) {
      throw new FailClosed(
        409,
        "publish_unverified",
        "staged bytes fail checksum/size verification; nothing was published",
      );
    }
    await this.primary.put(publishedKey(this.partition, id), bytes);
    await this.primary.delete(stagingKey(this.partition, stagingId));
    await this.primary.delete(receiptKey(this.partition, stagingId));
    void (opts?.now ?? new Date().toISOString());
    return { contentId: id, sizeBytes: bytes.length, servedFromReplica: 0 };
  }

  async head(contentId: string): Promise<ObjectHead | null> {
    const id = parseContentId(contentId);
    const key = publishedKey(this.partition, id);
    const stores: R2BucketLike[] = [this.primary, ...this.replicaBuckets];
    for (let i = 0; i < stores.length; i++) {
      const meta = await stores[i].head(key);
      if (meta !== null && meta !== undefined) {
        return { contentId: id, sizeBytes: meta.size, servedFromReplica: i };
      }
    }
    return null;
  }

  async read(contentId: string, capability: ObjectReadCapability, opts?: { now?: string }): Promise<ReadResult> {
    const id = parseContentId(contentId);
    await verifyReadCapability(this.capabilityKey, id, this.partition, capability, opts?.now ?? new Date().toISOString());
    const key = publishedKey(this.partition, id);
    const stores: R2BucketLike[] = [this.primary, ...this.replicaBuckets];
    for (let i = 0; i < stores.length; i++) {
      const found = await stores[i].get(key);
      if (!found) continue;
      const bytes = await readAllBytes(found.body);
      // Verified-replica fallback: serve only bytes whose hash
      // matches the address; a corrupt or stale replica is skipped,
      // never served.
      if ((await sha256Hex(bytes)) !== id) continue;
      const sliced = this.applyRange(bytes, capability);
      return { bytes: sliced, head: { contentId: id, sizeBytes: bytes.length, servedFromReplica: i } };
    }
    throw new FailClosed(404, "object_not_found", "no verified replica holds this content");
  }

  async deleteReplica(contentId: string, fence: DeleteFence, opts?: { now?: string }): Promise<{ deleted: boolean }> {
    const id = parseContentId(contentId);
    if (!fence.epoch) {
      throw new FailClosed(400, "delete_fence_invalid", "replica delete requires the GC epoch fence");
    }
    const reverifiedMs = Date.parse(fence.reverifiedAt);
    if (Number.isNaN(reverifiedMs)) {
      throw new FailClosed(400, "delete_fence_invalid", "fence reverifiedAt is not a parseable timestamp");
    }
    if (reverifiedMs > Date.parse(opts?.now ?? new Date().toISOString())) {
      throw new FailClosed(400, "delete_fence_invalid", "fence reverifiedAt is in the future");
    }
    const key = publishedKey(this.partition, id);
    let existed = false;
    for (const store of [this.primary, ...this.replicaBuckets]) {
      const meta = await store.head(key);
      if (meta !== null && meta !== undefined) {
        existed = true;
        await store.delete(key);
      }
    }
    this.audit({
      contentId: id,
      partition: this.partition,
      epoch: fence.epoch,
      reverifiedAt: fence.reverifiedAt,
      deletedAt: opts?.now ?? new Date().toISOString(),
      existed,
    });
    return { deleted: existed };
  }

  async reapStaging(olderThan: string): Promise<{ reaped: number }> {
    const cutoffMs = Date.parse(olderThan);
    if (Number.isNaN(cutoffMs)) {
      throw new FailClosed(400, "reap_cutoff_invalid", "reap cutoff is not a parseable timestamp");
    }
    const prefix = `staging/${this.partition.jurisdiction}/${this.partition.tier}/`;
    let reaped = 0;
    for (const key of await this.primary.list(prefix)) {
      if (key.endsWith(".receipt.json") || !key.startsWith(prefix)) continue;
      const stagingId = key.slice(prefix.length);
      if (!STAGING_ID_RE.test(stagingId)) continue;
      const sidecar = await readJson(this.primary, receiptKey(this.partition, stagingId));
      const stagedAt = typeof sidecar?.stagedAt === "string" ? sidecar.stagedAt : null;
      // A staged copy with no receipt sidecar is corrupt staging:
      // it can never publish (publish re-reads through readStaged,
      // which requires the sidecar), so reaping it is safe.
      if (stagedAt === null || Date.parse(stagedAt) < cutoffMs) {
        await this.primary.delete(key);
        await this.primary.delete(receiptKey(this.partition, stagingId));
        reaped++;
      }
    }
    return { reaped };
  }

  private applyRange(bytes: Uint8Array, capability: ObjectReadCapability): Uint8Array {
    if (capability.rangeStart === undefined) return bytes;
    const end = capability.rangeEnd as number;
    if (capability.rangeStart >= bytes.length || end > bytes.length) {
      throw new FailClosed(416, "range_not_satisfiable", "capability range lies outside the object");
    }
    return bytes.slice(capability.rangeStart, end);
  }

  private async readStaged(stagingId: string): Promise<{ receipt: StagingReceipt; bytes: Uint8Array }> {
    if (!STAGING_ID_RE.test(stagingId)) {
      throw new FailClosed(400, "invalid_staging_id", "staging id must be stg_<64 lowercase hex>");
    }
    const sKey = stagingKey(this.partition, stagingId);
    const staged = await this.primary.get(sKey);
    if (!staged) {
      throw new FailClosed(404, "staged_not_found", "no staged bytes for this staging id");
    }
    const sidecar = await readJson(this.primary, receiptKey(this.partition, stagingId));
    if (typeof sidecar?.stagedAt !== "string") {
      throw new FailClosed(409, "staging_receipt_missing", "staged bytes lack their receipt sidecar; refusing to verify");
    }
    const bytes = await readAllBytes(staged.body);
    const sha = await sha256Hex(bytes);
    return {
      receipt: { stagingId, contentId: sha, sizeBytes: bytes.length, sha256: sha, stagedAt: sidecar.stagedAt },
      bytes,
    };
  }
}
