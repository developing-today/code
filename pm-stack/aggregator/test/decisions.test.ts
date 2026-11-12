import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { esc } from "../src/render.ts";
import {
  decisionSetOpenState,
  decisionSetProgress,
  getDecisionDrafts,
  upsertDecisionDrafts,
  validateDecisionDraftUpsert,
  deferOpenQuestions,
  deferWithSubmissions,
  getDecisionSet,
  isDecisionSetFullyResolved,
  listDecisionResolutions,
  listOpenDecisionSets,
  raiseDecisionSet,
  recordResolutions,
  requestInfoOpenQuestions,
  validateDecisionSet,
  decisionsDraftKey,
  decisionsDraftScript,
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

describe("request-info, comments, filled-answers-win (#8766 follow-up)", () => {
  test("request-info records the optional note, counts separately, and is not overwritten in-set", async () => {
    await raiseDecisionSet(file, sampleInput());
    const res = await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "request-info", note: "need the cost numbers" },
      { ledgerId: "item-a", qid: "item-a#2", kind: "request-info" },
    ]);
    expect(res?.outcomes).toEqual([
      { qid: "item-a#1", applied: true },
      { qid: "item-a#2", applied: true },
    ]);
    const progress = decisionSetProgress(res!.set);
    expect(progress.infoRequested).toBe(2);
    expect(progress.answered).toBe(0);
    expect(progress.deferred).toBe(0);
    expect(decisionSetOpenState(res!.set).items.find((i) => i.ledgerId === "item-a")?.openQids).toEqual([]);
    const records = await listDecisionResolutions(file);
    expect(records.find((r) => r.qid === "item-a#1")).toMatchObject({ kind: "request-info", note: "need the cost numbers" });
    expect(records.find((r) => r.qid === "item-a#2")?.note).toBeUndefined();
    // First resolution still wins inside one set: a later answer in the
    // SAME set does not overwrite the request-info.
    const again = await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "opt-1" },
    ]);
    expect(again?.outcomes).toEqual([{ qid: "item-a#1", applied: false, reason: "already-resolved" }]);
  });

  test("bulk request-info: checked items only, resolved questions untouched, shared note", async () => {
    await raiseDecisionSet(file, sampleInput());
    await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "opt-1" },
    ]);
    const out = await requestInfoOpenQuestions(file, "set-test-1", { ledgerIds: ["item-a", "item-b"] }, "missing the timeline");
    expect(out?.requestedCount).toBe(2); // item-a#2 + item-b#1; item-a#1 already answered
    const records = await listDecisionResolutions(file);
    expect(records.find((r) => r.qid === "item-a#1")?.kind).toBe("answered");
    expect(records.find((r) => r.qid === "item-b#1")).toMatchObject({ kind: "request-info", note: "missing the timeline" });
  });

  test("re-raise: an enriched question returns in a later set carrying infoProvided and can be answered there", async () => {
    await raiseDecisionSet(file, sampleInput("set-first"));
    await recordResolutions(file, "set-first", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "request-info", note: "what did it cost?" },
    ]);
    // The orchestrator gathers the information and raises a later set
    // whose question carries it, visibly marked as the requested info.
    const enriched = sampleInput("set-reraise");
    enriched.items[0]!.questions[0]!.infoProvided = "It cost $4,100 over March-May (export linked on the issue).";
    expect(validateDecisionSet(enriched).ok).toBe(true);
    const set2 = await raiseDecisionSet(file, enriched);
    expect(set2.items[0]!.questions[0]!.infoProvided).toContain("$4,100");
    const res = await recordResolutions(file, "set-reraise", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "opt-2" },
    ]);
    expect(res?.outcomes).toEqual([{ qid: "item-a#1", applied: true }]);
    // Both resolutions for the same qid reach the feed, keyed by set.
    const forQ = (await listDecisionResolutions(file)).filter((r) => r.qid === "item-a#1");
    expect(forQ.map((r) => `${r.setId}:${r.kind}`)).toEqual(["set-first:request-info", "set-reraise:answered"]);
    // infoProvided validation: a non-string is rejected.
    const bad = sampleInput("set-bad");
    (bad.items[0]!.questions[0] as { infoProvided?: unknown }).infoProvided = 42;
    expect(validateDecisionSet(bad).ok).toBe(false);
  });

  test("filled answers win over bulk defer: typed answer recorded as a ruling, the rest deferred", async () => {
    await raiseDecisionSet(file, sampleInput());
    const out = await deferWithSubmissions(
      file,
      "set-test-1",
      [
        { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "opt-2" },
        { ledgerId: "item-a", qid: "item-a#2", kind: "deferred", lean: "skeptical", comment: "not without the numbers" },
      ],
      { allRemaining: true },
      "none",
      "bulk note",
    );
    expect(out?.outcomes).toEqual([
      { qid: "item-a#1", applied: true },
      { qid: "item-a#2", applied: true },
    ]);
    expect(out?.deferredCount).toBe(1); // only item-b#1 was still unanswered
    const records = await listDecisionResolutions(file);
    const byQid = new Map(records.map((r) => [r.qid, r]));
    // The filled-in answer is an answer resolution, never a deferral.
    expect(byQid.get("item-a#1")).toMatchObject({ kind: "answered", optionId: "opt-2" });
    // The per-question defer kept its own lean and comment.
    expect(byQid.get("item-a#2")).toMatchObject({ kind: "deferred", lean: "skeptical", comment: "not without the numbers" });
    // The bulk defer took the bulk lean and bulk comment.
    expect(byQid.get("item-b#1")).toMatchObject({ kind: "deferred", lean: "none", comment: "bulk note" });
    expect(isDecisionSetFullyResolved(out!.set)).toBe(true);
  });

  test("comments ride on every kind: answer, defer, request-info", async () => {
    await raiseDecisionSet(file, sampleInput());
    await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "opt-1", comment: "but revisit in Q1" },
      { ledgerId: "item-a", qid: "item-a#2", kind: "deferred", lean: "approve", comment: "fine either way" },
      { ledgerId: "item-b", qid: "item-b#1", kind: "request-info", note: "need dates", comment: "and who owns it" },
    ]);
    const records = await listDecisionResolutions(file);
    expect(records.find((r) => r.qid === "item-a#1")?.comment).toBe("but revisit in Q1");
    expect(records.find((r) => r.qid === "item-a#2")?.comment).toBe("fine either way");
    expect(records.find((r) => r.qid === "item-b#1")).toMatchObject({ note: "need dates", comment: "and who owns it" });
  });
});

