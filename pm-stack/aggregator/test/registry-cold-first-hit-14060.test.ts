import { afterAll, describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, rename, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";

const root = await mkdtemp(join(tmpdir(), "registry-cold-first-hit-14060-"));
afterAll(() => rm(root, { recursive: true, force: true }));

function feed(count: number) {
  return {
    version: 1,
    generatedAt: "2026-11-27T05:00:00.000Z",
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    registry: Array.from({ length: count }, (_, index) => ({
      id: `synthetic-session-${index}`,
      source: "opencode-sandbox",
      label: `Synthetic session ${index}`,
      agentId: "opencode:synthetic",
      agentLabel: "Synthetic",
      parentId: null,
      status: "complete",
      taskRefs: [],
    })),
  };
}

function makeSource(name: string, timings: string[] = []) {
  const dir = join(root, name);
  const cfg = loadConfig({
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: join(dir, "fleet-feed.json"),
    PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
    PM_AGG_OPENCODE_SERVER_STATE: join(dir, "no-opencode-state.json"),
    PM_AGG_OPENCHAMBER_STATE: join(dir, "no-openchamber-state.json"),
    PM_PROJECTOR_URL: "off",
  });
  return liveSource(cfg, { onRegistryTiming: (stage) => timings.push(stage) });
}

describe("cold /sessions registry first hit (#14060)", () => {
  // Oracle: concurrent callers must receive one identical registry snapshot,
  // backed by exactly one measured read/merge/map stage in this source instance.
  test("shares one cold feed read and merge across concurrent readers", async () => {
    const dir = join(root, "concurrent");
    await mkdir(dir);
    await Bun.write(join(dir, "fleet-feed.json"), JSON.stringify(feed(2)));
    const timings: string[] = [];
    const source = makeSource("concurrent", timings);

    const [first, second, third] = await Promise.all([source.registry(), source.registry(), source.registry()]);

    expect(first).toHaveLength(2);
    expect(second).toBe(first);
    expect(third).toBe(first);
    expect(timings.filter((stage) => stage === "feedReadMs")).toHaveLength(1);
    expect(timings.filter((stage) => stage === "registryMergeMs")).toHaveLength(1);
    expect(timings.filter((stage) => stage === "laptopMapMs")).toHaveLength(1);
  });

  test("reuses an unchanged projection and invalidates on atomic feed replacement", async () => {
    const dir = join(root, "replacement");
    await mkdir(dir);
    const feedPath = join(dir, "fleet-feed.json");
    await Bun.write(feedPath, JSON.stringify(feed(1)));
    const source = makeSource("replacement");

    const first = await source.registry();
    const unchanged = await source.registry();
    expect(unchanged).toBe(first);

    const next = join(dir, "fleet-feed.next.json");
    await Bun.write(next, JSON.stringify(feed(2)));
    await rename(next, feedPath);
    const replaced = await source.registry();
    expect(replaced).not.toBe(first);
    expect(replaced).toHaveLength(2);
  });
});
