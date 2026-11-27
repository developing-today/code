/**
 * Async read-path parity suite (CF-5c, #14969).
 *
 * Oracle: #14969's acceptance — the async store layer serves the
 * existing read API's #9060 §6 shapes byte-identically. The fixture
 * DB is seeded exactly like test/read-api.test.ts (slice 1's real
 * migrations, slice 2's real landing path, the §8 crate-split
 * chain), then every route's status and body text from the async
 * handler (over an AsyncSqlDb backed by the same bun:sqlite store)
 * must equal the synchronous handler's, byte for byte.
 *
 * The libSQL HTTP adapter is pinned separately against a stub
 * fetch: request shape (pipeline URL, auth header, Hrana arg
 * encoding) and response decoding (Hrana integers arrive as
 * decimal strings and must serve as numbers, like bun:sqlite's).
 */
import { describe, expect, test } from "bun:test";
import { createReadApiHandler } from "../src/read-api/api.ts";
import { createAsyncReadApiHandler } from "../src/read-api/async-api.ts";
import { landSnapshot } from "../src/collector/store.ts";
import type { ProjectAudit, ProjectPoolItem } from "../src/collector/pool.ts";
import { tenantStoreMigrations } from "../src/tenant-store/load.ts";
import { libsqlHttpDb, type AsyncSqlDb, type AsyncSqlValue } from "../src/tenant-store/libsql-http.ts";
import { applyMigrations, type SqlDb } from "../src/tenant-store/migrate.ts";
import type { BoardDef } from "../src/tenant-store/schema.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";
import { handlePmServiceFetch } from "../src/worker.ts";

const T1 = "2026-11-23T06:00:00.000Z";
const T2 = "2026-11-23T07:00:00.000Z";
const REPO = "developing-today/code";
const ref = (number: number) => `${REPO}#${number}`;

const BOARDS: BoardDef[] = [
  {
    id: "devtoday-main",
    org: "developing-today",
    number: 4,
    nodeId: "PVT_kwDOAZNvF84Bouay",
    url: "https://github.com/orgs/developing-today/projects/4",
    role: "overall",
    siblingOrder: 0,
    title: "Developing Today",
  },
  {
    id: "forks-main",
    org: "developing-today-forks",
    number: 1,
    nodeId: "PVT_kwDOCEOo084BrYAh",
    url: "https://github.com/orgs/developing-today-forks/projects/1",
    role: "project",
    parentBoardId: "devtoday-main",
    siblingOrder: 1,
    title: "Developing Today Forks",
  },
];

function poolItem(partial: Partial<ProjectPoolItem> & { number: number; title: string }): ProjectPoolItem {
  const url = `https://github.com/${REPO}/issues/${partial.number}`;
  return {
    id: `PVTI_${partial.number}`,
    databaseId: partial.number * 10,
    url,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    repo: REPO,
    issueUrl: url,
    state: "open",
    contentKind: "issue",
    labels: [],
    status: "Todo",
    area: "ID",
    horizon: null,
    kind: null,
    ...partial,
    ref: ref(partial.number),
  };
}

const POOL = [
  poolItem({
    number: 4403,
    title: "crate-split step: extract the id-events crate",
    kind: "Task",
    status: "In Progress",
    labels: ["primary-parent/id-crate-split"],
    parentRef: ref(3447),
    milestone: { title: "ID program", dueOn: null, closedAt: null },
  }),
  poolItem({
    number: 3447,
    title: "epic: ID crate split",
    kind: "Epic",
    labels: ["epic", "epic/id-crate-split", "design/id-crate-split"],
    parentRef: ref(3195),
  }),
  poolItem({
    number: 3195,
    title: "design: ID crate split",
    labels: ["kind/design", "design/id-crate-split"],
    parentRef: ref(2056),
  }),
  poolItem({
    number: 2056,
    title: "ID platform roadmap",
    labels: ["kind/roadmap-item"],
    milestone: { title: "ID platform — release 2026-12-01", dueOn: "2026-12-01", closedAt: null },
    blockedBy: [ref(1999)],
  }),
  poolItem({ number: 7777, title: "orphan task with no chain", kind: "Task" }),
];

