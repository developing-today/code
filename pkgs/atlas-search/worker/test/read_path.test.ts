import { env } from "cloudflare:test";
import { describe, expect, it, beforeAll, vi } from "vitest";
import { HotCacheStore } from "../src/hot_cache";
import { tryServeFromHotCache, type HotCacheReader, type SkipLog } from "../src/read_path";
import schemaSql from "../migrations/0001_hot_cache.sql?raw";

const db = (env as unknown as { HOT_CACHE: D1Database }).HOT_CACHE;
const store = new HotCacheStore(db);

const NOW = 1_700_000_000;

beforeAll(async () => {
  const statements = schemaSql
    .split("\n")
    .filter((line) => !line.trim().startsWith("--"))
    .join("\n")
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  for (const stmt of statements) {
    await db.prepare(stmt).run();
  }
});

function jsonPayload(value: unknown): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(value));
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as unknown as ArrayBuffer);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

describe("read-path hot-cache skip (#13105 slice 1)", () => {
  it("serves a fresh Class B entry and logs the §2 skip shape", async () => {
    const payload = jsonPayload({ results: [1, 2, 3] });
    await store.put({
      tenantId: "tenant-skip",
      cacheKey: "key-fresh-b",
      entryClass: "B",
      payload,
      sourceWatermark: "wm-1",
      filledAt: NOW - 10,
    });
    const skips: SkipLog[] = [];
    const outcome = await tryServeFromHotCache({
      store,
      source: async () => "wm-1",
      tenantId: "tenant-skip",
      cacheKey: "key-fresh-b",
      dataClass: "B",
      nowSeconds: () => NOW,
      onSkip: (log) => skips.push(log),
    });
    expect(outcome.served).toBe(true);
    if (!outcome.served) return;
    expect(outcome.entry.payload).toEqual(payload);
    expect(skips).toEqual([
      {
        tenant: "tenant-skip",
        key: "key-fresh-b",
        dataClass: "B",
        entryWatermark: "wm-1",
        entryAge: 10,
        reason: "watermark-current",
        degraded: false,
      },
    ]);
    const reread = await store.get("tenant-skip", "key-fresh-b");
    expect(reread!.hitCount).toBe(1);
  });

  it("revalidates on a watermark advance (stale Class B is never served)", async () => {
    await store.put({
      tenantId: "tenant-skip",
      cacheKey: "key-advanced",
      entryClass: "B",
      payload: jsonPayload({ results: [] }),
      sourceWatermark: "wm-1",
      filledAt: NOW - 10,
    });
    const outcome = await tryServeFromHotCache({
      store,
      source: async () => "wm-2",
      tenantId: "tenant-skip",
      cacheKey: "key-advanced",
      dataClass: "B",
      nowSeconds: () => NOW,
    });
    expect(outcome).toEqual({ served: false, reason: "watermark-advanced" });
  });

  it("revalidates on TTL expiry even with a current watermark", async () => {
    await store.put({
      tenantId: "tenant-skip",
      cacheKey: "key-expired",
      entryClass: "B",
      payload: jsonPayload({ results: [] }),
      sourceWatermark: "wm-1",
      filledAt: NOW - 61,
    });
    const outcome = await tryServeFromHotCache({
      store,
      source: async () => "wm-1",
      tenantId: "tenant-skip",
      cacheKey: "key-expired",
      dataClass: "B",
      nowSeconds: () => NOW,
    });
    expect(outcome).toEqual({ served: false, reason: "ttl-expired" });
  });

  it("misses a key with no entry", async () => {
    const outcome = await tryServeFromHotCache({
      store,
      source: async () => "wm-1",
      tenantId: "tenant-skip",
      cacheKey: "key-absent",
      dataClass: "B",
      nowSeconds: () => NOW,
    });
    expect(outcome).toEqual({ served: false, reason: "absent" });
  });

  it("falls through when the D1 read throws: no error surface change", async () => {
    const throwing: HotCacheReader = {
      get: async () => {
        throw new Error("D1 outage");
      },
      touch: async () => {},
    };
    const onSkip = vi.fn();
    const outcome = await tryServeFromHotCache({
      store: throwing,
      source: async () => "wm-1",
      tenantId: "tenant-skip",
      cacheKey: "key-whatever",
      dataClass: "B",
      nowSeconds: () => NOW,
      onSkip,
    });
    expect(outcome).toEqual({ served: false, reason: "store-unavailable" });
    expect(onSkip).not.toHaveBeenCalled();
  });

  it("serves Class A on a content-hash match, never on a mismatch", async () => {
    const payload = jsonPayload({ body: "chunk-bytes" });
    const hash = await sha256Hex(payload);
    await store.put({
      tenantId: "tenant-skip",
      cacheKey: "hash-key",
      entryClass: "A",
      payload,
      sourceWatermark: "wm-1",
      filledAt: NOW - 1_000_000,
    });
    const hit = await tryServeFromHotCache({
      store,
      source: async () => {
        throw new Error("watermark must not be read for class A");
      },
      tenantId: "tenant-skip",
      cacheKey: "hash-key",
      dataClass: "A",
      requestedContentHash: hash,
      nowSeconds: () => NOW,
    });
    expect(hit.served).toBe(true);

    const miss = await tryServeFromHotCache({
      store,
      source: async () => "wm-1",
      tenantId: "tenant-skip",
      cacheKey: "hash-key",
      dataClass: "A",
      requestedContentHash: "deadbeef",
      nowSeconds: () => NOW,
    });
    expect(miss).toEqual({ served: false, reason: "content-hash-mismatch" });
  });

  it("refuses to serve an entry of the wrong class", async () => {
    await store.put({
      tenantId: "tenant-skip",
      cacheKey: "key-wrong-class",
      entryClass: "B",
      payload: jsonPayload({ results: [] }),
      sourceWatermark: "wm-1",
      filledAt: NOW - 10,
    });
    const outcome = await tryServeFromHotCache({
      store,
      source: async () => "wm-1",
      tenantId: "tenant-skip",
      cacheKey: "key-wrong-class",
      dataClass: "A",
      nowSeconds: () => NOW,
    });
    expect(outcome).toEqual({ served: false, reason: "class-mismatch" });
  });
});
