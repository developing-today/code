import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { summarizeItemCoverage } from "../src/health.ts";
import type { GhItem, Provenance, RefKey } from "../src/model.ts";
import { parseProjectItems } from "../src/projects.ts";
import { renderItem } from "../src/render.ts";
import { liveSource } from "../src/sources/live.ts";

// #1141: every item Source.items() serves carries its resolution source
// (pool | projector | fixture); fixture-resolved items render hollow +
// "unverified"; health publishes the live-vs-fixture coverage ratio
// computed from the same resolution pass.

const GQL = {
  data: {
    node: {
      title: "Developing Today",
      number: 4,
      url: "https://github.com/orgs/developing-today/projects/4",
      items: {
        totalCount: 1,
        pageInfo: { hasNextPage: false, endCursor: null },
        nodes: [
          {
            id: "PVTI_1",
            databaseId: 111,
            type: "ISSUE",
            content: {
              number: 508,
              title: "EPIC: Basement cluster",
              state: "OPEN",
              url: "https://github.com/dezren39/nix/issues/508",
              repository: { nameWithOwner: "dezren39/nix" },
              updatedAt: "2026-10-20T12:05:58Z",
              author: { login: "dezren39" },
            },
            fieldValues: { nodes: [{ name: "In Progress", field: { name: "Status" } }] },
          },
        ],
      },
    },
  },
};

async function liveCfg(projectorUrl: string, withPool: boolean) {
  const dir = await mkdtemp(join(tmpdir(), "pm-res-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(
    feedFile,
    JSON.stringify({
      version: 1,
      generatedAt: new Date().toISOString(),
      quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
      attention: [],
      projects: withPool ? parseProjectItems(GQL) : [],
    }),
  );
  return loadConfig({ PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile, PM_PROJECTOR_URL: projectorUrl });
}

const gh = (ref: RefKey, resolution?: GhItem["resolution"]): GhItem => ({
  ref,
  kind: "issue",
  url: `https://github.com/${ref.replace("#", "/issues/")}`,
  title: `Item ${ref}`,
  state: "open",
  labels: [],
  author: "dezren39",
  updatedAt: "2026-10-29T00:00:00Z",
  ...(resolution ? { resolution } : {}),
});

describe("items() resolution tagging (#1141)", () => {
  test("pool refs tag pool; refs outside the pool fall back to fixtures and tag fixture", async () => {
    const src = liveSource(await liveCfg("off", true));
    const items = await src.items(["dezren39/nix#508", "developing-today/code#752", "developing-today/code#999999"]);
    expect(items.get("dezren39/nix#508")?.resolution).toBe("pool");
    // developing-today/code#752 exists only in the fixture data.
    expect(items.get("developing-today/code#752")?.resolution).toBe("fixture");
    expect(items.has("developing-today/code#999999")).toBe(false);
  });

  test("projector-served refs tag projector", async () => {
    const row = {
      ref: "dezren39/nix#501",
      repo: "dezren39/nix",
      number: 501,
      kind: "issue",
      url: "https://github.com/dezren39/nix/issues/501",
      title: "Basement cluster",
      state: "open",
      state_reason: null,
      draft: 0,
      merged: 0,
      author: "dezren39",
      labels: "[]",
      assignees: "[]",
      created_at: "2026-10-24T11:00:00Z",
      updated_at: "2026-10-24T13:40:00Z",
      closed_at: null,
      merged_at: null,
    };
    const server = Bun.serve({
      port: 0,
      fetch: () => Response.json({ items: [row] }),
    });
    try {
      const src = liveSource(await liveCfg(`http://127.0.0.1:${server.port}`, false));
      const items = await src.items(["dezren39/nix#501"]);
      expect(items.get("dezren39/nix#501")?.resolution).toBe("projector");
    } finally {
      server.stop(true);
    }
  });
});

describe("summarizeItemCoverage (#1141)", () => {
  test("counts live vs fixture, ignores untagged, ratio is the live share", () => {
    const items = new Map<RefKey, GhItem>([
      ["a/b#1", gh("a/b#1", "pool")],
      ["a/b#2", gh("a/b#2", "projector")],
      ["a/b#3", gh("a/b#3", "fixture")],
      ["a/b#4", gh("a/b#4")],
    ]);
    const cov = summarizeItemCoverage(items);
    expect(cov.resolvedLive).toBe(2);
    expect(cov.fixture).toBe(1);
    expect(cov.total).toBe(3);
    expect(cov.ratio).toBeCloseTo(2 / 3);
  });

  test("empty pass reports total 0 and a null ratio — never an invented number", () => {
    const cov = summarizeItemCoverage(new Map());
    expect(cov.total).toBe(0);
    expect(cov.ratio).toBeNull();
  });
});

describe("fixture-resolved badges render unverified (#1141)", () => {
  const PROV = {
    board: "x", roadmap: "x", items: "x", comments: "x", quota: "x", usage: "x",
    attention: "x", orchestration: "x", dns: "x", registry: "x", aa: "x",
  } as Provenance;

  test("item page: fixture badge is hollow + named unverified; pool badge is untouched", () => {
    const items = new Map<RefKey, GhItem>([
      ["developing-today/code#752", gh("developing-today/code#752", "fixture")],
      ["dezren39/nix#508", gh("dezren39/nix#508", "pool")],
    ]);
    const links = [
      { ref: "developing-today/code#752", relation: "tracks" as const },
      { ref: "dezren39/nix#508", relation: "tracks" as const },
    ];
    const html = renderItem("Subject", "subj", links, items, [], PROV);
    expect(html).toContain("badge open unverified");
    expect(html).toContain("developing-today/code#752 · unverified");
    expect(html).toContain("unverified — fixture state, not live GitHub data");
    expect(html).not.toContain("dezren39/nix#508 · unverified");
  });
});
