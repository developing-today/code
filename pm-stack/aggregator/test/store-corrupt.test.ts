// #1452: the dashboard stores are the append-only audit history. A
// missing file is "no store yet" (silent empty start); an unparseable
// or wrongly-shaped file must be quarantined to
// <file>.corrupt-<timestamp> with its original bytes preserved, never
// silently overwritten by the next persist.
import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ActionsStore, type NewAction } from "../src/actions.ts";
import { CommentsStore, pendingCommentCounts } from "../src/comments.ts";
import type { InvalidStoreRecord, StoreArchive } from "../src/store-recovery.ts";

const drew = { kind: "drew" as const, id: "drew", label: "Drew" };

let tokenSeq = 0;
const newAction = (over: Partial<NewAction> = {}): NewAction => ({
  clientToken: `ct-corrupt-${++tokenSeq}-abcdefgh`,
  kind: "triage",
  verb: "acknowledge",
  objectRef: "orch:item-1",
  payload: {},
  author: drew,
  ...over,
});

const quarantinedIn = (dir: string, base: string) =>
  readdirSync(dir).filter((f) => f.startsWith(`${base}.corrupt-`) && !f.endsWith(".diagnostics.json"));
const diagnosticsIn = (dir: string, base: string) =>
  readdirSync(dir).filter((f) => f.startsWith(`${base}.corrupt-`) && f.endsWith(".diagnostics.json"));

const validComment = (id: string, extra: Record<string, unknown> = {}) => ({
  id,
  objectRef: "gh:developing-today/code#2347",
  kind: "note",
  author: drew,
  body: "historical note",
  createdAt: "2026-10-30T12:00:00Z",
  ...extra,
});

const validAction = (id: string, extra: Record<string, unknown> = {}) => ({
  id,
  clientToken: `ct-${id}`,
  kind: "triage",
  verb: "answer",
  objectRef: "orch:legacy-item",
  payload: { answers: "legacy payload shape" },
  author: drew,
  createdAt: "2026-10-30T12:00:00Z",
  state: "new",
  ...extra,
});

function diagnosticEntries(dir: string, base: string): InvalidStoreRecord[] {
  const files = diagnosticsIn(dir, base);
  expect(files).toHaveLength(1);
  return JSON.parse(readFileSync(join(dir, files[0]), "utf8")).invalidRecords;
}

