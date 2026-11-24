import { describe, expect, it } from "vitest";
import {
  HotCacheMeasurement,
  TURSO_COST_AVOIDED_METHOD,
  formatMeasurementReport,
  renderMeasurementReport,
  type FillCost,
} from "../src/measurement";

// Oracle for every test in this file: design #9134 §7 (merged via
// PR #13098). The measurement report surfaces hits, skips,
// revalidations per class and tenant, stale_serves (expected zero),
// estimated Turso cost avoided (skips x recorded fill cost, labelled
// an estimate with its method), and D1 write/storage bytes per tenant
// vs quota — for #9098's embed-storage economics prototype.
//
// The counters are the measurement surface §2's skip log and §7 feed
// into; the read-path integration that records live events is
// follow-on 4 (#13105). This slice is the accumulator + the report
// renderer, driven here through its public record API.

const COST_A: FillCost = { latencyMs: 300, rowsRead: 12_000 };
const COST_B: FillCost = { latencyMs: 100, rowsRead: 2_000 };

function filled(tenantId = "tenant-1"): HotCacheMeasurement {
  const m = new HotCacheMeasurement();
  m.recordFill(tenantId, "B", "key-b", 1_024, COST_A);
  m.recordFill(tenantId, "A", "key-a", 512, COST_B);
  return m;
}

describe("counters accumulate per class and per tenant", () => {
  it("records hits, skips, revalidations and stale serves in the right cells", () => {
    const m = filled();
    m.recordHit("tenant-1", "B");
    m.recordSkip("tenant-1", "B", "key-b");
    m.recordHit("tenant-1", "B");
    m.recordRevalidation("tenant-1", "B");
    m.recordHit("tenant-2", "C");
    m.recordStaleServe("tenant-2", "C");

    const s = m.snapshot();
    expect(s.byClass.B.hits).toBe(2);
    expect(s.byClass.B.skips).toBe(1);
    expect(s.byClass.B.revalidations).toBe(1);
    expect(s.byClass.C.hits).toBe(1);
    expect(s.byClass.C.staleServes).toBe(1);
    expect(s.byTenant["tenant-1"].hits).toBe(2);
    expect(s.byTenant["tenant-2"].staleServes).toBe(1);
    expect(s.byTenantClass["tenant-1"].B.skips).toBe(1);
    expect(s.byTenantClass["tenant-2"].C.staleServes).toBe(1);
  });

  it("a coherent read sequence keeps hits = skips + revalidations", () => {
    const m = filled();
    // Every found entry is either served (skip) or revalidated: the
    // read path records the hit first, then the outcome.
    m.recordHit("tenant-1", "B");
    m.recordSkip("tenant-1", "B", "key-b");
    m.recordHit("tenant-1", "B");
    m.recordRevalidation("tenant-1", "B");
    const s = m.snapshot();
    expect(s.byClass.B.hits).toBe(s.byClass.B.skips + s.byClass.B.revalidations);
  });

  it("snapshots are frozen: later records do not rewrite an earlier snapshot", () => {
    const m = filled();
    m.recordHit("tenant-1", "B");
    const before = m.snapshot();
    m.recordHit("tenant-1", "B");
    expect(before.byClass.B.hits).toBe(1);
    expect(m.snapshot().byClass.B.hits).toBe(2);
  });
});

