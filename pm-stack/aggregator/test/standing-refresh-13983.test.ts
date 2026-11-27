import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "path";
import { getDecisionSet, listDecisionSets, recordResolutions } from "../src/decisions.ts";
import { STANDING_OPEN_QUESTIONS_SET_ID } from "../src/standing-seed.ts";
import { refreshStandingDecisionSets, standingInputFromLedger } from "../src/standing-refresh.ts";

// #13983 slice 2: the standing open-questions set is refreshed from
// the live pending-questions ledger on cadence. Oracle for every
// test below: the built DecisionSetInput (for the ledger mapping)
// and the decisions store itself (for refresh/carry-over) — no
// mocks of the code under test.

let dir: string;
let file: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "standing-refresh-13983-test-"));
  file = join(dir, "decisions.json");
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

function ledgerFixture(): Record<string, unknown> {
  return {
    version: 1,
    items: [
      {
        id: "alpha",
        title: "Alpha question",
        status: "asked",
        links: ["https://example.com/alpha", "not-a-url"],
        questions: [
          { q: "Pick one?", context: "ctx", options: ["Keep pace", "Throttle it"] },
          { q: "Free text?", context: "no options recorded" },
        ],
      },
      {
        id: "beta",
        title: "Beta question",
        status: "pending",
        links: [],
        questions: [{ q: "Beta?", options: [{ id: "yes", label: "Yes" }, { key: "no", label: "No" }] }],
      },
      {
        id: "gamma",
        title: "Gamma answered",
        status: "answered",
        links: [],
        questions: [{ q: "Gamma?", options: [] }],
      },
      {
        id: "delta",
        title: "Delta dropped",
        status: "dropped",
        links: [],
        questions: [{ q: "Delta?", options: [] }],
      },
    ],
  };
}

describe("ledger mapping (#13983 slice 2)", () => {
  test("only open (asked/pending) items are mirrored; string options slugify, dict options keep ids, non-http links drop — oracle: the built input", () => {
    const build = standingInputFromLedger(ledgerFixture());
    expect(build.openItems).toBe(2);
    expect(build.skippedItems).toBe(0);
    expect(build.truncated).toBe(false);
    expect(build.input.setId).toBe(STANDING_OPEN_QUESTIONS_SET_ID);
    expect(build.input.items.map((i) => i.ledgerId)).toEqual(["alpha", "beta"]);
    const alpha = build.input.items[0];
    expect(alpha?.links).toEqual(["https://example.com/alpha"]);
    expect(alpha?.questions[0]?.options).toEqual([
      { id: "keep-pace", label: "Keep pace" },
      { id: "throttle-it", label: "Throttle it" },
    ]);
    expect(alpha?.questions[1]?.options).toEqual([]);
    const beta = build.input.items[1];
    expect(beta?.questions[0]?.options).toEqual([
      { id: "yes", label: "Yes" },
      { id: "no", label: "No" },
    ]);
  });

  test("malformed ledger and no-open-items ledger are refused — oracle: the thrown error", () => {
    expect(() => standingInputFromLedger(null)).toThrow("items array");
    expect(() => standingInputFromLedger({ items: "nope" })).toThrow("items array");
    expect(() => standingInputFromLedger({ items: [{ id: "x", title: "X", status: "answered", questions: [{ q: "?" }] }] })).toThrow(
      "no open items",
    );
  });
});

describe("refresh through the #9360 path (#13983 slice 2)", () => {
  test("refresh creates the set from the ledger, a re-refresh preserves the token and carries a resolution by qid, and an answered item leaves the mirror — oracle: the stored set", async () => {
    const first = await refreshStandingDecisionSets(file, ledgerFixture());
    expect(first.created).toBe(true);
    expect(first.refreshed).toBe(false);
    expect(first.set.items.map((i) => i.ledgerId)).toEqual(["alpha", "beta"]);

    const applied = await recordResolutions(file, STANDING_OPEN_QUESTIONS_SET_ID, [
      { ledgerId: "alpha", qid: "alpha#1", kind: "answered", optionId: "keep-pace" },
    ]);
    expect(applied?.outcomes).toEqual([{ qid: "alpha#1", applied: true }]);

    const ledger2 = ledgerFixture();
    (ledger2.items as Record<string, unknown>[])[1]!.status = "answered"; // beta leaves the mirror
    const second = await refreshStandingDecisionSets(file, ledger2);
    expect(second.created).toBe(false);
    expect(second.refreshed).toBe(true);
    expect(second.set.token).toBe(first.set.token);
    expect(second.set.createdAt).toBe(first.set.createdAt);
    expect(second.set.items.map((i) => i.ledgerId)).toEqual(["alpha"]);
    const q1 = second.set.items[0]?.questions[0];
    expect(q1?.resolution?.kind).toBe("answered");
    expect(q1?.resolution?.optionId).toBe("keep-pace");
    const sets = await listDecisionSets(file);
    expect(sets.filter((s) => s.setId === STANDING_OPEN_QUESTIONS_SET_ID).length).toBe(1);
  });

  test("a no-open-items refresh leaves the store untouched — oracle: the stored set before/after", async () => {
    const first = await refreshStandingDecisionSets(file, ledgerFixture());
    await expect(
      refreshStandingDecisionSets(file, { items: [{ id: "x", title: "X", status: "answered", questions: [{ q: "?" }] }] }),
    ).rejects.toThrow("no open items");
    const stored = await getDecisionSet(file, STANDING_OPEN_QUESTIONS_SET_ID);
    expect(stored?.token).toBe(first.set.token);
    expect(stored?.items.map((i) => i.ledgerId)).toEqual(["alpha", "beta"]);
  });
});
