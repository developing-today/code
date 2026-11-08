// #4952: the aggregator's POST boundary validators (comments githubRef,
// actions targetRef) and the comments store-load gate must reject ref keys
// parseRef rejects — GH_REF's number group was an unbounded \d+, so refs
// like owner/repo#99999999999999999999 validated, stored as RefKey, and
// then threw in parseRef/refUrl consumers. The shared rule is isValidRef
// (model.ts), which routes through parseRef.
import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { validateNewAction } from "../src/actions.ts";
import { CommentsStore, validateNewComment } from "../src/comments.ts";
import { isValidRef, parseRef } from "../src/model.ts";

const drew = { kind: "drew" as const, id: "drew", label: "Drew" };
const OVERSIZED = "developing-today/code#99999999999999999999";
const JUST_OVER = "developing-today/code#9007199254740992"; // MAX_SAFE_INTEGER + 1
const MAX_SAFE = "developing-today/code#9007199254740991"; // Number.MAX_SAFE_INTEGER
const NORMAL = "developing-today/code#123";

describe("#4952 isValidRef — the shared parseRef rule", () => {
  test("rejects numbers parseRef rejects, accepts the boundary and ordinary refs", () => {
    expect(isValidRef(OVERSIZED)).toBe(false);
    expect(isValidRef(JUST_OVER)).toBe(false);
    expect(() => parseRef(JUST_OVER as never)).toThrow("not a safe integer");
    expect(isValidRef(MAX_SAFE)).toBe(true);
    expect(isValidRef(NORMAL)).toBe(true);
    expect(isValidRef("not a ref")).toBe(false);
    expect(isValidRef(123)).toBe(false);
    expect(isValidRef(undefined)).toBe(false);
  });
});

describe("#4952 validateNewComment", () => {
  const comment = (githubRef: string) => ({
    objectRef: "gh:developing-today/code#4952",
    kind: "github",
    author: drew,
    body: "see the linked issue",
    githubRef,
  });
  test("rejects oversized githubRef numbers", () => {
    expect(validateNewComment(comment(OVERSIZED)).ok).toBe(false);
    expect(validateNewComment(comment(JUST_OVER)).ok).toBe(false);
  });
  test("accepts MAX_SAFE_INTEGER and ordinary refs", () => {
    expect(validateNewComment(comment(MAX_SAFE)).ok).toBe(true);
    expect(validateNewComment(comment(NORMAL)).ok).toBe(true);
  });
});

describe("#4952 validateNewAction", () => {
  const base = {
    clientToken: "ct-4952-token-1",
    kind: "triage",
    verb: "acknowledge",
    objectRef: "orch:item-1",
    author: drew,
    payload: {},
  };
  test("rejects oversized targetRef, accepts the boundary and ordinary refs", () => {
    expect(validateNewAction({ ...base, targetRef: OVERSIZED }).ok).toBe(false);
    expect(validateNewAction({ ...base, targetRef: JUST_OVER }).ok).toBe(false);
    expect(validateNewAction({ ...base, targetRef: MAX_SAFE }).ok).toBe(true);
    expect(validateNewAction({ ...base, targetRef: NORMAL }).ok).toBe(true);
  });
  test("rejects a request-status target derived from an oversized gh: objectRef", () => {
    const mk = (objectRef: string) => ({
      clientToken: "ct-4952-token-2",
      kind: "status-request",
      verb: "request-status",
      objectRef,
      author: drew,
      payload: { status: "Done" },
    });
    expect(validateNewAction(mk(`gh:${OVERSIZED}`)).ok).toBe(false);
    expect(validateNewAction(mk(`gh:${JUST_OVER}`)).ok).toBe(false);
    const ok = validateNewAction(mk(`gh:${NORMAL}`));
    expect(ok.ok).toBe(true);
    if (ok.ok) expect(ok.value.targetRef).toBe(NORMAL);
  });
});

describe("#4952 comments store-load gate", () => {
  const record = (id: string, githubRef: string) => ({
    id,
    objectRef: "gh:developing-today/code#4952",
    kind: "github",
    author: drew,
    body: "linked comment",
    createdAt: "2026-11-03T12:00:00Z",
    githubRef,
    githubState: "posted",
  });
  test("a stored github comment with an oversized githubRef is not re-admitted on load", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-4952-comments-"));
    const file = join(dir, "comments.json");
    writeFileSync(
      file,
      JSON.stringify({ version: 1, comments: [record("good", NORMAL), record("poisoned", OVERSIZED)] }),
    );
    const store = new CommentsStore(file);
    const all = await store.all();
    expect(all.map((c) => c.id)).toEqual(["good"]);
  });
});
