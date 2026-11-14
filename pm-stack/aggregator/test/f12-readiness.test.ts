// #9926: tests for the Phase 12 readiness projection — the #1235
// Tier-2 F-item parser, the #534 design-checklist parser, the
// timeline PR parser, and the join (including a design-but-no-issue
// item and an item with a merged PR). Fixture bodies reproduce the
// real grammars verified against the live issues on 2026-11-14.
import { describe, expect, test } from "bun:test";
import { parseF12Designs, parseF12Items, parseTimelinePrRefs, projectF12Readiness } from "../src/f12-readiness.ts";
import { PHASE12_REPO } from "../src/phase12.ts";
import type { GhItem } from "../src/model.ts";

const PHASE12_BODY = `## How work is picked (so nobody has to guess)
2. **Tier 2 — Phase 12** (label \`next\`): #807 F12.1, #556 F12.2, #835 F12.3 remainder, #1061 F12.4, #1062 F12.5, #1063 F12.6, #1064 F12.7, #1065 F12.8, #1066 F12.9, #1067 F12.10, #805 F12.11, #557 F12.12, #806 F12.13, #1068 F12.14, #558 F12.15, #559 done F12.16, #1069 F12.17, #560 F12.18. Designs run through #534 first.
`;

const DESIGN_BODY = `## Items
- [x] **D1-019 — F12.1 Declarative world manifest (world.toml) with apply / --dry-run / diff** (L, design)
  A world.toml naming module (hash or source), seed, grants, ceilings.
- [ ] **D1-022 — F12.4 Provider registry** (L, design)
- [x] **D1-025 — F12.7 ABI spec** (M, design)
`;

describe("parseF12Items (#1235 Tier-2 grammar)", () => {
  const items = parseF12Items(PHASE12_BODY);
  test("parses all 18 F-items in order with their issue numbers", () => {
    expect(items.map((i) => i.f)).toEqual(Array.from({ length: 18 }, (_, k) => `F12.${k + 1}`));
    expect(items[0]).toEqual({ f: "F12.1", n: 1, issueNumber: 807 });
    expect(items[15]).toEqual({ f: "F12.16", n: 16, issueNumber: 559 });
    expect(items[17]).toEqual({ f: "F12.18", n: 18, issueNumber: 560 });
  });
  test("a body without the Tier-2 line parses to an empty list", () => {
    expect(parseF12Items("## nothing\n")).toEqual([]);
  });
});

describe("parseF12Designs (#534 checklist grammar)", () => {
  const designs = parseF12Designs(DESIGN_BODY);
  test("maps F-numbers to design ids and checkbox state", () => {
    expect(designs.get(1)).toEqual({ designId: "D1-019", done: true });
    expect(designs.get(4)).toEqual({ designId: "D1-022", done: false });
    expect(designs.get(7)).toEqual({ designId: "D1-025", done: true });
  });
  test("F-items with no #534 entry are absent from the map", () => {
    expect(designs.has(2)).toBe(false);
  });
});

describe("parseTimelinePrRefs", () => {
  test("keeps cross-referenced PRs only, marking merged from merged_at", () => {
    const refs = parseTimelinePrRefs([
      {
        event: "cross-referenced",
        source: {
          issue: {
            number: 801,
            state: "closed",
            html_url: "https://x/pull/801",
            pull_request: { merged_at: "2026-10-25T01:41:35Z" },
          },
        },
      },
      {
        event: "cross-referenced",
        source: { issue: { number: 816, state: "open", html_url: "https://x/issues/816" } },
      },
      {
        event: "cross-referenced",
        source: {
          issue: { number: 900, state: "open", html_url: "https://x/pull/900", pull_request: { merged_at: null } },
        },
      },
      { event: "labeled" },
    ]);
    expect(refs).toEqual([
      { number: 801, state: "merged", url: "https://x/pull/801" },
      { number: 900, state: "open", url: "https://x/pull/900" },
    ]);
  });
  test("a non-array response throws", () => {
    expect(() => parseTimelinePrRefs({})).toThrow();
  });
});

describe("projectF12Readiness", () => {
  const items = parseF12Items(PHASE12_BODY);
  const designs = parseF12Designs(DESIGN_BODY);
  const gh = (number: number, state: GhItem["state"]): GhItem => ({
    ref: `${PHASE12_REPO}#${number}`,
    kind: "issue",
    url: `https://github.com/${PHASE12_REPO}/issues/${number}`,
    title: `live title ${number}`,
    state,
    labels: [],
    author: "dezren39",
    updatedAt: "2026-11-01T00:00:00Z",
  });
  // F12.1 (#807) resolved open; F12.16 (#559) resolved closed.
  // F12.4 (#1061) deliberately NOT resolved: design listed, no issue state.
  const ghItems = new Map([
    [`${PHASE12_REPO}#807`, gh(807, "open")],
    [`${PHASE12_REPO}#559`, gh(559, "closed")],
  ]);
  const prRefs = new Map([[559, [{ number: 801, state: "merged" as const, url: "https://x/pull/801" }]]]);
  const rows = projectF12Readiness(items, designs, ghItems, prRefs);
  const byF = new Map(rows.map((r) => [r.f, r]));

  test("a resolved item carries live issue state, title, and design state", () => {
    const row = byF.get("F12.1")!;
    expect(row.issueState).toBe("open");
    expect(row.title).toBe("live title 807");
    expect(row.design).toEqual({ designId: "D1-019", done: true });
    expect(row.prs).toEqual([]);
  });

  test("the design-but-no-issue case: design present, issue state null, never invented", () => {
    const row = byF.get("F12.4")!;
    expect(row.design).toEqual({ designId: "D1-022", done: false });
    expect(row.issueState).toBeNull();
    expect(row.title).toBeNull();
  });

  test("an item with a merged PR carries it, and its closed issue state", () => {
    const row = byF.get("F12.16")!;
    expect(row.issueState).toBe("closed");
    expect(row.prs).toEqual([{ number: 801, state: "merged", url: "https://x/pull/801" }]);
    expect(row.design).toBeNull(); // F12.16 has no #534 design entry
  });
});
