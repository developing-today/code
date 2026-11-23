import { beforeAll, describe, expect, it } from "vitest";
import { HotCacheStore } from "../src/hot_cache";
import {
  InvalidationStore,
  invalidationEventFromCompletionRecord,
  invalidationStore,
} from "../src/invalidation";
import schemaSql from "../migrations/0001_hot_cache.sql?raw";
import invalidationSql from "../migrations/0002_hot_cache_invalidation.sql?raw";

const db = (env as unknown as { HOT_CACHE: D1Database }).HOT_CACHE;
const cache = new HotCacheStore(db);
const invalidation = new InvalidationStore(db);

async function applyMigration(sql: string): Promise<void> {
  // D1's exec is line-oriented and cannot run the migrations'
  // multi-line statements; apply them statement by statement instead
  // (same approach as test/hot_cache.test.ts).
  const statements = sql
    .split("\n")
    .filter((line) => !line.trim().startsWith("--"))
    .join("\n")
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  for (const stmt of statements) {
    await db.prepare(stmt).run();
  }
}

beforeAll(async () => {
  await applyMigration(schemaSql);
  await applyMigration(invalidationSql);
});

const encoder = new TextEncoder();

async function seedEntry(tenantId: string, cacheKey: string, entryClass: "A" | "B" | "C", watermark = "wm-1") {
  await cache.put({
    tenantId,
    cacheKey,
    entryClass,
    payload: encoder.encode(`payload-${tenantId}-${cacheKey}`),
    sourceWatermark: watermark,
    filledAt: 1_700_000_000,
  });
}

describe("invalidation event extraction (piggyback contract)", () => {
  it("yields undefined for a completion record carrying no event", () => {
    expect(invalidationEventFromCompletionRecord({ tenant_id: "t", watermark: "wm-2" })).toBeUndefined();
    expect(invalidationEventFromCompletionRecord({ invalidation: null })).toBeUndefined();
  });

  it("parses the event off a completion record", () => {
    const event = invalidationEventFromCompletionRecord({
      batch: "b-7",
      invalidation: { tenant_id: "tenant-x", new_watermark: "wm-9", emitted_at: 1_700_000_900 },
    });
    expect(event).toEqual({ tenantId: "tenant-x", newWatermark: "wm-9", emittedAt: 1_700_000_900 });
  });

  it("fails loudly on a present but malformed event", () => {
    expect(() => invalidationEventFromCompletionRecord({ invalidation: { new_watermark: "wm-9" } })).toThrow(
      TypeError,
    );
    expect(() =>
      invalidationEventFromCompletionRecord({
        invalidation: { tenant_id: "t", new_watermark: "wm-9", emitted_at: "soon" },
      }),
    ).toThrow(TypeError);
    expect(() => invalidationEventFromCompletionRecord("not-a-record")).toThrow(TypeError);
  });
});