describe("guided defer + mixed flavors (#8766 follow-up)", () => {
  test("guided defer: option pick + comment ride along, kind stays deferred", async () => {
    await raiseDecisionSet(file, sampleInput());
    const res = await recordResolutions(file, "set-test-1", [
      {
        ledgerId: "item-a",
        qid: "item-a#1",
        kind: "deferred",
        lean: "skeptical",
        optionId: "opt-1",
        comment: "leaning yes on the tech, no on the timing",
      },
      { ledgerId: "item-a", qid: "item-a#2", kind: "deferred", lean: "none", optionId: "bogus" },
    ]);
    expect(res?.outcomes).toEqual([
      { qid: "item-a#1", applied: true },
      { qid: "item-a#2", applied: false, reason: "bad-option" },
    ]);
    const records = await listDecisionResolutions(file);
    expect(records.find((r) => r.qid === "item-a#1")).toMatchObject({
      kind: "deferred",
      lean: "skeptical",
      optionId: "opt-1",
      optionLabel: "Yes",
      comment: "leaning yes on the tech, no on the timing",
    });
    const progress = decisionSetProgress(res!.set);
    expect(progress.deferred).toBe(1);
    expect(progress.answered).toBe(0);
  });

  test("bulk send-all precedence: guided defer > filled answer > untouched (plain defer)", async () => {
    await raiseDecisionSet(file, sampleInput());
    // One bulk action carrying: item-a#1 = option + comment + defer set
    // (guided defer), item-a#2 = free-text answer only (an answer).
    // item-b#1 is untouched and must defer plain with the bulk lean.
    const out = await deferWithSubmissions(
      file,
      "set-test-1",
      [
        { ledgerId: "item-a", qid: "item-a#1", kind: "deferred", lean: "approve", optionId: "opt-2", comment: "if we must" },
        { ledgerId: "item-a", qid: "item-a#2", kind: "answered", freeText: "Rockbot was the 2024 prototype" },
      ],
      { allRemaining: true },
      "none",
    );
    expect(out?.deferredCount).toBe(1);
    const records = await listDecisionResolutions(file);
    const byQid = new Map(records.map((r) => [r.qid, r]));
    // Defer setting wins over the filled option: kind is deferred, and
    // the option + comment ride as guidance.
    expect(byQid.get("item-a#1")).toMatchObject({
      kind: "deferred",
      lean: "approve",
      optionId: "opt-2",
      optionLabel: "No",
      comment: "if we must",
    });
    // The answer-only question in the same bulk action is an answer.
    expect(byQid.get("item-a#2")).toMatchObject({ kind: "answered", freeText: "Rockbot was the 2024 prototype" });
    // The untouched question defers plain — no flavor stamped on it.
    expect(byQid.get("item-b#1")).toMatchObject({ kind: "deferred", lean: "none" });
    expect(byQid.get("item-b#1")?.optionId).toBeUndefined();
  });
});

