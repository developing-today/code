import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  convertToChecklist,
  needsDrewAnswerHandle,
  raiseNeedsDrew,
  recordAnswer,
  resolveBrokerAnswer,
  validateNeedsDrew,
  type NeedsDrewInput,
} from "../src/needs-drew.ts";

// #14149: generalised question schema (radio / checkbox / text) and the
// per-type resolution of a broker-sourced answer into {handle, approve}.

const tempDirs: string[] = [];

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

async function storeFile(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "needs-drew-14149-"));
  tempDirs.push(dir);
  return join(dir, "needs-drew.json");
}

const approveDeny = [
  { id: "approve", label: "Approve" },
  { id: "deny", label: "Deny" },
];

async function brokerQuestion(extra: Partial<NeedsDrewInput> = {}, handle = "session") {
  const file = await storeFile();
  const ledgerId = `actions-exec-ext:${handle}`;
  const raised = await raiseNeedsDrew(
    file,
    { ledgerId, question: "Extend the run?", options: approveDeny, links: [], ...extra },
    "fleet-actions-exec",
    ledgerId,
  );
  return { file, ledgerId, raised };
}

describe("validateNeedsDrew accepts the generalised fields (#14149 §1)", () => {
  const base = { ledgerId: "q-1", question: "Pick", options: approveDeny };

  test("absent fields keep today's radio shape", () => {
    const v = validateNeedsDrew(base);
    expect(v.ok).toBe(true);
    if (v.ok) {
      expect(v.value.answerType).toBeUndefined();
      expect(v.value.resolutionMap).toBeUndefined();
    }
  });

  test("answerType, optionSet, resolution and resolutionMap round-trip", () => {
    const v = validateNeedsDrew({
      ...base,
      answerType: "checkbox",
      optionSet: { setId: "approve-deny", version: 1 },
      resolution: "any-deny",
      resolutionMap: { approve: true, deny: false },
    });
    expect(v).toEqual({
      ok: true,
      value: {
        ledgerId: "q-1",
        question: "Pick",
        options: approveDeny,
        links: [],
        answerType: "checkbox",
        optionSet: { setId: "approve-deny", version: 1 },
        resolution: "any-deny",
        resolutionMap: { approve: true, deny: false },
      },
    });
  });

  test("bad values are refused with a named field", () => {
    expect(validateNeedsDrew({ ...base, answerType: "slider" })).toMatchObject({ ok: false });
    expect(validateNeedsDrew({ ...base, optionSet: { setId: "x", version: 0 } })).toMatchObject({ ok: false });
    expect(validateNeedsDrew({ ...base, resolution: "majority" })).toMatchObject({ ok: false });
    expect(validateNeedsDrew({ ...base, resolutionMap: { maybe: true } })).toMatchObject({
      ok: false,
      error: 'needsDrew.resolutionMap names unknown option "maybe"',
    });
    expect(validateNeedsDrew({ ...base, resolutionMap: { approve: "yes" } })).toMatchObject({ ok: false });
  });
});

describe("raise freezes the resolution map for broker questions", () => {
  test("a broker question gets the approve/deny map at raise", async () => {
    const { raised } = await brokerQuestion();
    expect(raised.resolutionMap).toEqual({ approve: true, deny: false });
  });

  test("a non-broker question carries no map", async () => {
    const file = await storeFile();
    const raised = await raiseNeedsDrew(file, { ledgerId: "plain", question: "?", options: approveDeny, links: [] });
    expect(raised.resolutionMap).toBeUndefined();
  });

  test("an explicit map is stored as given", async () => {
    const { raised } = await brokerQuestion({
      options: [...approveDeny, { id: "later", label: "Later" }],
      resolutionMap: { approve: true, deny: false, later: false },
    });
    expect(raised.resolutionMap).toEqual({ approve: true, deny: false, later: false });
  });
});

