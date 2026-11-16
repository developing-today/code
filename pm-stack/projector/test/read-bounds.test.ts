// Read-path bounds (#10595, #10601).
//
// #10595: /items?refs= and /comments?refs= split an unbounded list into one
// SQL placeholder per ref, and the by-refs items query had no row LIMIT at
// all. The routes now reject a ref list over MAX_REFS and the store applies
// the same `limit` bound as the list path.
//
// #10601: GET /links returned the whole pmp_links table with no limit. It now
// takes the sibling `limit` semantics (digit-only parseLimit, documented
// default + max) plus an `offset`, and the store enforces LIMIT/OFFSET for
// every shape (unfiltered, subject, ref), which also bounds the
// /subjects/:subject fan-out through getLinks/getItems.
//
// Run: bun test test/read-bounds.test.ts
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { migrate, upsertItem, upsertComment, upsertLink, type ItemRow, type CommentRow } from "../src/store";
import {
  handleRequest,
  DEFAULT_LINKS_LIMIT,
  MAX_LINKS_LIMIT,
  MAX_REFS,
  parseRefs,
  type ServerEnv,
} from "../src/server";

const env: ServerEnv = {};
const REPO = "developing-today/code";
const N = 12;

let db: Client;
let dir: string;

const item = (n: number): Omit<ItemRow, "fetched_at"> => ({
  ref: `${REPO}#${n}`,
  repo: REPO,
  number: n,
  kind: "issue",
  url: `https://github.com/${REPO}/issues/${n}`,
  api_url: `https://api.github.com/repos/${REPO}/issues/${n}`,
  title: `Item ${n}`,
  state: "open",
  state_reason: null,
  draft: 0,
  merged: 0,
  author: "dezren39",
  labels: "[]",
  assignees: "[]",
  created_at: "2026-10-25T00:00:00Z",
  updated_at: `2026-10-25T00:00:${String(n).padStart(2, "0")}Z`,
  closed_at: null,
  merged_at: null,
  source_updated_at: `2026-10-25T00:00:${String(n).padStart(2, "0")}Z`,
});

const comment = (n: number): Omit<CommentRow, "fetched_at"> => ({
  id: `c${n}`,
  ref: `${REPO}#${n}`,
  kind: "issue_comment",
  author: "dezren39",
  body: `comment ${n}`,
  created_at: `2026-10-25T00:00:${String(n).padStart(2, "0")}Z`,
  updated_at: null,
  url: null,
  path: null,
  line: null,
  diff_hunk: null,
  review_state: null,
  submitted_at: null,
  commit_id: null,
  deleted: 0,
  source_updated_at: `2026-10-25T00:00:${String(n).padStart(2, "0")}Z`,
});

async function get(path: string): Promise<{ status: number; body: any }> {
  const res = await handleRequest(new Request(`http://127.0.0.1${path}`), db, env);
  return { status: res.status, body: await res.json() };
}

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-read-bounds-"));
  db = createClient({ url: `file:${join(dir, "bounds.db")}` });
  await migrate(db);
  for (let n = 1; n <= N; n++) await upsertItem(db, item(n));
  for (let n = 1; n <= N; n++) await upsertComment(db, comment(n));
  for (let n = 1; n <= N; n++) {
    await upsertLink(db, { subject: `subj:${n}`, ref: `${REPO}#${n}`, relation: "relates" });
  }
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

function refList(count: number): string {
  // Unique refs: only the first N resolve to seeded rows, but every entry
  // still occupies a SQL placeholder, which is the quantity under test. Each
  // ref is URL-encoded, since a raw `#` would start the URL fragment.
  return Array.from({ length: count }, (_, i) => encodeURIComponent(`${REPO}#${i + 1}`)).join(",");
}

describe("parseRefs", () => {
  test("splits, trims, drops blanks, and de-duplicates", () => {
    expect(parseRefs(null)).toEqual([]);
    expect(parseRefs("")).toEqual([]);
    expect(parseRefs(" a/b#1 , a/b#1 ,,a/b#2 ")).toEqual(["a/b#1", "a/b#2"]);
  });
});

