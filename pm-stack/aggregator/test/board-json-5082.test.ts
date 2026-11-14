import { describe, expect, test } from "bun:test";
import { boardJsonBody } from "../src/board-json.ts";
import { DONE_STRIP_RENDER_LIMIT } from "../src/board-policy.ts";
import type { BoardItem, GhItem, RefKey } from "../src/model.ts";

// #5082: `/api/board.json` is the machine twin of `/board`, so it carries
// the same bounded board (done policy + collapsed-strip limit) and only
// the items the visible cards resolve. The unbounded form — every card
// plus the full item closure — was ~2.4 MB and blew past the aggregator's
// own 2 MB stale-cache limit.

const DAY = 24 * 60 * 60 * 1000;
const now = Date.now();
const iso = (msAgo: number) => new Date(now - msAgo).toISOString();

const item = (ref: RefKey): GhItem => ({
  ref,
  kind: "issue",
  url: `https://github.com/${ref.replace("#", "/issues/")}`,
  title: `item ${ref}`,
  state: "open",
  labels: [],
  author: "someone",
  updatedAt: iso(DAY),
});

const card = (title: string, status: BoardItem["status"], links: string[], updatedAt: string = iso(0)): BoardItem => ({
  id: `kaneo:code/${title}`,
  title,
  repo: "developing-today/code",
  source: "kaneo",
  status,
  updatedAt,
  links: links.map((ref) => ({ ref, relation: "tracks" as const })),
});

const itemsMap = (refs: string[]): Map<RefKey, GhItem> => new Map(refs.map((ref) => [ref, item(ref)] as const));

describe("boardJsonBody (#5082)", () => {
  test("returns every live card and only the items those cards link", () => {
    const board = [
      card("live", "in-progress", ["developing-today/code#1"]),
      card("review", "in-review", ["developing-today/code#2", "developing-today/code#3"]),
    ];
    const items = itemsMap([
      "developing-today/code#1",
      "developing-today/code#2",
      "developing-today/code#3",
      // Not linked by any visible card: must not ship.
      "developing-today/code#999",
    ]);

    const body = boardJsonBody(board, items);

    expect(body.board.map((c) => c.title)).toEqual(["live", "review"]);
    expect(body.items.map((i) => i.ref).sort()).toEqual([
      "developing-today/code#1",
      "developing-today/code#2",
      "developing-today/code#3",
    ]);
  });

  test("drops done cards outside the 7-day window and the retired Done-table class", () => {
    const board = [
      card("live", "in-progress", ["developing-today/code#1"]),
      card("recent finish", "done", ["developing-today/code#2"], iso(2 * DAY)),
      card("ancient finish", "done", ["developing-today/code#3"], iso(30 * DAY)),
      card("ROADMAP Done table omits merged PRs", "done", ["developing-today/code#4"], iso(DAY)),
    ];
    const items = itemsMap([
      "developing-today/code#1",
      "developing-today/code#2",
      "developing-today/code#3",
      "developing-today/code#4",
    ]);

    const body = boardJsonBody(board, items);

    expect(body.board.map((c) => c.title)).toEqual(["live", "recent finish"]);
    // The aged-out and retired-class cards' items must not leak in.
    expect(body.items.map((i) => i.ref).sort()).toEqual(["developing-today/code#1", "developing-today/code#2"]);
  });

  test("caps the done strip at the render limit, newest first, and bounds items with it", () => {
    const done = Array.from({ length: DONE_STRIP_RENDER_LIMIT + 5 }, (_, i) =>
      card(`done-${i}`, "done", [`developing-today/code#d${i}`], iso(i * 60_000)),
    );
    const board = [card("live", "in-progress", ["developing-today/code#1"]), ...done];
    const allRefs = ["developing-today/code#1", ...done.flatMap((c) => c.links.map((l) => l.ref))];

    const body = boardJsonBody(board, itemsMap(allRefs));

    expect(body.board.filter((c) => c.status === "done")).toHaveLength(DONE_STRIP_RENDER_LIMIT);
    // done-0 is the newest (0 minutes ago); done-104 was pushed out.
    expect(body.board.map((c) => c.title)).toContain("done-0");
    expect(body.board.map((c) => c.title)).not.toContain(`done-${DONE_STRIP_RENDER_LIMIT + 4}`);
    expect(body.items).toHaveLength(1 + DONE_STRIP_RENDER_LIMIT);
    expect(body.items.map((i) => i.ref)).not.toContain(`developing-today/code#d${DONE_STRIP_RENDER_LIMIT + 4}`);
  });

  test("showAllDone restores the full done strip and its items (the ?done=all escape hatch)", () => {
    const done = Array.from({ length: DONE_STRIP_RENDER_LIMIT + 5 }, (_, i) =>
      card(`done-${i}`, "done", [`developing-today/code#d${i}`], iso(i * 60_000)),
    );
    const items = itemsMap(done.flatMap((c) => c.links.map((l) => l.ref)));

    const body = boardJsonBody(done, items, { showAllDone: true });

    expect(body.board).toHaveLength(done.length);
    expect(body.items).toHaveLength(done.length);
  });

  test("a custom limit bounds the strip for a caller that is not the page", () => {
    const done = Array.from({ length: 10 }, (_, i) =>
      card(`done-${i}`, "done", [`developing-today/code#d${i}`], iso(i * 60_000)),
    );
    const body = boardJsonBody(done, itemsMap(done.flatMap((c) => c.links.map((l) => l.ref))), { limit: 3 });

    expect(body.board).toHaveLength(3);
    expect(body.items).toHaveLength(3);
    expect(body.board.map((c) => c.title)).toEqual(["done-0", "done-1", "done-2"]);
  });

  test("is bounded even when the board is the whole completion log", () => {
    const done = Array.from({ length: 2400 }, (_, i) =>
      card(`done-${i}`, "done", [`developing-today/code#d${i}`], iso(DAY)),
    );
    const items = itemsMap(done.flatMap((c) => c.links.map((l) => l.ref)));

    const bounded = boardJsonBody(done, items);
    const unbounded = { board: done, items: [...items.values()] };

    expect(bounded.board.length).toBe(DONE_STRIP_RENDER_LIMIT);
    expect(bounded.items.length).toBe(DONE_STRIP_RENDER_LIMIT);
    expect(JSON.stringify(bounded, null, 2).length).toBeLessThan(JSON.stringify(unbounded, null, 2).length / 10);
  });
});
