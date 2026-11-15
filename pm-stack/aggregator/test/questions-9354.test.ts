import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  decisionSetSummary,
  getDecisionSet,
  raiseDecisionSet,
  recordResolutions,
  refreshDecisionSet,
  type DecisionSet,
  type DecisionSetInput,
} from "../src/decisions.ts";
import { questionsSectionHtml, navLinks } from "../src/render.ts";
import { primaryGroupedPages } from "../src/pages.ts";

// #9354: the decision-set question stack surfaces inside Today /
// Needs-you and in the primary nav, and standing sets can be
// refreshed in place. Oracle for every test below: the decisions
// store itself (for refresh) and the rendered HTML string (for the
// section / nav) — no mocks of the code under test.

let dir: string;
let file: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "questions-9354-test-"));
  file = join(dir, "decisions.json");
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

function input(setId = "standing-1", question = "Keep this?"): DecisionSetInput {
  return {
    setId,
    title: "Standing set",
    items: [
      {
        ledgerId: "item-a",
        title: "Item A",
        links: [],
        questions: [
          { question, options: [{ id: "yes", label: "Yes" }] },
          { question: "Second question?", options: [] },
        ],
      },
    ],
  };
}

describe("refreshDecisionSet (#9354)", () => {
  test("creates when absent — oracle: the stored set is a fresh raise with a token and the input contents", async () => {
    const set = await refreshDecisionSet(file, input(), "test-source");
    expect(set.setId).toBe("standing-1");
    expect(set.token.length).toBeGreaterThan(0);
    expect(set.source).toBe("test-source");
    expect(set.items[0]?.questions.map((q) => q.qid)).toEqual(["item-a#1", "item-a#2"]);
    const stored = await getDecisionSet(file, "standing-1");
    expect(stored?.title).toBe("Standing set");
  });

  test("refresh preserves token + createdAt + source, replaces title/items, carries a resolution by qid, carries versions for identical wording, and drops absent questions — oracle: the stored set after refresh", async () => {
    const created = await raiseDecisionSet(file, input(), "original-source");
    // Resolve the first question so the refresh must carry it by qid.
    const applied = await recordResolutions(file, "standing-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "yes" },
    ]);
    expect(applied?.outcomes).toEqual([{ qid: "item-a#1", applied: true }]);
    const before = await getDecisionSet(file, "standing-1");
    const beforeQ1 = before?.items[0]?.questions[0];

    const next = input("standing-1", "Keep this?");
    next.title = "Standing set — refreshed";
    // Drop the second question entirely; keep the first with identical wording.
    next.items[0]!.questions = [next.items[0]!.questions[0]!];
    const refreshed = await refreshDecisionSet(file, next, "other-source");

    expect(refreshed.token).toBe(created.token);
    expect(refreshed.createdAt).toBe(created.createdAt);
    expect(refreshed.source).toBe("original-source");
    expect(refreshed.title).toBe("Standing set — refreshed");
    // Dropped question is gone.
    expect(refreshed.items[0]?.questions.map((q) => q.qid)).toEqual(["item-a#1"]);
    const q1 = refreshed.items[0]?.questions[0];
    // Resolution carried by qid.
    expect(q1?.resolution?.kind).toBe("answered");
    expect(q1?.resolution?.optionId).toBe("yes");
    // Identical wording: versions + currentVersion carried.
    expect(q1?.versions).toEqual(beforeQ1?.versions);
    expect(q1?.currentVersion).toBe(beforeQ1?.currentVersion);
    // Exactly one stored set with this id.
    const stored = await getDecisionSet(file, "standing-1");
    expect(stored?.items[0]?.questions.length).toBe(1);
  });

  test("refresh with changed wording does not carry the old version history — oracle: the refreshed question starts its own v1 history with the new text", async () => {
    await raiseDecisionSet(file, input());
    const changed = input("standing-1", "Completely different?");
    const refreshed = await refreshDecisionSet(file, changed);
    const q1 = refreshed.items[0]?.questions[0];
    expect(q1?.question).toBe("Completely different?");
    expect(q1?.versions?.length).toBe(1);
    expect(q1?.versions?.[0]?.question).toBe("Completely different?");
    expect(q1?.currentVersion).toBe(1);
  });
});

