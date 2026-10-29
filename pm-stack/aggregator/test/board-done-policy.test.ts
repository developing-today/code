import { describe, expect, test } from "bun:test";
import type { BoardItem } from "../src/model.ts";
import { renderBoardBody } from "../src/render.ts";
import { applyBoardDonePolicy, isRetiredDoneTableTitle } from "../src/board-policy.ts";

// #1146: the board is a WIP view, not a completion log. Done cards
// older than 7 days drop off entirely, the surviving done renders
// collapsed behind a "Done recently (N)" expander, and Kaneo cards of
// the retired ROADMAP Done-table bookkeeping class are filtered at
// the aggregator.

const DAY = 24 * 60 * 60 * 1000;
const now = Date.now();
const iso = (msAgo: number) => new Date(now - msAgo).toISOString();

const card = (title: string, status: BoardItem["status"], updatedAt?: string): BoardItem => ({
  id: `kaneo:code/${title}`,
  title,
  repo: "developing-today/code",
  source: "kaneo",
  status,
  updatedAt,
  links: [],
});

describe("board done policy (#1146)", () => {
  test("isRetiredDoneTableTitle matches the retired bookkeeping class", () => {
    expect(isRetiredDoneTableTitle("ROADMAP Done table omits merged PRs")).toBe(true);
    expect(isRetiredDoneTableTitle("roadmap  done  table refresh")).toBe(true);
    expect(isRetiredDoneTableTitle("Roadmap planning for Q4")).toBe(false);
    expect(isRetiredDoneTableTitle("Done table manners")).toBe(false);
  });

  test("applyBoardDonePolicy drops old done and the Done-table class, keeps the rest", () => {
    const board = [
      card("Recent finish", "done", iso(1 * DAY)),
      card("Ancient finish", "done", iso(30 * DAY)),
      card("ROADMAP Done table omits merged PRs", "done", iso(1 * DAY)),
      card("Undated finish", "done"),
      card("Live work", "in-progress", iso(30 * DAY)),
      card("Under review", "in-review", iso(30 * DAY)),
    ];
    const out = applyBoardDonePolicy(board, now).map((i) => i.title);
    expect(out).toContain("Recent finish");
    expect(out).toContain("Undated finish"); // age unproven — kept
    expect(out).not.toContain("Ancient finish");
    expect(out).not.toContain("ROADMAP Done table omits merged PRs");
    expect(out).toContain("Live work"); // age-out applies to done only
    expect(out).toContain("Under review");
  });

  test("renderBoardBody renders only recent done, inside a collapsed expander", () => {
    const board = [
      card("Live work", "in-progress", iso(0)),
      card("Recent finish", "done", iso(2 * DAY)),
      card("Ancient finish", "done", iso(30 * DAY)),
      card("ROADMAP Done table omits merged PRs", "done", iso(1 * DAY)),
    ];
    const html = renderBoardBody(board, new Map());
    expect(html).toContain("Recent finish");
    expect(html).not.toContain("Ancient finish");
    expect(html).not.toContain("ROADMAP Done table");
    // Collapsed expander with the surviving count, after the live lanes.
    expect(html).toContain("<details class=\"donestrip\">");
    expect(html).toContain("Done recently (1)");
    expect(html).not.toContain("done (1)</h2>");
    expect(html.indexOf("Live work")).toBeLessThan(html.indexOf("Done recently"));
    // The recent card lives inside the expander, not a full column.
    expect(html.indexOf("Done recently")).toBeLessThan(html.indexOf("Recent finish"));
  });
});
