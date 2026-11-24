// Shared object-plane conformance suite (component F slice 2,
// issue #12804; design record
// doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane).
//
// Written against the ObjectPlaneAdapter interface ONLY
// (object-plane.ts): no R2-specific consistency, URL, multipart, or
// lifecycle behavior is assumed anywhere here, so the RustFS adapter
// (component I, #9087) runs this exact suite by supplying its own
// factory. Fault injection (delayed visibility, interrupted writes,
// corrupt replicas) happens through TestObjectBackend wrappers the
// suite provides -- adapters under test only ever see the interface.
//
// This module imports nothing Node- or worker-specific: the case
// runner returns plain { name, run } cases and the host test file
// (test/object-plane-r2.test.ts here) executes them under its own
// harness. Failure reporting uses plain Errors; FailClosed codes
// are asserted through expectFailClosed.

import { FailClosed } from "./routing.ts";
import { mintReadCapability } from "./object-plane.ts";
import type { ObjectPlaneAdapter, Partition } from "./object-plane.ts";

/** Minimal object-store surface the conformance backends speak.
//  Deliberately smaller than any real binding: an adapter that
//  passes against these fakes leans on nothing provider-specific. */
export interface TestObjectBackend {
  head(key: string): Promise<{ size: number } | null>;
  get(key: string): Promise<{ bytes: Uint8Array; size: number } | null>;
  put(key: string, bytes: Uint8Array): Promise<void>;
  delete(key: string): Promise<void>;
  list(prefix: string): Promise<string[]>;
}

export class MemoryTestBackend implements TestObjectBackend {
  private objects = new Map<string, Uint8Array>();

  async head(key: string): Promise<{ size: number } | null> {
    const found = this.objects.get(key);
    return found === undefined ? null : { size: found.length };
  }

  async get(key: string): Promise<{ bytes: Uint8Array; size: number } | null> {
    const found = this.objects.get(key);
    return found === undefined ? null : { bytes: found.slice(), size: found.length };
  }

  async put(key: string, bytes: Uint8Array): Promise<void> {
    this.objects.set(key, bytes.slice());
  }

  async delete(key: string): Promise<void> {
    this.objects.delete(key);
  }

  async list(prefix: string): Promise<string[]> {
    return [...this.objects.keys()].filter((k) => k.startsWith(prefix)).sort();
  }
}

/** Hides puts under the given prefixes from head/get until settled:
//  the delayed-visibility case. Staging traffic is untouched by
//  default (prefix-scoped to published objects), because publish
//  must re-read staged bytes through the same backend. */
export class DelayedVisibilityBackend implements TestObjectBackend {
  private hidden = new Set<string>();

  constructor(
    private inner: TestObjectBackend,
    private hiddenPrefixes: string[] = ["objects/"],
  ) {}

  settleAll(): void {
    this.hidden.clear();
  }

  settle(key: string): void {
    this.hidden.delete(key);
  }

  private visible(key: string): boolean {
    return !this.hidden.has(key);
  }

  async head(key: string): Promise<{ size: number } | null> {
    if (!this.visible(key)) return null;
    return this.inner.head(key);
  }

  async get(key: string): Promise<{ bytes: Uint8Array; size: number } | null> {
    if (!this.visible(key)) return null;
    return this.inner.get(key);
  }

  async put(key: string, bytes: Uint8Array): Promise<void> {
    await this.inner.put(key, bytes);
    if (this.hiddenPrefixes.some((p) => key.includes(p))) this.hidden.add(key);
  }

  async delete(key: string): Promise<void> {
    this.hidden.delete(key);
    await this.inner.delete(key);
  }

  async list(prefix: string): Promise<string[]> {
    return this.inner.list(prefix);
  }
}

/** Throws on the next put without writing: the interrupted-write
 *  (multipart-interruption analog) case. Single-part adapters have
 *  no resume protocol; retrying the whole put must converge. */
export class FailOnceBackend implements TestObjectBackend {
  failNextPut = true;

  constructor(private inner: TestObjectBackend) {}

  async head(key: string): Promise<{ size: number } | null> {
    return this.inner.head(key);
  }

  async get(key: string): Promise<{ bytes: Uint8Array; size: number } | null> {
    return this.inner.get(key);
  }

  async put(key: string, bytes: Uint8Array): Promise<void> {
    if (this.failNextPut) {
      this.failNextPut = false;
      throw new Error("injected write failure");
    }
    await this.inner.put(key, bytes);
  }