describe("bulk default flavor (#8766 follow-up)", () => {
  test("bulk dropdown is only a default: own flavor beats it, filled answer beats it, untouched takes it", async () => {
    await raiseDecisionSet(file, sampleInput());
    // Bulk send-all with the bottom dropdown on "skeptical":
    // item-a#1 has its OWN defer flavor (approve) + a tentative pick —
    // level 1 wins. item-a#2 has only a filled answer — level 2 wins.
    // item-b#1 is untouched — it takes the bulk default, level 3.
    const out = await deferWithSubmissions(
      file,
      "set-test-1",
      [
        { ledgerId: "item-a", qid: "item-a#1", kind: "deferred", lean: "approve", optionId: "opt-1", comment: "guidance" },
        { ledgerId: "item-a", qid: "item-a#2", kind: "answered", freeText: "the answer" },
      ],
      { allRemaining: true },
      "skeptical",
    );
    expect(out?.deferredCount).toBe(1); // only the untouched question is bulk-deferred
    const byQid = new Map((await listDecisionResolutions(file)).map((r) => [r.qid, r]));
    expect(byQid.get("item-a#1")).toMatchObject({ kind: "deferred", lean: "approve", optionId: "opt-1", comment: "guidance" });
    expect(byQid.get("item-a#2")).toMatchObject({ kind: "answered", freeText: "the answer" });
    expect(byQid.get("item-b#1")).toMatchObject({ kind: "deferred", lean: "skeptical" });
  });
});

