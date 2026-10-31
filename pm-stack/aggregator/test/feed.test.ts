import { describe, expect, test } from "bun:test";
import { isFleetFeed, isProjectPoolSection, poolFromFeed, projectPoolSectionProblem } from "../src/feed.ts";
import { isProjectPoolItem, parseProjectItems } from "../src/projects.ts";

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
    quota: { pools: [] },
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
