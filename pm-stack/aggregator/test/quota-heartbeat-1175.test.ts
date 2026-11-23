import { expect, test } from "bun:test";
import { mkdtemp, writeFile, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";
import {
  parseQuotaGatherHeartbeat,
  latestQuotaHeartbeat,
  overlayStandaloneQuotaHeartbeat,
  overlayQuotaCurrent,
} from "../src/quota-current.ts";
import { renderQuota, renderTodayBody } from "../src/render.ts";

const old = "2026-11-18T01:00:00Z";
const tickAt = "2026-11-18T02:00:00Z";
const heartbeat = {
  tickAt,
  ok: false,
  durationMs: 15,
  error: "credential store unavailable <failed>",
  perProvider: {},
};
test("#1175 oracle: standalone failure survives missing or malformed current without inventing quota", async () => {
  const dir = await mkdtemp(join(tmpdir(), "quota-heartbeat-"));
  try {
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: join(dir, "no-feed"),
      PM_AGG_QUOTA_VIEW: join(dir, "no-view"),
      PM_AGG_QUOTA_CURRENT_FILE: join(dir, "quota-current.json"),
      PM_PROJECTOR_URL: "off",
    });
    expect(cfg.quotaHeartbeatFile).toBe(join(dir, "quota-gatherer-heartbeat.json"));
    await writeFile(cfg.quotaHeartbeatFile, JSON.stringify(heartbeat));
    for (const content of [null, "invalid JSON"]) {
      if (content) await writeFile(cfg.quotaCurrentFile, content);
      const source = liveSource(cfg);
      const view = await source.quota();
      expect(view.gatherer).toMatchObject({ tickAt, ok: false, durationMs: 15, error: heartbeat.error });
      const html = renderQuota(view, source.provenance());
      expect(html).toContain("credential store unavailable &lt;failed&gt;");
      expect(html).toContain("Gatherer tick");
    }
    await rm(cfg.quotaCurrentFile);
    await mkdir(cfg.quotaCurrentFile); // EISDIR exercises exhausted transport retry.
    expect((await liveSource(cfg).quota()).gatherer).toMatchObject({ tickAt, ok: false, error: heartbeat.error });
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
test("#1175 oracle: newest valid heartbeat wins; malformed and future dates rejected", () => {
  const embedded = { ...heartbeat, tickAt: old, ok: true };
  expect(latestQuotaHeartbeat(embedded, heartbeat)).toEqual(heartbeat);
  expect(latestQuotaHeartbeat(heartbeat, embedded)).toEqual(heartbeat);
  expect(latestQuotaHeartbeat(embedded, null)).toEqual(embedded);
  for (const invalid of [
    { ...heartbeat, tickAt: "2026-02-30" },
    { ...heartbeat, tickAt: "2999-01-01" },
    { ...heartbeat, durationMs: -1 },
    { ...heartbeat, perProvider: { p: { state: "ok", ms: 0, rows: -1 } } },
  ]) {
    expect(parseQuotaGatherHeartbeat(invalid)).toBeNull();
  }
  expect(
    parseQuotaGatherHeartbeat({ ...heartbeat, perProvider: { p: { state: "lapsed", ms: 0, rows: 0 } } }),
  ).not.toBeNull();
  expect(
    loadConfig({ PM_AGG_DEV: "1", PM_AGG_QUOTA_HEARTBEAT_FILE: "/tmp/explicit-heartbeat" }).quotaHeartbeatFile,
  ).toBe("/tmp/explicit-heartbeat");
});
test("#1175 oracle: heartbeat-only providers never create measurements or pools", () => {
  const base = { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } };
  const view = overlayStandaloneQuotaHeartbeat(base, {
    ...heartbeat,
    perProvider: { p: { state: "error", ms: 15, rows: 0, error: "read failed" } },
  });
  expect(view.pools).toBe(base.pools);
  expect(view.gatherer?.providers[0]).toMatchObject({ providerId: "p", state: "error", error: "read failed" });
});

test("#1175 live source oracle: new standalone failure overrides embedded success, not quota values", async () => {
  const dir = await mkdtemp(join(tmpdir(), "quota-heartbeat-current-"));
  try {
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: join(dir, "no-feed"),
      PM_AGG_QUOTA_VIEW: join(dir, "no-view"),
      PM_AGG_QUOTA_CURRENT_FILE: join(dir, "quota-current.json"),
      PM_PROJECTOR_URL: "off",
    });
    const embedded = { ...heartbeat, tickAt: old, ok: true, error: undefined };
    await writeFile(
      cfg.quotaCurrentFile,
      JSON.stringify({ version: 1, generatedAt: old, source: "fixture", heartbeat: embedded, providers: [] }),
    );
    await writeFile(cfg.quotaHeartbeatFile, JSON.stringify(heartbeat));
    expect((await liveSource(cfg).quota()).gatherer).toMatchObject({ tickAt, ok: false, error: heartbeat.error });
    await writeFile(cfg.quotaHeartbeatFile, JSON.stringify({ ...heartbeat, tickAt: "2026-11-17T00:00:00Z" }));
    expect((await liveSource(cfg).quota()).gatherer).toMatchObject({ tickAt: old, ok: true });
    await writeFile(cfg.quotaHeartbeatFile, JSON.stringify({ ...heartbeat, durationMs: -1 }));
    expect((await liveSource(cfg).quota()).gatherer).toMatchObject({ tickAt: old, ok: true });
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
test("#1175 heartbeat error redaction uses existing provider sanitizer", () => {
  const parsed = parseQuotaGatherHeartbeat({
    ...heartbeat,
    error: "token=fixture-secret",
    perProvider: { p: { state: "error", ms: 1, rows: 0, error: "Bearer fixture-secret" } },
  })!;
  expect(parsed.error).not.toContain("fixture-secret");
  expect(parsed.perProvider.p.error).not.toContain("fixture-secret");
  const base = { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } };
  const view = overlayStandaloneQuotaHeartbeat(base, {
    ...heartbeat,
    perProvider: { p: { state: "ok", ms: 1, rows: 0 } },
  });
  expect(view.gatherer?.providers).toEqual([]);
  expect(view.pools).toEqual([]);
});

test("#11998 P2 oracle: folded provider errors stay on failed credentials only", () => {
  const base = { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } };
  const row = {
    providerName: "OpenAI",
    providerId: "_openai",
    serviceId: "openai",
    measuredAt: tickAt,
    source: "fixture",
    windows: [],
  };
  const file = {
    version: 1 as const,
    generatedAt: tickAt,
    source: "fixture",
    heartbeat: {
      ...heartbeat,
      ok: true,
      error: undefined,
      perProvider: { _openai: { state: "ok" as const, ms: 1, rows: 3, error: "aggregate failure" } },
    },
    providers: [
      { ...row, state: "ok" as const, credentialLabel: "healthy" },
      { ...row, state: "error" as const, credentialLabel: "failed", error: "credential-specific failure" },
      { ...row, state: "error" as const, credentialLabel: "fallback" },
      { ...row, state: "ok" as const, retainedFromFailedTick: tickAt, credentialLabel: "retained" },
    ],
  };
  const providers = overlayQuotaCurrent(base, file, new Date(tickAt)).gatherer!.providers;
  expect(providers.find((p) => p.credentialLabel === "healthy")!.error).toBeUndefined();
  expect(providers.find((p) => p.credentialLabel === "failed")!.error).toBe("credential-specific failure");
  expect(providers.find((p) => p.credentialLabel === "fallback")!.error).toBe("aggregate failure");
  expect(providers.find((p) => p.credentialLabel === "retained")!.error).toBe("aggregate failure");
});