function summary(set: DecisionSet) {
  return decisionSetSummary(set);
}

function makeSet(setId: string, createdAt: string, open: number, settled: number): DecisionSet {
  const questions = [
    ...Array.from({ length: open }, (_, i) => ({
      qid: `${setId}-item#${i + 1}`,
      question: `Open ${i + 1}?`,
      options: [],
    })),
    ...Array.from({ length: settled }, (_, i) => ({
      qid: `${setId}-item#${open + i + 1}`,
      question: `Settled ${i + 1}?`,
      options: [],
      resolution: { kind: "answered" as const, at: createdAt, via: "test" },
    })),
  ];
  return {
    setId,
    title: `Set ${setId}`,
    token: `tok-${setId}`,
    createdAt,
    source: "test",
    items: [{ ledgerId: `${setId}-item`, title: "Item", group: "G", links: [], questions }],
  };
}

describe("questionsSectionHtml (#9354)", () => {
  const urlFor = (s: { setId: string; token?: string }) => `/decisions/${s.setId}?t=tok-${s.setId}`;

  test("open counts, token link for an open set, newest first — oracle: the rendered section HTML", () => {
    const older = makeSet("set-old", "2026-11-10T12:00:00.000Z", 2, 1);
    const newer = makeSet("set-new", "2026-11-12T12:00:00.000Z", 1, 0);
    const html = questionsSectionHtml([summary(older), summary(newer)], (s) => urlFor(s));
    expect(html).toContain("<h2");
    expect(html).toContain("Questions");
    expect(html).toContain("2 open sets");
    expect(html).toContain("3 open questions");
    expect(html).toContain('href="/decisions/set-new?t=tok-set-new"');
    expect(html).toContain('href="/decisions/set-old?t=tok-set-old"');
    expect(html).toContain('href="/decisions"');
    expect(html).toContain("Live from the decision-set store at page load.");
    // Newest first.
    expect(html.indexOf("set-new")).toBeLessThan(html.indexOf("set-old"));
  });

  test("fully-resolved sets are excluded — oracle: a settled set contributes no row, link, or count", () => {
    const settled = makeSet("set-done", "2026-11-12T12:00:00.000Z", 0, 2);
    const open = makeSet("set-open", "2026-11-11T12:00:00.000Z", 1, 0);
    const html = questionsSectionHtml([summary(settled), summary(open)], (s) => urlFor(s));
    expect(html).not.toContain("set-done");
    expect(html).toContain("1 open set");
    expect(html).toContain("1 open question");
  });

  test("empty state when nothing is open — oracle: the clear-stack wording plus the index link, and no confident counts", () => {
    const settled = makeSet("set-done", "2026-11-12T12:00:00.000Z", 0, 1);
    for (const html of [
      questionsSectionHtml([], (s) => urlFor(s)),
      questionsSectionHtml([summary(settled)], (s) => urlFor(s)),
    ]) {
      expect(html).toContain("No open questions right now — the stack is clear.");
      expect(html).toContain('href="/decisions"');
    }
  });

  test("null is unavailable, never a zero — oracle: the section says Unavailable and does not claim the stack is clear", () => {
    const html = questionsSectionHtml(null, (s) => urlFor(s));
    expect(html).toContain("Unavailable");
    expect(html).not.toContain("stack is clear");
    expect(html).not.toContain("0 open");
  });
});

describe("decisions in the primary nav (#9354)", () => {
  test("navLinks contains an anchor to /decisions — oracle: the rendered nav-bar HTML", () => {
    // navLinks is the bar every page renders; /decisions must be an anchor there.
    expect(navLinks("/")).toContain('<a href="/decisions"');
    expect(navLinks("/today")).toContain("Decision sets");
  });

  test("the manifest's primary tier includes decisions — oracle: primaryGroupedPages()", () => {
    const flat = primaryGroupedPages().flatMap((g) => g.pages.map((p) => p.id));
    expect(flat).toContain("decisions");
  });
});
