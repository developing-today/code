import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  decisionSetAging,
  decisionSetLastActivityMs,
  decisionSetsForView,
  dismissDecisionSets,
  isDecisionSetView,
  sweepDecisionSets,
  type DecisionSet,
} from "../src/decisions.ts";

// Decision-set aging policy (#14064): rolling recent view, abandoned
// vs active vs settled statuses, the answered-elsewhere sweep, and
// bulk dismiss. The one-time Nov-16 live-store sweep is an operator
// follow-up against the live store, not something a unit test runs.

let dir: string;
let file: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "decisions-aging-test-"));
  file = join(dir, "decisions.json");
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

function makeSet(
  setId: string,
  createdAt: string,
  questions: Array<{ qid: string; resolutionAt?: string; options?: Array<{ id: string; label: string }> }>,
): DecisionSet {
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
        questions: questions.map((q) => ({
          qid: q.qid,
          question: `${q.qid}?`,
          options: q.options ?? [{ id: "opt-1", label: "Yes" }],
          ...(q.resolutionAt ? { resolution: { kind: "answered" as const, at: q.resolutionAt, via: "test" } } : {}),
        })),
      },
    ],
  };
}

async function writeStore(sets: DecisionSet[]): Promise<void> {
  await writeFile(file, JSON.stringify({ version: 1, sets }, null, 2));
}

describe("decision-set aging (#14064)", () => {
  test("recent view is the rolling last-7-days window; view names validate", () => {
    expect(isDecisionSetView("recent")).toBe(true);
    expect(isDecisionSetView("all")).toBe(true);
    expect(isDecisionSetView("fortnight")).toBe(false);
    const now = Date.parse("2026-11-24T12:00:00.000Z");
    const fresh = makeSet("fresh", "2026-11-20T12:00:00.000Z", [{ qid: "fresh-item#1" }]);
    const stale = makeSet("stale", "2026-11-16T12:00:00.000Z", [{ qid: "stale-item#1" }]);
    const recent = decisionSetsForView([stale, fresh], "recent", now).map((s) => s.setId);
    expect(recent).toEqual(["fresh"]);
    expect(decisionSetsForView([stale, fresh], "all", now).map((s) => s.setId)).toEqual(["fresh", "stale"]);
  });

  test("aging statuses: settled wins, idle-open is abandoned, fresh-open is active", () => {
    const now = Date.parse("2026-11-24T12:00:00.000Z");
    const settled = makeSet("settled", "2026-10-01T12:00:00.000Z", [
      { qid: "settled-item#1", resolutionAt: "2026-10-02T12:00:00.000Z" },
    ]);
    const abandoned = makeSet("abandoned", "2026-11-16T12:00:00.000Z", [{ qid: "abandoned-item#1" }]);
    const active = makeSet("active", "2026-11-23T12:00:00.000Z", [{ qid: "active-item#1" }]);
    // Answered long ago is settled, not abandoned.
    expect(decisionSetAging(settled, now)).toBe("settled");
    expect(decisionSetAging(abandoned, now)).toBe("abandoned");
    expect(decisionSetAging(active, now)).toBe("active");
    // Last activity follows the latest resolution, not raise time.
    expect(decisionSetLastActivityMs(settled)).toBe(Date.parse("2026-10-02T12:00:00.000Z"));
  });

  test("sweep closes answered-elsewhere questions; first resolution wins", async () => {
    const open1 = makeSet("s1", "2026-11-16T12:00:00.000Z", [
      { qid: "s1-item#1", options: [{ id: "opt-1", label: "Yes" }] },
    ]);
    await writeStore([open1]);
    const result = await sweepDecisionSets(
      file,
      [{ qid: "s1-item#1", optionId: "opt-1", comment: "ruled in chat" }],
      "test-sweep",
      Date.parse("2026-11-24T12:00:00.000Z"),
    );
    expect(result.applied).toBe(1);
    expect(result.skipped).toEqual([]);
    expect(result.aging).toEqual([{ setId: "s1", status: "settled" }]);
    // A second sweep for the same question is skipped, never overwritten.
    const again = await sweepDecisionSets(
      file,
      [{ qid: "s1-item#1", optionId: "opt-1" }],
      "test-sweep",
      Date.parse("2026-11-24T12:00:00.000Z"),
    );
    expect(again.applied).toBe(0);
    expect(again.skipped).toEqual([{ qid: "s1-item#1", reason: "already-resolved" }]);
  });

  test("sweep reports bad options, empty answers, and unknown questions", async () => {
    const open1 = makeSet("s1", "2026-11-16T12:00:00.000Z", [{ qid: "s1-item#1" }, { qid: "s1-item#2" }]);
    await writeStore([open1]);
    const result = await sweepDecisionSets(
      file,
      [
        { qid: "s1-item#1", optionId: "nope" },
        { qid: "s1-item#2" },
        { qid: "ghost-item#9", freeText: "answered elsewhere" },
      ],
      "test-sweep",
      Date.parse("2026-11-24T12:00:00.000Z"),
    );
    expect(result.applied).toBe(0);
    expect(result.skipped.map((s) => s.reason).sort()).toEqual(["bad-option", "empty-answer", "unknown-question"]);
  });

  test("dismiss closes every open question as a lean-none deferral; unknown ids reported", async () => {
    const s1 = makeSet("s1", "2026-11-16T12:00:00.000Z", [{ qid: "s1-item#1" }, { qid: "s1-item#2" }]);
    const s2 = makeSet("s2", "2026-11-16T12:00:00.000Z", [
      { qid: "s2-item#1", resolutionAt: "2026-11-17T12:00:00.000Z" },
    ]);
    await writeStore([s1, s2]);
    const result = await dismissDecisionSets(file, ["s1", "s2", "ghost"], "test-dismiss");
    expect(result.dismissed).toBe(2);
    expect(result.sets).toEqual(["s1"]);
    expect(result.unknown).toEqual(["ghost"]);
  });
});
