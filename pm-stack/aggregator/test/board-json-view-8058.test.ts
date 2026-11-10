import { describe, expect, test } from "bun:test";
import { boardJsonViewBody } from "../src/board-json.ts";
import { DONE_STRIP_RENDER_LIMIT } from "../src/board-policy.ts";
import type { BoardItem, GhItem, RefKey } from "../src/model.ts";

// #8058: the /api/board.json view branch applied the done-strip limit
// (newest 100 done cards globally) BEFORE the view filter, so a view
// whose matching done cards fell outside the newest 100 returned no
// done cards at all while /board showed them. boardJsonViewBody is
// the handler's composition: done policy, then the view filter
// against the full item lookup, then the strip limit.

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

const card = (
  title: string,
  status: BoardItem["status"],
  links: string[],
  updatedAt: string = iso(0),
): BoardItem => ({
  id: `kaneo:code/${title}`,
  title,
  repo: "developing-today/code",
  source: "kaneo",
  status,
  updatedAt,
  links: links.map((ref) => ({ ref, relation: "tracks" as const })),
});

const itemsMap = (refs: string[]): Map<RefKey, GhItem> =>
  new Map(refs.map((ref) => [ref, item(ref)] as const));

const isMatch = (c: BoardItem) => c.title.startsWith("match");

describe("boardJsonViewBody (#8058)", () => {
  test("returns a view's done cards that fall outside the global newest 100", () => {
    // 105 newer done cards the view does not match...
    const others = Array.from({ length: DONE_STRIP_RENDER_LIMIT + 5 }, (_, i) =>
      card(`other-${i}`, "done", [`developing-today/code#o${i}`], iso(i * 60_000)),
    );
    // ...push the view's 3 matching done cards out of the global strip.
    const matching = [0, 1, 2].map((i) =>
      card(`match-${i}`, "done", [`developing-today/code#m${i}`], iso((200 + i) * 60_000)),
    );
    const board = [...others, ...matching];
    const allRefs = board.flatMap((c) => c.links.map((l) => l.ref));

    const body = boardJsonViewBody(board, itemsMap(allRefs), isMatch);

    expect(body.board.map((c) => c.title).sort()).toEqual(["match-0", "match-1", "match-2"]);
    expect(body.items.map((i) => i.ref).sort()).toEqual([
      "developing-today/code#m0",
      "developing-today/code#m1",
      "developing-today/code#m2",
    ]);
  });

  test("still bounds the filtered strip at the render limit, newest first", () => {
    const matching = Array.from({ length: DONE_STRIP_RENDER_LIMIT + 5 }, (_, i) =>
      card(`match-${i}`, "done", [`developing-today/code#m${i}`], iso(i * 60_000)),
    );
    const allRefs = matching.flatMap((c) => c.links.map((l) => l.ref));

    const body = boardJsonViewBody(matching, itemsMap(allRefs), isMatch);

    expect(body.board).toHaveLength(DONE_STRIP_RENDER_LIMIT);
    expect(body.board.map((c) => c.title)).toContain("match-0");
    expect(body.board.map((c) => c.title)).not.toContain(
      `match-${DONE_STRIP_RENDER_LIMIT + 4}`,
    );
    expect(body.items).toHaveLength(DONE_STRIP_RENDER_LIMIT);
  });

  test("done policy still applies before the filter: aged-out and retired-class matches stay out", () => {
    const board = [
      card("match-recent", "done", ["developing-today/code#m1"], iso(DAY)),
      card("match-ancient", "done", ["developing-today/code#m2"], iso(30 * DAY)),
      card("match ROADMAP Done table x", "done", ["developing-today/code#m3"], iso(DAY)),
    ];
    const allRefs = board.flatMap((c) => c.links.map((l) => l.ref));

    const body = boardJsonViewBody(board, itemsMap(allRefs), isMatch);

    expect(body.board.map((c) => c.title)).toEqual(["match-recent"]);
    expect(body.items.map((i) => i.ref)).toEqual(["developing-today/code#m1"]);
  });
});
