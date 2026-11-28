// Object-plane adapter contract for the content-addressed
// media/data plane (component F of #9078, issue #12804, slice 2 of
// #9084; design record
// doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane,
// "Adapter and boundary requirements").
//
// This module is the provider-neutral interface that R2 (this slice)
// and RustFS (component I, #9087) both implement. Nothing here may
// assume R2-specific consistency, URL, multipart, or lifecycle
// behavior (#9087 peer principle): publication is the service's
// verification step, never multipart completion; reads are served
// from verified replicas with fallback during placement movement;
// deletes are conditional on the GC epoch fence; and the capability
// matrix states what the adapter can and cannot do so the shared
// conformance suite (object-plane-conformance.ts) only leans on
// declared strengths.
//
// Content identity is the sha256 content ID of #9106 Part A
// (references.ts parseContentId): a hash is an identifier, never a
// credential -- bytes are served on capability presentation only.

import { FailClosed } from "./routing.ts";
import { parseContentId, type Partition } from "./references.ts";

export type { Partition };

/** A short-lived, object+operation-scoped read capability. Minted
 *  only after the design section 5 access check grants (capability
 *  minting itself is slice #12807; this is the shape it must mint).
 *  The MAC binds every field, so a capability cannot be retargeted
 *  at another object, partition, range, or expiry. */
export interface ObjectReadCapability {
  contentId: string;
  jurisdiction: string;
  sensitivityTier: string;
  operation: "read";
  /** RFC 3339 expiry. Revocation lag is bounded by this TTL (design
   *  risks); capabilities expire on their own, they are never
   *  individually revoked. */
  expiresAt: string;
  /** Byte range, end-exclusive. Absent means the whole object. */
  rangeStart?: number;
  rangeEnd?: number;
  /** Hex HMAC-SHA256 over the canonical binding (see
   *  capabilityBindingString). */
  mac: string;
}

/** Result of staging bytes for later publication. Staging is the
 *  only way unverified content exists in the plane (design tier
 *  `staging`); staged bytes are never referenceable. */
export interface StagingReceipt {
  stagingId: string;
  contentId: string;
  sizeBytes: number;
  sha256: string;
  stagedAt: string;
}

/** What `head` reports. Sizes from object-store head calls are
 *  advisory; `read` verifies bytes against the content ID before
 *  serving, so a corrupt replica can never be served as verified. */
export interface ObjectHead {
  contentId: string;
  sizeBytes: number;
  /** Index into the adapter's replica set that answered: 0 is the
   *  primary, higher numbers are fallback replicas. */
  servedFromReplica: number;
}

export interface ReadResult {
  bytes: Uint8Array;
  head: ObjectHead;
}

/** The GC epoch fence (design section 6, sweep step 4). A delete
 *  records (content ID, epoch, re-verify timestamp); the adapter
 *  performs the replica delete only under this fence, and the delete
 *  is conditional/idempotent. */
export interface DeleteFence {
  epoch: string;
  /** RFC 3339 timestamp of the authoritative re-verify that cleared
   *  this content for collection. */
  reverifiedAt: string;
}

/** Audit record for every replica delete (design section 6: every
 *  delete emits an audit event). */
export interface DeleteAuditEvent {
  contentId: string;
  partition: Partition;
  epoch: string;
  reverifiedAt: string;
  deletedAt: string;
  /** Whether a published replica actually existed to remove.
   *  Deleting an absent object is a no-op success, never an error. */
  existed: boolean;
}

export type ConditionalWriteStrength = "none" | "if-none-match" | "if-match-etag";
export type RangeReadSupport = "none" | "server" | "client-slice";

/** The adapter capability matrix (design: "capability matrix
 *  (conditional-write strength, range reads, multipart bounds)").
 *  Recorded per provider so callers and the conformance suite never
 *  assume an undeclared strength. */
export interface AdapterCapabilityMatrix {
  provider: string;
  conditionalWrite: ConditionalWriteStrength;
  rangeReads: RangeReadSupport;
  multipart: { supported: false } | { supported: true; maxPartBytes: number; maxParts: number };
  /** Replica classes this adapter serves (e.g. standard; the CDN
   *  edge class arrives with publication semantics, slice #12807). */
  replicaClasses: string[];
  cdnEligible: boolean;
  /** Whether a published object is visible to head/read immediately
   *  after publish returns on this adapter. False means the suite
   *  must tolerate delayed visibility (never R2-specific: any
   *  eventually-consistent backend declares it here). */
  readAfterWrite: boolean;
  /** Seconds after which abandoned staged bytes may be reaped.
   *  Object-store lifecycle rules may reap staging only; they must
   *  never independently expire published content. */
  stagingTtlSeconds: number;
  notes: string[];
}

/** Sink for delete audit events. Wiring to durable audit storage is
 *  the GC worker's job (slice #12806); the adapter only emits. */
export type DeleteAuditSink = (event: DeleteAuditEvent) => void;

export interface VerifyExpectation {
  sizeBytes: number;
  sha256: string;
}

/** The object-plane adapter contract. All content IDs are validated
 *  as #9106 addresses on entry; every failure is a FailClosed, never
 *  a silent fallback. */
