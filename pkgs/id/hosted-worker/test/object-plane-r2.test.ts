// Issue #12804 (component F slice 2 of #9084): the R2 object-plane
// adapter against the real contract, proven by running the shared
// adapter conformance suite (object-plane-conformance.ts -- the same
// runner the RustFS adapter of component I, #9087, will use) over an
// in-memory R2BucketLike, plus R2-specific assertions on the
// capability matrix, partition key isolation, audit emission, and
// publish retry-safety.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  EU_PRIVATE,
  objectPlaneConformanceCases,
  US_PRIVATE,
  type AdapterFactory,
  type TestObjectBackend,
} from "../src/object-plane-conformance.ts";
import { mintReadCapability } from "../src/object-plane.ts";
import { R2ObjectPlaneAdapter, type R2ListableBucket } from "../src/object-plane-r2.ts";
import type { Partition } from "../src/references.ts";

const KEY = Uint8Array.from({ length: 32 }, (_, i) => (i * 13 + 5) & 0xff);
const NOW = "2026-11-24T12:00:00.000Z";
const FUTURE = "2030-01-01T00:00:00.000Z";
const PAYLOAD = new TextEncoder().encode("r2 adapter case payload");

function streamOf(bytes: Uint8Array): ReadableStream {
  return new ReadableStream({
    start(controller) {
      controller.enqueue(bytes);
      controller.close();
    },
  });
}

/** An R2BucketLike over a conformance backend. The adapter under
 *  test only ever sees this binding surface. */
function asR2Bucket(backend: TestObjectBackend): R2ListableBucket {
  return {
    head: async (key: string) => {
      const found = await backend.head(key);
      return found === null ? null : { size: found.size };
    },
    get: async (key: string) => {
      const found = await backend.get(key);
      return found === null ? null : { body: streamOf(found.bytes), size: found.size };
    },
    put: async (key: string, value: ArrayBuffer | Uint8Array) => {
      await backend.put(key, value instanceof Uint8Array ? value : new Uint8Array(value));
    },
    delete: (key: string) => backend.delete(key),
    list: (prefix: string) => backend.list(prefix),
  };
}

class MapBackend implements TestObjectBackend {
  objects = new Map<string, Uint8Array>();

  async head(key: string) {
    const found = this.objects.get(key);
    return found === undefined ? null : { size: found.length };
  }

  async get(key: string) {
    const found = this.objects.get(key);
    return found === undefined ? null : { bytes: found.slice(), size: found.length };
  }

  async put(key: string, bytes: Uint8Array) {
    this.objects.set(key, bytes.slice());
  }

  async delete(key: string) {
    this.objects.delete(key);
  }

  async list(prefix: string) {
    return [...this.objects.keys()].filter((k) => k.startsWith(prefix)).sort();
  }
}

const factory: AdapterFactory = (key, primary, replicas, partition) =>
  new R2ObjectPlaneAdapter({
    bucket: asR2Bucket(primary),
    replicas: (replicas ?? []).map((backend) => ({ bucket: asR2Bucket(backend) })),
    partition: partition ?? US_PRIVATE,
    capabilityKey: key,
  });

for (const c of objectPlaneConformanceCases(factory)) {
  test(`conformance: ${c.name}`, () => c.run());
}

function makeAdapter(partition: Partition = US_PRIVATE, audit?: (e: { epoch: string }) => void) {
  const backend = new MapBackend();
  const adapter = new R2ObjectPlaneAdapter({
    bucket: asR2Bucket(backend),
    partition,
    capabilityKey: KEY,
    audit: audit as never,
  });
  return { adapter, backend };
}

test("R2 capability matrix records the binding's honest limits", () => {
  const { adapter } = makeAdapter();
  const matrix = adapter.capabilities();
  assert.equal(matrix.provider, "r2");
  assert.equal(matrix.conditionalWrite, "none");
  assert.equal(matrix.rangeReads, "client-slice");
  assert.deepEqual(matrix.multipart, { supported: false });
  assert.equal(matrix.cdnEligible, false);
  assert.equal(matrix.readAfterWrite, true);
  assert.ok(matrix.stagingTtlSeconds > 0);
});

test("R2 keys carry the partition prefix and partitions are isolated in one bucket", async () => {
  const backend = new MapBackend();
  const usAdapter = new R2ObjectPlaneAdapter({ bucket: asR2Bucket(backend), partition: US_PRIVATE, capabilityKey: KEY });
  const euAdapter = new R2ObjectPlaneAdapter({ bucket: asR2Bucket(backend), partition: EU_PRIVATE, capabilityKey: KEY });
  const receipt = await usAdapter.putStaged(PAYLOAD, { now: NOW });
  await usAdapter.publish(receipt.stagingId, receipt.contentId);
  assert.ok([...backend.objects.keys()].every((k) => k.includes("/us/tenant-private/")), "us keys stay in the us partition");
  assert.equal(await euAdapter.head(receipt.contentId), null, "eu partition sees none of it");
  // A capability minted for the eu partition is well-formed there --
  // the object simply does not exist in that partition's namespace.
  const foreignCap = await mintReadCapability(KEY, { contentId: receipt.contentId, partition: EU_PRIVATE, expiresAt: FUTURE });
  await assert.rejects(() => euAdapter.read(receipt.contentId, foreignCap, { now: NOW }), /no verified replica/);
  // And a us-minted capability presented to the eu adapter is refused
  // as cross-partition before any backend read.
  const usCap = await mintReadCapability(KEY, { contentId: receipt.contentId, partition: US_PRIVATE, expiresAt: FUTURE });
  await assert.rejects(() => euAdapter.read(receipt.contentId, usCap, { now: NOW }), /different partition/);
});

test("R2 publish is retry-safe after the staged copy is dropped", async () => {
  const { adapter } = makeAdapter();
  const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
  const first = await adapter.publish(receipt.stagingId, receipt.contentId);
  assert.equal(first.sizeBytes, PAYLOAD.length);
  // The success response was "lost": retrying converges on the
  // published object instead of failing on the missing stage.
  const second = await adapter.publish(receipt.stagingId, receipt.contentId);
  assert.equal(second.contentId, receipt.contentId);
  assert.equal(second.sizeBytes, PAYLOAD.length);
});

test("R2 fenced deletes emit audit events with the epoch", async () => {
  const events: { contentId: string; epoch: string; existed: boolean }[] = [];
  const { adapter } = makeAdapter(US_PRIVATE, (e) => events.push(e as { contentId: string; epoch: string; existed: boolean }));
  const receipt = await adapter.putStaged(PAYLOAD, { now: NOW });
  await adapter.publish(receipt.stagingId, receipt.contentId);
  await adapter.deleteReplica(receipt.contentId, { epoch: "epoch-7", reverifiedAt: NOW }, { now: NOW });
  assert.equal(events.length, 1);
  assert.equal(events[0].contentId, receipt.contentId);
  assert.equal(events[0].epoch, "epoch-7");
  assert.equal(events[0].existed, true);
});

test("R2 adapter refuses an empty capability key at construction", () => {
  assert.throws(
    () =>
      new R2ObjectPlaneAdapter({
        bucket: asR2Bucket(new MapBackend()),
        partition: US_PRIVATE,
        capabilityKey: new Uint8Array(0),
      }),
    /capability key must not be empty/,
  );
});