// --- Crash hardening (#8777) -------------------------------------------
// The 2026-11-11 incident: the aggregator was killed while Drew was
// mid-set on the decisions page. Two properties keep that class of
// failure from ever costing typed work or breaking the page: question
// text reaches the page only through esc(), and in-progress form state
// is continuously drafted to localStorage and restored on load.
describe("decisions page crash hardening (#8777)", () => {
  test("esc neutralizes script-breaking and attribute-breaking text", () => {
    const hostile = `</script><script>alert(1)</script> "quoted" & <b onmouseover="x">`;
    const out = esc(hostile);
    expect(out).not.toContain("</script>");
    expect(out).not.toContain("<script");
    expect(out).not.toContain(`"`);
    expect(out).toContain("&lt;/script&gt;");
    expect(out).toContain("&quot;");
  });

  test("draft key is per-set", () => {
    expect(decisionsDraftKey("set-a")).toBe("decisions-drafts:set-a");
    expect(decisionsDraftKey("set-b")).toBe("decisions-drafts:set-b");
  });

  test("draft script parses as JS and embeds only its own set's key", () => {
    const js = decisionsDraftScript("set-a");
    expect(js).toContain(`"decisions-drafts:set-a"`);
    expect(js).not.toContain("decisions-drafts:set-b");
    expect(() => new Function(js)).not.toThrow();
  });

  test("draft script wires save-on-input, restore-on-load and prune-on-remove", () => {
    const js = decisionsDraftScript("set-a");
    expect(js).toContain("localStorage.setItem");
    expect(js).toContain("localStorage.getItem");
    expect(js).toContain("localStorage.removeItem");
    expect(js).toContain("addEventListener('input', draftSave)");
    expect(js).toContain("addEventListener('change', draftSave)");
    // Recorded questions leave the DOM on submit; the observer re-save
    // is what prunes their drafts so a stale draft can never resurrect.
    expect(js).toContain("MutationObserver(draftSave)");
    expect(js).toContain("draftRestore()");
  });
});