test("#11998 P2 oracle: standalone tick cannot erase pool errors or prove quota reads", async () => {
  const { fixtureSource } = await import("../src/sources/fixtures.ts");
  const { deriveToday, buildAttention } = await import("../src/derive.ts");
  const src = fixtureSource();
  const today = deriveToday(await src.board(), await src.roadmap(), new Map(), null);
  const attention = buildAttention(await src.attention(), null);
  const base = {
    pools: [{ id: "broken", provider: "broken", plan: "test", state: "error" as const, windows: [] }],
    burn: [],
    choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
  };
  const success = {
    ...heartbeat,
    ok: true,
    error: undefined,
    perProvider: { healthy: { state: "ok" as const, ms: 1, rows: 1 } },
  };
  const render = async (view: ReturnType<typeof overlayStandaloneQuotaHeartbeat>) =>
    renderTodayBody(today, attention, view, await src.coverage());
  expect(await render(overlayStandaloneQuotaHeartbeat(base, success))).toContain("quota errors 1");
  const failure = {
    ...heartbeat,
    perProvider: { another: { state: "error" as const, ms: 1, rows: 0, error: "failed" } },
  };
  expect(await render(overlayStandaloneQuotaHeartbeat(base, failure))).toContain("quota errors 2");
  expect(
    await render(
      overlayStandaloneQuotaHeartbeat(base, { ...failure, perProvider: { broken: failure.perProvider.another } }),
    ),
  ).toContain("quota errors 1");
  const empty = await render(overlayStandaloneQuotaHeartbeat({ ...base, pools: [] }, success));
  expect(empty).toContain("quota unknown");
  expect(empty).not.toContain("quota reads OK");
});