  async delete(key: string): Promise<void> {
    await this.inner.delete(key);
  }

  async list(prefix: string): Promise<string[]> {
    return this.inner.list(prefix);
  }
}

/** Serves bit-flipped bytes for armed keys: the corrupt-replica
 *  case. A verified-replica adapter must skip these, never serve
 *  them. */
export class CorruptingBackend implements TestObjectBackend {
  private corruptKeys = new Set<string>();

  constructor(private inner: TestObjectBackend) {}

  corrupt(keySubstring: string): void {
    this.corruptKeys.add(keySubstring);
  }

  private async maybeCorrupt(key: string, bytes: Uint8Array): Promise<Uint8Array> {
    for (const sub of this.corruptKeys) {
      if (key.includes(sub) && bytes.length > 0) {
        const out = bytes.slice();
        out[0] ^= 0xff;
        return out;
      }
    }
    return bytes;
  }

  async head(key: string): Promise<{ size: number } | null> {
    return this.inner.head(key);
  }

  async get(key: string): Promise<{ bytes: Uint8Array; size: number } | null> {
    const found = await this.inner.get(key);
    if (!found) return null;
    return { bytes: await this.maybeCorrupt(key, found.bytes), size: found.size };
  }

  async put(key: string, bytes: Uint8Array): Promise<void> {
    await this.inner.put(key, bytes);
  }

  async delete(key: string): Promise<void> {
    await this.inner.delete(key);
  }

  async list(prefix: string): Promise<string[]> {
    return this.inner.list(prefix);
  }
}

/** Builds an adapter over the given backends with the given
 *  capability key. The suite generates a fresh key per case, so
 *  capabilities minted in one case verify in none other. */
export type AdapterFactory = (
  key: Uint8Array,
  primary: TestObjectBackend,
  replicas?: TestObjectBackend[],
  partition?: Partition,
) => ObjectPlaneAdapter;

export interface ConformanceCase {
  name: string;
  run(): Promise<void>;
}

export const US_PRIVATE: Partition = { jurisdiction: "us", tier: "tenant-private" };
export const EU_PRIVATE: Partition = { jurisdiction: "eu", tier: "tenant-private" };

const NOW = "2026-11-24T12:00:00.000Z";
const FUTURE = "2030-01-01T00:00:00.000Z";
const PAST = "2020-01-01T00:00:00.000Z";
const FENCE = { epoch: "epoch-7", reverifiedAt: NOW };

const PAYLOAD = new TextEncoder().encode("hello object plane: staged, verified, published");
const OTHER_PAYLOAD = new TextEncoder().encode("a different object with different bytes inside");

function check(cond: unknown, message: string): asserts cond {
  if (!cond) throw new Error(`conformance: ${message}`);
}

function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

export async function expectFailClosed(work: Promise<unknown>, code: string): Promise<void> {
  try {
    await work;
  } catch (e) {
    if (e instanceof FailClosed && e.code === code) return;
    const got = e instanceof FailClosed ? `FailClosed(${e.code})` : String(e);
    throw new Error(`conformance: expected FailClosed(${code}), got ${got}`);
  }
  throw new Error(`conformance: expected FailClosed(${code}), got success`);
}

function freshKey(seed: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => (seed * 31 + i * 7) & 0xff);
}

/** The shared suite. Every case builds fresh backends through the
 *  factory, so no case can depend on another's state. */