describe("estimated Turso cost avoided (§7)", () => {
  it("is skips x the skipped entry's recorded fill cost", () => {
    const m = filled();
    m.recordSkip("tenant-1", "B", "key-b");
    m.recordSkip("tenant-1", "B", "key-b");
    m.recordSkip("tenant-1", "A", "key-a");
    const s = m.snapshot();
    expect(s.tursoCostAvoided.skips).toBe(3);
    expect(s.tursoCostAvoided.latencyMs).toBe(300 + 300 + 100);
    expect(s.tursoCostAvoided.rowsRead).toBe(12_000 + 12_000 + 2_000);
    expect(s.byClass.B.costAvoidedLatencyMs).toBe(600);
    expect(s.byClass.A.costAvoidedLatencyMs).toBe(100);
  });

  it("a skip with no recorded fill cost contributes 0 and is counted, not invented", () => {
    const m = new HotCacheMeasurement();
    m.recordSkip("tenant-1", "B", "never-filled");
    const s = m.snapshot();
    expect(s.tursoCostAvoided.latencyMs).toBe(0);
    expect(s.tursoCostAvoided.rowsRead).toBe(0);
    expect(s.tursoCostAvoided.skips).toBe(0);
    expect(s.byClass.B.skipsWithUnknownFillCost).toBe(1);
    expect(s.byClass.B.skips).toBe(1);
  });

  it("a refill replaces the recorded fill cost for later skips", () => {
    const m = filled();
    m.recordFill("tenant-1", "B", "key-b", 1_024, { latencyMs: 50, rowsRead: 500 });
    m.recordSkip("tenant-1", "B", "key-b");
    expect(m.snapshot().tursoCostAvoided.latencyMs).toBe(50);
    expect(m.snapshot().tursoCostAvoided.rowsRead).toBe(500);
  });

  it("an evicted entry no longer contributes its fill cost to later skips", () => {
    const m = filled();
    m.recordEvict("tenant-1", "B", "key-b", 1_024);
    m.recordSkip("tenant-1", "B", "key-b");
    const s = m.snapshot();
    expect(s.tursoCostAvoided.latencyMs).toBe(0);
    expect(s.byClass.B.skipsWithUnknownFillCost).toBe(1);
  });
});

describe("report renders from live counters (§7 acceptance)", () => {
  it("renders per-class and per-tenant totals with hit/skip ratios", () => {
    const m = filled();
    m.recordHit("tenant-1", "B");
    m.recordSkip("tenant-1", "B", "key-b");
    m.recordHit("tenant-1", "B");
    m.recordRevalidation("tenant-1", "B");
    const r = renderMeasurementReport(m.snapshot());

    expect(r.totals.hits).toBe(2);
    expect(r.totals.skips).toBe(1);
    expect(r.totals.revalidations).toBe(1);
    expect(r.byClass.B.skipRatio).toBe(0.5);
    expect(r.byClass.B.serveRatio).toBe(0.5);
    expect(r.byTenant["tenant-1"].hits).toBe(2);
    expect(r.byTenant["tenant-1"].skipRatio).toBe(0.5);
  });

  it("empty counters render a valid zero report, never NaN", () => {
    const r = renderMeasurementReport(new HotCacheMeasurement().snapshot());
    expect(r.totals.hits).toBe(0);
    expect(r.totals.skipRatio).toBe(0);
    expect(r.totals.serveRatio).toBe(0);
    expect(r.byClass.A.skipRatio).toBe(0);
    expect(r.staleServes.total).toBe(0);
    expect(r.staleServes.defect).toBe(false);
    expect(r.tursoCostAvoided.latencyMs).toBe(0);
    expect(r.storage).toEqual([]);
    expect(Number.isNaN(r.totals.skipRatio)).toBe(false);
  });

  it("stale_serves is flagged as a defect when non-zero (expected value zero, §1)", () => {
    const m = new HotCacheMeasurement();
    m.recordStaleServe("tenant-1", "B");
    const r = renderMeasurementReport(m.snapshot());
    expect(r.staleServes.total).toBe(1);
    expect(r.staleServes.expectedZero).toBe(true);
    expect(r.staleServes.defect).toBe(true);
    expect(r.byClass.B.staleServes).toBe(1);
  });

  it("labels the Turso cost avoided an estimate and states its method", () => {
    const m = filled();
    m.recordSkip("tenant-1", "B", "key-b");
    const r = renderMeasurementReport(m.snapshot());
    expect(r.tursoCostAvoided.kind).toBe("estimate");
    expect(r.tursoCostAvoided.method).toBe(TURSO_COST_AVOIDED_METHOD);
    expect(r.tursoCostAvoided.latencyMs).toBe(300);
    expect(r.tursoCostAvoided.rowsRead).toBe(12_000);
    expect(r.tursoCostAvoided.skips).toBe(1);
  });

  it("reports D1 storage bytes per tenant against the supplied quota", () => {
    const m = filled();
    const r = renderMeasurementReport(m.snapshot(), {
      quotaBytesByTenant: { "tenant-1": 1_000 },
    });
    expect(r.storage).toEqual([
      { tenantId: "tenant-1", storedBytes: 1_536, quotaBytes: 1_000, overQuota: true },
    ]);
    const ok = renderMeasurementReport(m.snapshot(), {
      quotaBytesByTenant: { "tenant-1": 2_000 },
    });
    expect(ok.storage[0].overQuota).toBe(false);
  });

  it("a tenant with no supplied quota reports a null quota, never over-quota", () => {
    const m = filled();
    const r = renderMeasurementReport(m.snapshot());
    expect(r.storage).toEqual([
      { tenantId: "tenant-1", storedBytes: 1_536, quotaBytes: null, overQuota: false },
    ]);
  });

  it("echoes the active config so the tuning round has its before-values", () => {
    const m = filled();
    const r = renderMeasurementReport(m.snapshot(), {
      config: { ttlBSeconds: 60, heavyQueryLatencyMs: 250, tierWeights: "1:4:16" },
    });
    expect(r.config).toEqual({ ttlBSeconds: 60, heavyQueryLatencyMs: 250, tierWeights: "1:4:16" });
  });

  it("formats as readable text carrying the numbers, the verdict and the method", () => {
    const m = filled();
    m.recordHit("tenant-1", "B");
    m.recordSkip("tenant-1", "B", "key-b");
    m.recordStaleServe("tenant-2", "C");
    const text = formatMeasurementReport(
      renderMeasurementReport(m.snapshot(), { quotaBytesByTenant: { "tenant-1": 1_000 } }),
    );
    expect(text).toContain("D1 hot-cache measurement report");
    expect(text).toContain("stale_serves: 1 (expected 0 — DEFECT");
    expect(text).toContain("estimate");
    expect(text).toContain(TURSO_COST_AVOIDED_METHOD);
    expect(text).toContain("tenant-1");
    expect(text).toContain("over quota");
  });

  it("formats a clean zero report without a defect verdict", () => {
    const text = formatMeasurementReport(renderMeasurementReport(new HotCacheMeasurement().snapshot()));
    expect(text).toContain("stale_serves: 0 (expected 0 — OK)");
    expect(text).not.toContain("DEFECT");
  });
});

