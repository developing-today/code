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
    expect(usage.attention).toContainEqual(
      expect.objectContaining({ kind: "over-limit", label: "Turso — Rows" }),
    );
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