export function objectPlaneConformanceCases(factory: AdapterFactory): ConformanceCase[] {
  return [
    {
      name: "put/verify/publish/head/read round trip serves verified bytes",
      run: async () => {
        const adapter = factory(freshKey(1), new MemoryTestBackend());
        const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
        check(receipt.sizeBytes === PAYLOAD.length, "receipt carries the staged size");
        check(receipt.contentId === receipt.sha256, "content id is the staged sha256");
        const verified = await adapter.verify(receipt.stagingId, { sizeBytes: PAYLOAD.length, sha256: receipt.sha256 });
        check(verified.stagingId === receipt.stagingId, "verify returns the same receipt");
        const head = await adapter.publish(receipt.stagingId, receipt.contentId);
        check(head.sizeBytes === PAYLOAD.length, "published head carries the size");
        check(head.servedFromReplica === 0, "fresh publish is served from the primary");
        const seen = await adapter.head(receipt.contentId);
        check(seen !== null && seen.sizeBytes === PAYLOAD.length, "head finds the published object");
        const cap = await mintReadCapability(freshKey(1), { contentId: receipt.contentId, partition: US_PRIVATE, expiresAt: FUTURE });
        const read = await adapter.read(receipt.contentId, cap, { now: NOW });
        check(sameBytes(read.bytes, PAYLOAD), "read serves the exact staged bytes");
      },
    },
    {
      name: "duplicate put_staged converges on one receipt",
      run: async () => {
        const adapter = factory(freshKey(2), new MemoryTestBackend());
        const first = await adapter.putStaged(PAYLOAD, { now: NOW });
        const second = await adapter.putStaged(PAYLOAD.slice(), { now: NOW });
        check(first.stagingId === second.stagingId, "duplicate stage returns the same staging id");
        check(first.contentId === second.contentId, "duplicate stage returns the same content id");
        await adapter.publish(first.stagingId, first.contentId);
        const seen = await adapter.head(first.contentId);
        check(seen !== null && seen.sizeBytes === PAYLOAD.length, "one publish after duplicate stages");
      },
    },
    {
      name: "interrupted write retries cleanly with nothing half-published",
      run: async () => {
        const backend = new FailOnceBackend(new MemoryTestBackend());
        const adapter = factory(freshKey(3), backend);
        let failed = false;
        try {
          await adapter.putStaged(PAYLOAD, { now: NOW });
        } catch {
          failed = true;
        }
        check(failed, "injected write failure surfaces");
        const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
        check((await adapter.head(receipt.contentId)) === null, "failed attempt published nothing");
        await adapter.publish(receipt.stagingId, receipt.contentId);
        const seen = await adapter.head(receipt.contentId);
        check(seen !== null && seen.sizeBytes === PAYLOAD.length, "retry converges to a published object");
      },
    },
    {
      name: "orphan staged bytes cannot verify and are reaped",
      run: async () => {
        const backend = new MemoryTestBackend();
        const adapter = factory(freshKey(4), backend);
        // Bypass the adapter: raw bytes with no receipt sidecar.
        await backend.put("staging/us/tenant-private/stg_" + "00".repeat(32), PAYLOAD);
        await expectFailClosed(
          adapter.verify("stg_" + "00".repeat(32), { sizeBytes: PAYLOAD.length, sha256: receiptSha(PAYLOAD) }),
          "staging_receipt_missing",
        );
        const reaped = await adapter.reapStaging(FUTURE);
        check(reaped.reaped === 1, "orphan staged bytes are reaped");
      },
    },
    {
      name: "checksum mismatch refuses verification and publication",
      run: async () => {
        const adapter = factory(freshKey(5), new MemoryTestBackend());
        const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
        await expectFailClosed(
          adapter.verify(receipt.stagingId, { sizeBytes: PAYLOAD.length + 1, sha256: receipt.sha256 }),
          "staged_checksum_mismatch",
        );
        await expectFailClosed(
          adapter.verify(receipt.stagingId, { sizeBytes: PAYLOAD.length, sha256: "ff".repeat(32) }),
          "staged_checksum_mismatch",
        );
        await expectFailClosed(adapter.publish(receipt.stagingId, "ff".repeat(32)), "publish_staging_mismatch");
        check((await adapter.head(receipt.contentId)) === null, "refused content is never published");
      },
    },
    {
      name: "delayed visibility is tolerated and settles to verified bytes",
      run: async () => {
        const backend = new DelayedVisibilityBackend(new MemoryTestBackend());
        const adapter = factory(freshKey(6), backend);
        const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
        await adapter.publish(receipt.stagingId, receipt.contentId);
        const early = await adapter.head(receipt.contentId);
        check(early === null || early.sizeBytes === PAYLOAD.length, "pre-settle head is absent or correct, never wrong");
        backend.settleAll();
        const seen = await adapter.head(receipt.contentId);
        check(seen !== null && seen.sizeBytes === PAYLOAD.length, "post-settle head finds the object");
        const cap = await mintReadCapability(freshKey(6), { contentId: receipt.contentId, partition: US_PRIVATE, expiresAt: FUTURE });
        const read = await adapter.read(receipt.contentId, cap, { now: NOW });
        check(sameBytes(read.bytes, PAYLOAD), "post-settle read serves verified bytes");
      },
    },
    {
      name: "corrupt primary falls back to a verified replica",
      run: async () => {
        const full = new MemoryTestBackend();
        const seeder = factory(freshKey(7), full);
        const receipt = await seeder.putStaged(PAYLOAD, { now: NOW });
        await seeder.publish(receipt.stagingId, receipt.contentId);
        const corrupting = new CorruptingBackend(full);
        corrupting.corrupt(receipt.contentId);
        const adapter = factory(freshKey(7), corrupting, [full]);
        const cap = await mintReadCapability(freshKey(7), { contentId: receipt.contentId, partition: US_PRIVATE, expiresAt: FUTURE });
        const read = await adapter.read(receipt.contentId, cap, { now: NOW });
        check(sameBytes(read.bytes, PAYLOAD), "fallback serves the verified replica bytes");
        check(read.head.servedFromReplica === 1, "corrupt primary is skipped for the replica");
      },
    },
    {
      name: "missing primary serves from a verified replica",
      run: async () => {
        const full = new MemoryTestBackend();
        const seeder = factory(freshKey(8), full);
        const receipt = await seeder.putStaged(OTHER_PAYLOAD, { now: NOW });
        await seeder.publish(receipt.stagingId, receipt.contentId);
        const adapter = factory(freshKey(8), new MemoryTestBackend(), [full]);
        const seen = await adapter.head(receipt.contentId);
        check(seen !== null && seen.servedFromReplica === 1, "head finds the replica copy");
        const cap = await mintReadCapability(freshKey(8), { contentId: receipt.contentId, partition: US_PRIVATE, expiresAt: FUTURE });
        const read = await adapter.read(receipt.contentId, cap, { now: NOW });
        check(sameBytes(read.bytes, OTHER_PAYLOAD), "read serves the replica bytes");
      },
    },
    {
      name: "capability scoping is enforced before any byte is touched",
      run: async () => {
        const key = freshKey(9);
        const adapter = factory(key, new MemoryTestBackend());
        const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
        await adapter.publish(receipt.stagingId, receipt.contentId);
        const good = await mintReadCapability(key, { contentId: receipt.contentId, partition: US_PRIVATE, expiresAt: FUTURE });
        // Control: the valid capability works.
        check((await adapter.read(receipt.contentId, good, { now: NOW })).bytes.length === PAYLOAD.length, "control read works");
        // Wrong object.
        const other = await mintReadCapability(key, { contentId: "ff".repeat(32), partition: US_PRIVATE, expiresAt: FUTURE });
        await expectFailClosed(adapter.read(receipt.contentId, other, { now: NOW }), "capability_wrong_object");
        // Expired.
        const expired = await mintReadCapability(key, { contentId: receipt.contentId, partition: US_PRIVATE, expiresAt: PAST });
        await expectFailClosed(adapter.read(receipt.contentId, expired, { now: NOW }), "capability_expired");
        // Foreign partition.
        const foreign = await mintReadCapability(key, { contentId: receipt.contentId, partition: EU_PRIVATE, expiresAt: FUTURE });
        await expectFailClosed(adapter.read(receipt.contentId, foreign, { now: NOW }), "capability_wrong_partition");
        // Tampered MAC.
        await expectFailClosed(
          adapter.read(receipt.contentId, { ...good, mac: "00".repeat(32) }, { now: NOW }),
          "capability_bad_mac",
        );
      },
    },
    {
      name: "a hash alone grants nothing; ranged reads stay inside the object",
      run: async () => {
        const key = freshKey(10);
        const adapter = factory(key, new MemoryTestBackend());
        const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
        await adapter.publish(receipt.stagingId, receipt.contentId);
        const ranged = await mintReadCapability(key, {
          contentId: receipt.contentId,
          partition: US_PRIVATE,
          expiresAt: FUTURE,
          rangeStart: 6,
          rangeEnd: 12,
        });
        const sliced = await adapter.read(receipt.contentId, ranged, { now: NOW });
        check(sameBytes(sliced.bytes, PAYLOAD.slice(6, 12)), "ranged read returns exactly the granted slice");
        const over = await mintReadCapability(key, {
          contentId: receipt.contentId,
          partition: US_PRIVATE,
          expiresAt: FUTURE,
          rangeStart: 0,
          rangeEnd: PAYLOAD.length + 100,
        });
        await expectFailClosed(adapter.read(receipt.contentId, over, { now: NOW }), "range_not_satisfiable");
      },
    },
    {
      name: "staged content is not referenceable until published",
      run: async () => {
        const key = freshKey(11);
        const adapter = factory(key, new MemoryTestBackend());
        const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
        check((await adapter.head(receipt.contentId)) === null, "staged content has no head");
        const cap = await mintReadCapability(key, { contentId: receipt.contentId, partition: US_PRIVATE, expiresAt: FUTURE });
        await expectFailClosed(adapter.read(receipt.contentId, cap, { now: NOW }), "object_not_found");
      },
    },
    {
      name: "replica delete is conditional on the epoch fence and idempotent",
      run: async () => {
        const audits: { epoch: string; existed: boolean }[] = [];
        void audits;
        const key = freshKey(12);
        const backend = new MemoryTestBackend();
        const adapter = factory(key, backend);
        const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
        await adapter.publish(receipt.stagingId, receipt.contentId);
        await expectFailClosed(adapter.deleteReplica(receipt.contentId, { epoch: "", reverifiedAt: NOW }), "delete_fence_invalid");
        await expectFailClosed(
          adapter.deleteReplica(receipt.contentId, { epoch: "epoch-7", reverifiedAt: "not-a-time" }),
          "delete_fence_invalid",
        );
        await expectFailClosed(
          adapter.deleteReplica(receipt.contentId, { epoch: "epoch-7", reverifiedAt: FUTURE }, { now: NOW }),
          "delete_fence_invalid",
        );
        check((await adapter.head(receipt.contentId)) !== null, "refused deletes remove nothing");
        const first = await adapter.deleteReplica(receipt.contentId, FENCE, { now: NOW });
        check(first.deleted === true, "fenced delete removes the replica");
        check((await adapter.head(receipt.contentId)) === null, "deleted content has no head");
        const cap = await mintReadCapability(key, { contentId: receipt.contentId, partition: US_PRIVATE, expiresAt: FUTURE });
        await expectFailClosed(adapter.read(receipt.contentId, cap, { now: NOW }), "object_not_found");
        const second = await adapter.deleteReplica(receipt.contentId, FENCE, { now: NOW });
        check(second.deleted === false, "deleting an absent replica succeeds idempotently");
      },
    },
    {
      name: "reaping spares published content and only takes expired staging",
      run: async () => {
        const adapter = factory(freshKey(13), new MemoryTestBackend());
        const published = await adapter.putStaged(PAYLOAD, { now: NOW });
        await adapter.publish(published.stagingId, published.contentId);
        const staged = await adapter.putStaged(OTHER_PAYLOAD, { now: NOW });
        // Nothing is older than the past: nothing reaped.
        check((await adapter.reapStaging(PAST)).reaped === 0, "fresh staging survives a past cutoff");
        // Everything staged is older than the future: staged goes, published stays.
        check((await adapter.reapStaging(FUTURE)).reaped === 1, "expired staging is reaped");
        check((await adapter.head(published.contentId))?.sizeBytes === PAYLOAD.length, "published content survives reaping");
        await expectFailClosed(
          adapter.verify(staged.stagingId, { sizeBytes: OTHER_PAYLOAD.length, sha256: staged.sha256 }),
          "staged_not_found",
        );
      },
    },
    {
      name: "adapter capabilities declare an honest matrix",
      run: async () => {
        const adapter = factory(freshKey(14), new MemoryTestBackend());
        const matrix = adapter.capabilities();
        check(matrix.provider.length > 0, "matrix names its provider");
        check(
          matrix.conditionalWrite === "none" || matrix.conditionalWrite === "if-none-match" || matrix.conditionalWrite === "if-match-etag",
          "matrix states a known conditional-write strength",
        );
        check(matrix.rangeReads === "none" || matrix.rangeReads === "server" || matrix.rangeReads === "client-slice", "matrix states range support");
        check(matrix.stagingTtlSeconds > 0, "matrix states a positive staging TTL");
        check(typeof matrix.readAfterWrite === "boolean", "matrix states its read-after-write behavior");
      },
    },
  ];
}

/** sha256 of bytes, hex. The orphan-staging case needs an expectation
 *  for bytes it never staged through an adapter. */
function receiptSha(bytes: Uint8Array): string {
  // Computed synchronously is impossible for SHA-256; the case only
  // needs *a* well-formed wrong-or-right hash, and verify reads the
  // actual staged bytes. Any 64-hex string exercises the
  // receipt-missing path before hashing matters.
  void bytes;
  return "ab".repeat(32);
}
