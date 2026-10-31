import { describe, expect, test } from "bun:test";
import { isFleetFeed, isProjectPoolSection, loadFeed, poolFromFeed, projectPoolSectionProblem, sanitizeQuotaView } from "../src/feed.ts";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { isProjectPoolItem, parseProjectItems } from "../src/projects.ts";
import type { QuotaPool, QuotaView, QuotaWindow } from "../src/model.ts";

// Minimal valid pool item: edges entirely absent (the missing-edge
// shape must validate cleanly — absence is data, not an error).
const EDGElESS = {
  id: "PVTI_0",
  databaseId: 100,
  url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=100",
  projectUrl: "https://github.com/orgs/developing-today/projects/4",
  ref: "developing-today/code#2399",
  repo: "developing-today/code",
  number: 2399,
  issueUrl: "https://github.com/developing-today/code/issues/2399",
  title: "contract",
  state: "open",
  contentKind: "issue",
  labels: [],
  author: "dezren39",
  updatedAt: "2026-10-29T00:00:00Z",
  status: null,
  area: null,
  horizon: null,
  kind: "Task",
};

// Fully graphed item: every edge field present and well-shaped.
const GRAPHED = {
  ...EDGElESS,
  id: "PVTI_1",
  ref: "developing-today/code#2402",
  number: 2402,
  parentRef: "developing-today/code#2401",
  blockedBy: ["developing-today/code#910"],
  blocks: ["developing-today/code#2403"],
  epicRef: "developing-today/code#2401",
};

describe("feed project-pool section gate (#2399)", () => {
  test("absent and empty sections are valid", () => {
    expect(projectPoolSectionProblem(undefined)).toBeNull();
    expect(isProjectPoolSection(undefined)).toBe(true);
    expect(projectPoolSectionProblem([])).toBeNull();
  });

  test("edgeless and graphed items both validate (lockstep with the collector)", () => {
    expect(isProjectPoolItem(EDGElESS)).toBe(true);
    expect(isProjectPoolItem(GRAPHED)).toBe(true);
    expect(projectPoolSectionProblem([EDGElESS, GRAPHED])).toBeNull();
    expect(isProjectPoolSection([EDGElESS, GRAPHED])).toBe(true);
  });

  test("malformed edges fail the item, diagnosed by index", () => {
    expect(projectPoolSectionProblem("nope")).toBe("not an array");
    const badEdges = { ...GRAPHED, blockedBy: ["ok#1", 42] };
    expect(isProjectPoolItem(badEdges)).toBe(false);
    expect(projectPoolSectionProblem([EDGElESS, badEdges])).toBe("item 1");
    expect(isProjectPoolSection([EDGElESS, badEdges])).toBe(false);
    expect(projectPoolSectionProblem([{ ...EDGElESS, parentRef: 7 }])).toBe("item 0");
    expect(projectPoolSectionProblem([{ ...EDGElESS, epicRef: null }])).toBe("item 0");
    expect(projectPoolSectionProblem([{ ...EDGElESS, blocks: "developing-today/code#1" }])).toBe("item 0");
  });

  test("poolFromFeed is fail-soft: garbage sections yield no items, bad items drop", () => {
    expect(poolFromFeed(null)).toEqual([]);
    expect(poolFromFeed(undefined)).toEqual([]);
    expect(poolFromFeed({})).toEqual([]);
    expect(poolFromFeed({ projects: "nope" as unknown as never })).toEqual([]);
    expect(poolFromFeed({ projects: { items: [] } as unknown as never })).toEqual([]);
    const mixed = poolFromFeed({ projects: [EDGElESS, { nope: true }, GRAPHED] as unknown as never });
    expect(mixed.map((p) => p.ref)).toEqual(["developing-today/code#2399", "developing-today/code#2402"]);
  });

  test("collector-parsed pools (graphed and edgeless) pass the feed gate", () => {
    const pool = parseProjectItems({
      data: {
        node: {
          items: {
            nodes: [
              {
                id: "PVTI_1",
                type: "ISSUE",
                content: {
                  number: 2402,
                  title: "child",
                  state: "OPEN",
                  url: "https://github.com/developing-today/code/issues/2402",
                  repository: { nameWithOwner: "developing-today/code" },
                  updatedAt: "2026-10-29T00:00:00Z",
                  parent: {
                    number: 2401,
                    repository: { nameWithOwner: "developing-today/code" },
                  },
                  blockedBy: { nodes: [] },
                  blocking: { nodes: [] },
                },
                fieldValues: { nodes: [] },
              },
              {
                id: "PVTI_2",
                type: "ISSUE",
                content: {
                  number: 2407,
                  title: "plain",
                  state: "OPEN",
                  url: "https://github.com/developing-today/code/issues/2407",
                  repository: { nameWithOwner: "developing-today/code" },
                  updatedAt: "2026-10-29T00:00:00Z",
                },
                fieldValues: { nodes: [] },
              },
            ],
          },
        },
      },
    });
    expect(pool.length).toBe(2);
    expect(isProjectPoolSection(pool)).toBe(true);
    expect(poolFromFeed({ projects: pool }).length).toBe(2);
  });
});