describe("refs list cap (#10595)", () => {
  test("a ref list at the cap is accepted", async () => {
    const { status, body } = await get(`/items?refs=${refList(MAX_REFS)}`);
    expect(status).toBe(200);
    expect(body.items.length).toBeGreaterThan(0);
  });

  test("GET /items over the cap returns a JSON 400 and never hits the store", async () => {
    const { status, body } = await get(`/items?refs=${refList(MAX_REFS + 1)}`);
    expect(status).toBe(400);
    expect(body).toEqual({ error: "too_many_refs", max: MAX_REFS });
  });

  test("GET /comments over the cap returns a JSON 400", async () => {
    const { status, body } = await get(`/comments?refs=${refList(MAX_REFS + 1)}`);
    expect(status).toBe(400);
    expect(body).toEqual({ error: "too_many_refs", max: MAX_REFS });
  });
});

describe("refs path honours limit (#10595)", () => {
  test("GET /items?refs=...&limit=5 returns at most 5 rows", async () => {
    const { status, body } = await get(`/items?refs=${refList(N)}&limit=5`);
    expect(status).toBe(200);
    expect(body.items).toHaveLength(5);
  });

  test("GET /comments?refs=...&limit=5 returns at most 5 rows", async () => {
    const { status, body } = await get(`/comments?refs=${refList(N)}&limit=5`);
    expect(status).toBe(200);
    expect(body.comments).toHaveLength(5);
  });

  test("an invalid refs-path limit falls back to the default bound, never a 500", async () => {
    for (const raw of ["-1", "0", "2.7", "abc", "1e2"]) {
      const { status, body } = await get(`/items?refs=${refList(N)}&limit=${encodeURIComponent(raw)}`);
      expect(status).toBe(200);
      expect(body.items).toHaveLength(N);
    }
  });
});

describe("GET /links bound and paging (#10601)", () => {
  test("unfiltered read is bounded and reports the next page", async () => {
    const { status, body } = await get("/links?limit=5");
    expect(status).toBe(200);
    expect(body.links).toHaveLength(5);
    expect(body.next_offset).toBe(5);
  });

  test("offset pages through the table to a bounded tail", async () => {
    const seen = new Set<string>();
    let offset: number | null = 0;
    while (offset !== null) {
      const { status, body } = await get(`/links?limit=5&offset=${offset}`);
      expect(status).toBe(200);
      expect(body.links.length).toBeLessThanOrEqual(5);
      for (const l of body.links) seen.add(`${l.subject}|${l.ref}|${l.relation}`);
      offset = body.next_offset;
    }
    expect(seen.size).toBe(N);
  });

  test("an invalid or over-max limit clamps without erroring", async () => {
    const bad = await get("/links?limit=abc");
    expect(bad.status).toBe(200);
    expect(bad.body.links).toHaveLength(N);
    const over = await get("/links?limit=999999");
    expect(over.status).toBe(200);
    expect(over.body.links).toHaveLength(N);
    expect(DEFAULT_LINKS_LIMIT).toBeLessThan(MAX_LINKS_LIMIT);
  });

  test("subject- and ref-filtered reads honour the same cap", async () => {
    const subject = await get("/links?subject=subj:1&limit=1");
    expect(subject.status).toBe(200);
    expect(subject.body.links).toHaveLength(1);
    const ref = await get(`/links?ref=${encodeURIComponent(`${REPO}#1`)}&limit=1`);
    expect(ref.status).toBe(200);
    expect(ref.body.links).toHaveLength(1);
  });

  test("the /subjects/:subject fan-out is bounded, never the whole links table", async () => {
    const { status, body } = await get("/subjects/subj:1");
    expect(status).toBe(200);
    expect(body.links.length).toBeLessThanOrEqual(DEFAULT_LINKS_LIMIT);
    expect(body.activity.length).toBeLessThanOrEqual(500);
  });
});
