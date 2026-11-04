import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { loadFeed, sanitizeSaasServices } from "../src/feed.ts";
import type { SaasService } from "../src/model.ts";
import { renderQuota } from "../src/render.ts";
import { liveSource } from "../src/sources/live.ts";
import { extractSaasServices } from "../src/saas-extract.ts";

const VALID: SaasService = {
  id: "turso",
  name: "Turso",
  group: "database",
  plan: "Starter",
  status: "active",
  metrics: [{ id: "rows", label: "Rows", used: 9, limit: 10, unit: "count", percent: 90, highlight: true }],
  paygEstimate: null,
  measuredAt: "2026-10-30",
  sourceClass: "manual",
  sourceNote: "test record",
  nextDate: null,
  cadenceDays: 30,
  conflicts: [],
  reads: [{ at: "2026-10-30", text: "9 rows used" }],
};

const VIEW = `
## Summary (b) — subscriptions, infrastructure, account tooling

| Service | Plan / price | Allowance sketch | Usage / status | Next date recorded |
|---|---|---|---|---|
| Supabase | Free plan (muse-dev) | DB + egress + storage | manual dashboard read needed | unknown |
`;

function feed(saas?: unknown) {
  return {
    version: 1,
    generatedAt: new Date().toISOString(),
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    ...(saas === undefined ? {} : { saas }),
  };
}

async function load(saas?: unknown) {
  const dir = await mkdtemp(join(tmpdir(), "pm-saas-feed-"));
  const path = join(dir, "fleet-feed.json");
  await writeFile(path, JSON.stringify(feed(saas)));
  return loadFeed(path);
}

