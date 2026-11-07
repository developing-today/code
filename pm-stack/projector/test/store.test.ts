import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  migrate,
  upsertItem,
  upsertComment,
  markCommentDeleted,
  deleteItem,
  getItemDeletion,
  upsertLink,
  getItems,
  getComments,
  getLinks,
  getSubject,
  type ItemRow,
  type CommentRow,
} from "../src/store";

let db: Client;
let dir: string;

const item = (over: Partial<ItemRow>): Omit<ItemRow, "fetched_at"> => ({
  ref: "developing-today/code#1",
  repo: "developing-today/code",
  number: 1,
  kind: "issue",
  url: "https://github.com/developing-today/code/issues/1",
  api_url: "https://api.github.com/repos/developing-today/code/issues/1",
  title: "Fix widget",
  state: "open",
  state_reason: null,
  draft: 0,
  merged: 0,
  author: "dezren39",
  labels: "[]",
  assignees: "[]",
  created_at: "2026-10-25T00:00:00Z",
  updated_at: "2026-10-25T00:00:00Z",
  closed_at: null,
  merged_at: null,
  source_updated_at: "2026-10-25T00:00:00Z",
  ...over,
});

const comment = (over: Partial<CommentRow> = {}): Omit<CommentRow, "fetched_at"> => ({
  id: "comment-default",
  ref: "developing-today/code#2",
  kind: "issue_comment",
  author: "dezren39",
  body: "original",
  created_at: "2026-10-25T00:00:00Z",
  updated_at: null,
  url: null,
  path: null,
  line: null,
  diff_hunk: null,
  review_state: null,
  submitted_at: null,
  commit_id: null,
  deleted: 0,
  source_updated_at: "2026-10-25T00:00:00Z",
  ...over,
});

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-"));
  db = createClient({ url: `file:${join(dir, "test.db")}` });
  await migrate(db);
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("store: items", () => {
  test("upserts and reads an item", async () => {
    await upsertItem(db, item({}));
    const rows = await getItems(db, ["developing-today/code#1"]);
    expect(rows).toHaveLength(1);
    expect(rows[0].title).toBe("Fix widget");
    expect(rows[0].state).toBe("open");
  });

  test("monotonic guard: stale updated_at does not overwrite", async () => {
    await upsertItem(
      db,
      item({ title: "Newer title", updated_at: "2026-10-25T01:00:00Z", source_updated_at: "2026-10-25T01:00:00Z" }),
    );
    await upsertItem(
      db,
      item({ title: "STALE title", updated_at: "2026-10-24T00:00:00Z", source_updated_at: "2026-10-24T00:00:00Z" }),
    );
    const rows = await getItems(db, ["developing-today/code#1"]);
    expect(rows[0].title).toBe("Newer title");
  });

  test("monotonic guard: equal updated_at may overwrite", async () => {
    await upsertItem(
      db,
      item({
        title: "Equal ts",
        updated_at: "2026-10-25T02:00:00Z",
        source_updated_at: "2026-10-25T02:00:00Z",
        state: "closed",
      }),
    );
    await upsertItem(
      db,
      item({
        title: "Equal ts again",
        updated_at: "2026-10-25T02:00:00Z",
        source_updated_at: "2026-10-25T02:00:00Z",
        state: "closed",
      }),
    );
    const rows = await getItems(db, ["developing-today/code#1"]);
    expect(rows[0].title).toBe("Equal ts again");
  });
});

describe("store: deleteItem (#4210)", () => {
  test("removes the item row, its live comments, and its subject links; tombstones survive", async () => {
    const ref = "developing-today/code#4210";
    await upsertItem(db, item({ ref, number: 4210 }));
    await upsertComment(db, comment({ id: "c-4210", ref }));
    await markCommentDeleted(db, comment({ id: "c-4210-gone", ref }));
    await upsertLink(db, { subject: `gh:${ref}`, ref: "developing-today/code#1", relation: "tracks" });
    await deleteItem(db, ref);
    expect(await getItems(db, [ref])).toHaveLength(0);
    // The live comment is gone; the tombstone is retained so a stale
    // replay of it stays absorbed (Codex review on PR #6775).
    const remaining = await getComments(db, [ref], { includeDeleted: true });
    expect(remaining.map((c) => c.id)).toEqual(["c-4210-gone"]);
    expect(remaining[0].deleted).toBe(1);
    expect(await getLinks(db, { subject: `gh:${ref}` })).toHaveLength(0);
    // The linked-to item itself is untouched: only the subject side goes.
    expect(await getItems(db, ["developing-today/code#1"])).toHaveLength(1);
  });
});

