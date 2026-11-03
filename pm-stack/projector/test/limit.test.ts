// Limit validation for the read API (#1547): the `limit` query parameter on
// /items and /comments must be parsed as a positive integer at the server
// layer and clamped to a named per-endpoint maximum. Negative, fractional,
// non-numeric, and zero values fall back to the endpoint default — they must
// never reach the store, where a negative LIMIT is unbounded in SQLite and a
// fractional one raises a driver error that escaped as a 500.
//
// Run: bun test test/limit.test.ts
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { migrate, upsertItem, upsertComment, upsertLink, type ItemRow, type CommentRow } from "../src/store";
import {
  handleRequest,
  parseLimit,
  DEFAULT_ITEMS_LIMIT,
  MAX_ITEMS_LIMIT,
  DEFAULT_COMMENTS_LIMIT,
  MAX_COMMENTS_LIMIT,
  type ServerEnv,
} from "../src/server";

let db: Client;
let dir: string;

const env: ServerEnv = {};

const item = (n: number): Omit<ItemRow, "fetched_at"> => ({
  ref: `developing-today/code#${n}`,
  repo: "developing-today/code",
  number: n,
  kind: "issue",
  url: `https://github.com/developing-today/code/issues/${n}`,
  api_url: `https://api.github.com/repos/developing-today/code/issues/${n}`,
  title: `Item ${n}`,
  state: "open",
  state_reason: null,
  draft: 0,
  merged: 0,
  author: "dezren39",
  labels: "[]",
  assignees: "[]",
  created_at: "2026-10-25T00:00:00Z",
  updated_at: `2026-10-25T0${n}:00:00Z`,
  closed_at: null,
  merged_at: null,
  source_updated_at: `2026-10-25T0${n}:00:00Z`,
});

const comment = (n: number): Omit<CommentRow, "fetched_at"> => ({
  id: `c${n}`,
  ref: "developing-today/code#1",
  kind: "issue_comment",
  author: "dezren39",
  body: `comment ${n}`,
  created_at: `2026-10-25T0${n}:00:00Z`,
  updated_at: null,
  url: null,
  path: null,
  line: null,
  diff_hunk: null,
  review_state: null,
  submitted_at: null,
  commit_id: null,
  deleted: 0,
  source_updated_at: `2026-10-25T0${n}:00:00Z`,
});