const AUDIT: ProjectAudit = {
  title: "Developing Today",
  number: 4,
  url: "https://github.com/orgs/developing-today/projects/4",
  generatedAt: "2026-11-23T05:59:00.000Z",
  totalItems: POOL.length,
  fields: [{ id: "PVTF_status", name: "Status", dataType: "SINGLE_SELECT" }],
  views: [{ id: "PVTV_1", name: "ID Program", layout: "TABLE_LAYOUT", filter: null }],
  counts: { status: { Todo: 4, "In Progress": 1 }, area: { ID: 5 }, repo: { [REPO]: 5 } },
};

async function seededDb(): Promise<SqlDb> {
  const { sql } = openTenantDb();
  applyMigrations(sql, await tenantStoreMigrations(), () => T1);
  for (const board of BOARDS) {
    sql.run(
      `INSERT INTO pm_boards (board_id, org, number, node_id, url, role, parent_board_id, sibling_order, title, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        board.id,
        board.org,
        board.number,
        board.nodeId,
        board.url,
        board.role,
        board.parentBoardId ?? null,
        board.siblingOrder,
        board.title,
        T1,
      ],
    );
  }
  landSnapshot(sql, {
    snapshotId: "snap-main-1",
    boardId: "devtoday-main",
    generatedAt: AUDIT.generatedAt,
    audit: AUDIT,
    counts: { reportedTotal: POOL.length, emittedEntries: POOL.length },
    items: POOL,
    now: T1,
  });
  sql.run(
    `INSERT INTO pm_roadmap_rows (row_id, subject, title, area, status, horizon, horizon_error, source, notes,
       links_json, priority_ref, after_refs_json, release_ref, grist_url, file_url, recorded_at)
     VALUES (1, 'grist:docA/tblA/1', 'ID crate split', 'ID', 'Active', '', NULL, 'grist', '',
       ?, NULL, '[]', NULL, NULL, NULL, ?)`,
    [JSON.stringify([{ ref: ref(3447), relation: "tracks" }]), T1],
  );
  sql.run(
    `INSERT INTO pm_roadmap_rows (row_id, subject, title, area, status, horizon, horizon_error, source, notes,
       links_json, priority_ref, after_refs_json, release_ref, grist_url, file_url, recorded_at)
     VALUES (2, 'grist:docA/tblA/2', 'Stored overlay row', 'ID', 'Active', '', NULL, 'grist', '',
       '[]', ?, ?, ?, NULL, NULL, ?)`,
    [ref(2000), JSON.stringify([ref(1999)]), ref(2056), T1],
  );
  sql.run(
    `INSERT INTO pm_milestones (repo, number, title, kind, due_on, closed_at, state, recorded_at)
     VALUES (?, 3, 'ID platform — release 2026-11-01', 'release', '2026-11-01', '2026-11-01T17:00:00.000Z', 'closed', ?)`,
    [REPO, T1],
  );
  sql.run(
    `INSERT INTO pm_release_snapshots (repo, milestone_number, release_date,
       issues_covered, issues_closed, issues_carried_over,
       designs_covered, designs_closed, designs_carried_over,
       epics_covered, epics_closed, epics_carried_over,
       carried_over_epics_json, computed_at, snapshot_json)
     VALUES (?, 3, '2026-11-01', 10, 8, 2, 3, 3, 0, 2, 1, 1, ?, ?, '{}')`,
    [REPO, JSON.stringify([ref(2056)]), T1],
  );
  return sql;
}

/** The async surface over the same bun:sqlite store — the parity
 * claim is about the store layer and handler, not the transport;
 * the transport itself is pinned in the adapter suite below. */
const asyncFromSync = (db: SqlDb): AsyncSqlDb => ({
  all: async <T>(sql: string, params: readonly AsyncSqlValue[] = []) =>
    db.all<T>(sql, params as readonly (string | number | null)[]),
});

const ROUTES: [string, string][] = [
  ["projects", "/api/projects.json"],
  ["item (known ref)", `/api/item/${encodeURIComponent(ref(4403))}.json`],
  ["item (unknown ref)", `/api/item/${encodeURIComponent(ref(424242))}.json`],
  ["roadmap", "/api/roadmap.json"],
  ["releases", "/api/releases.json"],
  ["program (CF-5d wave)", "/api/program.json"],
  ["overview (CF-5d wave)", "/api/overview.json"],
  ["queue (CF-5d wave)", "/api/queue.json"],
  ["unknown path", "/api/nope.json"],
];

describe("async read path parity with the synchronous handler", () => {
  for (const [name, path] of ROUTES) {
    test(`GET ${name}: status and body are byte-identical`, async () => {
      const db = await seededDb();
      const syncHandler = createReadApiHandler({ db, now: () => T2 });
      const asyncHandler = createAsyncReadApiHandler({ db: asyncFromSync(db), now: () => T2 });
      const syncRes = await syncHandler(new Request(`https://pm.test${path}`));
      const asyncRes = await asyncHandler(new Request(`https://pm.test${path}`));
      expect(asyncRes.status).toBe(syncRes.status);
      expect(await asyncRes.text()).toBe(await syncRes.text());
    });
  }

  test("POST is a 404 on both handlers", async () => {
    const db = await seededDb();
    const asyncHandler = createAsyncReadApiHandler({ db: asyncFromSync(db), now: () => T2 });
    const res = await asyncHandler(new Request("https://pm.test/api/projects.json", { method: "POST" }));
    expect(res.status).toBe(404);
  });
});

