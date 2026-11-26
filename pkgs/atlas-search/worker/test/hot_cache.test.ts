import { env } from "cloudflare:test";
import { beforeAll, describe, expect, it } from "vitest";
import { HotCacheStore, hotCacheEnabled, hotCacheStore, type HotCacheClass } from "../src/hot_cache";
import schemaSql from "../migrations/0001_hot_cache.sql?raw";

const db = (env as unknown as { HOT_CACHE: D1Database }).HOT_CACHE;
const store = new HotCacheStore(db);

beforeAll(async () => {
  // D1's exec is line-oriented and cannot run the migration's
  // multi-line statements; apply it statement by statement instead.
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

const CLASSES: HotCacheClass[] = ["A", "B", "C"];

describe("hot cache store round-trip", () => {
  it.each(CLASSES)("round-trips a class %s entry byte-for-byte", async (entryClass) => {
    const payload = new TextEncoder().encode(`payload-for-${entryClass}`);
    await store.put({
      tenantId: "tenant-1",
      cacheKey: `key-${entryClass}`,
      entryClass,
      payload,
      sourceWatermark: "wm-42",
      filledAt: 1_700_000_000,
    });
    const got = await store.get("tenant-1", `key-${entryClass}`);
    expect(got).toBeDefined();
    expect(got!.entryClass).toBe(entryClass);
    expect(got!.payload).toEqual(payload);
    expect(got!.payloadBytes).toBe(payload.byteLength);
    expect(got!.sourceWatermark).toBe("wm-42");
    expect(got!.filledAt).toBe(1_700_000_000);
    expect(got!.lastHitAt).toBe(1_700_000_000);
    expect(got!.hitCount).toBe(0);
  });

  it("returns undefined for a missing entry", async () => {
    expect(await store.get("tenant-1", "no-such-key")).toBeUndefined();
  });

  it("keeps tenants apart under the same key", async () => {
    const payload = new TextEncoder().encode("tenant-2-bytes");
    await store.put({
      tenantId: "tenant-2",
      cacheKey: "key-A",
      entryClass: "A",
      payload,
      sourceWatermark: "wm-7",
      filledAt: 1_700_000_100,
    });
    const t1 = await store.get("tenant-1", "key-A");
    const t2 = await store.get("tenant-2", "key-A");
    expect(t1!.sourceWatermark).toBe("wm-42");
    expect(t2!.payload).toEqual(payload);
  });

  it("put replaces the payload and resets hit bookkeeping", async () => {
    await store.touch("tenant-1", "key-B", 1_700_000_500);
    const refill = new TextEncoder().encode("refilled");
    await store.put({
      tenantId: "tenant-1",
      cacheKey: "key-B",
      entryClass: "B",
      payload: refill,
      sourceWatermark: "wm-43",
      filledAt: 1_700_000_600,
    });
    const got = await store.get("tenant-1", "key-B");
    expect(got!.payload).toEqual(refill);
    expect(got!.sourceWatermark).toBe("wm-43");
    expect(got!.hitCount).toBe(0);
    expect(got!.lastHitAt).toBe(1_700_000_600);
  });

  it("touch records hits for the eviction order", async () => {
    await store.touch("tenant-1", "key-C", 1_700_001_000);
    await store.touch("tenant-1", "key-C", 1_700_001_100);
    const got = await store.get("tenant-1", "key-C");
    expect(got!.hitCount).toBe(2);
    expect(got!.lastHitAt).toBe(1_700_001_100);
  });

  it("evict removes exactly the named tenant's entry", async () => {
    expect(await store.evict("tenant-2", "key-A")).toBe(true);
    expect(await store.get("tenant-2", "key-A")).toBeUndefined();
    expect(await store.get("tenant-1", "key-A")).toBeDefined();
    expect(await store.evict("tenant-2", "key-A")).toBe(false);
  });
});

describe("hot cache flag gating", () => {
  it("is off unless the flag is the literal true", () => {
    expect(hotCacheEnabled({})).toBe(false);
    expect(hotCacheEnabled({ HOT_CACHE_ENABLED: "false" })).toBe(false);
    expect(hotCacheEnabled({ HOT_CACHE_ENABLED: "true" })).toBe(true);
  });

  it("yields no store when off or unbound, a store when on and bound", () => {
    expect(hotCacheStore({ HOT_CACHE: db })).toBeUndefined();
    expect(hotCacheStore({ HOT_CACHE_ENABLED: "true" })).toBeUndefined();
    expect(hotCacheStore({ HOT_CACHE: db, HOT_CACHE_ENABLED: "true" })).toBeInstanceOf(HotCacheStore);
  });
});