describe("store: item deletion marker (#6782)", () => {
  const T0 = "2026-10-25T00:00:00Z";
  const T1 = "2026-10-25T01:00:00Z";
  const T2 = "2026-10-25T02:00:00Z";
  const T3 = "2026-10-25T03:00:00Z";

  test("deleteItem persists a marker even when the item was never upserted", async () => {
    const ref = "developing-today/code#67821";
    await deleteItem(db, ref, T2);
    const marker = await getItemDeletion(db, ref);
    expect(marker?.source_updated_at).toBe(T2);
    // A stale create arriving after the delete is absorbed.
    await upsertItem(db, item({ ref, number: 67821, source_updated_at: T1, updated_at: T1 }));
    expect(await getItems(db, [ref])).toHaveLength(0);
  });

  test("opened then deleted: stale replays stay absorbed, at and before the marker", async () => {
    const ref = "developing-today/code#67822";
    await upsertItem(db, item({ ref, number: 67822, source_updated_at: T0, updated_at: T0 }));
    await deleteItem(db, ref, T2);
    expect(await getItems(db, [ref])).toHaveLength(0);
    await upsertItem(db, item({ ref, number: 67822, source_updated_at: T1, updated_at: T1 }));
    await upsertItem(db, item({ ref, number: 67822, source_updated_at: T2, updated_at: T2 }));
    expect(await getItems(db, [ref])).toHaveLength(0);
    expect((await getItemDeletion(db, ref))?.source_updated_at).toBe(T2);
  });

  test("a genuinely newer upsert recreates the item and clears the marker", async () => {
    const ref = "developing-today/code#67823";
    await deleteItem(db, ref, T2);
    await upsertItem(db, item({ ref, number: 67823, source_updated_at: T3, updated_at: T3 }));
    const rows = await getItems(db, [ref]);
    expect(rows).toHaveLength(1);
    expect(await getItemDeletion(db, ref)).toBeNull();
  });

  test("a later delete refreshes the marker; an older delete does not move it back", async () => {
    const ref = "developing-today/code#67824";
    await deleteItem(db, ref, T2);
    await deleteItem(db, ref, T1);
    expect((await getItemDeletion(db, ref))?.source_updated_at).toBe(T2);
    await deleteItem(db, ref, T3);
    expect((await getItemDeletion(db, ref))?.source_updated_at).toBe(T3);
  });
});

describe("store: comments + links + subject view", () => {
  test("comments upsert and read, newest first", async () => {
    const base: Omit<CommentRow, "fetched_at"> = {
      id: "c1",
      ref: "developing-today/code#1",
      kind: "issue_comment",
      author: "dezren39",
      body: "first",
      created_at: "2026-10-25T00:00:00Z",
      updated_at: null,
      url: null,
      path: null,
      line: null,
      diff_hunk: null,
      review_state: null,
      submitted_at: null,
      commit_id: null,
      deleted: 0,
      source_updated_at: "2026-10-25T00:00:00Z",
    };
    await upsertComment(db, base);
    await upsertComment(db, {
      ...base,
      id: "c2",
      body: "second",
      created_at: "2026-10-25T01:00:00Z",
      source_updated_at: "2026-10-25T01:00:00Z",
    });
    const rows = await getComments(db, ["developing-today/code#1"]);
    expect(rows.map((r) => r.id)).toEqual(["c2", "c1"]);
  });

  test("an edited comment ignores a stale create replay", async () => {
    const created = comment({ id: "comment-edit-stale" });
    await upsertComment(db, created);
    await upsertComment(
      db,
      comment({
        ...created,
        body: "edited",
        updated_at: "2026-10-25T01:00:00Z",
        source_updated_at: "2026-10-25T01:00:00Z",
      }),
    );
    await upsertComment(db, { ...created, body: "stale original replay" });

    const row = (await getComments(db, [created.ref])).find((c) => c.id === created.id);
    expect(row?.body).toBe("edited");
    expect(row?.updated_at).toBe("2026-10-25T01:00:00Z");
    expect(row?.source_updated_at).toBe("2026-10-25T01:00:00Z");
  });

  test("a deleted comment is an absorbing tombstone for stale upserts", async () => {
    const created = comment({ id: "comment-delete-stale" });
    await upsertComment(db, created);
    await upsertComment(
      db,
      comment({
        ...created,
        body: "edited before delete",
        updated_at: "2026-10-25T01:00:00Z",
        source_updated_at: "2026-10-25T01:00:00Z",
      }),
    );
    await markCommentDeleted(db, created);

    // A stale pre-edit event is older; equal- and later-version upserts are
    // also rejected because the delete tombstone, not fetched_at, wins.
    await upsertComment(db, { ...created, body: "stale original replay" });
    await upsertComment(
      db,
      comment({
        ...created,
        body: "same-version replay",
        updated_at: "2026-10-25T01:00:00Z",
        source_updated_at: "2026-10-25T01:00:00Z",
      }),
    );
    await upsertComment(
      db,
      comment({
        ...created,
        body: "post-delete update",
        updated_at: "2026-10-25T02:00:00Z",
        source_updated_at: "2026-10-25T02:00:00Z",
      }),
    );

    const row = (await getComments(db, [created.ref], { includeDeleted: true })).find((c) => c.id === created.id);
    expect(row?.body).toBe("edited before delete");
    expect(row?.deleted).toBe(1);
    expect(row?.source_updated_at).toBe("2026-10-25T01:00:00Z");
  });

  test("a delete arriving before an upsert leaves a durable tombstone", async () => {
    const tombstone = comment({ id: "comment-delete-before-create", body: "deleted" });
    await markCommentDeleted(db, tombstone);
    await upsertComment(db, { ...tombstone, body: "stale create replay" });

    const row = (await getComments(db, [tombstone.ref], { includeDeleted: true })).find((c) => c.id === tombstone.id);
    expect(row?.body).toBe("deleted");
    expect(row?.deleted).toBe(1);
  });

  test("links round-trip and subject view joins item + activity", async () => {
    await upsertLink(db, { subject: "grist:doc1/table1/row1", ref: "developing-today/code#1", relation: "tracks" });
    const links = await getLinks(db, { subject: "grist:doc1/table1/row1" });
    expect(links).toHaveLength(1);
    expect(links[0].relation).toBe("tracks");

    const view = await getSubject(db, "grist:doc1/table1/row1");
    expect(view).not.toBeNull();
    expect(view!.links[0].item?.title).toBe("Equal ts again");
    expect(view!.activity.length).toBeGreaterThan(0);
  });
});