describe("D1 write and storage accounting (§7 fill cost)", () => {
  it("recordFill counts D1 writes and bytes; recordEvict counts evictions and frees storage", () => {
    const m = filled();
    m.recordEvict("tenant-1", "B", "key-b", 1_024);
    const s = m.snapshot();
    expect(s.byClass.B.fills).toBe(1);
    expect(s.byClass.B.d1Writes).toBe(1);
    expect(s.byClass.B.d1BytesWritten).toBe(1_024);
    expect(s.byClass.B.d1BytesEvicted).toBe(1_024);
    expect(s.storedBytesByTenant["tenant-1"]).toBe(512);
  });

  it("a refill adjusts stored bytes by the size delta, not by double counting", () => {
    const m = filled();
    m.recordFill("tenant-1", "B", "key-b", 2_048, COST_A);
    expect(m.snapshot().storedBytesByTenant["tenant-1"]).toBe(2_048 + 512);
    expect(m.snapshot().byClass.B.d1BytesWritten).toBe(1_024 + 2_048);
  });

  it("evicting an unknown key never drives stored bytes negative", () => {
    const m = new HotCacheMeasurement();
    m.recordEvict("tenant-1", "B", "ghost", 1_024);
    expect(m.snapshot().storedBytesByTenant["tenant-1"]).toBe(0);
  });

  it("negative sizes and costs clamp to zero instead of corrupting the totals", () => {
    const m = new HotCacheMeasurement();
    m.recordFill("tenant-1", "B", "key-b", -5, { latencyMs: -10, rowsRead: -1 });
    const s = m.snapshot();
    expect(s.storedBytesByTenant["tenant-1"]).toBe(0);
    expect(s.byClass.B.d1BytesWritten).toBe(0);
    m.recordSkip("tenant-1", "B", "key-b");
    expect(m.snapshot().tursoCostAvoided.latencyMs).toBe(0);
    expect(m.snapshot().tursoCostAvoided.skips).toBe(1);
  });
});
