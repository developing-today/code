import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  migrate,
  upsertItem,
  upsertComment,
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
