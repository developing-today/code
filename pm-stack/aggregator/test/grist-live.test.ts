import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { FIXTURE_ROADMAP } from "../src/sources/fixtures.ts";
import { liveSource } from "../src/sources/live.ts";

async function testConfig() {
  const dir = await mkdtemp(join(tmpdir(), "pm-grist-live-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(
    feedFile,
    JSON.stringify({
      version: 1,
      generatedAt: new Date().toISOString(),
      quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
      attention: [],
    }),
  );
  return loadConfig({
    ...Bun.env,
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: feedFile,
    PM_PROJECTOR_URL: "off",
    GRIST_URL: "http://grist.test",
    GRIST_ROADMAP_DOC: "test-roadmap-doc",
    GRIST_API_KEY: "test-grist-api-key",
  });
}

describe("live Grist roadmap reader (#1213)", () => {
  test("reads the records endpoint with bearer auth and maps roadmap fields and references", async () => {
    const cfg = await testConfig();
    const requests: { url: string; authorization: string | null }[] = [];
    const realFetch = globalThis.fetch;
    globalThis.fetch = (async (input: unknown, init?: RequestInit) => {
      requests.push({
        url: String(input),
        authorization: new Headers(init?.headers).get("authorization"),
      });
      return new Response(
        JSON.stringify({
          records: [
            {
              id: 1213,
              fields: {
                Title: "Live roadmap row",
                Area: "Aggregator",
                Status: "In progress",
                Horizon: "Q4 2026",
                Source: "Issue: https://github.com/developing-today/code/issues/1213",
                Notes: "Related PR: https://github.com/developing-today/code/pull/2435",
              },
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch;

    try {
      const src = liveSource(cfg);
      const rows = await src.roadmap();

      expect(requests).toEqual([
        {
          url: "http://grist.test/api/docs/test-roadmap-doc/tables/Roadmap/records",
          authorization: "Bearer test-grist-api-key",
        },
      ]);
      expect(rows).toEqual([
        {
          id: 1213,
          subject: "grist:test-roadmap-doc/Roadmap/1213",
          title: "Live roadmap row",
          area: "Aggregator",
          status: "In progress",
          horizon: "Q4 2026",
          source: "Issue: https://github.com/developing-today/code/issues/1213",
          notes: "Related PR: https://github.com/developing-today/code/pull/2435",
          links: [
            { ref: "developing-today/code#1213", relation: "tracks" },
            { ref: "developing-today/code#2435", relation: "tracks" },
          ],
          gristUrl: "http://amd-6.raptor-pumpkinseed.ts.net:8484/doc/test-roadmap-doc",
        },
      ]);
      expect(src.provenance().roadmap).toBe("live (Grist)");
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test("falls back to fixture rows and reports the Grist read failure", async () => {
    const cfg = await testConfig();
    const requests: { url: string; authorization: string | null }[] = [];
    const realFetch = globalThis.fetch;
    globalThis.fetch = (async (input: unknown, init?: RequestInit) => {
      requests.push({
        url: String(input),
        authorization: new Headers(init?.headers).get("authorization"),
      });
      return new Response("Grist unavailable", { status: 503 });
    }) as typeof fetch;

    try {
      const src = liveSource(cfg);

      expect(await src.roadmap()).toEqual(FIXTURE_ROADMAP);
      expect(src.provenance().roadmap).toBe("fixtures (grist unreachable: grist roadmap read failed)");
      expect(requests).toEqual([
        {
          url: "http://grist.test/api/docs/test-roadmap-doc/tables/Roadmap/records",
          authorization: "Bearer test-grist-api-key",
        },
      ]);
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
