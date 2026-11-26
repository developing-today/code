/**
 * Dashboard pages, Workers edition (#12829).
 *
 * Oracle: design doc/2026-11-23T03-22-03Z_design_pm-service-workers-edition
 * §4's must-have list — boards, item genealogy, roadmap with the
 * overall priorities view, releases, needs-attention (coverage and
 * audit plus open findings), and quota/usage named honestly — each
 * served as a page over the same async loads the JSON API uses.
 * The fixture DB is seeded through slice 1's real migrations and
 * slice 2's real landing path, like test/read-api-async.test.ts.
 */
import { describe, expect, test } from "bun:test";
import { landSnapshot } from "../src/collector/store.ts";
import type { ProjectAudit, ProjectPoolItem } from "../src/collector/pool.ts";
import { auditChecks, createDashboardHandler, escapeHtml } from "../src/dashboard/dashboard.ts";
import { tenantStoreMigrations } from "../src/tenant-store/load.ts";
import type { AsyncSqlDb, AsyncSqlValue } from "../src/tenant-store/libsql-http.ts";
import { applyMigrations, type SqlDb } from "../src/tenant-store/migrate.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";
import { handlePmServiceFetch } from "../src/worker.ts";

const T1 = "2026-11-23T06:00:00.000Z";
const T2 = "2026-11-23T07:00:00.000Z";
const REPO = "developing-today/code";
const ref = (number: number) => `${REPO}#${number}`;

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
    parentRef: ref(3447),
  }),
  poolItem({ number: 3447, title: "epic: ID crate split", kind: "Epic", labels: ["epic"], parentRef: ref(3195) }),
  poolItem({ number: 3195, title: "design: ID crate split", labels: ["kind/design"] }),
  poolItem({ number: 7777, title: "orphan <task> with no chain", kind: "Task" }),
];

const AUDIT: ProjectAudit = {
  title: "Developing Today",
  number: 4,
  url: "https://github.com/orgs/developing-today/projects/4",
  generatedAt: "2026-11-23T05:59:00.000Z",
  totalItems: POOL.length,
  fields: [{ id: "PVTF_status", name: "Status", dataType: "SINGLE_SELECT" }],
  views: [{ id: "PVTV_1", name: "ID Program", layout: "TABLE_LAYOUT", filter: null }],
  counts: { status: { Todo: 3, "In Progress": 1 }, area: { ID: 4 }, repo: { [REPO]: 4 } },
};