describe("actions store corruption handling (#1452)", () => {
  test("missing file -> empty store, no quarantine", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-missing-"));
    const file = join(dir, "actions.json");
    const store = new ActionsStore(file);
    expect(await store.all()).toEqual([]);
    expect(existsSync(file)).toBe(false);
    expect(quarantinedIn(dir, "actions.json")).toEqual([]);
  });

  test("corrupt file -> quarantine with original bytes, empty store, fresh persist is valid and leaves the quarantine untouched", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-corrupt-"));
    const file = join(dir, "actions.json");
    const garbage = '{"version":1,"actions":[{"id":"da-1"'; // truncated write
    writeFileSync(file, garbage);
    const store = new ActionsStore(file);
    expect(await store.all()).toEqual([]);
    const quarantined = quarantinedIn(dir, "actions.json");
    expect(quarantined.length).toBe(1);
    expect(readFileSync(join(dir, quarantined[0]), "utf8")).toBe(garbage);
    expect(existsSync(file)).toBe(false);
    // The next persist writes a fresh valid version:1 file.
    await store.add(newAction());
    const persisted = JSON.parse(readFileSync(file, "utf8"));
    expect(persisted.version).toBe(1);
    expect(persisted.actions.length).toBe(1);
    // The quarantined copy is untouched by the persist.
    expect(readFileSync(join(dir, quarantined[0]), "utf8")).toBe(garbage);
    expect(quarantinedIn(dir, "actions.json").length).toBe(1);
  });

  test("wrongly-shaped file -> quarantine, empty store", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-shape-"));
    const file = join(dir, "actions.json");
    writeFileSync(file, JSON.stringify({ version: 1, nope: [] }));
    const store = new ActionsStore(file);
    expect(await store.all()).toEqual([]);
    expect(quarantinedIn(dir, "actions.json").length).toBe(1);
  });

  test("unreadable store (directory in place of file) -> loud quarantine, empty store, fresh persist leaves the quarantine untouched", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-unreadable-"));
    const file = join(dir, "actions.json");
    mkdirSync(file); // readFile on a directory fails with EISDIR — a non-ENOENT read error (#2980)
    const errors: string[] = [];
    const origError = console.error;
    console.error = (...args: unknown[]) => {
      errors.push(args.map(String).join(" "));
    };
    let store: ActionsStore;
    try {
      store = new ActionsStore(file);
      expect(await store.all()).toEqual([]);
    } finally {
      console.error = origError;
    }
    expect(errors.some((m) => m.includes("could not be read"))).toBe(true);
    const quarantined = quarantinedIn(dir, "actions.json");
    expect(quarantined.length).toBe(1);
    expect(statSync(join(dir, quarantined[0])).isDirectory()).toBe(true);
    expect(existsSync(file)).toBe(false);
    await store!.add(newAction());
    const persisted = JSON.parse(readFileSync(file, "utf8"));
    expect(persisted.version).toBe(1);
    expect(persisted.actions.length).toBe(1);
    expect(quarantinedIn(dir, "actions.json").length).toBe(1);
  });

  test("valid file -> loads unchanged, no quarantine", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-valid-"));
    const file = join(dir, "actions.json");
    const first = new ActionsStore(file);
    await first.add(newAction());
    const reloaded = new ActionsStore(file);
    expect((await reloaded.all()).length).toBe(1);
    expect(quarantinedIn(dir, "actions.json")).toEqual([]);
  });

  test("mixed malformed records are archived byte-for-byte with diagnostics while valid history survives persistence", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-records-"));
    const file = join(dir, "actions.json");
    const source = {
      version: 1,
      extension: { source: "older-writer" },
      actions: [
        validAction("good-action", { historicalExtension: { retained: true } }),
        { ...validAction("missing-created-at"), createdAt: undefined },
        { ...validAction("numeric-created-at"), createdAt: 123 },
        { ...validAction("missing-object-ref"), objectRef: undefined },
        { ...validAction("missing-kind"), kind: undefined },
        { ...validAction("missing-state"), state: undefined },
        { ...validAction("missing-payload"), payload: undefined },
        { ...validAction("legacy-no-token"), clientToken: undefined },
      ],
    };
    const raw = Buffer.from(JSON.stringify(source));
    writeFileSync(file, raw);

    const store = new ActionsStore(file);
    expect((await store.all()).map((action) => action.id)).toEqual(["good-action", "legacy-no-token"]);
    expect((await store.forObject("orch:legacy-item")).map((action) => action.id)).toEqual(["good-action", "legacy-no-token"]);
    expect((await store.byObject()).get("orch:legacy-item")?.map((action) => action.id)).toEqual(["good-action", "legacy-no-token"]);
    expect(await store.pendingCounts()).toEqual({ triage: 2, statusRequests: 0, session: 0, preference: 0 });

    const quarantine = quarantinedIn(dir, "actions.json");
    expect(quarantine).toHaveLength(1);
    expect(readFileSync(join(dir, quarantine[0]))).toEqual(raw);
    expect(diagnosticEntries(dir, "actions.json")).toEqual([
      { index: 1, reason: "createdAt must be a valid ISO date/time string" },
      { index: 2, reason: "createdAt must be a valid ISO date/time string" },
      { index: 3, reason: "objectRef must be a non-empty ref string" },
      { index: 4, reason: "kind must be triage, status-request, session, or preference" },
      { index: 5, reason: "state must be new, applied, rejected, or superseded" },
      { index: 6, reason: "payload must be an object" },
    ]);

    await store.add(newAction({ objectRef: "orch:legacy-item" }));
    const persisted = JSON.parse(readFileSync(file, "utf8"));
    expect(persisted.extension).toEqual({ source: "older-writer" });
    expect(persisted.actions.find((action: { id: string }) => action.id === "good-action").historicalExtension).toEqual({
      retained: true,
    });
    expect(persisted.actions.find((action: { id: string }) => action.id === "legacy-no-token").clientToken).toBeUndefined();
    expect(persisted.actions.map((action: { id: string }) => action.id)).not.toContain("missing-created-at");
    // Deliberately retain old verb-specific payload shapes; consumers validate those safely.
    expect(persisted.actions[0].payload).toEqual({ answers: "legacy payload shape" });
    expect(readFileSync(join(dir, quarantine[0]))).toEqual(raw);
  });

  test("archive failure rejects reads and writes without changing the source", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-archive-failure-"));
    const file = join(dir, "actions.json");
    const raw = Buffer.from(JSON.stringify({ version: 1, actions: [validAction("bad", { createdAt: 123 })] }));
    writeFileSync(file, raw);
    const archiveFails: StoreArchive = async () => {
      throw new Error("simulated archive failure");
    };
    const store = new ActionsStore(file, archiveFails);

    await expect(store.all()).rejects.toThrow("simulated archive failure");
    expect(readFileSync(file)).toEqual(raw);
    await expect(store.add(newAction())).rejects.toThrow("simulated archive failure");
    expect(readFileSync(file)).toEqual(raw);
    expect(quarantinedIn(dir, "actions.json")).toEqual([]);
  });
});

