import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { filterBoardForView, fragBoardUrl, viewParamsQuery } from "../src/board-view.ts";
import type { BoardItem, GhItem, ProjectPoolItem, RefKey } from "../src/model.ts";

// #8521: PR #4013 applied saved-view filtering (resolveView) to
// /board but not to its Datastar fragment twin /frag/board — the
// fragment rendered the full unfiltered board and its refresh URLs
// carried no view params, so a saved view was silently dropped on
// the fragment path. Oracle: the /board rule itself (#4013) — the
// same view must yield the same card set from both twins, and the
// fragment's own refresh URL must re-name the view. Both handlers
// now compose through filterBoardForView / fragBoardUrl, so these
// tests pin the shared composition plus the handler wiring.

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

const card = (title: string, repo: string, status: BoardItem["status"], links: string[]): BoardItem => ({
  id: `kaneo:${title}`,
  title,
  repo,
  source: "kaneo",
  status,
  updatedAt: new Date().toISOString(),
  links: links.map((ref) => ({ ref, relation: "tracks" as const })),
});

const noPool = new Map<RefKey, ProjectPoolItem>();

describe("filterBoardForView (#8521, the /board rule shared with /frag/board)", () => {
  test("a card is kept when a linked item matches, dropped when neither it nor its links match", () => {
    const board = [
      card("via-link", "dezren39/darwin-nix", "in-progress", ["developing-today/code#1"]),
      card("no-match", "dezren39/darwin-nix", "in-progress", ["dezren39/darwin-nix#2"]),
      card("via-own-repo", "developing-today/code", "in-progress", []),
    ];
    const items = new Map([
      ["developing-today/code#1", item("developing-today/code#1")],
      ["dezren39/darwin-nix#2", item("dezren39/darwin-nix#2")],
    ] as [RefKey, GhItem][]);
    const out = filterBoardForView(board, items, noPool, { repo: "developing-today/code" });
    expect(out.board.map((b) => b.title)).toEqual(["via-link", "via-own-repo"]);
  });

  test("the items map keeps a kept card's non-matching links and directly matching items only", () => {
    const board = [
      card("kept", "example/other", "in-progress", ["developing-today/code#1", "developing-today/code#2"]),
      card("dropped", "example/other", "in-progress", ["developing-today/code#3"]),
    ];
    const items = new Map([
      ["developing-today/code#1", item("developing-today/code#1", "alice")],
      ["developing-today/code#2", item("developing-today/code#2", "bob")],
      ["developing-today/code#3", item("developing-today/code#3", "bob")],
      ["developing-today/code#4", item("developing-today/code#4", "alice")],
    ] as [RefKey, GhItem][]);
    const out = filterBoardForView(board, items, noPool, { owner: "alice" });
    expect(out.board.map((b) => b.title)).toEqual(["kept"]);
    // #1 matches and is linked from the kept card; #2 stays because a
    // kept card links it (its metadata renders on the card); #3 is
    // linked only from the dropped card; #4 matches directly.
    expect([...out.items.keys()].sort()).toEqual([
      "developing-today/code#1",
      "developing-today/code#2",
      "developing-today/code#4",
    ]);
  });

  test("a state filter matches the card's own status", () => {
    const board = [
      card("done-card", "example/other", "done", []),
      card("wip-card", "example/other", "in-progress", []),
    ];
    const out = filterBoardForView(board, new Map(), noPool, { state: "done" });
    expect(out.board.map((b) => b.title)).toEqual(["done-card"]);
  });
});

describe("fragBoardUrl (#8521 view-param propagation)", () => {
  test("carries view, viewer and vf.* params and drops unrelated params", () => {
    const params = new URLSearchParams("view=vw-1&viewer=drew%3Adrew&vf.repo=developing-today%2Fcode&done=all&full=1");
    const url = fragBoardUrl(params);
    expect(url.startsWith("/frag/board?")).toBe(true);
    const q = new URLSearchParams(url.split("?")[1]);
    expect(q.get("view")).toBe("vw-1");
    expect(q.get("viewer")).toBe("drew:drew");
    expect(q.get("vf.repo")).toBe("developing-today/code");
    // #7937: done=all is board view state and is carried (see below);
    // unrelated params like full=1 are still dropped.
    expect(q.get("done")).toBe("all");
    expect(q.get("full")).toBeNull();
  });

  test("is the bare fragment path when no view is named", () => {
    expect(fragBoardUrl(new URLSearchParams("full=1"))).toBe("/frag/board");
    expect(viewParamsQuery(new URLSearchParams())).toBe("");
  });

  test("carries done=all on its own, so a fragment refresh keeps the full Done strip (#7937)", () => {
    expect(fragBoardUrl(new URLSearchParams("done=all"))).toBe("/frag/board?done=all");
    expect(viewParamsQuery(new URLSearchParams("done=all"))).toBe("done=all");
  });
});

describe("/frag/board handler wiring (#8521 regression pin)", () => {
  const server = readFileSync(join(import.meta.dir, "../src/server.ts"), "utf8");
  const fragBlock = server.slice(
    server.indexOf('if (path === "/frag/board")'),
    server.indexOf('if (path === "/today")'),
  );
  const boardBlock = server.slice(
    server.indexOf('if (path === "/board")'),
    server.indexOf('if (path === "/frag/board")'),
  );

  test("the fragment handler resolves the view and filters through the shared composition", () => {
    expect(fragBlock).toContain("resolveView(url)");
    expect(fragBlock).toContain("filterBoardForView");
    expect(fragBlock).toContain("fragBoardUrl(url.searchParams)");
  });

  test("/board uses the same shared composition, so the twins cannot drift", () => {
    expect(boardBlock).toContain("filterBoardForView");
  });

  test("the fragment handler renders show-all Done under ?done=all (#7937)", () => {
    expect(fragBlock).toContain('showAllDone: url.searchParams.get("done") === "all"');
  });
});