async function seededDb(opts: { truncated?: boolean } = {}): Promise<SqlDb> {
  const { sql } = openTenantDb();
  applyMigrations(sql, await tenantStoreMigrations(), () => T1);
  const insertBoard = (
    id: string,
    org: string,
    number: number,
    role: string,
    parent: string | null,
    order: number,
    title: string,
  ) =>
    sql.run(
      `INSERT INTO pm_boards (board_id, org, number, node_id, url, role, parent_board_id, sibling_order, title, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        id,
        org,
        number,
        `PVT_${id}`,
        `https://github.com/orgs/${org}/projects/${number}`,
        role,
        parent,
        order,
        title,
        T1,
      ],
    );
  insertBoard("devtoday-main", "developing-today", 4, "overall", null, 0, "Developing Today");
  insertBoard("forks-main", "developing-today-forks", 1, "project", "devtoday-main", 1, "Developing Today Forks");
  landSnapshot(sql, {
    snapshotId: "snap-main-1",
    boardId: "devtoday-main",
    generatedAt: AUDIT.generatedAt,
    audit: AUDIT,
    counts: { reportedTotal: opts.truncated ? POOL.length + 2 : POOL.length, emittedEntries: POOL.length },
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
     VALUES (2, 'grist:docA/tblA/2', 'Stored overlay row', 'Forks', 'Active', '', NULL, 'grist', '',
       '[]', ?, '[]', NULL, NULL, NULL, ?)`,
    [ref(2000), T1],
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
    [REPO, JSON.stringify([ref(3447)]), T1],
  );
  sql.run(
    `INSERT INTO pm_reconcile_findings (finding_id, board_id, kind, severity, subject_ref, summary, recorded_at, resolved_at)
     VALUES ('f-open', 'devtoday-main', 'field-drift', 'error', ?, 'status differs from GitHub', ?, NULL),
            ('f-done', 'devtoday-main', 'field-drift', 'warning', NULL, 'already resolved finding', ?, ?)`,
    [ref(4403), T1, T1, T2],
  );
  return sql;
}

const asyncFromSync = (db: SqlDb): AsyncSqlDb => ({
  all: async <T>(sql: string, params: readonly AsyncSqlValue[] = []) =>
    db.all<T>(sql, params as readonly (string | number | null)[]),
});

async function get(path: string, opts: { truncated?: boolean; method?: string } = {}) {
  const db = await seededDb(opts);
  const handler = createDashboardHandler({ db: asyncFromSync(db), now: () => T2 });
  const res = await handler(new Request(`https://pm.test${path}`, { method: opts.method ?? "GET" }));
  return { res, html: res ? await res.text() : "" };
}

describe("dashboard pages", () => {
  test("boards page lists the whole registry, with absence said honestly", async () => {
    const { res, html } = await get("/");
    expect(res?.status).toBe(200);
    expect(res?.headers.get("content-type")).toContain("text/html");
    expect(res?.headers.get("cache-control")).toBe("no-store");
    expect(html).toContain('<a href="/board/devtoday-main">Developing Today</a>');
    expect(html).toContain("Developing Today Forks");
    expect(html).toContain("no snapshot yet");
    expect(html).toContain("4/4 items");
    expect(html).toContain(`Rendered ${T2}`);
  });

  test("board page lists the board's items and escapes their text", async () => {
    const { res, html } = await get("/board/devtoday-main");
    expect(res?.status).toBe(200);
    expect(html).toContain(`/item/${encodeURIComponent(ref(4403))}`);
    expect(html).toContain("orphan &lt;task&gt; with no chain");
    expect(html).not.toContain("orphan <task>");
    expect(html).toContain("Views: ID Program");
  });

  test("a board without a snapshot says so; an unknown board is a 404 page", async () => {
    const forks = await get("/board/forks-main");
    expect(forks.res?.status).toBe(200);
    expect(forks.html).toContain("No snapshot has landed for this board yet.");
    const missing = await get("/board/nope");
    expect(missing.res?.status).toBe(404);
    expect(missing.html).toContain("No board nope in the registry.");
  });

  test("item page carries the genealogy header, root first, and the sentence", async () => {
    const { res, html } = await get(`/item/${encodeURIComponent(ref(4403))}`);
    expect(res?.status).toBe(200);
    const design = html.indexOf("design: ID crate split");
    const epic = html.indexOf("epic: ID crate split");
    const self = html.indexOf("<b>crate-split step: extract the id-events crate</b>");
    expect(design).toBeGreaterThan(-1);
    expect(epic).toBeGreaterThan(design);
    expect(self).toBeGreaterThan(epic);
    expect(html).toContain("Working on crate-split step: extract the id-events crate");
    expect(html).toContain(`href="https://github.com/${REPO}/issues/4403"`);
  });

  test("an unknown or malformed item ref is a 404 page", async () => {
    expect((await get(`/item/${encodeURIComponent(ref(424242))}`)).res?.status).toBe(404);
    expect((await get("/item/%E0%A4%A")).res?.status).toBe(404);
  });

  test("roadmap page shows the overall priorities and the per-project view", async () => {
    const { res, html } = await get("/roadmap");
    expect(res?.status).toBe(200);
    expect(html).toContain("<h2>Overall priorities</h2>");
    expect(html).toContain(`<h3><a href="/item/${encodeURIComponent(ref(2000))}">${escapeHtml(ref(2000))}</a></h3>`);
    expect(html).toContain("<h2>By project</h2>");
    expect(html).toContain("<h3>ID</h3>");
    expect(html).toContain("<h3>Forks</h3>");
  });

  test("releases page shows the close-out card with carried-over epics", async () => {
    const { res, html } = await get("/releases");
    expect(res?.status).toBe(200);
    expect(html).toContain("ID platform — release 2026-11-01");
    expect(html).toContain(`Carried over: <a href="/item/${encodeURIComponent(ref(3447))}"`);
  });

  test("needs-attention lists open findings only, plus coverage checks", async () => {
    const clean = await get("/attention");
    expect(clean.res?.status).toBe(200);
    expect(clean.html).toContain("status differs from GitHub");
    expect(clean.html).not.toContain("already resolved finding");
    expect(clean.html).toContain("no snapshot has landed for this board yet");
    const truncated = await get("/attention", { truncated: true });
    expect(truncated.html).toContain("latest snapshot emitted 4 of 6 reported items");
  });

  test("quota page names the gap instead of inventing numbers", async () => {
    const { res, html } = await get("/quota");
    expect(res?.status).toBe(200);
    expect(html).toContain("Quota and usage are not in the tenant store yet.");
  });

  test("the dashboard is read-only and leaves other paths to the API", async () => {
    expect((await get("/roadmap", { method: "POST" })).res?.status).toBe(405);
    expect((await get("/api/projects.json")).res).toBeNull();
  });
});

describe("auditChecks", () => {
  test("flags a missing snapshot, a truncated ingest and an audit total mismatch", () => {
    const board = {
      id: "b",
      org: "o",
      number: 1,
      nodeId: "n",
      url: "https://x",
      role: "project",
      siblingOrder: 0,
      title: "B",
    } as never;
    const checks = auditChecks([
      { board, snapshot: null, audit: null },
      {
        board,
        snapshot: { snapshotId: "s", generatedAt: T1, reportedTotal: 5, emittedEntries: 3, writer: "hosted", epoch: 1 },
        audit: { ...AUDIT, totalItems: 7 },
      },
    ]);
    expect(checks.map((c) => c.severity)).toEqual(["warning", "error", "warning"]);
  });
});

describe("worker entry serves the dashboard", () => {
  test("dashboard pages answer the same honest 503 until the store is configured", async () => {
    const res = await handlePmServiceFetch(new Request("https://pm.test/"), {});
    expect(res.status).toBe(503);
  });
});