async function sourceWith(saas: unknown, quotaView = VIEW) {
  const dir = await mkdtemp(join(tmpdir(), "pm-saas-live-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(feedFile, JSON.stringify(feed(saas)));
  const quotaViewFile = join(dir, "quota-view.md");
  await writeFile(quotaViewFile, quotaView);
  const cfg = loadConfig({
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: feedFile,
    PM_AGG_QUOTA_VIEW: quotaViewFile,
    PM_AGG_SUPABASE_EXPORT: join(dir, "no-supabase-export.json"),
    PM_AGG_CLOUDFLARE_EXPORT: join(dir, "no-cloudflare-export.json"),
    PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
    PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
    PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
    PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
    PM_PROJECTOR_URL: "off",
    KANEO_ADMIN_EMAIL: "",
    KANEO_ADMIN_PASSWORD: "",
    GRIST_API_KEY: "",
  });
  return liveSource(cfg);
}

describe("fleet feed SaaS sanitization (#2451)", () => {
  test("drops null rows and preserves valid services in a mixed array", async () => {
    const loaded = await load([VALID, null]);
    expect(loaded?.saas).toEqual([VALID]);
    expect(loaded?.saasDropped).toBe(1);
  });

  test("a null-only array drops all services but keeps the feed", async () => {
    const loaded = await load([null]);
    expect(loaded).not.toBeNull();
    expect(loaded?.saas).toEqual([]);
    expect(loaded?.saasDropped).toBe(1);
  });

  test("non-array strings and objects are treated as absent and counted", async () => {
    for (const invalid of ["oops", { id: "turso" }]) {
      const loaded = await load(invalid);
      expect(loaded).not.toBeNull();
      expect(loaded?.saas).toBeUndefined();
      expect(loaded?.saasDropped).toBe(1);
      expect(sanitizeSaasServices(invalid)).toEqual({ dropped: 1 });
    }
  });

  test("service rows missing metrics or conflicts drop individually", async () => {
    const { metrics: _metrics, ...withoutMetrics } = VALID;
    const { conflicts: _conflicts, ...withoutConflicts } = VALID;
    const loaded = await load([withoutMetrics, VALID, withoutConflicts]);
    expect(loaded?.saas).toEqual([VALID]);
    expect(loaded?.saasDropped).toBe(2);
  });

  test("valid remainder renders and contributes attention flags", async () => {
    const src = await sourceWith([VALID, null]);
    const usage = await src.usage();
    expect(usage.services).toEqual([VALID]);
    expect(usage.attention).toContainEqual(expect.objectContaining({ kind: "over-limit", label: "Turso — Rows" }));
    const html = renderQuota(await src.quota(), src.provenance(), usage);
    expect(html).toContain("Turso");
    expect(src.provenance().usage).toContain("dropped 1 malformed SaaS service row(s)");
  });

  test("all-dropped and non-array sections expose loss on the fallback path", async () => {
    for (const malformed of [[null], { invalid: true }]) {
      const src = await sourceWith(malformed);
      const usage = await src.usage();
      expect(usage.services.length).toBeGreaterThan(0);
      expect(src.provenance().usage).toContain("dropped 1 malformed SaaS service row(s)");
      expect(src.provenance().usage).toContain("direct read");
    }
  });

  test("clean and collector-shaped services are unchanged with no diagnostic", async () => {
    const collectorServices = extractSaasServices(VIEW);
    expect(collectorServices.length).toBeGreaterThan(0);
    expect(sanitizeSaasServices(collectorServices)).toEqual({ rows: collectorServices, dropped: 0 });
    const collectorFeed = await load(collectorServices);
    expect(collectorFeed?.saas).toEqual(collectorServices);
    expect(collectorFeed?.saasDropped).toBeUndefined();

    const loaded = await load([VALID]);
    expect(loaded?.saas).toEqual([VALID]);
    expect(loaded?.saasDropped).toBeUndefined();
    const src = await sourceWith([VALID]);
    await src.usage();
    expect(src.provenance().usage).not.toContain("sanitized at load");
  });
});

describe("fleet feed SaaS range/order gates (#5188)", () => {
  const kept = (row: unknown) => sanitizeSaasServices([row]).rows?.length ?? 0;

  test("paygEstimate: inverted and negative pairs drop, ordered and zero pairs keep", () => {
    const withEstimate = (low: number, high: number) => ({
      ...VALID,
      paygEstimate: { low, high, period: "mo" },
    });
    expect(kept(withEstimate(100, 60))).toBe(0);
    expect(kept(withEstimate(-60, -10))).toBe(0);
    expect(kept(withEstimate(-1, 10))).toBe(0);
    expect(kept(withEstimate(10, -1))).toBe(0);
    expect(kept(withEstimate(60, 60))).toBe(1);
    expect(kept(withEstimate(0, 0))).toBe(1);
    expect(kept(withEstimate(10, 100))).toBe(1);
    expect(sanitizeSaasServices([withEstimate(100, 60), VALID]).dropped).toBe(1);
  });

  test("metrics: negative used/limit/percent drop the service, null and absent keep", () => {
    const metric = VALID.metrics[0]!;
    const withMetric = (patch: Record<string, unknown>) => ({
      ...VALID,
      metrics: [{ ...metric, ...patch }],
    });
    expect(kept(withMetric({ used: -1 }))).toBe(0);
    expect(kept(withMetric({ limit: -10 }))).toBe(0);
    expect(kept(withMetric({ percent: -5 }))).toBe(0);
    expect(kept(withMetric({ used: null, limit: null }))).toBe(1);
    expect(kept(withMetric({ percent: undefined }))).toBe(1);
    expect(kept(withMetric({ used: 0, limit: 0, percent: 0 }))).toBe(1);
  });

  test("service: negative billedUsd and cadenceDays drop, zero keeps", () => {
    expect(kept({ ...VALID, billedUsd: -0.01 })).toBe(0);
    expect(kept({ ...VALID, billedUsd: 0 })).toBe(1);
    expect(kept({ ...VALID, billedUsd: null })).toBe(1);
    expect(kept({ ...VALID, cadenceDays: -30 })).toBe(0);
    expect(kept({ ...VALID, cadenceDays: 0 })).toBe(1);
  });
});

describe("fleet feed SaaS measuredAt ISO gate (#5281)", () => {
  const kept = (row: unknown) => sanitizeSaasServices([row]).rows?.length ?? 0;

  test("impossible and non-ISO measuredAt drop the service and are counted", () => {
    for (const measuredAt of ["2026-02-30", "2026-04-31", "March 2, 2026", "not-a-date"]) {
      expect(kept({ ...VALID, measuredAt })).toBe(0);
    }
    expect(sanitizeSaasServices([{ ...VALID, measuredAt: "2026-02-30" }, VALID]).dropped).toBe(1);
  });

  test("valid ISO measuredAt and null measuredAt keep the service", () => {
    expect(kept({ ...VALID, measuredAt: "2026-03-01" })).toBe(1);
    expect(kept({ ...VALID, measuredAt: null })).toBe(1);
    expect(kept(VALID)).toBe(1);
  });
});
