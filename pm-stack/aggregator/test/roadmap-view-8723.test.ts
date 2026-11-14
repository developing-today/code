import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { filterRoadmapForView } from "../src/board-view.ts";
import type { GhItem, ProjectPoolItem, RefKey, RoadmapRow } from "../src/model.ts";

// #8723: PR #4013 gave the HTML /roadmap handler the keptRefs rule --
// items linked from a KEPT row keep their metadata even when they
// fail the view filter themselves -- but /api/roadmap.json filtered
// items by the view alone, so a kept row in the JSON payload could
// reference items absent from its own items[]. Oracle: the HTML
// handler's rule itself (#4013, Codex P2). Both handlers now compose
// through filterRoadmapForView, so these tests pin the shared
// composition plus the handler wiring, following the #8521 pattern.

const item = (ref: RefKey, author = "someone", state: GhItem["state"] = "open"): GhItem => ({
  ref,
  kind: "issue",
  url: `https://github.com/${ref.replace("#", "/issues/")}`,
  title: `item ${ref}`,
  state,
  labels: [],
  author,
  updatedAt: new Date().toISOString(),
});

const row = (subject: string, area: string, links: string[]): RoadmapRow => ({
  id: subject.length,
  subject,
  title: `row ${subject}`,
  area,
  status: "active",
  horizon: "Now",
  source: "grist",
  notes: "",
  links: links.map((ref) => ({ ref: ref as RefKey, relation: "tracks" as const })),
});

const noPool = new Map<RefKey, ProjectPoolItem>();

describe("filterRoadmapForView (#8723, the /roadmap rule shared with /api/roadmap.json)", () => {
  test("an item linked from a kept row ships even when it fails the view itself", () => {
    const rows = [
      row("grist:kept", "Fleet", ["developing-today/code#1", "dezren39/darwin-nix#2"]),
      row("grist:dropped", "Other", ["developing-today/code#3"]),
    ];
    const items = new Map([
      ["developing-today/code#1", item("developing-today/code#1", "alice")],
      ["dezren39/darwin-nix#2", item("dezren39/darwin-nix#2", "bob")],
      ["developing-today/code#3", item("developing-today/code#3", "bob")],
      ["developing-today/code#4", item("developing-today/code#4", "alice")],
    ] as [RefKey, GhItem][]);
    // View keeps only owner alice's items: row "kept" matches via its
    // link to #1; #2 fails the view (bob) but is linked from the kept
    // row, so it ships -- the parity the JSON twin was missing. #3 is
    // linked only from the dropped row; #4 matches directly.
    const out = filterRoadmapForView(rows, items, noPool, { owner: "alice" });
    expect(out.rows.map((r) => r.subject)).toEqual(["grist:kept"]);
    expect([...out.items.keys()].sort()).toEqual([
      "developing-today/code#1",
      "developing-today/code#4",
      "dezren39/darwin-nix#2",
    ]);
  });

  test("a row kept by its own fields keeps all of its linked items", () => {
    const rows = [row("grist:area", "Fleet", ["dezren39/darwin-nix#9"])];
    const items = new Map([["dezren39/darwin-nix#9", item("dezren39/darwin-nix#9", "bob")]] as [RefKey, GhItem][]);
    const out = filterRoadmapForView(rows, items, noPool, { area: "Fleet" });
    expect(out.rows).toHaveLength(1);
    expect([...out.items.keys()]).toEqual(["dezren39/darwin-nix#9"]);
  });

  test("an empty filter set keeps every row and item (the composition's identity case)", () => {
    const rows = [row("grist:a", "Fleet", ["developing-today/code#1"]), row("grist:b", "Other", [])];
    const items = new Map([["developing-today/code#1", item("developing-today/code#1")]] as [RefKey, GhItem][]);
    const out = filterRoadmapForView(rows, items, noPool, {});
    expect(out.rows).toHaveLength(2);
    expect([...out.items.keys()]).toEqual(["developing-today/code#1"]);
  });
});

describe("roadmap handler wiring (#8723 regression pin)", () => {
  const server = readFileSync(join(import.meta.dir, "../src/server.ts"), "utf8");
  const htmlBlock = server.slice(
    server.indexOf('if (path === "/roadmap")'),
    server.indexOf('if (path === "/api/flow-analytics.json")'),
  );
  const jsonBlock = server.slice(
    server.indexOf('if (path === "/api/roadmap.json")'),
    server.indexOf('if (path === "/api/quota.json")'),
  );

  test("the HTML handler filters through the shared composition", () => {
    expect(htmlBlock).toContain("filterRoadmapForView");
  });

  test("the JSON twin filters through the same shared composition, so the twins cannot drift", () => {
    expect(jsonBlock).toContain("filterRoadmapForView");
    expect(jsonBlock).not.toContain("itemMatchesView(ghItemLike(it");
  });

  test("the unfiltered JSON branch stays untouched by view filtering", () => {
    const unfiltered = jsonBlock.slice(0, jsonBlock.indexOf("const pool = await source.projects()"));
    expect(unfiltered).toContain("items: [...items.values()]");
    expect(unfiltered).not.toContain("filterRoadmapForView");
  });
});
