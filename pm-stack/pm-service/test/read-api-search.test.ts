/**
 * Hosted search suite (CF-7, #14947).
 *
 * Oracle: #14947's acceptance — the heavy surfaces answer hosted
 * over indexed projections, never over the laptop's unbounded reads
 * (#14060). The fixture DB is seeded through the real migrations
 * (0001 + the 0002 FTS5 index) and the real landing path, so what is
 * pinned here is the served behaviour of GET /api/search.json:
 * which items a query returns (hand-computed from the fixture
 * titles), that the page is bounded, that a re-landed snapshot's new
 * titles are what search finds (the triggers keep the index), and
 * that operator-shaped input is treated as words, not syntax.
 */
import { describe, expect, test } from "bun:test";
import { createAsyncReadApiHandler } from "../src/read-api/async-api.ts";
import { ftsMatch, searchItemsAsync, SEARCH_MAX_LIMIT } from "../src/read-api/search.ts";
import { landSnapshot } from "../src/collector/store.ts";
import type { ProjectAudit, ProjectPoolItem } from "../src/collector/pool.ts";
import { tenantStoreMigrations } from "../src/tenant-store/load.ts";
import type { AsyncSqlDb, AsyncSqlValue } from "../src/tenant-store/libsql-http.ts";
import { applyMigrations, type SqlDb } from "../src/tenant-store/migrate.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";

const T1 = "2026-11-26T06:00:00.000Z";
const REPO = "developing-today/code";
const ref = (number: number) => `${REPO}#${number}`;

function poolItem(number: number, title: string): ProjectPoolItem {
  const url = `https://github.com/${REPO}/issues/${number}`;
  return {
    id: `PVTI_${number}`,
    databaseId: number * 10,
    url,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    repo: REPO,
    number,
    issueUrl: url,
    state: "open",
    contentKind: "issue",
    labels: [],
    status: "Todo",
    area: "CF",
    horizon: null,
    kind: null,
    ref: ref(number),
    title,
  };
}

const POOL = [
  poolItem(101, "cloudflare tunnel tier for the front door"),
  poolItem(102, "front door route state"),
  poolItem(103, "tunnel drill: laptop off"),
  poolItem(104, "queue cutover to D1"),
  poolItem(105, "an unrelated note about lunches"),
];

const AUDIT: ProjectAudit = {
  title: "Developing Today",
  number: 4,
  url: "https://github.com/orgs/developing-today/projects/4",
  generatedAt: "2026-11-26T05:59:00.000Z",
  totalItems: POOL.length,
  fields: [],
  views: [],
  counts: { status: { Todo: POOL.length }, area: { CF: POOL.length }, repo: { [REPO]: POOL.length } },
} as unknown as ProjectAudit;

async function seededDb(): Promise<SqlDb> {
  const { sql } = openTenantDb();
  applyMigrations(sql, await tenantStoreMigrations(), () => T1);
  sql.run(
    `INSERT INTO pm_boards (board_id, org, number, node_id, url, role, parent_board_id, sibling_order, title, recorded_at)
     VALUES ('devtoday-main', 'developing-today', 4, 'PVT_x', 'https://github.com/orgs/developing-today/projects/4', 'overall', NULL, 0, 'Developing Today', ?)`,
    [T1],
  );
  landSnapshot(sql, {
    snapshotId: "snap-1",
    boardId: "devtoday-main",
    generatedAt: AUDIT.generatedAt,
    audit: AUDIT,
    counts: { reportedTotal: POOL.length, emittedEntries: POOL.length },
    items: POOL,
    now: T1,
  });
  return sql;
}

const asyncFromSync = (db: SqlDb): AsyncSqlDb => ({
  all: async <T>(sql: string, params: readonly AsyncSqlValue[] = []) =>
    db.all<T>(sql, params as readonly (string | number | null)[]),
});

describe("hosted search over the FTS projection (#14947)", () => {
  test("a query returns exactly the items whose titles carry its words", async () => {
    const db = asyncFromSync(await seededDb());
    const tunnel = await searchItemsAsync(db, "tunnel");
    expect(tunnel.hits.map((h) => h.ref).sort()).toEqual([ref(101), ref(103)]);
    expect(tunnel.total).toBe(2);
    // Several words are ANDed, as a reader expects.
    const both = await searchItemsAsync(db, "front door");
    expect(both.hits.map((h) => h.ref).sort()).toEqual([ref(101), ref(102)]);
    // A prefix matches, so half-typed words still find their item.
    const prefix = await searchItemsAsync(db, "tun");
    expect(prefix.hits.map((h) => h.ref).sort()).toEqual([ref(101), ref(103)]);
    // Nothing matched is an honest empty, not an error.
    const none = await searchItemsAsync(db, "nonexistentword");
    expect(none).toEqual({ query: "nonexistentword", total: 0, hits: [] });
  });

  test("the page is bounded however large the ask", async () => {
    const db = asyncFromSync(await seededDb());
    const page = await searchItemsAsync(db, "tunnel", { limit: 1 });
    expect(page.hits).toHaveLength(1);
    expect(page.total).toBe(2);
    const greedy = await searchItemsAsync(db, "tunnel", { limit: 100_000 });
    expect(greedy.hits.length).toBeLessThanOrEqual(SEARCH_MAX_LIMIT);
  });

  test("a re-landed snapshot is what search finds: the index follows the landing", async () => {
    const sql = await seededDb();
    const db = asyncFromSync(sql);
    expect((await searchItemsAsync(db, "lunches")).hits.map((h) => h.ref)).toEqual([ref(105)]);
    landSnapshot(sql, {
      snapshotId: "snap-2",
      boardId: "devtoday-main",
      generatedAt: "2026-11-26T06:30:00.000Z",
      audit: AUDIT,
      counts: { reportedTotal: POOL.length, emittedEntries: POOL.length },
      items: POOL.map((item) => (item.ref === ref(105) ? poolItem(105, "a note about dinners") : item)),
      now: "2026-11-26T06:30:00.000Z",
    });
    expect((await searchItemsAsync(db, "lunches")).hits).toEqual([]);
    expect((await searchItemsAsync(db, "dinners")).hits.map((h) => h.ref)).toEqual([ref(105)]);
  });

  test("operator-shaped input is words, not MATCH syntax", async () => {
    expect(ftsMatch('title: tunnel OR "queue"')).toBe('"title:"* "tunnel"* "OR"* "queue"*');
    expect(ftsMatch("   ")).toBeNull();
    expect(ftsMatch('*"^')).toBeNull();
    const db = asyncFromSync(await seededDb());
    // Would be a syntax error (or a column filter) if it reached the
    // grammar raw; as words it simply finds nothing extra.
    const res = await searchItemsAsync(db, "title: tunnel");
    expect(res.hits.map((h) => h.ref)).toEqual([]);
  });

  test("the route serves the same read, and refuses a missing query", async () => {
    const handler = createAsyncReadApiHandler({ db: asyncFromSync(await seededDb()), now: () => T1 });
    const ok = await handler(new Request("https://pm.invalid/api/search.json?q=tunnel"));
    expect(ok.status).toBe(200);
    const body = (await ok.json()) as { total: number; hits: { ref: string }[] };
    expect(body.total).toBe(2);
    expect(body.hits.map((h) => h.ref).sort()).toEqual([ref(101), ref(103)]);
    const missing = await handler(new Request("https://pm.invalid/api/search.json"));
    expect(missing.status).toBe(400);
  });
});
