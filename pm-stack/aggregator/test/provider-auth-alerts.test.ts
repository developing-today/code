// #6671: the provider-auth keeper alert projection — fixture state
// files in the shapes the keepers actually write (following the
// keeper test patterns: injected clock, temp state files, no network).
// Cases: attention, valid, missing file, stale file, invalid file —
// plus the gather->overlay->board hops that carry the projection to
// the surface a human reads.

import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { providerAuthPathsIn, providerAuthProblemLines, readProviderAuthAlerts } from "../src/provider-auth-alerts.ts";
import { overlayQuotaCurrent, parseQuotaCurrent } from "../src/quota-current.ts";
import { renderQuota } from "../src/render.ts";
import type { QuotaCurrentFile, QuotaView } from "../src/model.ts";

const NOW = new Date("2026-11-06T12:00:00.000Z");
const iso = (msAgo: number) => new Date(NOW.getTime() - msAgo).toISOString();

const opendesignAttention = {
  provider: "opendesign",
  checkedAt: iso(5 * 60_000),
  status: "attention",
  sessionExpiresAt: iso(-48 * 3_600_000),
  horizonHours: 48,
  capture: { attempted: true, result: "failed" },
  alert: "OpenDesign session expires in 48h and browser capture failed",
  alertSince: iso(3_600_000),
};

const metaValid = {
  provider: "meta",
  checkedAt: iso(5 * 60_000),
  status: "valid",
  exporterStatus: "ok",
  lastGoodAt: iso(5 * 60_000),
  recovery: { attempted: false, result: null },
  alert: null,
  alertSince: null,
};

async function fixtureDir(files: Record<string, unknown>): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "provider-auth-"));
  for (const [name, doc] of Object.entries(files)) {
    await writeFile(join(dir, name), typeof doc === "string" ? doc : JSON.stringify(doc));
  }
  return dir;
}

describe("readProviderAuthAlerts", () => {
  test("attention state projects as attention, naming alertSince and the horizon", async () => {
    const dir = await fixtureDir({ "opendesign-renewal.json": opendesignAttention, "meta-renewal.json": metaValid });
    const alerts = await readProviderAuthAlerts(providerAuthPathsIn(dir), { now: NOW });
    const od = alerts.find((a) => a.provider === "opendesign")!;
    expect(od.reporting).toBe("attention");
    expect(od.alertSince).toBe(opendesignAttention.alertSince);
    expect(od.sessionExpiresAt).toBe(opendesignAttention.sessionExpiresAt);
    const lines = providerAuthProblemLines(alerts);
    expect(lines).toHaveLength(1);
    expect(lines[0]).toContain("opendesign");
    expect(lines[0]).toContain(opendesignAttention.alertSince);
  });

  test("both keepers valid: no problems, no alert lines", async () => {
    const odValid = { ...opendesignAttention, status: "valid", alert: null, alertSince: null };
    const dir = await fixtureDir({ "opendesign-renewal.json": odValid, "meta-renewal.json": metaValid });
    const alerts = await readProviderAuthAlerts(providerAuthPathsIn(dir), { now: NOW });
    expect(alerts.map((a) => a.reporting)).toEqual(["ok", "ok"]);
    expect(providerAuthProblemLines(alerts)).toEqual([]);
  });

  test("missing files project as keeper-not-reporting, never silence", async () => {
    const dir = await fixtureDir({});
    const alerts = await readProviderAuthAlerts(providerAuthPathsIn(dir), { now: NOW });
    expect(alerts.map((a) => a.reporting)).toEqual(["missing", "missing"]);
    const lines = providerAuthProblemLines(alerts);
    expect(lines).toHaveLength(2);
    expect(lines[0]).toContain("keeper not reporting");
  });

  test("stale file projects as stale even when its last status was valid", async () => {
    const staleMeta = { ...metaValid, checkedAt: iso(2 * 3_600_000) };
    const dir = await fixtureDir({
      "opendesign-renewal.json": { ...opendesignAttention, status: "valid", alert: null, alertSince: null },
      "meta-renewal.json": staleMeta,
    });
    const alerts = await readProviderAuthAlerts(providerAuthPathsIn(dir), { now: NOW });
    expect(alerts.find((a) => a.provider === "meta")!.reporting).toBe("stale");
    expect(providerAuthProblemLines(alerts)[0]).toContain("keeper not reporting");
  });

  test("invalid file projects as invalid, distinct from missing", async () => {
    const dir = await fixtureDir({ "opendesign-renewal.json": "{not json", "meta-renewal.json": metaValid });
    const alerts = await readProviderAuthAlerts(providerAuthPathsIn(dir), { now: NOW });
    expect(alerts.find((a) => a.provider === "opendesign")!.reporting).toBe("invalid");
  });

  test("projection whitelists fields: an extra value in the state file never rides along", async () => {
    const poisoned = { ...opendesignAttention, cookie: "session-secret-value", token: "abc" };
    const dir = await fixtureDir({ "opendesign-renewal.json": poisoned, "meta-renewal.json": metaValid });
    const alerts = await readProviderAuthAlerts(providerAuthPathsIn(dir), { now: NOW });
    expect(JSON.stringify(alerts)).not.toContain("session-secret-value");
    expect(JSON.stringify(alerts)).not.toContain("abc");
  });
});

