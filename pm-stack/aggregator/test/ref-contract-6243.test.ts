// Ref contract Phase 4 (#6243, contract #6242): closing pinning tests.
// - sessionRecordLine alignment (required disposition 5): an invalid
//   stored ref now renders as plain text (the rule-4 treatment), where
//   it previously was silently omitted by a try/catch; a valid ref is
//   the unchanged anchor control. Probe forms: unsafe-integer and
//   non-GitHub shapes, per the phase acceptance.
// - The internal assertion stays: parseRef/refUrl still throw on a
//   ref the code constructs wrongly (contract rule 1) — the safe
//   treatment lives at the stored/feed boundary, not in parseRef.
// Run: bun test test/ref-contract-6243.test.ts
import { describe, expect, test } from "bun:test";
import { isValidRef, parseRef, refUrl, type RefKey, type SessionRecord } from "../src/model.ts";
import { sessionRecordLine } from "../src/render.ts";

function rec(taskRefs: string[]): SessionRecord {
  return {
    id: "ses_phase4",
    source: "claude-code",
    label: "Phase 4 probe",
    agentId: "claude-code",
    agentLabel: "Muse",
    parentId: null,
    status: "succeeded",
    startedAt: "2026-11-08T00:00:00Z",
    lastSeenAt: "2026-11-08T01:00:00Z",
    taskRefs: taskRefs as RefKey[],
  } as SessionRecord;
}

const GOOD = "developing-today/code#6243";
const BAD_UNSAFE = "developing-today/code#99999999999999999999";
const BAD_SHAPE = "goal:keep-the-four-repos-healthy";

describe("sessionRecordLine renders invalid stored refs as plain text (#6243)", () => {
  test("unsafe-integer ref: no throw, ref visible as plain text, no anchor", () => {
    const html = sessionRecordLine(rec([BAD_UNSAFE]));
    expect(html).toContain(BAD_UNSAFE);
    expect(html).not.toContain(`href="https://github.com/developing-today/code/issues/99999999999999999999"`);
  });
  test("non-GitHub ref: no throw, ref visible as plain text", () => {
    const html = sessionRecordLine(rec([BAD_SHAPE]));
    expect(html).toContain(BAD_SHAPE);
    expect(html).not.toContain("<a href=\"https://github.com/goal");
  });
  test("valid ref control: anchor unchanged", () => {
    const html = sessionRecordLine(rec([GOOD]));
    expect(html).toContain(`<a href="https://github.com/developing-today/code/issues/6243">${GOOD}</a>`);
  });
  test("mixed refs: valid anchors, invalid is plain text, session survives", () => {
    const html = sessionRecordLine(rec([GOOD, BAD_UNSAFE]));
    expect(html).toContain(`<a href="https://github.com/developing-today/code/issues/6243">${GOOD}</a>`);
    expect(html).toContain(BAD_UNSAFE);
  });
});

describe("the internal ref assertion stays total-checked (#6242 rule 1)", () => {
  test("isValidRef is the shared rule", () => {
    expect(isValidRef(GOOD)).toBe(true);
    expect(isValidRef(BAD_UNSAFE)).toBe(false);
    expect(isValidRef(BAD_SHAPE)).toBe(false);
  });
  test("parseRef and refUrl still throw on invalid refs", () => {
    expect(() => parseRef(BAD_UNSAFE as RefKey)).toThrow();
    expect(() => refUrl(BAD_SHAPE as RefKey)).toThrow();
  });
});