async function get(path: string): Promise<{ status: number; body: any }> {
  const res = await handleRequest(new Request(`http://127.0.0.1${path}`), db, env);
  return { status: res.status, body: await res.json() };
}

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-limit-"));
  db = createClient({ url: `file:${join(dir, "limit.db")}` });
  await migrate(db);
  for (let n = 1; n <= 8; n++) await upsertItem(db, item(n));
  for (let n = 1; n <= 6; n++) await upsertComment(db, comment(n));
  await upsertLink(db, { subject: "subj:1", ref: "developing-today/code#1", relation: "tracks" });
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("parseLimit", () => {
  test("absent parameter keeps the endpoint default", () => {
    expect(parseLimit(null, DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT)).toBe(200);
    expect(parseLimit(null, DEFAULT_COMMENTS_LIMIT, MAX_COMMENTS_LIMIT)).toBe(500);
  });

  test("a well-formed positive integer is honoured exactly", () => {
    expect(parseLimit("3", DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT)).toBe(3);
    expect(parseLimit("1", DEFAULT_COMMENTS_LIMIT, MAX_COMMENTS_LIMIT)).toBe(1);
  });

  test("negative, zero, fractional, and non-numeric values fall back to the default", () => {
    for (const raw of ["-1", "-5", "0", "2.7", "abc", "", "NaN", "Infinity"]) {
      expect(parseLimit(raw, DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT)).toBe(DEFAULT_ITEMS_LIMIT);
      expect(parseLimit(raw, DEFAULT_COMMENTS_LIMIT, MAX_COMMENTS_LIMIT)).toBe(DEFAULT_COMMENTS_LIMIT);
    }
  });

  test("coercible non-decimal spellings fall back to the default (#2971)", () => {
    for (const raw of ["1e2", "0x10", "5.0", "+5", " 5", "5 "]) {
      expect(parseLimit(raw, DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT)).toBe(DEFAULT_ITEMS_LIMIT);
      expect(parseLimit(raw, DEFAULT_COMMENTS_LIMIT, MAX_COMMENTS_LIMIT)).toBe(DEFAULT_COMMENTS_LIMIT);
    }
    expect(parseLimit("5", DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT)).toBe(5);
    expect(parseLimit("5", DEFAULT_COMMENTS_LIMIT, MAX_COMMENTS_LIMIT)).toBe(5);
    expect(parseLimit("99999", DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT)).toBe(MAX_ITEMS_LIMIT);
    expect(parseLimit("99999", DEFAULT_COMMENTS_LIMIT, MAX_COMMENTS_LIMIT)).toBe(MAX_COMMENTS_LIMIT);
  });

  test("over-max values clamp to the named per-endpoint maximum", () => {
    expect(parseLimit("999999999", DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT)).toBe(MAX_ITEMS_LIMIT);
    expect(parseLimit("1001", DEFAULT_COMMENTS_LIMIT, MAX_COMMENTS_LIMIT)).toBe(MAX_COMMENTS_LIMIT);
    expect(parseLimit(String(MAX_ITEMS_LIMIT), DEFAULT_ITEMS_LIMIT, MAX_ITEMS_LIMIT)).toBe(MAX_ITEMS_LIMIT);
  });
});

describe("read API limit handling", () => {
  test("GET /items?limit=3 returns exactly 3 items", async () => {
    const { status, body } = await get("/items?limit=3");
    expect(status).toBe(200);
    expect(body.items).toHaveLength(3);
  });

  test("GET /items with an invalid limit never 500s and stays bounded by the default", async () => {
    for (const raw of ["-1", "0", "2.7", "abc", "1e2", "0x10", "5.0", "+5"]) {
      const { status, body } = await get(`/items?limit=${encodeURIComponent(raw)}`);
      expect(status).toBe(200);
      // 8 seeded items < default 200, so the default-bounded result is all 8;
      // parseLimit's unit tests pin that the bound itself is the default.
      expect(body.items).toHaveLength(8);
    }
  });

  test("GET /items?limit=999999999 is clamped, not an error", async () => {
    const { status, body } = await get("/items?limit=999999999");
    expect(status).toBe(200);
    expect(body.items).toHaveLength(8);
  });

  test("GET /items with no limit keeps the default behaviour", async () => {
    const { status, body } = await get("/items");
    expect(status).toBe(200);
    expect(body.items).toHaveLength(8);
  });

  const REFS = "refs=developing-today%2Fcode%231";

  test("GET /comments?limit=3 returns exactly 3 comments", async () => {
    const { status, body } = await get(`/comments?${REFS}&limit=3`);
    expect(status).toBe(200);
    expect(body.comments).toHaveLength(3);
  });

  test("GET /comments with an invalid limit never 500s and stays bounded by the default", async () => {
    for (const raw of ["-1", "0", "2.7", "abc", "999999999"]) {
      const { status, body } = await get(`/comments?${REFS}&limit=${raw}`);
      expect(status).toBe(200);
      expect(body.comments).toHaveLength(6);
    }
  });

  test("GET /comments?subject=...&limit=2 honours the exact limit", async () => {
    const { status, body } = await get("/comments?subject=subj%3A1&limit=2");
    expect(status).toBe(200);
    expect(body.comments).toHaveLength(2);
  });

  test("GET /comments?subject=...&limit=-1 falls back instead of returning unbounded", async () => {
    const { status, body } = await get("/comments?subject=subj%3A1&limit=-1");
    expect(status).toBe(200);
    expect(body.comments).toHaveLength(6);
  });
});