describe("isFleetFeed generatedAt stamps (#2578 Phase 2, #2242)", () => {
  const feed = (generatedAt: string) => ({
    version: 1,
    generatedAt,
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
  });

  test("a past real stamp validates; an impossible date does not", () => {
    expect(isFleetFeed(feed("2026-10-01T00:00:00Z"))).toBe(true);
    expect(isFleetFeed(feed("2026-02-30T00:00:00Z"))).toBe(false);
  });

  test("a stamp far in the future is invalid, not maximally fresh", () => {
    const future = new Date(Date.now() + 3_600_000).toISOString();
    expect(isFleetFeed(feed(future))).toBe(false);
  });
});

describe("feed quota section gate + sanitization (#2429)", () => {
  const WINDOW: QuotaWindow = { name: "5h", unit: "tokens", limit: 1000, used: 250, resetsAt: "2026-10-29T05:00:00Z", percent: 25 };
  const POOL: QuotaPool = { id: "opencode-go", provider: "opencode", plan: "go", state: "healthy", windows: [WINDOW] };
  const CHOICES = { hasHeadroom: ["opencode-go"], burnBeforeReset: [], doNotStart: [] };
  const feedWith = (quota: unknown) => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota,
    attention: [],
  });
  const quotaWith = (pools: unknown, extra: Record<string, unknown> = {}) => ({
    pools,
    burn: [],
    choices: CHOICES,
    ...extra,
  });

  test("a feed missing burn or choices is rejected whole (old-collector shape)", () => {
    expect(isFleetFeed(feedWith({ pools: [] }))).toBe(false);
    expect(isFleetFeed(feedWith({ pools: [], burn: [] }))).toBe(false);
    expect(isFleetFeed(feedWith(quotaWith([])))).toBe(true);
  });

  test("pools: [null] drops the pool, keeps the feed", () => {
    const out = sanitizeQuotaView(quotaWith([null, POOL]) as never);
    expect(out.droppedPools).toBe(1);
    expect(out.view.pools).toEqual([POOL]);
  });

  test("a pool missing windows drops; a non-array windows is never iterated", () => {
    const noWindows = { id: "a", provider: "p", plan: "x", state: "near" };
    const stringWindows = { ...POOL, id: "b", windows: "soon" };
    const out = sanitizeQuotaView(quotaWith([noWindows, stringWindows, POOL]) as never);
    expect(out.droppedPools).toBe(2);
    expect(out.view.pools).toEqual([POOL]);
  });

  test("windows: [null] drops the bad window, keeps pool and good windows", () => {
    const pool = { ...POOL, windows: [null, WINDOW, { bogus: true }] };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);
    expect(out.droppedPools).toBe(0);
    expect(out.droppedWindows).toBe(2);
    expect(out.view.pools[0]?.windows).toEqual([WINDOW]);
  });

  test("malformed burn segments drop individually", () => {
    const seg = { provider: "opencode", pool: "go", from: "2026-10-01T00:00:00Z", to: "2026-10-01T01:00:00Z", tokens: 42 };
    const out = sanitizeQuotaView(quotaWith([POOL], { burn: [seg, null, { provider: "x" }] }) as never);
    expect(out.droppedBurn).toBe(2);
    expect(out.view.burn).toEqual([seg]);
  });

  test("a fully well-formed quota view is unchanged", () => {
    const q = quotaWith([POOL], { burn: [] }) as unknown as QuotaView;
    const out = sanitizeQuotaView(q);
    expect(out.droppedPools + out.droppedWindows + out.droppedBurn).toBe(0);
    expect(out.view).toEqual(q);
  });

  test("loadFeed returns the sanitized remainder with drop counts attached", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-quota-"));
    const file = join(dir, "feed.json");
    const pool = { ...POOL, windows: [WINDOW, null] };
    writeFileSync(file, JSON.stringify(feedWith(quotaWith([null, pool]))));
    const loaded = await loadFeed(file);
    expect(loaded).not.toBeNull();
    expect(loaded?.quota.pools.length).toBe(1);
    expect(loaded?.quota.pools[0]?.windows).toEqual([WINDOW]);
    expect(loaded?.quotaDropped).toEqual({ pools: 1, windows: 1, burn: 0 });
  });

  test("loadFeed rejects a feed missing burn/choices (fixture-fallback path)", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-quota-"));
    const file = join(dir, "feed.json");
    writeFileSync(file, JSON.stringify(feedWith({ pools: [POOL] })));
    expect(await loadFeed(file)).toBeNull();
  });
});
