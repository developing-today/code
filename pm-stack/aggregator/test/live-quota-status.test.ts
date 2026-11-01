import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";

const dirs: string[] = [];
afterEach(async () => {
  await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

async function setup() {
  const dir = await mkdtemp(join(tmpdir(), "quota-plane-status-"));
  dirs.push(dir);
  const current = join(dir, "quota-current.json");
  const heartbeat = join(dir, "quota-gatherer-heartbeat.json");
  const cfg = loadConfig({
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: join(dir, "missing-feed.json"),
    PM_AGG_QUOTA_CURRENT_FILE: current,
    PM_AGG_QUOTA_HEARTBEAT_FILE: heartbeat,
    PM_PROJECTOR_URL: "off",
  });
  return { cfg, current, heartbeat };
}

describe("live quota-plane heartbeat read (#1175)", () => {
  test("a newer standalone failure is visible over an older embedded success", async () => {
    const { cfg, current, heartbeat } = await setup();
    await writeFile(
      current,
      JSON.stringify({
        version: 1,
        generatedAt: "2026-10-27T06:45:00.000Z",
        source: "openchamber-quota",
        heartbeat: { tickAt: "2026-10-27T06:45:00.000Z", ok: true, durationMs: 50, perProvider: {} },
        providers: [],
      }),
    );
    await writeFile(
      heartbeat,
      JSON.stringify({
        tickAt: "2026-10-27T06:59:00.000Z",
        ok: false,
        durationMs: 9,
        perProvider: {},
        error: "credential store unavailable <redacted>",
      }),
    );

    const view = await liveSource(cfg).quota();
    expect(view.gatherer?.tickAt).toBe("2026-10-27T06:59:00.000Z");
    expect(view.gatherer?.ok).toBe(false);
    expect(view.gatherer?.error).toBe("credential store unavailable <redacted>");
  });

  test("standalone heartbeat remains visible when current quota data is missing", async () => {
    const { cfg, heartbeat } = await setup();
    await writeFile(
      heartbeat,
      JSON.stringify({
        tickAt: "2026-10-27T06:59:00.000Z",
        ok: true,
        durationMs: 9,
        perProvider: { codex: { state: "error", ms: 9, rows: 0, adapterStatus: "session-expired", error: "login expired" } },
      }),
    );

    const view = await liveSource(cfg).quota();
    expect(view.gatherer?.providers[0]?.health).toBe("expired");
    expect(view.gatherer?.providers[0]?.measuredAt).toBeNull();
  });
});