describe("comments store corruption handling (#1452)", () => {
  test("missing file -> empty store, no quarantine", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-comments-missing-"));
    const file = join(dir, "comments.json");
    const store = new CommentsStore(file);
    expect(await store.all()).toEqual([]);
    expect(existsSync(file)).toBe(false);
    expect(quarantinedIn(dir, "comments.json")).toEqual([]);
  });

  test("corrupt file -> quarantine with original bytes, empty store, fresh persist is valid and leaves the quarantine untouched", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-comments-corrupt-"));
    const file = join(dir, "comments.json");
    const garbage = '{"version":1,"comments":[{"id":"dc-1","body":"audit';
    writeFileSync(file, garbage);
    const store = new CommentsStore(file);
    expect(await store.all()).toEqual([]);
    const quarantined = quarantinedIn(dir, "comments.json");
    expect(quarantined.length).toBe(1);
    expect(readFileSync(join(dir, quarantined[0]), "utf8")).toBe(garbage);
    expect(existsSync(file)).toBe(false);
    await store.add({ objectRef: "gh:developing-today/code#1452", kind: "note", author: drew, body: "fresh start" });
    const persisted = JSON.parse(readFileSync(file, "utf8"));
    expect(persisted.version).toBe(1);
    expect(persisted.comments.length).toBe(1);
    expect(readFileSync(join(dir, quarantined[0]), "utf8")).toBe(garbage);
    expect(quarantinedIn(dir, "comments.json").length).toBe(1);
  });

  test("wrongly-shaped file -> quarantine, empty store", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-comments-shape-"));
    const file = join(dir, "comments.json");
    writeFileSync(file, JSON.stringify({ version: 1, nope: [] }));
    const store = new CommentsStore(file);
    expect(await store.all()).toEqual([]);
    expect(quarantinedIn(dir, "comments.json").length).toBe(1);
  });

  test("unreadable store (directory in place of file) -> loud quarantine, empty store, fresh persist leaves the quarantine untouched", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-comments-unreadable-"));
    const file = join(dir, "comments.json");
    mkdirSync(file); // readFile on a directory fails with EISDIR — a non-ENOENT read error (#2980)
    const errors: string[] = [];
    const origError = console.error;
    console.error = (...args: unknown[]) => {
      errors.push(args.map(String).join(" "));
    };
    let store: CommentsStore;
    try {
      store = new CommentsStore(file);
      expect(await store.all()).toEqual([]);
    } finally {
      console.error = origError;
    }
    expect(errors.some((m) => m.includes("could not be read"))).toBe(true);
    const quarantined = quarantinedIn(dir, "comments.json");
    expect(quarantined.length).toBe(1);
    expect(statSync(join(dir, quarantined[0])).isDirectory()).toBe(true);
    expect(existsSync(file)).toBe(false);
    await store!.add({ objectRef: "gh:developing-today/code#2980", kind: "note", author: drew, body: "fresh start" });
    const persisted = JSON.parse(readFileSync(file, "utf8"));
    expect(persisted.version).toBe(1);
    expect(persisted.comments.length).toBe(1);
    expect(quarantinedIn(dir, "comments.json").length).toBe(1);
  });

  test("valid file -> loads unchanged, no quarantine", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-comments-valid-"));
    const file = join(dir, "comments.json");
    const first = new CommentsStore(file);
    await first.add({ objectRef: "gh:developing-today/code#1452", kind: "note", author: drew, body: "kept" });
    const reloaded = new CommentsStore(file);
    expect((await reloaded.all()).length).toBe(1);
    expect(quarantinedIn(dir, "comments.json")).toEqual([]);
  });

  test("mixed malformed records are archived byte-for-byte with diagnostics while valid history survives persistence", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-comments-records-"));
    const file = join(dir, "comments.json");
    const source = {
      version: 1,
      extension: { source: "older-writer" },
      comments: [
        validComment("good-comment", { historicalExtension: { retained: true } }),
        { ...validComment("missing-created-at"), createdAt: undefined },
        { ...validComment("numeric-created-at"), createdAt: 123 },
        { ...validComment("missing-object-ref"), objectRef: undefined },
        { ...validComment("missing-kind"), kind: undefined },
        { ...validComment("missing-author"), author: undefined },
        { ...validComment("missing-body"), body: undefined },
        validComment("legacy-message-without-state", { kind: "message" }),
      ],
    };
    const raw = Buffer.from(JSON.stringify(source));
    writeFileSync(file, raw);

    const store = new CommentsStore(file);
    expect((await store.all()).map((comment) => comment.id)).toEqual(["good-comment", "legacy-message-without-state"]);
    expect((await store.forObject("gh:developing-today/code#2347")).map((comment) => comment.id)).toEqual([
      "good-comment",
      "legacy-message-without-state",
    ]);
    expect([...((await store.byObject()).keys())]).toEqual(["gh:developing-today/code#2347"]);
    expect([...((await store.counts()).entries())]).toEqual([["gh:developing-today/code#2347", 2]]);
    expect(pendingCommentCounts(await store.all())).toEqual({ pendingMessages: 0, pendingGithub: 0, total: 2 });

    const quarantine = quarantinedIn(dir, "comments.json");
    expect(quarantine).toHaveLength(1);
    expect(readFileSync(join(dir, quarantine[0]))).toEqual(raw);
    expect(diagnosticEntries(dir, "comments.json")).toEqual([
      { index: 1, reason: "createdAt must be a valid ISO date/time string" },
      { index: 2, reason: "createdAt must be a valid ISO date/time string" },
      { index: 3, reason: "objectRef must be a non-empty ref string" },
      { index: 4, reason: "kind must be message, note, or github" },
      { index: 5, reason: "author must have a supported kind, id, and label" },
      { index: 6, reason: "body must be a string" },
    ]);

    await store.add({ objectRef: "gh:developing-today/code#2347", kind: "note", author: drew, body: "new note" });
    const persisted = JSON.parse(readFileSync(file, "utf8"));
    expect(persisted.extension).toEqual({ source: "older-writer" });
    expect(persisted.comments.find((comment: { id: string }) => comment.id === "good-comment").historicalExtension).toEqual({
      retained: true,
    });
    expect(persisted.comments.find((comment: { id: string }) => comment.id === "legacy-message-without-state").triageState).toBeUndefined();
    expect(persisted.comments.map((comment: { id: string }) => comment.id)).not.toContain("missing-created-at");
    expect(readFileSync(join(dir, quarantine[0]))).toEqual(raw);
  });

  test("archive failure rejects reads and writes without changing the source", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-comments-archive-failure-"));
    const file = join(dir, "comments.json");
    const raw = Buffer.from(JSON.stringify({ version: 1, comments: [validComment("bad", { createdAt: 123 })] }));
    writeFileSync(file, raw);
    const archiveFails: StoreArchive = async () => {
      throw new Error("simulated archive failure");
    };
    const store = new CommentsStore(file, archiveFails);

    await expect(store.all()).rejects.toThrow("simulated archive failure");
    expect(readFileSync(file)).toEqual(raw);
    await expect(store.add({ objectRef: "gh:developing-today/code#2347", kind: "note", author: drew, body: "blocked" })).rejects.toThrow(
      "simulated archive failure",
    );
    expect(readFileSync(file)).toEqual(raw);
    expect(quarantinedIn(dir, "comments.json")).toEqual([]);
  });
});
