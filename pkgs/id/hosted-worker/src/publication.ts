// Publication semantics for the content-addressed media/data plane
// (component F of #9078, issue #12807, slice 5 of #9084; design record
// doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane,
// sections 3-6).
//
// Publication is a live reference record in the public database, never
// a byte copy (design section 3: "Publication is always a reference").
// This module owns four things:
//
// 1. publish: create a `publication` reference in the public DB for
//    content that already has a verified replica in the public
//    partition and that the publisher already references. Nothing is
//    staged, copied or re-published: the adapter's write path is never
//    touched.
// 2. capability minting from a publication reference (design section 5,
//    rule 2): a short-lived read capability is minted only while a live
//    publication reference exists.
// 3. the CDN path for the public tier (design section 4): a
//    publication-scoped signed URL minted from that reference, valid
//    only for the current CDN epoch and only on CDN-eligible adapters.
// 4. unpublish: reference removal. New capability minting stops the
//    moment the reference is tombstoned. Already-minted capabilities
//    expire on their own TTL, and CDN paths stop verifying at the next
//    epoch advance (the GC sweep's cadence). Invalidation is
//    epoch-paced, not instant; instant takedown would be a CDN-purge
//    integration slice, not a change to the access rule (design
//    "Risks").

import { FailClosed } from "./routing.ts";
import { checkAccess, parseContentId, type Partition, type ReferenceStore } from "./references.ts";
import {
  mintReadCapability,
  type ObjectPlaneAdapter,
  type ObjectReadCapability,
  type ReadResult,
} from "./object-plane.ts";

/** Minimal SQL surface, the same shape ReferenceStore uses. */
interface SqlStorage {
  exec(query: string, ...params: unknown[]): { toArray(): Record<string, unknown>[] };
}

export const DEFAULT_CAPABILITY_TTL_SECONDS = 300;
export const DEFAULT_CDN_PATH_TTL_SECONDS = 3600;
const CDN_BINDING_VERSION = "pubcdn/v1";

export const PUBLICATION_SCHEMA_STATEMENTS: readonly string[] = [
  `CREATE TABLE IF NOT EXISTS publication_cdn_epoch (
     id INTEGER PRIMARY KEY CHECK (id = 1),
     epoch INTEGER NOT NULL,
     advanced_at TEXT NOT NULL
   )`,
];

export interface PublicationServiceOptions {
  /** The public database's reference store. Must be the public DB at
   *  tier `public`. */
  publicDb: ReferenceStore;
  /** The public partition's object-plane adapter. */
  adapter: ObjectPlaneAdapter;
  /** HMAC key for read capabilities (the same key the adapter
   *  verifies with). */
  capabilityKey: Uint8Array;
  /** HMAC key for CDN paths. Defaults to the capability key. */
  cdnKey?: Uint8Array;
  capabilityTtlSeconds?: number;
  cdnPathTtlSeconds?: number;
}

export interface PublishInput {
  contentId: string;
  /** 32-hex binding id of the publisher. */
  publisher: string;
  /** Databases the publisher is authorized for (component E check,
   *  resolved by the caller). One of them must hold a live reference
   *  to the content in the public partition: a content hash alone is
   *  never enough to publish it. */
  authorizedDbs: ReferenceStore[];
  now?: string;
}

export interface PublishResult {
  publicationId: string;
  contentId: string;
  /** False when the publisher already had a live publication
   *  reference for this content (publish is idempotent). */
  created: boolean;
}

export interface UnpublishResult {
  removed: boolean;
  /** CDN paths minted before this unpublish stop verifying once the
   *  CDN epoch advances past this value. */
  cdnInvalidAfterEpoch: number;
}

export interface CdnPath {
  url: string;
  publicationId: string;
  epoch: number;
  expiresAt: string;
}

function addSeconds(now: string, seconds: number): string {
  const ms = Date.parse(now);
  if (Number.isNaN(ms)) {
    throw new FailClosed(400, "invalid_now", "current time is not a parseable timestamp");
  }
  return new Date(ms + seconds * 1000).toISOString();
}

