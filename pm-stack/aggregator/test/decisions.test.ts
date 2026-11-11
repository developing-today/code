import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  decisionSetOpenState,
  decisionSetProgress,
  deferOpenQuestions,
  getDecisionSet,
  isDecisionSetFullyResolved,
  listDecisionResolutions,
  listOpenDecisionSets,
  raiseDecisionSet,
  recordResolutions,
  validateDecisionSet,
  type DecisionSetInput,
} from "../src/decisions.ts";

let dir: string;
let file: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "decisions-test-"));
  file = join(dir, "decisions.json");
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

function sampleInput(setId = "set-test-1"): DecisionSetInput {
  return {
    setId,
    title: "Test set",
    items: [
      {
        ledgerId: "item-a",
        title: "Item A",
        group: "Group 1",
        links: ["https://github.com/developing-today/code/issues/1"],
        questions: [
          {
            question: "A Q1?",
            context: "context a1",
            options: [
              { id: "opt-1", label: "Yes" },
              { id: "opt-2", label: "No" },
            ],
          },
          { question: "A Q2 (free text only)?", options: [] },
        ],
      },
      {
        ledgerId: "item-b",
        title: "Item B",
        links: [],
        questions: [{ question: "B Q1?", options: [{ id: "opt-1", label: "Go" }] }],
      },
    ],
  };
}

describe("validateDecisionSet (#8766)", () => {
  test("accepts a well-formed set, including a zero-option question", () => {
    const v = validateDecisionSet(sampleInput());
    expect(v.ok).toBe(true);
  });

  test("rejects duplicate ledgerIds and duplicate option ids", () => {
    const dup = sampleInput();
    dup.items.push({ ...dup.items[0]! });
    expect(validateDecisionSet(dup).ok).toBe(false);
    const dupOpt = sampleInput();
    dupOpt.items[0]!.questions[0]!.options.push({ id: "opt-1", label: "Again" });
    expect(validateDecisionSet(dupOpt).ok).toBe(false);
  });

  test("rejects a bad setId and an item with no questions", () => {
    expect(validateDecisionSet({ ...sampleInput(), setId: "has space" }).ok).toBe(false);
    const noQ = sampleInput();
    noQ.items[0]!.questions = [];
    expect(validateDecisionSet(noQ).ok).toBe(false);
  });

  test("rejects non-http links", () => {
    const bad = sampleInput();
    bad.items[0]!.links = ["javascript:alert(1)"];
    expect(validateDecisionSet(bad).ok).toBe(false);
  });
});

describe("decision set store (#8766)", () => {
  test("raise assigns qids by 1-based index and a token; re-raise is idempotent", async () => {
    const set = await raiseDecisionSet(file, sampleInput());
    expect(set.token.length).toBeGreaterThan(10);
    expect(set.items[0]!.questions.map((q) => q.qid)).toEqual(["item-a#1", "item-a#2"]);
    expect(set.items[0]!.group).toBe("Group 1");
    expect(set.items[1]!.group).toBe("Decisions");
    const again = await raiseDecisionSet(file, { ...sampleInput(), title: "Changed title" });
    expect(again.token).toBe(set.token);
    expect(again.title).toBe("Test set");
  });

  test("partial submit: a subset of an item's questions resolves, the rest stay open", async () => {
    await raiseDecisionSet(file, sampleInput());
    const res = await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "opt-2" },
    ]);
    expect(res?.outcomes).toEqual([{ qid: "item-a#1", applied: true }]);
    const state = decisionSetOpenState(res!.set);
    expect(state.remainingOpenQuestions).toBe(2);
    expect(state.items.find((i) => i.ledgerId === "item-a")?.openQids).toEqual(["item-a#2"]);
    expect(state.items.find((i) => i.ledgerId === "item-a")?.fullyResolved).toBe(false);
    const progress = decisionSetProgress(res!.set);
    expect(progress.answered).toBe(1);
    expect(isDecisionSetFullyResolved(res!.set)).toBe(false);
  });

  test("first resolution wins; bad option and empty answer are skipped per-submission", async () => {
    await raiseDecisionSet(file, sampleInput());
    const res = await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "nope" },
      { ledgerId: "item-a", qid: "item-a#2", kind: "answered" },
      { ledgerId: "item-b", qid: "item-b#1", kind: "answered", optionId: "opt-1" },
      { ledgerId: "item-b", qid: "item-b#1", kind: "deferred", lean: "approve" },
      { ledgerId: "ghost", qid: "ghost#1", kind: "answered", optionId: "opt-1" },
    ]);
    expect(res?.outcomes).toEqual([
      { qid: "item-a#1", applied: false, reason: "bad-option" },
      { qid: "item-a#2", applied: false, reason: "empty-answer" },
      { qid: "item-b#1", applied: true },
      { qid: "item-b#1", applied: false, reason: "already-resolved" },
      { qid: "ghost#1", applied: false, reason: "unknown-question" },
    ]);
  });

  test("defer records the lean; free-text-only questions answer with text", async () => {
    await raiseDecisionSet(file, sampleInput());
    const res = await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "deferred", lean: "skeptical" },
      { ledgerId: "item-a", qid: "item-a#2", kind: "answered", freeText: "Rockbot was the 2024 prototype" },
    ]);
    expect(res?.outcomes.every((o) => o.applied)).toBe(true);
    const records = await listDecisionResolutions(file);
    expect(records).toHaveLength(2);
    expect(records[0]).toMatchObject({ ledgerId: "item-a", questionIndex: 1, kind: "deferred", lean: "skeptical" });
    expect(records[1]).toMatchObject({
      ledgerId: "item-a",
      questionIndex: 2,
      kind: "answered",
      freeText: "Rockbot was the 2024 prototype",
    });
  });

  test("bulk defer: checked items only, then all remaining; resolved questions untouched", async () => {
    await raiseDecisionSet(file, sampleInput());
    await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "opt-1" },
    ]);
    const checked = await deferOpenQuestions(file, "set-test-1", { ledgerIds: ["item-b"] }, "none");
    expect(checked?.deferredCount).toBe(1);
    // item-a#2 is still open: it was not in the checked scope.
    expect(decisionSetOpenState(checked!.set).remainingOpenQuestions).toBe(1);
    const rest = await deferOpenQuestions(file, "set-test-1", { allRemaining: true }, "approve");
    expect(rest?.deferredCount).toBe(1);
    expect(isDecisionSetFullyResolved(rest!.set)).toBe(true);
    const records = await listDecisionResolutions(file);
    // The answered question kept its answer (not overwritten by defers).
    expect(records.find((r) => r.qid === "item-a#1")?.kind).toBe("answered");
    expect(records.find((r) => r.qid === "item-a#2")?.lean).toBe("approve");
  });

  test("listOpenDecisionSets drops fully resolved and 7-day-old sets", async () => {
    await raiseDecisionSet(file, sampleInput("set-open"));
    await raiseDecisionSet(file, sampleInput("set-done"));
    await deferOpenQuestions(file, "set-done", { allRemaining: true }, "none");
    const open = await listOpenDecisionSets(file);
    expect(open.map((s) => s.setId)).toEqual(["set-open"]);
    const future = await listOpenDecisionSets(file, Date.now() + 8 * 24 * 60 * 60_000);
    expect(future).toEqual([]);
    expect(await getDecisionSet(file, "set-open")).not.toBeNull();
    expect(await getDecisionSet(file, "nope")).toBeNull();
  });
});
