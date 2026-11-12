import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  decisionSetSummary,
  decisionSetsForView,
  isDecisionSetView,
  listDecisionSets,
  listOpenDecisionSets,
  type DecisionSet,
} from "../src/decisions.ts";

// Decision-sets index (#8867): the store listing, per-set summary
// counts, and the America/Chicago today/week view boundaries behind
// GET /decisions.

let dir: string;
let file: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "decisions-index-test-"));
  file = join(dir, "decisions.json");
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

function makeSet(setId: string, createdAt: string, resolutions: Array<"answered" | "deferred" | "request-info" | null>): DecisionSet {
  return {
    setId,
    title: `Set ${setId}`,
    token: `tok-${setId}`,
    createdAt,
    source: "test",
    items: [
      {
        ledgerId: `${setId}-item`,
        title: "Item",
        group: "G",
        links: [],
        questions: resolutions.map((kind, i) => ({
          qid: `${setId}-item#${i + 1}`,
          question: `Q${i + 1}?`,
          options: [],
          ...(kind ? { resolution: { kind, at: createdAt, via: "test" } } : {}),
        })),
      },
    ],
  };
}

async function writeStore(sets: DecisionSet[]): Promise<void> {
  await writeFile(file, JSON.stringify({ version: 1, sets }, null, 2));
}

describe("decision-sets index store listing (#8867)", () => {
  test("lists every set — fully resolved and older than 7 days included", async () => {
    const old = makeSet("set-old", "2026-10-01T12:00:00.000Z", [null]);
    const settled = makeSet("set-settled", "2026-11-11T12:00:00.000Z", ["answered", "deferred"]);
    const fresh = makeSet("set-fresh", "2026-11-11T18:00:00.000Z", [null, "request-info"]);
    await writeStore([old, settled, fresh]);
    const all = await listDecisionSets(file);
    expect(all.map((s) => s.setId)).toEqual(["set-old", "set-settled", "set-fresh"]);
    // Contrast: the open-sets view keeps only the fresh one (settled is
    // fully resolved; old is past the 7-day open window).
    const open = await listOpenDecisionSets(file, Date.parse("2026-11-11T19:00:00.000Z"));
    expect(open.map((s) => s.setId)).toEqual(["set-fresh"]);
  });

  test("empty store: missing file lists as no sets, every view empty", async () => {
    expect(await listDecisionSets(file)).toEqual([]);
    expect(decisionSetsForView([], "all")).toEqual([]);
    expect(decisionSetsForView([], "today")).toEqual([]);
    expect(decisionSetsForView([], "week")).toEqual([]);
  });
});

describe("decisionSetSummary (#8867)", () => {
  test("open counts only unresolved questions — answered, deferred and request-info are settled", () => {
    const set = makeSet("set-x", "2026-11-11T12:00:00.000Z", ["answered", "deferred", "request-info", null, null]);
    const s = decisionSetSummary(set);
    expect(s.items).toBe(1);
    expect(s.questions).toBe(5);
    expect(s.openQuestions).toBe(2);
    expect(s.answered).toBe(1);
    expect(s.deferred).toBe(1);
    expect(s.infoRequested).toBe(1);
    expect(s.fullyResolved).toBe(false);
  });

  test("a set with every question resolved is fully resolved with zero open", () => {
    const s = decisionSetSummary(makeSet("set-y", "2026-11-11T12:00:00.000Z", ["answered", "request-info"]));
    expect(s.openQuestions).toBe(0);
    expect(s.fullyResolved).toBe(true);
  });
});

describe("decisionSetsForView — America/Chicago boundaries (#8867)", () => {
  // Wednesday Nov 11 2026, 12:00 CST = 18:00Z. Chicago is UTC-6 (CST)
  // for all of these instants.
  const NOW = Date.parse("2026-11-11T18:00:00.000Z");

  test("isDecisionSetView accepts only the three views", () => {
    expect(isDecisionSetView("today")).toBe(true);
    expect(isDecisionSetView("week")).toBe(true);
    expect(isDecisionSetView("all")).toBe(true);
    expect(isDecisionSetView("month")).toBe(false);
    expect(isDecisionSetView(undefined)).toBe(false);
    expect(isDecisionSetView(null)).toBe(false);
  });

  test("today: the Chicago calendar day, not the UTC day", () => {
    const justBeforeMidnightChicago = makeSet("a", "2026-11-11T05:59:59.000Z", [null]); // Nov 10, 23:59:59 CST
    const midnightChicago = makeSet("b", "2026-11-11T06:00:00.000Z", [null]); // Nov 11, 00:00:00 CST
    const utcMidnight = makeSet("c", "2026-11-11T00:30:00.000Z", [null]); // Nov 10, 18:30 CST
    const ids = decisionSetsForView([justBeforeMidnightChicago, midnightChicago, utcMidnight], "today", NOW).map((s) => s.setId);
    expect(ids).toEqual(["b"]);
  });

  test("week: Monday 00:00 Chicago through Sunday 23:59:59 Chicago", () => {
    const sundayBefore = makeSet("sun-before", "2026-11-09T05:59:59.000Z", [null]); // Sun Nov 8, 23:59:59 CST
    const mondayStart = makeSet("mon", "2026-11-09T06:00:00.000Z", [null]); // Mon Nov 9, 00:00 CST
    const sundayEnd = makeSet("sun-end", "2026-11-16T05:59:59.000Z", [null]); // Sun Nov 15, 23:59:59 CST
    const mondayAfter = makeSet("mon-after", "2026-11-16T06:00:00.000Z", [null]); // Mon Nov 16, 00:00 CST
    const ids = decisionSetsForView([sundayBefore, mondayStart, sundayEnd, mondayAfter], "week", NOW).map((s) => s.setId);
    expect(ids.sort()).toEqual(["mon", "sun-end"]);
  });

  test("all: everything, newest first", () => {
    const older = makeSet("older", "2026-10-01T12:00:00.000Z", [null]);
    const newer = makeSet("newer", "2026-11-11T12:00:00.000Z", [null]);
    const middle = makeSet("middle", "2026-11-01T12:00:00.000Z", [null]);
    expect(decisionSetsForView([older, newer, middle], "all", NOW).map((s) => s.setId)).toEqual(["newer", "middle", "older"]);
    // "today"/"week" keep the same newest-first order within the view.
    const t1 = makeSet("t1", "2026-11-11T08:00:00.000Z", [null]);
    const t2 = makeSet("t2", "2026-11-11T16:00:00.000Z", [null]);
    expect(decisionSetsForView([t1, t2], "today", NOW).map((s) => s.setId)).toEqual(["t2", "t1"]);
  });

  test("a set with an unparseable createdAt appears only in All", () => {
    const broken = makeSet("broken", "not-a-date", [null]);
    expect(decisionSetsForView([broken], "all", NOW).map((s) => s.setId)).toEqual(["broken"]);
    expect(decisionSetsForView([broken], "today", NOW)).toEqual([]);
    expect(decisionSetsForView([broken], "week", NOW)).toEqual([]);
  });
});