describe("worker entry", () => {
  test("uses the tenant D1 binding for hosted reads", async () => {
    const queries: { sql: string; args: unknown[] }[] = [];
    const d1 = {
      prepare(sql: string) {
        let args: unknown[] = [];
        const statement = {
          bind(...values: unknown[]) {
            args = values;
            return statement;
          },
          async all() {
            queries.push({ sql, args });
            return { success: true, results: [] };
          },
        };
        return statement;
      },
    };
    const env = { PM_TENANT_D1: d1 } as unknown as Parameters<typeof handlePmServiceFetch>[1];
    const response = await handlePmServiceFetch(new Request("https://pm.test/api/projects.json"), env);

    expect(response.status).toBe(200);
    expect(queries.length).toBeGreaterThan(0);
    expect(queries.every(({ sql }) => /^\s*SELECT\b/i.test(sql))).toBe(true);
  });

  test("/health reports the commit and age of its deployment", async () => {
    const deployedAt = new Date(Date.now() - 5_000).toISOString();
    const res = await handlePmServiceFetch(new Request("https://pm.test/health"), {
      VERSION: "abc123",
      DEPLOYED_AT: deployedAt,
    });
    expect(res.status).toBe(200);
    const health = (await res.json()) as {
      ok: boolean;
      service: string;
      version: string;
      deployedAt: string;
      ageSeconds: number;
    };
    expect(health).toMatchObject({ ok: true, service: "pm-service", version: "abc123", deployedAt });
    expect(Number.isInteger(health.ageSeconds)).toBe(true);
    expect(health.ageSeconds).toBeGreaterThanOrEqual(5);
  });

  test("/health does not invent a deployment age when the stamp is missing or invalid", async () => {
    for (const deployedAt of [undefined, "not-a-timestamp"]) {
      const res = await handlePmServiceFetch(new Request("https://pm.test/health"), {
        VERSION: "abc123",
        DEPLOYED_AT: deployedAt,
      });
      expect(await res.json()).toMatchObject({ deployedAt: null, ageSeconds: null });
    }
  });

  test("API routes answer an honest 503 until the tenant store is configured", async () => {
    const res = await handlePmServiceFetch(new Request("https://pm.test/api/projects.json"), {});
    expect(res.status).toBe(503);
    expect(await res.json()).toEqual({ error: "pm-service tenant store is not configured" });
  });
});

