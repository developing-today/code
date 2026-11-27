import { afterAll, describe, expect, test } from "bun:test";
import { mkdtemp, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";

const root = await mkdtemp(join(tmpdir(), "search-registry-cache-15785-"));
afterAll(() => rm(root, { recursive: true, force: true }));

function feed(generatedAt: string, count: number) {
  return {
    version: 1,
    generatedAt,
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    registry: Array.from({ length: count }, (_, index) => ({
      id: `session-${index}`,
      source: "opencode-sandbox",
      label: `Session ${index}`,
      agentId: "opencode:test",
      agentLabel: "test",
      parentId: null,
      status: "complete",
      taskRefs: [],
    })),
  };
}

describe("search registry snapshot reuse (#15785)", () => {
  test("reuses the same merged array until the feed generation changes", async () => {
    const feedFile = join(root, "fleet-feed.json");
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_REGISTRY_FILE: join(root, "registry.json"),
      PM_AGG_OPENCODE_SERVER_STATE: join(root, "no-opencode-state.json"),
      PM_AGG_OPENCHAMBER_STATE: join(root, "no-openchamber-state.json"),
      PM_PROJECTOR_URL: "off",
    });
    const source = liveSource(cfg);
    await writeFile(feedFile, JSON.stringify(feed("2026-11-27T05:00:00.000Z", 1)));

    const first = await source.registry();
    const sameGeneration = await source.registry();
    expect(sameGeneration).toBe(first);

    const nextFeedFile = join(root, "fleet-feed-next.json");
    await writeFile(nextFeedFile, JSON.stringify(feed("2026-11-27T05:00:00.000Z", 2)));
    await rename(nextFeedFile, feedFile);
    const nextGeneration = await source.registry();
    expect(nextGeneration).not.toBe(first);
    expect(nextGeneration).toHaveLength(2);
  });
});