// --- Server-side drafts (#8766 both-layers upgrade) --------------------
// Drafts mirror the page's in-progress form state server-side so work
// survives the device it was typed on. They are NOT resolutions: they
// never enter the feed the catcher consumes, and a question's draft
// dies the moment the question resolves, on every resolution path.
describe("decisions server drafts", () => {
  test("upsert, read back, and clear-on-answer", async () => {
    await raiseDecisionSet(file, sampleInput());
    const up = await upsertDecisionDrafts(file, "set-test-1", {
      questions: {
        "item-a#1": { opt: "opt-2", ft: "draft words", updatedAt: "2026-11-11T20:00:00.000Z" },
        "item-a#2": { note: "need the numbers", updatedAt: "2026-11-11T20:00:00.000Z" },
      },
      bulk: ["item-b"],
      bulkLean: "skeptical",
      bulkNote: "",
    });
    expect(up?.applied.slice().sort()).toEqual(["item-a#1", "item-a#2"]);
    const drafts = await getDecisionDrafts(file, "set-test-1");
    expect(drafts?.questions["item-a#1"]).toMatchObject({ opt: "opt-2", ft: "draft words" });
    expect(drafts?.questions["item-a#2"]).toMatchObject({ note: "need the numbers" });
    expect(drafts?.bulk).toEqual(["item-b"]);
    expect(drafts?.bulkLean).toBe("skeptical");
    // Answering item-a#1 kills only its own draft.
    await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionId: "opt-1" },
    ]);
    const after = await getDecisionDrafts(file, "set-test-1");
    expect(after?.questions["item-a#1"]).toBeUndefined();
    expect(after?.questions["item-a#2"]).toMatchObject({ note: "need the numbers" });
  });

  test("drafts never enter the resolutions feed", async () => {
    await raiseDecisionSet(file, sampleInput());
    await upsertDecisionDrafts(file, "set-test-1", {
      questions: { "item-a#1": { ft: "unsubmitted", updatedAt: "2026-11-11T20:00:00.000Z" } },
    });
    expect(await listDecisionResolutions(file)).toEqual([]);
    await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "deferred", lean: "none" },
    ]);
    const feed = await listDecisionResolutions(file);
    expect(feed).toHaveLength(1);
    expect(feed[0]).toMatchObject({ qid: "item-a#1", kind: "deferred" });
  });

  test("ghost qids are skipped and resolved questions never resurrect", async () => {
    await raiseDecisionSet(file, sampleInput());
    const ghost = await upsertDecisionDrafts(file, "set-test-1", {
      questions: { "nope#9": { ft: "boo", updatedAt: "2026-11-11T20:00:00.000Z" } },
    });
    expect(ghost?.applied).toEqual([]);
    expect(ghost?.skipped).toEqual([{ qid: "nope#9", reason: "unknown-question" }]);
    await recordResolutions(file, "set-test-1", [
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", freeText: "done" },
    ]);
    const zombie = await upsertDecisionDrafts(file, "set-test-1", {
      questions: { "item-a#1": { ft: "too late", updatedAt: "2026-11-11T21:00:00.000Z" } },
    });
    expect(zombie?.skipped).toEqual([{ qid: "item-a#1", reason: "already-resolved" }]);
    expect((await getDecisionDrafts(file, "set-test-1"))?.questions["item-a#1"]).toBeUndefined();
  });

  test("tombstones delete; defer and request-info clear drafts too", async () => {
    await raiseDecisionSet(file, sampleInput());
    await upsertDecisionDrafts(file, "set-test-1", {
      questions: {
        "item-a#1": { ft: "x", updatedAt: "2026-11-11T20:00:00.000Z" },
        "item-a#2": { ft: "y", updatedAt: "2026-11-11T20:00:00.000Z" },
        "item-b#1": { ft: "z", updatedAt: "2026-11-11T20:00:00.000Z" },
      },
    });
    const del = await upsertDecisionDrafts(file, "set-test-1", { questions: { "item-a#1": null } });
    expect(del?.applied).toEqual(["item-a#1"]);
    expect((await getDecisionDrafts(file, "set-test-1"))?.questions["item-a#1"]).toBeUndefined();
    await deferOpenQuestions(file, "set-test-1", { ledgerIds: ["item-a"] }, "approve");
    expect((await getDecisionDrafts(file, "set-test-1"))?.questions["item-a#2"]).toBeUndefined();
    await requestInfoOpenQuestions(file, "set-test-1", { ledgerIds: ["item-b"] });
    expect((await getDecisionDrafts(file, "set-test-1"))?.questions["item-b#1"]).toBeUndefined();
  });

  test("newest updatedAt wins at the store; a stale push is skipped", async () => {
    await raiseDecisionSet(file, sampleInput());
    await upsertDecisionDrafts(file, "set-test-1", {
      questions: { "item-a#1": { ft: "newer", updatedAt: "2026-11-11T21:00:00.000Z" } },
    });
    const stale = await upsertDecisionDrafts(file, "set-test-1", {
      questions: { "item-a#1": { ft: "older", updatedAt: "2026-11-11T20:00:00.000Z" } },
    });
    expect(stale?.skipped).toEqual([{ qid: "item-a#1", reason: "stale" }]);
    expect((await getDecisionDrafts(file, "set-test-1"))?.questions["item-a#1"]?.ft).toBe("newer");
  });

  test("an unknown set reads back null", async () => {
    expect(await getDecisionDrafts(file, "no-such-set")).toBeNull();
    expect(
      await upsertDecisionDrafts(file, "no-such-set", { questions: {} }),
    ).toBeNull();
  });

  test("validateDecisionDraftUpsert rejects malformed payloads", () => {
    expect(validateDecisionDraftUpsert(null).ok).toBe(false);
    expect(validateDecisionDraftUpsert({}).ok).toBe(false);
    expect(validateDecisionDraftUpsert({ questions: { "item-a#1": { updatedAt: "x" } } }).ok).toBe(false);
    expect(validateDecisionDraftUpsert({ questions: { "item-a#1": { lean: "bogus" } } }).ok).toBe(false);
    expect(validateDecisionDraftUpsert({ questions: { "item-a#1": null } }).ok).toBe(true);
    const okRes = validateDecisionDraftUpsert({
      questions: { "item-a#1": { ft: "hi", lean: "approve", updatedAt: "2026-11-11T20:00:00.000Z" } },
      bulk: ["item-a"],
    });
    expect(okRes.ok).toBe(true);
  });

  test("draft script wires IndexedDB, the server drafts API and the merge", () => {
    const js = decisionsDraftScript("set-a");
    expect(js).toContain("indexedDB.open");
    expect(js).toContain("/drafts");
    expect(js).toContain("visibilitychange");
    expect(js).toContain("draftMergeServer");
    expect(js).toContain("draftFetchServer");
    expect(() => new Function(js)).not.toThrow();
  });
});