describe("gather -> overlay -> board", () => {
  const file = (): QuotaCurrentFile =>
    parseQuotaCurrent({
      version: 1,
      generatedAt: NOW.toISOString(),
      source: "test",
      heartbeat: { tickAt: NOW.toISOString(), ok: true, durationMs: 1, perProvider: {} },
      providers: [],
      providerAuth: [
        {
          provider: "meta",
          status: "attention",
          alert: "meta session expired and exporter recovery failed",
          alertSince: iso(3_600_000),
          checkedAt: iso(60_000),
          sessionExpiresAt: null,
          horizonHours: null,
          lastGoodAt: iso(86_400_000),
          reporting: "attention",
        },
      ],
    })!;

  const view = (): QuotaView => ({
    pools: [],
    burn: [],
    choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
  });

  test("overlay carries providerAuth onto the quota view", () => {
    const out = overlayQuotaCurrent(view(), file(), NOW);
    expect(out.providerAuth).toHaveLength(1);
    expect(out.providerAuth![0].provider).toBe("meta");
  });

  test("board renders the attention alert with provider and alertSince", () => {
    const out = overlayQuotaCurrent(view(), file(), NOW);
    const html = renderQuota(out, {} as never);
    expect(html).toContain("Provider auth");
    expect(html).toContain("meta session expired and exporter recovery failed");
  });

  test("a non-array providerAuth section is isolated at parse, never throws in the overlay", () => {
    const bad = parseQuotaCurrent({
      version: 1,
      generatedAt: NOW.toISOString(),
      source: "test",
      heartbeat: { tickAt: NOW.toISOString(), ok: true, durationMs: 1, perProvider: {} },
      providers: [],
      providerAuth: { provider: "meta" },
    })!;
    expect(bad.providerAuth).toBeUndefined();
    expect(() => overlayQuotaCurrent(view(), bad, NOW)).not.toThrow();
  });

  test("serve-time freshness: an ok keeper past the stale horizon reclassifies as stale", () => {
    const staleFile = file();
    staleFile.providerAuth = [
      {
        provider: "meta",
        status: "valid",
        alert: null,
        alertSince: null,
        checkedAt: iso(2 * 3_600_000),
        sessionExpiresAt: null,
        horizonHours: null,
        lastGoodAt: iso(2 * 3_600_000),
        reporting: "ok",
      },
    ];
    const out = overlayQuotaCurrent(view(), staleFile, NOW);
    expect(out.providerAuth![0].reporting).toBe("stale");
    expect(renderQuota(out, {} as never)).toContain("keeper not reporting");
  });

  test("board renders nothing when every keeper is ok", () => {
    const okFile = file();
    okFile.providerAuth = [
      { ...okFile.providerAuth![0], status: "valid", alert: null, alertSince: null, reporting: "ok" },
    ];
    const html = renderQuota(overlayQuotaCurrent(view(), okFile, NOW), {} as never);
    expect(html).not.toContain("Provider auth");
  });
});