describe("event consumption (design §4 mechanism 2)", () => {
  it("drops the tenant's Class C entries, sets forced-revalidation, and leaves Class A byte-for-byte untouched", async () => {
    const tenant = "inv-tenant-events";
    await seedEntry(tenant, "agg-1", "C");
    await seedEntry(tenant, "agg-2", "C");
    await seedEntry(tenant, "results-1", "B");
    await seedEntry(tenant, "body-1", "A");
    await seedEntry("inv-tenant-bystander", "agg-1", "C");
    const bodyBefore = await cache.get(tenant, "body-1");

    // The full round trip: event emitted on an ingest completion
    // record, extracted at the boundary, consumed by the Worker.
    const record = {
      batch: "b-42",
      invalidation: { tenant_id: tenant, new_watermark: "wm-2", emitted_at: 1_700_001_000 },
    };
    const event = invalidationEventFromCompletionRecord(record);
    expect(event).toBeDefined();
    const dropped = await invalidation.applyEvent(event!);

    expect(dropped).toBe(2);
    expect(await cache.get(tenant, "agg-1")).toBeUndefined();
    expect(await cache.get(tenant, "agg-2")).toBeUndefined();
    // Class B is not deleted by an event: its staleness is the lazy
    // watermark rule (mechanism 1), not eager deletion.
    expect(await cache.get(tenant, "results-1")).toBeDefined();
    // Class A is provably untouched: same bytes, same bookkeeping.
    const bodyAfter = await cache.get(tenant, "body-1");
    expect(bodyAfter).toEqual(bodyBefore);
    // The bystander tenant's aggregates are not this event's business.
    expect(await cache.get("inv-tenant-bystander", "agg-1")).toBeDefined();

    const state = await invalidation.tenantState(tenant);
    expect(state?.forcedRevalidation).toBe(true);
    expect(state?.lastInvalidationAt).toBe(1_700_001_000);
    expect(state?.lastInvalidatedWatermark).toBe("wm-2");
    expect(await invalidation.isForcedRevalidation(tenant)).toBe(true);
    expect(await invalidation.lastInvalidationAt(tenant)).toBe(1_700_001_000);
  });

  it("keeps the recorded state monotonic when events arrive out of order", async () => {
    const tenant = "inv-tenant-order";
    await invalidation.applyEvent({ tenantId: tenant, newWatermark: "wm-5", emittedAt: 1_700_005_000 });
    await invalidation.applyEvent({ tenantId: tenant, newWatermark: "wm-4", emittedAt: 1_700_004_000 });
    const state = await invalidation.tenantState(tenant);
    expect(state?.lastInvalidationAt).toBe(1_700_005_000);
    expect(state?.lastInvalidatedWatermark).toBe("wm-5");
    expect(state?.forcedRevalidation).toBe(true);
  });

  it("clears forced-revalidation at the next successful revalidation, keeping the event history", async () => {
    const tenant = "inv-tenant-events";
    await invalidation.noteRevalidation(tenant, 1_700_002_000);
    const state = await invalidation.tenantState(tenant);
    expect(state?.forcedRevalidation).toBe(false);
    expect(await invalidation.isForcedRevalidation(tenant)).toBe(false);
    // The Class C freshness rule still needs the event time for
    // entries filled before the event.
    expect(state?.lastInvalidationAt).toBe(1_700_001_000);
  });

  it("reports no state for a tenant that was never invalidated", async () => {
    expect(await invalidation.tenantState("inv-tenant-never")).toBeUndefined();
    expect(await invalidation.isForcedRevalidation("inv-tenant-never")).toBe(false);
    expect(await invalidation.lastInvalidationAt("inv-tenant-never")).toBeNull();
  });
});

describe("operator purge (design §4 mechanism 3)", () => {
  it("deletes all classes for the one tenant, sets forced-revalidation, and writes the audit record", async () => {
    const tenant = "inv-tenant-purge";
    await seedEntry(tenant, "body-1", "A");
    await seedEntry(tenant, "results-1", "B");
    await seedEntry(tenant, "agg-1", "C");
    await seedEntry("inv-tenant-keeper", "body-1", "A");

    const audit = await invalidation.purgeTenant({
      tenantId: tenant,
      operator: "op-drew",
      reason: "mis-tiered ingest must stop being served",
      atSeconds: 1_700_010_000,
    });

    // The purge is the documented incident path: §4 mechanism 3
    // deletes all classes, Class A included — unlike mechanisms 1–2,
    // from which Class A is exempt.
    expect(audit.entriesDeleted).toBe(3);
    expect(await cache.get(tenant, "body-1")).toBeUndefined();
    expect(await cache.get(tenant, "results-1")).toBeUndefined();
    expect(await cache.get(tenant, "agg-1")).toBeUndefined();
    expect(await cache.get("inv-tenant-keeper", "body-1")).toBeDefined();

    // The audit record as written carries operator, tenant, reason.
    expect(audit.tenantId).toBe(tenant);
    expect(audit.operator).toBe("op-drew");
    expect(audit.reason).toBe("mis-tiered ingest must stop being served");
    expect(audit.purgedAt).toBe(1_700_010_000);
    const recorded = await invalidation.listPurges(tenant);
    expect(recorded).toEqual([audit]);

    const state = await invalidation.tenantState(tenant);
    expect(state?.forcedRevalidation).toBe(true);
    expect(state?.lastInvalidationAt).toBe(1_700_010_000);
  });
});

describe("invalidation flag gating", () => {
  it("yields no store when off or unbound, a store when on and bound", () => {
    expect(invalidationStore({ HOT_CACHE: db })).toBeUndefined();
    expect(invalidationStore({ HOT_CACHE_ENABLED: "true" })).toBeUndefined();
    expect(invalidationStore({ HOT_CACHE: db, HOT_CACHE_ENABLED: "true" })).toBeInstanceOf(InvalidationStore);
  });
});