export interface ObjectPlaneAdapter {
  readonly partition: Partition;
  capabilities(): AdapterCapabilityMatrix;
  /** Stage bytes for publication. Idempotent on identical bytes:
   *  re-staging the same content returns an equal receipt without
   *  duplicating storage (duplicate-request conformance case). */
  putStaged(bytes: Uint8Array, opts?: { now?: string }): Promise<StagingReceipt>;
  /** Re-read staged bytes and check them against the expectation.
   *  Refuses (never publishes) on size or hash mismatch. */
  verify(stagingId: string, expected: VerifyExpectation): Promise<StagingReceipt>;
  /** Make staged bytes referenceable: re-read, verify checksum and
   *  size against the content ID, copy to the published key, and
   *  drop the staged copy. This service-side verification IS the
   *  publication step -- it is never multipart completion. */
  publish(stagingId: string, contentId: string, opts?: { now?: string }): Promise<ObjectHead>;
  /** Published-object metadata, or null when no verified replica
   *  is visible. Never synthesizes metadata for staged content. */
  head(contentId: string): Promise<ObjectHead | null>;
  /** Serve verified bytes under a capability. The capability's
   *  content, partition, operation, expiry, range, and MAC are all
   *  checked before any byte is touched; bytes are verified against
   *  the content ID on every serve, falling back to the next
   *  verified replica on mismatch. */
  read(contentId: string, capability: ObjectReadCapability, opts?: { now?: string }): Promise<ReadResult>;
  /** Remove the published replica under the GC epoch fence.
   *  Idempotent: absent content succeeds with existed=false. */
  deleteReplica(contentId: string, fence: DeleteFence, opts?: { now?: string }): Promise<{ deleted: boolean }>;
  /** Reap staged bytes (and only staged bytes) older than the
   *  cutoff. Published content is never touched here. */
  reapStaging(olderThan: string): Promise<{ reaped: number }>;
}

const CAPABILITY_BINDING_VERSION = "objcap/v1";

/** Canonical string the capability MAC covers. Every field that
 *  constrains the grant is bound; adding an optional field to the
 *  capability requires extending this string (and the version). */
export function capabilityBindingString(cap: Omit<ObjectReadCapability, "mac">): string {
  return [
    CAPABILITY_BINDING_VERSION,
    cap.operation,
    cap.jurisdiction,
    cap.sensitivityTier,
    cap.contentId,
    cap.expiresAt,
    cap.rangeStart ?? "",
    cap.rangeEnd ?? "",
  ].join("|");
}

async function hmacHex(key: Uint8Array, message: string): Promise<string> {
  const cryptoKey = await crypto.subtle.importKey(
    "raw",
    key as BufferSource,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", cryptoKey, new TextEncoder().encode(message) as BufferSource);
  return Array.from(new Uint8Array(sig), (b) => b.toString(16).padStart(2, "0")).join("");
}

export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Mint a read capability. The caller must have already passed the
 *  design section 5 access check (references.ts checkAccess); this
 *  function binds the grant, it does not authorize it. */
export async function mintReadCapability(
  key: Uint8Array,
  input: {
    contentId: string;
    partition: Partition;
    expiresAt: string;
    rangeStart?: number;
    rangeEnd?: number;
  },
): Promise<ObjectReadCapability> {
  const contentId = parseContentId(input.contentId);
  if (Number.isNaN(Date.parse(input.expiresAt))) {
    throw new FailClosed(400, "invalid_capability_expiry", "capability expiresAt is not a parseable timestamp");
  }
  if (input.rangeStart !== undefined || input.rangeEnd !== undefined) {
    if (
      input.rangeStart === undefined ||
      input.rangeEnd === undefined ||
      !Number.isInteger(input.rangeStart) ||
      !Number.isInteger(input.rangeEnd) ||
      input.rangeStart < 0 ||
      input.rangeEnd <= input.rangeStart
    ) {
      throw new FailClosed(
        400,
        "invalid_capability_range",
        "capability range must be integer [start, end) with end > start >= 0",
      );
    }
  }
  const unsigned = {
    contentId,
    jurisdiction: input.partition.jurisdiction,
    sensitivityTier: input.partition.tier,
    operation: "read" as const,
    expiresAt: input.expiresAt,
    ...(input.rangeStart === undefined ? {} : { rangeStart: input.rangeStart, rangeEnd: input.rangeEnd as number }),
  };
  return { ...unsigned, mac: await hmacHex(key, capabilityBindingString(unsigned)) };
}

/** Verify a presented capability against the binding key and the
 *  object being read. Throws FailClosed on any mismatch: unknown
 *  object, foreign partition, wrong operation, expiry, range shape,
 *  or MAC. Uses a constant-time MAC comparison so validity is not
 *  oracleable byte-by-byte. */
export async function verifyReadCapability(
  key: Uint8Array,
  contentId: string,
  partition: Partition,
  capability: ObjectReadCapability,
  now: string,
): Promise<void> {
  const expectedContent = parseContentId(contentId);
  if (capability.operation !== "read") {
    throw new FailClosed(403, "capability_wrong_operation", "capability does not grant read");
  }
  if (capability.contentId !== expectedContent) {
    throw new FailClosed(403, "capability_wrong_object", "capability is for a different object");
  }
  if (capability.jurisdiction !== partition.jurisdiction || capability.sensitivityTier !== partition.tier) {
    throw new FailClosed(403, "capability_wrong_partition", "capability is for a different partition");
  }
  const expiresMs = Date.parse(capability.expiresAt);
  if (Number.isNaN(expiresMs)) {
    throw new FailClosed(401, "capability_bad_expiry", "capability expiry is not parseable");
  }
  const nowMs = Date.parse(now);
  if (Number.isNaN(nowMs)) {
    throw new FailClosed(400, "capability_bad_now", "current time is not a parseable timestamp");
  }
  if (nowMs >= expiresMs) {
    throw new FailClosed(401, "capability_expired", "capability has expired");
  }
  const { mac, ...unsigned } = capability;
  const recomputed = await hmacHex(key, capabilityBindingString(unsigned));
  if (mac.length !== recomputed.length || !timingSafeEqualHex(mac, recomputed)) {
    throw new FailClosed(401, "capability_bad_mac", "capability MAC does not verify");
  }
}

function timingSafeEqualHex(a: string, b: string): boolean {
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}
