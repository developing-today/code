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
import { CommentsStore } from "../src/comments.ts";

const drew = { kind: "drew" as const, id: "drew", label: "Drew" };

let tokenSeq = 0;
const newAction = (): NewAction => ({
  clientToken: `ct-corrupt-${++tokenSeq}-abcdefgh`,
  kind: "triage",
  verb: "acknowledge",
  objectRef: "orch:item-1",
  payload: {},
  author: drew,
});

const quarantinedIn = (dir: string, base: string) =>
  readdirSync(dir).filter((f) => f.startsWith(`${base}.corrupt-`));

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
    console.error = (...args: unknown[]) => { errors.push(args.map(String).join(" ")); };
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
    console.error = (...args: unknown[]) => { errors.push(args.map(String).join(" ")); };
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
});
