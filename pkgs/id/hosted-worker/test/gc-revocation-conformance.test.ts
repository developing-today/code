// Issue #12808 (component F slice 6 of #9084): GC race +
// revocation conformance. Runs the shared adapter-neutral suite
// (src/gc-revocation-conformance.ts -- the same runner the RustFS
// adapter of component I, #9087, will use by supplying its own
// harness factory) over the R2 object-plane adapter on an in-memory
// R2BucketLike, with real ReferenceStores, the real derived index,
// the real GcSweeper and the real PublicationService on in-memory
// SQLite (node:sqlite), the harness shape of test/gc-sweep.test.ts.
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import {
  gcRevocationConformanceCases,
  type Partition,
  type SliceHarness,
} from "../src/gc-revocation-conformance.ts";
import { ReferenceStore } from "../src/references.ts";
import { ReferenceIndex } from "../src/reference-index.ts";
import { GcSweeper } from "../src/gc-sweep.ts";
import { PublicationService } from "../src/publication.ts";
import { R2ObjectPlaneAdapter, type R2ListableBucket } from "../src/object-plane-r2.ts";
import type { TestObjectBackend } from "../src/object-plane-conformance.ts";

const KEY = Uint8Array.from({ length: 32 }, (_, i) => (i * 17 + 3) & 0xff);

function streamOf(bytes: Uint8Array): ReadableStream {
  return new ReadableStream({
    start(controller) {
      controller.enqueue(bytes);
      controller.close();
    },
  });
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

function makeHarness(partition: Partition): SliceHarness {
  const backend = new MapBackend();
  const plane = new R2ObjectPlaneAdapter({
    bucket: asR2Bucket(backend),
    partition,
    capabilityKey: KEY,
  });
  const locator = `r2/${partition.jurisdiction}`;
  const index = new ReferenceIndex(fakeSql(), { partition });
  index.ensureSchema();
  const gcSql = fakeSql();

  if (partition.tier === "public") {
    const publicDb = makeStore("pub0", partition, true);
    const tenantDb = makeStore("ten0", partition);
    const publication = new PublicationService(fakeSql(), { publicDb, adapter: plane, capabilityKey: KEY });
    publication.ensureSchema();
    const gc = new GcSweeper(gcSql, { partition, index, adapter: plane, locator });
    gc.ensureSchema();
    return { partition, plane, stores: [tenantDb], index, gc, publication, publicDb, capabilityKey: KEY, locator };
  }

  const stores =
    partition.tier === "tenant-restricted"
      ? [makeStore("tena", partition), makeStore("tenb", partition)]
      : [makeStore("tena", partition)];
  const gc = new GcSweeper(gcSql, {
    partition,
    index,
    adapter: plane,
    locator,
    policy: partition.tier === "tenant-restricted" ? { soleReferencerErasure: true } : undefined,
  });
  gc.ensureSchema();
  return { partition, plane, stores, index, gc, capabilityKey: KEY, locator };
}

for (const c of gcRevocationConformanceCases(makeHarness)) {
  test(`gc-revocation conformance (R2): ${c.name}`, () => c.run());
}