function samePartition(a: Partition, b: Partition): boolean {
  return a.jurisdiction === b.jurisdiction && a.tier === b.tier;
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

function timingSafeEqualHex(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

export class PublicationService {
  private sql: SqlStorage;
  private publicDb: ReferenceStore;
  private adapter: ObjectPlaneAdapter;
  private capabilityKey: Uint8Array;
  private cdnKey: Uint8Array;
  private capabilityTtlSeconds: number;
  private cdnPathTtlSeconds: number;
  readonly partition: Partition;

  constructor(sql: SqlStorage, opts: PublicationServiceOptions) {
    if (!opts.publicDb.isPublicDb || opts.publicDb.partition.tier !== "public") {
      throw new FailClosed(
        500,
        "publication_requires_public_db",
        "publication references live only in the public database at tier public",
      );
    }
    if (!samePartition(opts.adapter.partition, opts.publicDb.partition)) {
      throw new FailClosed(
        500,
        "publication_partition_mismatch",
        "the publication adapter must serve the public database's partition",
      );
    }
    const capabilityTtl = opts.capabilityTtlSeconds ?? DEFAULT_CAPABILITY_TTL_SECONDS;
    const cdnTtl = opts.cdnPathTtlSeconds ?? DEFAULT_CDN_PATH_TTL_SECONDS;
    if (!(capabilityTtl > 0) || !(cdnTtl > 0)) {
      throw new FailClosed(500, "invalid_publication_ttl", "capability and CDN path TTLs must be positive");
    }
    this.sql = sql;
    this.publicDb = opts.publicDb;
    this.adapter = opts.adapter;
    this.capabilityKey = opts.capabilityKey;
    this.cdnKey = opts.cdnKey ?? opts.capabilityKey;
    this.capabilityTtlSeconds = capabilityTtl;
    this.cdnPathTtlSeconds = cdnTtl;
    this.partition = opts.publicDb.partition;
  }

  ensureSchema(): void {
    for (const stmt of PUBLICATION_SCHEMA_STATEMENTS) this.sql.exec(stmt);
  }

  /** Publish content: create a publication reference in the public
   *  DB. Requires a verified replica already visible in the public
   *  partition and a live reference held by the publisher in a
   *  database they are authorized for. Never stages, copies or
   *  re-publishes bytes. */
  async publish(input: PublishInput): Promise<PublishResult> {
    const contentId = parseContentId(input.contentId);
    const now = input.now ?? new Date().toISOString();
    const access = checkAccess({ contentId, partition: this.partition, authorizedDbs: input.authorizedDbs });
    if (!access.allowed || access.via !== "reference") {
      throw new FailClosed(
        403,
        "publication_requires_reference",
        "publishing requires a live reference to the content in an authorized database; a content hash is not a capability",
      );
    }
    const head = await this.adapter.head(contentId);
    if (head === null) {
      throw new FailClosed(
        409,
        "publication_requires_verified_replica",
        "content has no verified replica in the public partition",
      );
    }
    const { row, created } = this.publicDb.createReference({
      contentId,
      refClass: "publication",
      createdBy: input.publisher,
      now,
    });
    return { publicationId: row.reference_id, contentId, created };
  }

  /** The live publication reference for this content, if any. */
  private livePublication(contentId: string): string | null {
    const decision = checkAccess({ contentId, partition: this.partition, authorizedDbs: [], publicDb: this.publicDb });
    return decision.allowed && decision.via === "publication" ? decision.referenceId : null;
  }

  /** Mint a short-lived read capability from a live publication
   *  reference (design section 5, rule 2). Refused as soon as the
   *  publication reference is removed. */
  async mintPublicCapability(input: {
    contentId: string;
    now?: string;
    rangeStart?: number;
    rangeEnd?: number;
  }): Promise<ObjectReadCapability> {
    const contentId = parseContentId(input.contentId);
    const now = input.now ?? new Date().toISOString();
    if (this.livePublication(contentId) === null) {
      throw new FailClosed(403, "not_published", "content has no live publication reference");
    }
    return mintReadCapability(this.capabilityKey, {
      contentId,
      partition: this.partition,
      expiresAt: addSeconds(now, this.capabilityTtlSeconds),
      ...(input.rangeStart === undefined ? {} : { rangeStart: input.rangeStart, rangeEnd: input.rangeEnd }),
    });
  }

  /** Read published content through the publication reference
   *  alone: mint a capability, then serve verified bytes. */
  async readPublished(contentId: string, opts: { now?: string } = {}): Promise<ReadResult> {
    const now = opts.now ?? new Date().toISOString();
    const capability = await this.mintPublicCapability({ contentId, now });
    return this.adapter.read(contentId, capability, { now });
  }

  /** The current CDN epoch. Starts at 0. */
  currentEpoch(): number {
    const row = this.sql.exec("SELECT epoch FROM publication_cdn_epoch WHERE id = 1").toArray()[0];
    return row ? Number(row.epoch) : 0;
  }

  /** Advance the CDN epoch. Called once per GC sweep epoch (design
   *  section 6); CDN paths minted in earlier epochs stop verifying. */
  advanceCdnEpoch(opts: { now?: string } = {}): number {
    const now = opts.now ?? new Date().toISOString();
    const next = this.currentEpoch() + 1;
    this.sql.exec(
      `INSERT INTO publication_cdn_epoch (id, epoch, advanced_at) VALUES (1, ?, ?)
         ON CONFLICT(id) DO UPDATE SET epoch = excluded.epoch, advanced_at = excluded.advanced_at`,
      next,
      now,
    );
    return next;
  }

  private cdnBinding(p: {
    jurisdiction: string;
    contentId: string;
    publicationId: string;
    epoch: number;
    expiresAt: string;
  }): string {
    return [CDN_BINDING_VERSION, p.jurisdiction, p.contentId, p.publicationId, p.epoch, p.expiresAt].join("|");
  }

  /** Mint a publication-scoped signed CDN path for the public tier.
   *  Only CDN-eligible adapters serve one; the path binds the
   *  publication reference and the current epoch. */
  async cdnPath(contentId: string, opts: { now?: string } = {}): Promise<CdnPath> {
    const id = parseContentId(contentId);
    const now = opts.now ?? new Date().toISOString();
    if (!this.adapter.capabilities().cdnEligible) {
      throw new FailClosed(
        409,
        "cdn_not_eligible",
        "this adapter declares no CDN-eligible replica class; serve under a capability instead",
      );
    }
    const publicationId = this.livePublication(id);
    if (publicationId === null) {
      throw new FailClosed(403, "not_published", "content has no live publication reference");
    }
    const epoch = this.currentEpoch();
    const expiresAt = addSeconds(now, this.cdnPathTtlSeconds);
    const jurisdiction = this.partition.jurisdiction;
    const sig = await hmacHex(
      this.cdnKey,
      this.cdnBinding({ jurisdiction, contentId: id, publicationId, epoch, expiresAt }),
    );
    const query = new URLSearchParams({ pub: publicationId, epoch: String(epoch), exp: expiresAt, sig });
    return { url: `/cdn/v1/${jurisdiction}/${id}?${query.toString()}`, publicationId, epoch, expiresAt };
  }

  /** Verify a CDN path at the edge: signature, expiry, current epoch
   *  and a still-live publication reference. Throws FailClosed. */
  async verifyCdnPath(url: string, opts: { now?: string } = {}): Promise<{ contentId: string; publicationId: string }> {
    const now = opts.now ?? new Date().toISOString();
    const parsed = new URL(url, "https://cdn.invalid");
    const parts = parsed.pathname.split("/");
    if (parts.length !== 5 || parts[1] !== "cdn" || parts[2] !== "v1") {
      throw new FailClosed(400, "cdn_path_malformed", "not a v1 CDN path");
    }
    const jurisdiction = parts[3];
    const contentId = parseContentId(parts[4]);
    const publicationId = parsed.searchParams.get("pub") ?? "";
    const epochRaw = parsed.searchParams.get("epoch") ?? "";
    const expiresAt = parsed.searchParams.get("exp") ?? "";
    const sig = parsed.searchParams.get("sig") ?? "";
    const epoch = Number(epochRaw);
    if (!publicationId || !/^\d+$/.test(epochRaw) || !expiresAt || !sig) {
      throw new FailClosed(400, "cdn_path_malformed", "CDN path is missing a required parameter");
    }
    if (jurisdiction !== this.partition.jurisdiction) {
      throw new FailClosed(403, "cdn_path_wrong_partition", "CDN path is for a different jurisdiction");
    }
    const expected = await hmacHex(
      this.cdnKey,
      this.cdnBinding({ jurisdiction, contentId, publicationId, epoch, expiresAt }),
    );
    if (!timingSafeEqualHex(sig, expected)) {
      throw new FailClosed(401, "cdn_path_bad_signature", "CDN path signature does not verify");
    }
    const expiresMs = Date.parse(expiresAt);
    const nowMs = Date.parse(now);
    if (Number.isNaN(expiresMs) || Number.isNaN(nowMs) || nowMs >= expiresMs) {
      throw new FailClosed(401, "cdn_path_expired", "CDN path has expired");
    }
    if (epoch !== this.currentEpoch()) {
      throw new FailClosed(410, "cdn_path_stale_epoch", "CDN path was minted in an earlier epoch");
    }
    if (this.livePublication(contentId) !== publicationId) {
      throw new FailClosed(410, "cdn_path_unpublished", "the publication reference behind this CDN path is gone");
    }
    return { contentId, publicationId };
  }

  /** Unpublish: remove the publication reference. Only its publisher
   *  (or an admin) may. New capability minting stops immediately;
   *  minted capabilities expire on their TTL; CDN paths stop
   *  verifying after the next epoch advance. */
  unpublish(publicationId: string, opts: { actor: string; admin?: boolean; now?: string }): UnpublishResult {
    const now = opts.now ?? new Date().toISOString();
    const live = this.publicDb.liveReferences().find((r) => r.reference_id === publicationId);
    if (live && live.class !== "publication") {
      throw new FailClosed(404, "publication_not_found", `reference ${publicationId} is not a publication`);
    }
    const row = live;
    if (!row) {
      // Already tombstoned or never a publication: removeReference
      // fails closed on unknown ids and no-ops on tombstoned ones.
      const removed = this.publicDb.removeReference(publicationId, { actor: opts.actor, now });
      return { removed, cdnInvalidAfterEpoch: this.currentEpoch() };
    }
    if (row.created_by !== opts.actor && opts.admin !== true) {
      throw new FailClosed(403, "unpublish_not_authorized", "only the publisher or an admin may unpublish");
    }
    const removed = this.publicDb.removeReference(publicationId, { actor: opts.actor, now });
    return { removed, cdnInvalidAfterEpoch: this.currentEpoch() };
  }
}