describe("resolveBrokerAnswer (#14149 §2)", () => {
  test("radio: a mapped option resolves", async () => {
    const { file, ledgerId } = await brokerQuestion();
    const q = await recordAnswer(file, ledgerId, { optionId: "deny", via: "test" });
    expect(resolveBrokerAnswer(q!)).toEqual({ kind: "resolved", handle: "session", approve: false });
  });

  test("radio: free text with no option is unresolvable, never dropped", async () => {
    const { file, ledgerId } = await brokerQuestion();
    const q = await recordAnswer(file, ledgerId, { freeText: "only on Tuesdays", via: "test" });
    expect(resolveBrokerAnswer(q!)).toEqual({
      kind: "unresolvable",
      handle: "session",
      reason: "free-text answer has no approve/deny mapping",
    });
  });

  test("checkbox unanimous: same-sided selection resolves, mixed is unresolvable", async () => {
    const opts = [...approveDeny, { id: "approve-once", label: "Approve once" }];
    const map = { approve: true, deny: false, "approve-once": true };
    const a = await brokerQuestion({ answerType: "checkbox", options: opts, resolutionMap: map }, "s:a");
    const qa = await recordAnswer(a.file, a.ledgerId, { optionIds: ["approve", "approve-once"], via: "test" });
    expect(resolveBrokerAnswer(qa!)).toEqual({ kind: "resolved", handle: "s:a", approve: true });

    const b = await brokerQuestion({ answerType: "checkbox", options: opts, resolutionMap: map }, "s:b");
    const qb = await recordAnswer(b.file, b.ledgerId, { optionIds: ["approve", "deny"], via: "test" });
    expect(resolveBrokerAnswer(qb!)).toMatchObject({ kind: "unresolvable", reason: expect.stringContaining("mixed") });
  });

  test("checkbox: empty selection and unmapped options are unresolvable", async () => {
    const opts = [...approveDeny, { id: "note", label: "Note" }];
    const a = await brokerQuestion({ answerType: "checkbox", options: opts }, "s:empty");
    const qa = await recordAnswer(a.file, a.ledgerId, { optionIds: [], freeText: "hmm", via: "test" });
    expect(resolveBrokerAnswer(qa!)).toMatchObject({ kind: "unresolvable", reason: "empty selection" });

    const b = await brokerQuestion({ answerType: "checkbox", options: opts }, "s:note");
    const qb = await recordAnswer(b.file, b.ledgerId, { optionIds: ["approve", "note"], via: "test" });
    expect(resolveBrokerAnswer(qb!)).toMatchObject({
      kind: "unresolvable",
      reason: 'option "note" has no approve/deny mapping',
    });
  });

  test("checkbox any-deny: one deny wins; otherwise approve", async () => {
    const a = await brokerQuestion({ answerType: "checkbox", resolution: "any-deny" }, "s:ad1");
    const qa = await recordAnswer(a.file, a.ledgerId, { optionIds: ["approve", "deny"], via: "test" });
    expect(resolveBrokerAnswer(qa!)).toEqual({ kind: "resolved", handle: "s:ad1", approve: false });

    const b = await brokerQuestion({ answerType: "checkbox", resolution: "any-deny" }, "s:ad2");
    const qb = await recordAnswer(b.file, b.ledgerId, { optionIds: ["approve"], via: "test" });
    expect(resolveBrokerAnswer(qb!)).toEqual({ kind: "resolved", handle: "s:ad2", approve: true });
  });

  test("checkbox all-approve: every approve-mapped option must be ticked", async () => {
    const opts = [
      { id: "tests-pass", label: "Tests pass" },
      { id: "docs-updated", label: "Docs updated" },
      { id: "deny", label: "Deny" },
    ];
    const map = { "tests-pass": true, "docs-updated": true, deny: false };
    const extra = {
      answerType: "checkbox" as const,
      resolution: "all-approve" as const,
      options: opts,
      resolutionMap: map,
    };
    const a = await brokerQuestion(extra, "s:aa1");
    const qa = await recordAnswer(a.file, a.ledgerId, { optionIds: ["tests-pass"], via: "test" });
    expect(resolveBrokerAnswer(qa!)).toEqual({ kind: "resolved", handle: "s:aa1", approve: false });

    const b = await brokerQuestion(extra, "s:aa2");
    const qb = await recordAnswer(b.file, b.ledgerId, { optionIds: ["tests-pass", "docs-updated"], via: "test" });
    expect(resolveBrokerAnswer(qb!)).toEqual({ kind: "resolved", handle: "s:aa2", approve: true });
  });

  test("text: always unresolvable", async () => {
    const { file, ledgerId } = await brokerQuestion({ answerType: "text" });
    const q = await recordAnswer(file, ledgerId, { freeText: "go ahead", via: "test" });
    expect(resolveBrokerAnswer(q!)).toMatchObject({ kind: "unresolvable" });
    expect(q!.answer!.answerType).toBe("text");
  });

  test("non-broker and unanswered questions resolve to null", async () => {
    const file = await storeFile();
    await raiseNeedsDrew(file, { ledgerId: "plain", question: "?", options: approveDeny, links: [] });
    const plain = await recordAnswer(file, "plain", { optionId: "approve", via: "test" });
    expect(resolveBrokerAnswer(plain!)).toBeNull();
    const { raised } = await brokerQuestion();
    expect(resolveBrokerAnswer(raised)).toBeNull();
  });

  test("a question stored before #14149 (no map) resolves by the approve/deny ids", async () => {
    const { file, ledgerId } = await brokerQuestion();
    const q = await recordAnswer(file, ledgerId, { optionId: "approve", via: "test" });
    const legacy = { ...q! };
    delete legacy.resolutionMap;
    expect(resolveBrokerAnswer(legacy)).toEqual({ kind: "resolved", handle: "session", approve: true });
  });
});

describe("radio -> checklist conversion is lossless (#14149 §3)", () => {
  test("ids, answer, mapping and frozen map survive; the handle still resolves", async () => {
    const { file, ledgerId } = await brokerQuestion();
    const answered = await recordAnswer(file, ledgerId, { optionId: "approve", via: "test" });
    const converted = convertToChecklist(answered!, [{ id: "approve-once", label: "Approve once" }]);

    expect(converted.answerType).toBe("checkbox");
    expect(converted.options.map((o) => o.id)).toEqual(["approve", "deny", "approve-once"]);
    expect(converted.answer).toEqual(answered!.answer);
    expect(converted.brokerMapping).toEqual(answered!.brokerMapping);
    expect(converted.resolutionMap).toEqual(answered!.resolutionMap);
    expect(needsDrewAnswerHandle(converted)).toBe("session");
    // The radio answer reads as a one-item selection after conversion.
    expect(resolveBrokerAnswer(converted)).toEqual({ kind: "resolved", handle: "session", approve: true });
  });

  test("conversion refuses to reuse an existing id", async () => {
    const { raised } = await brokerQuestion();
    expect(() => convertToChecklist(raised, [{ id: "deny", label: "Other deny" }])).toThrow();
  });

  test("checkbox answers de-duplicate their ticks", async () => {
    const { file, ledgerId } = await brokerQuestion({ answerType: "checkbox" });
    const q = await recordAnswer(file, ledgerId, { optionIds: ["approve", "approve"], via: "test" });
    expect(q!.answer!.optionIds).toEqual(["approve"]);
  });
});