describe("libsqlHttpDb (Hrana v3 pipeline over fetch)", () => {
  const canned = (rows: unknown[][], colNames: string[]) =>
    new Response(
      JSON.stringify({
        baton: null,
        base_url: null,
        results: [
          {
            type: "ok",
            response: {
              type: "execute",
              result: {
                cols: colNames.map((name) => ({ name, decltype: null })),
                rows,
                affected_row_count: 0,
                last_insert_rowid: null,
              },
            },
          },
          { type: "ok", response: { type: "close" } },
        ],
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );

  test("posts execute+close to /v3/pipeline with encoded args and the bearer token", async () => {
    let seenUrl = "";
    let seenInit: RequestInit | undefined;
    const fetchImpl = (async (url: string, init?: RequestInit) => {
      seenUrl = url;
      seenInit = init;
      return canned([[{ type: "integer", value: "7" }]], ["n"]);
    }) as typeof fetch;
    const db = libsqlHttpDb({ url: "libsql://tenant.example", token: "tok", fetchImpl });
    const rows = await db.all<{ n: number }>("SELECT ? + ?", [3, 4]);
    expect(seenUrl).toBe("https://tenant.example/v3/pipeline");
    expect((seenInit?.headers as Record<string, string>).authorization).toBe("Bearer tok");
    const body = JSON.parse(seenInit?.body as string) as {
      requests: { type: string; stmt?: { sql: string; args: unknown[] } }[];
    };
    expect(body.requests[0]?.type).toBe("execute");
    expect(body.requests[0]?.stmt?.args).toEqual([
      { type: "integer", value: "3" },
      { type: "integer", value: "4" },
    ]);
    expect(body.requests[1]?.type).toBe("close");
    expect(rows).toEqual([{ n: 7 }]);
  });

  test("decodes Hrana values to the shapes bun:sqlite serves", async () => {
    const fetchImpl = (async () =>
      canned(
        [
          [
            { type: "integer", value: "42" },
            { type: "float", value: 1.5 },
            { type: "text", value: "x" },
            { type: "null" },
            { type: "blob", value: "aGk=" },
          ],
        ],
        ["i", "f", "t", "n", "b"],
      )) as typeof fetch;
    const db = libsqlHttpDb({ url: "https://tenant.example/", fetchImpl });
    const rows = await db.all<{ i: number; f: number; t: string; n: null; b: Uint8Array }>("SELECT 1");
    expect(rows[0]?.i).toBe(42);
    expect(rows[0]?.f).toBe(1.5);
    expect(rows[0]?.t).toBe("x");
    expect(rows[0]?.n).toBeNull();
    expect(rows[0]?.b).toEqual(Uint8Array.from([104, 105]));
  });

  test("a pipeline error result throws with the server's message", async () => {
    const fetchImpl = (async () =>
      new Response(
        JSON.stringify({
          baton: null,
          base_url: null,
          results: [{ type: "error", error: { message: "no such table: pm_items", code: "SQLITE_ERROR" } }],
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      )) as typeof fetch;
    const db = libsqlHttpDb({ url: "https://tenant.example", fetchImpl });
    await expect(db.all("SELECT * FROM pm_items")).rejects.toThrow("no such table: pm_items");
  });

  test("a non-200 response throws with the status", async () => {
    const fetchImpl = (async () => new Response("unauthorized", { status: 401 })) as typeof fetch;
    const db = libsqlHttpDb({ url: "https://tenant.example", token: "bad", fetchImpl });
    await expect(db.all("SELECT 1")).rejects.toThrow("libsql http 401");
  });
});
