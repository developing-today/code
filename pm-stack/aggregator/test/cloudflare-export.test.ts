// Cloudflare live export overlay (#1048): a fresh file-export document
// wins for the card's used values; stale/failed/missing changes nothing.
import { describe, expect, test } from "bun:test";
import { applyCloudflareExport, extractSaasServices } from "../src/saas-extract.ts";
import type { CloudflareExportDoc } from "../src/saas-extract.ts";

const VIEW = `
# Part (b) — Subscriptions & SaaS/infrastructure

## Summary (b) — subscriptions, infrastructure, account tooling

| Service | Plan / price | Allowance sketch | Usage / status | Next date recorded |
|---|---|---|---|---|
| Cloudflare platform | Workers Paid $5/mo + R2 Paid $0 base | 10M requests / 30M CPU-ms monthly | Historically far behind | Subscription date unknown |

## Service details (b)

### Cloudflare — Workers platform, R2 and AI gateway
- **Plan / price:** Workers Paid $5/mo + R2 Paid $0 base, verified 2026-10-20.
- **Monthly allowances:** Workers 10M requests/30M CPU-ms; KV 10M reads, 1M writes/deletes/lists, 1 GB; D1 25B rows read/50M written, 5 GB; Queues 1M ops.
- **R2 / daily allowances / billing:** R2 Standard 10 GB-mo, 1M Class A and 10M Class B operations monthly. Workers AI 10k Neurons/day.
- **Usage / pace:** Tracker: small. Current-cycle percentages unknown.
- **Last checked:** 2026-10-20.
`;

const NOW = Date.parse("2026-10-29T10:05:00Z");
const freshDoc = (): CloudflareExportDoc => ({
  service: "cloudflare",
  source: "file-export/cloudflare",
  status: "ok",
  measuredAt: "2026-10-29T10:00:00.000Z",
  cycleStart: "2026-10-01T00:00:00.000Z",
  billing: { status: "invoice-only" },
  metrics: [
    { id: "workers-requests", used: 75225, unit: "count", source: "cloudflare-graphql-analytics" },
    { id: "kv-reads", used: 1420, unit: "count", source: "cloudflare-graphql-analytics" },
    { id: "r2-storage", used: 25.14, unit: "gb", source: "cloudflare-graphql-analytics" },
  ],
});

describe("applyCloudflareExport", () => {
  test("fresh export: live used values + percent, tracker limits kept", () => {
    const services = applyCloudflareExport(extractSaasServices(VIEW), freshDoc(), NOW);
    const cf = services.find((s) => s.id === "cloudflare")!;
    const byId = Object.fromEntries(cf.metrics.map((m) => [m.id, m]));
    expect(byId["workers-requests"].used).toBe(75225);
    expect(byId["workers-requests"].limit).toBe(10_000_000);
    expect(byId["workers-requests"].percent).toBe(0.8);
    expect(byId["workers-requests"].note).toContain("Live via cloudflare-graphql-analytics");
    expect(byId["kv-reads"].used).toBe(1420);
    expect(byId["r2-storage"].used).toBe(25.14);
    // A metric the export does not carry keeps its prose state.
    expect(byId["queues-ops"].used).toBeNull();
    expect(cf.sourceClass).toBe("api");
    expect(cf.measuredAt).toBe("2026-10-29");
    expect(cf.sourceNote).toContain("invoice-only");
  });

  test("stale export (>30 min) changes nothing", () => {
    const doc = freshDoc();
    doc.measuredAt = "2026-10-29T09:00:00.000Z";
    const services = applyCloudflareExport(extractSaasServices(VIEW), doc, NOW);
    const cf = services.find((s) => s.id === "cloudflare")!;
    const wr = cf.metrics.find((m) => m.id === "workers-requests")!;
    expect(wr.used).toBeNull();
    expect(cf.sourceClass).toBe("manual");
  });

  test("failure document and null change nothing", () => {
    for (const doc of [null, { status: "token-scope", measuredAt: null, metrics: [] } as CloudflareExportDoc]) {
      const services = applyCloudflareExport(extractSaasServices(VIEW), doc, NOW);
      const cf = services.find((s) => s.id === "cloudflare")!;
      expect(cf.metrics.find((m) => m.id === "workers-requests")!.used).toBeNull();
    }
  });

  test("previous cycle's document keeps the prose fallback at a month rollover", () => {
    const doc = freshDoc();
    doc.cycleStart = "2026-09-01T00:00:00.000Z";
    const services = applyCloudflareExport(extractSaasServices(VIEW), doc, NOW);
    const cf = services.find((s) => s.id === "cloudflare")!;
    expect(cf.metrics.find((m) => m.id === "workers-requests")!.used).toBeNull();
    expect(cf.sourceClass).toBe("manual");
  });

  test("alerts in a fresh export render loudly on the sourceNote (#2287)", () => {
    const doc = freshDoc();
    doc.alerts = [
      {
        kind: "workers-ai-neurons-day",
        date: "2026-10-24",
        totalNeurons: 3090000,
        threshold: 50000,
        topModels: [
          { model: "@cf/deepseek-ai/deepseek-v4-pro-0813", neurons: 2990000 },
          { model: "@cf/meta/llama-3.1-8b-instruct", neurons: 100000 },
        ],
      },
    ];
    const services = applyCloudflareExport(extractSaasServices(VIEW), doc, NOW);
    const cf = services.find((s) => s.id === "cloudflare")!;
    expect(cf.sourceNote).toContain("⚠ ALERT workers-ai-neurons-day");
    expect(cf.sourceNote).toContain("2026-10-24");
    expect(cf.sourceNote).toContain("3090000");
    expect(cf.sourceNote).toContain("@cf/deepseek-ai/deepseek-v4-pro-0813");
    // The pre-alert rendering is untouched apart from the appended alert.
    expect(cf.sourceNote).toContain("invoice-only");
  });

  test("a document without alerts renders the sourceNote exactly as before (#2287)", () => {
    const withEmpty = freshDoc();
    withEmpty.alerts = [];
    for (const doc of [freshDoc(), withEmpty]) {
      const services = applyCloudflareExport(extractSaasServices(VIEW), doc, NOW);
      const cf = services.find((s) => s.id === "cloudflare")!;
      expect(cf.sourceNote).not.toContain("ALERT");
      expect(cf.sourceNote).toContain("invoice-only");
    }
  });

  test("a lower-bound metric carries the marker onto the card", () => {
    const doc = freshDoc();
    doc.metrics = doc.metrics!.map((m) => (m.id === "r2-storage" ? { ...m, lowerBound: true } : m));
    const services = applyCloudflareExport(extractSaasServices(VIEW), doc, NOW);
    const cf = services.find((s) => s.id === "cloudflare")!;
    expect(cf.metrics.find((m) => m.id === "r2-storage")!.lowerBound).toBe(true);
  });
});
