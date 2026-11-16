// #1116 (2026-11-10): the production attention feed served the bundled
// demo fixtures for ~10 minutes. Chain: the OpenCode server restarted
// repeatedly, two collector runs wrote a feed with attention: [], the
// aggregator's direct laptop leg had been silently dead since the
// OpenChamber-managed server's state file went stale on 2026-10-29,
// and live attention()'s empty-result branch returned fix.attention().
// These tests pin the fix: live mode never serves fixtures on the
// attention path, and the standing server (service-prod.json) is read
// as a first-class laptop leg.
import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";

const FEED_ENTRY = {
  sessionId: "ses_feed_real_1",
  source: "opencode-laptop",
  title: "real feed session",
  lastActivityAt: new Date().toISOString(),
};

const SERVER_SESSION = {
  id: "ses_standing_real_1",
  title: "standing server session",
  time: { created: Date.now() - 60_000, updated: Date.now() - 30_000 },
  tokens: { input: 10, output: 5, reasoning: 0 },
};

async function makeSource(opts: {
  feedAttention: unknown[];
  standingState?: Record<string, unknown>;
  server?: Record<string, unknown>;
}): Promise<ReturnType<typeof liveSource>> {
  const dir = await mkdtemp(join(tmpdir(), "pm-attn-1116-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(
    feedFile,
    JSON.stringify({
      version: 1,
      generatedAt: new Date().toISOString(),
      quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
      attention: opts.feedAttention,
    }),
  );
  const standingFile = join(dir, "service-prod.json");
  if (opts.standingState) await writeFile(standingFile, JSON.stringify(opts.standingState));
  const cfg = loadConfig({
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: feedFile,
    PM_AGG_QUOTA_VIEW: join(dir, "no-quota-view.md"),
    PM_AGG_SUPABASE_EXPORT: join(dir, "no-supabase-export.json"),
    PM_AGG_CLOUDFLARE_EXPORT: join(dir, "no-cloudflare-export.json"),
    PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
    PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
    PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
    PM_AGG_OPENCHAMBER_STATE: join(dir, "no-openchamber-state.json"),
    PM_AGG_OPENCODE_SERVER_STATE: opts.standingState ? standingFile : join(dir, "no-service-prod.json"),
    PM_PROJECTOR_URL: "off",
    KANEO_ADMIN_EMAIL: "",
    KANEO_ADMIN_PASSWORD: "",
    GRIST_API_KEY: "",
  });
  return liveSource(cfg);
}

describe("live attention never serves fixtures (#1116)", () => {
  test("empty feed + dead laptop legs -> empty list, unavailable provenance, zero fixture IDs", async () => {
    const src = await makeSource({ feedAttention: [] });
    const attn = await src.attention();
    expect(attn).toEqual([]);
    expect(attn.some((e) => e.sessionId.startsWith("ses_h_") || e.sessionId.startsWith("ses_k_"))).toBe(false);
    expect(src.provenance().attention).toContain("unavailable");
    expect(src.provenance().attention).not.toContain("fixtures (");
  });

  test("standing-server leg serves real sessions when the feed is empty", async () => {
    const server = Bun.serve({
      port: 0,
      fetch(req) {
        const url = new URL(req.url);
        if (url.pathname === "/api/session") return Response.json({ data: [SERVER_SESSION] });
        if (url.pathname === "/api/session/active") return Response.json({ data: {} });
        if (url.pathname === "/api/permission/request") return Response.json({ data: [] });
        if (url.pathname.includes("/message")) return Response.json({ data: [] });
        if (url.pathname.includes("/form")) return Response.json({ data: [] });
        return new Response("not found", { status: 404 });
      },
    });
    try {
      const src = await makeSource({
        feedAttention: [],
        standingState: { url: `http://127.0.0.1:${server.port}`, password: "pw" },
      });
      const attn = await src.attention();
      expect(attn.map((e) => e.sessionId)).toContain("ses_standing_real_1");
      expect(src.provenance().attention).toContain("live");
    } finally {
      server.stop(true);
    }
  });

  test("non-empty feed passes through unchanged", async () => {
    const src = await makeSource({ feedAttention: [FEED_ENTRY] });
    const attn = await src.attention();
    expect(attn.map((e) => e.sessionId)).toContain("ses_feed_real_1");
    expect(src.provenance().attention).toContain("live");
  });
});
