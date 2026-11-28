/**
 * CF-5d program read wave.
 *
 * Oracle: every Project pool row appears exactly once: Epic rows are
 * group headers, a non-Epic row with epicRef belongs to that group, and
 * a non-Epic row without epicRef is unassigned. Repeated rows across
 * boards remain distinct; a missing Epic header yields an empty headers
 * list rather than invented data.
 */
import { describe, expect, test } from "bun:test";
import { createReadApiHandler } from "../src/read-api/api.ts";
import { landSnapshot } from "../src/collector/store.ts";
import type { ProjectAudit, ProjectPoolItem } from "../src/collector/pool.ts";
import { tenantStoreMigrations } from "../src/tenant-store/load.ts";
import { applyMigrations, type SqlDb } from "../src/tenant-store/migrate.ts";
import type { BoardDef } from "../src/tenant-store/schema.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";
import { programFromPool } from "../src/read-api/waves.ts";
import { createDashboardHandler, escapeHtml } from "../src/dashboard/dashboard.ts";
import { programJsonAsync } from "../src/read-api/async-api.ts";
import type { AsyncSqlDb, AsyncSqlValue } from "../src/tenant-store/libsql-http.ts";

const T1 = "2026-11-25T08:00:00.000Z";
const T3 = "2026-11-25T08:30:00.000Z";
const T2 = "2026-11-25T09:00:00.000Z";
const REPO = "developing-today/code";
const ref = (number: number) => `${REPO}#${number}`;

function poolItem(number: number, fields: Partial<ProjectPoolItem> = {}): ProjectPoolItem {
  const url = `https://github.com/${REPO}/issues/${number}`;
  return {
    id: `PVTI_${number}`,
    url,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    repo: REPO,
    number,
    ref: ref(number),
    issueUrl: url,
    title: `item ${number}`,
    state: "open",
    contentKind: "issue",
    status: "Todo",
    area: "PM",
    horizon: null,
    kind: null,
    ...fields,
  };
}

const POOL: ProjectPoolItem[] = [
  poolItem(200, { kind: "Epic" }),
  poolItem(201, { epicRef: ref(200), title: 'child <script>alert("x")</script> & task' }),
  poolItem(202),
  poolItem(203, { epicRef: ref(999) }),
  poolItem(204, { kind: "Epic", state: "closed" }),
];

const BOARD: BoardDef = {
  id: "devtoday-main",
  org: "developing-today",
  number: 4,
  nodeId: "PVT_kwDOAZNvF84Bouay",
  url: "https://github.com/orgs/developing-today/projects/4",
  role: "overall",
  siblingOrder: 0,
  title: "Developing Today",
};
const FORKS_BOARD: BoardDef = {
  id: "forks-main",
  org: "developing-today-forks",
  number: 1,
  nodeId: "PVT_kwDOCEOo084BrYAh",
  url: "https://github.com/orgs/developing-today-forks/projects/1",
  role: "project",
  parentBoardId: BOARD.id,
  siblingOrder: 1,
  title: "Developing Today Forks",
};
const FORKS_EPIC = { ...poolItem(206, { boardId: FORKS_BOARD.id, kind: "Epic" }), ref: ref(200) };
const FORKS_CHILD = poolItem(207, { boardId: FORKS_BOARD.id, epicRef: ref(200) });

async function seededDb(): Promise<SqlDb> {
  const { sql } = openTenantDb();
  applyMigrations(sql, await tenantStoreMigrations(), () => T1);
  const boardInsertSql =
    "INSERT INTO pm_boards (board_id, org, number, node_id, url, role, parent_board_id, sibling_order, title, recorded_at) " +
    "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
  sql.run(
    `INSERT INTO pm_boards (board_id, org, number, node_id, url, role, parent_board_id, sibling_order, title, recorded_at)
     VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)`,
    [BOARD.id, BOARD.org, BOARD.number, BOARD.nodeId, BOARD.url, BOARD.role, BOARD.siblingOrder, BOARD.title, T1],
  );
  sql.run(boardInsertSql, [
    FORKS_BOARD.id,
    FORKS_BOARD.org,
    FORKS_BOARD.number,
    FORKS_BOARD.nodeId,
    FORKS_BOARD.url,
    FORKS_BOARD.role,
    FORKS_BOARD.parentBoardId ?? null,
    FORKS_BOARD.siblingOrder,
    FORKS_BOARD.title,
    T1,
  ]);
  const audit: ProjectAudit = {
    title: BOARD.title,
    number: BOARD.number,
    url: BOARD.url,
    generatedAt: T1,
    totalItems: POOL.length,
    fields: [],
    views: [],
    counts: { status: {}, area: {}, repo: {} },
  };
  landSnapshot(sql, {
    snapshotId: "program-snap-1",
    boardId: BOARD.id,
    generatedAt: T1,
    audit,
    counts: { reportedTotal: POOL.length, emittedEntries: POOL.length },
    items: POOL,
    now: T1,
  });
  const forksAudit: ProjectAudit = {
    title: FORKS_BOARD.title,
    number: FORKS_BOARD.number,
    url: FORKS_BOARD.url,
    generatedAt: T3,
    totalItems: 2,
    fields: [],
    views: [],
    counts: { status: {}, area: {}, repo: {} },
  };
  landSnapshot(sql, {
    snapshotId: "program-snap-2",
    boardId: FORKS_BOARD.id,
    generatedAt: T3,
    audit: forksAudit,
    counts: { reportedTotal: 2, emittedEntries: 2 },
    items: [FORKS_EPIC, FORKS_CHILD],
    now: T1,
  });
  return sql;
}

const asyncFromSync = (db: SqlDb): AsyncSqlDb => ({
  all: async <T>(sql: string, params: readonly AsyncSqlValue[] = []) => {
    // Oracle: page/API reads must never add a write path.
    expect(sql.trimStart().toUpperCase().startsWith("SELECT ")).toBe(true);
    return db.all<T>(sql, params as readonly (string | number | null)[]);
  },
});

describe("CF-5d program HTML wave", () => {
  test("GET /program renders the same seeded projection as the JSON API, escaped and lossless", async () => {
    // Oracle: the landed program JSON projection over the exact same seeded DB.
    const db = await seededDb();
    db.run(
      `INSERT INTO pm_boards (board_id, org, number, node_id, url, role, sibling_order, title, recorded_at)
      VALUES ('empty<&', 'example', 9, 'PVT_empty', 'https://github.com/orgs/example/projects/9', 'project', 2, 'Empty', ?)`,
      [T1],
    );
    const deps = { db: asyncFromSync(db), now: () => T2 };
    const program = await programJsonAsync(deps);
    const response = await createDashboardHandler(deps)(new Request("https://pm.test/program"));
    expect(response?.status).toBe(200);
    expect(response?.headers.get("content-type")).toContain("text/html");
    expect(response?.headers.get("cache-control")).toBe("no-store");
    const html = await response!.text();
    expect(html).toContain('<a href="/program">Program</a>');
    for (const group of program.epics) {
      expect(html).toContain(`<h2><a href="/item/${encodeURIComponent(group.ref)}">${escapeHtml(group.ref)}</a></h2>`);
      if (!group.headers.length) expect(html).toContain("No Epic header in the tenant pool.");
    }
    const rows = [...program.epics.flatMap((group) => [...group.headers, ...group.items]), ...program.unassigned];
    for (const item of rows) {
      // Titles uniquely identify these fixture rows, including duplicate Epic refs across boards.
      expect(html.split(`<td>${escapeHtml(item.title)}</td>`).length - 1).toBe(1);
    }
    expect(html).not.toContain('<script>alert("x")</script>');
    expect(html).toContain("<h2>Unassigned</h2>");
    expect(html).toContain("empty&lt;&amp;: no snapshot yet");
    expect(html).toContain("No child items in the tenant pool.");
    expect(html).toContain(`Latest snapshot: ${program.freshness.latestSnapshotAt}`);
    for (const snapshot of program.freshness.snapshots) {
      expect(html).toContain(escapeHtml(snapshot.boardId));
      expect(html).toContain(escapeHtml(snapshot.snapshotId ?? "no snapshot yet"));
      if (snapshot.generatedAt) expect(html).toContain(snapshot.generatedAt);
    }
    expect(html).toContain(`Rendered ${T2}`);
    expect(html).toContain("Editorial program and health summaries are not in this projection.");
  });

  test("empty program states are honest; methods and JSON fallthrough match dashboard conventions", async () => {
    // Oracle: migrated empty store has neither program rows nor snapshot freshness.
    const { sql } = openTenantDb();
    applyMigrations(sql, await tenantStoreMigrations(), () => T1);
    const handler = createDashboardHandler({ db: asyncFromSync(sql), now: () => T2 });
    const response = await handler(new Request("https://pm.test/program"));
    expect(response?.status).toBe(200);
    const html = await response!.text();
    expect(html).toContain("No Epic groups in the tenant pool.");
    expect(html).toContain("No unassigned items.");
    expect(html).toContain("No snapshots have landed yet.");
    expect(html).toContain("The registry has no boards.");
    expect((await handler(new Request("https://pm.test/program", { method: "POST" })))?.status).toBe(405);
    expect(await handler(new Request("https://pm.test/api/program.json"))).toBeNull();
  });
});

describe("CF-5d program read wave", () => {
  test("pool grouping is a deterministic, lossless partition against a filter oracle", () => {
    const generated: ProjectPoolItem[] = [
      poolItem(300, { kind: "Epic" }),
      poolItem(301, { kind: "Epic" }),
      { ...poolItem(302, { kind: "Epic", boardId: "forks-main" }), ref: ref(300) },
      { ...poolItem(997, { epicRef: ref(300), boardId: "forks-main" }), ref: ref(400) },
      ...Array.from({ length: 96 }, (_, index) => {
        const number = 400 + index;
        if (index % 5 === 0) return poolItem(number);
        if (index % 5 === 1) return poolItem(number, { epicRef: ref(300) });
        if (index % 5 === 2) return poolItem(number, { epicRef: ref(301) });
        return poolItem(number, { epicRef: ref(9999) });
      }),
    ];
    const isEpic = (item: ProjectPoolItem) => item.kind?.toLowerCase() === "epic";
    const refs = new Set<string>();
    for (const item of generated) {
      if (isEpic(item)) refs.add(item.ref);
      else if (item.epicRef) refs.add(item.epicRef);
    }
    const expectedEpics = [...refs].sort().map((epicRef) => ({
      ref: epicRef,
      headers: generated.filter((item) => isEpic(item) && item.ref === epicRef),
      items: generated.filter((item) => !isEpic(item) && item.epicRef === epicRef),
    }));
    const expectedUnassigned = generated.filter((item) => !isEpic(item) && !item.epicRef);
    const program = programFromPool(generated);
    expect(program).toEqual({ epics: expectedEpics, unassigned: expectedUnassigned });

    const returnedRefs = [
      ...program.epics.flatMap((group) => [
        ...group.headers.map((item) => item.ref),
        ...group.items.map((item) => item.ref),
      ]),
      ...program.unassigned.map((item) => item.ref),
    ].filter((item): item is string => item !== undefined);
    expect(returnedRefs.length).toBe(generated.length);
    expect(returnedRefs.slice().sort()).toEqual(generated.map((item) => item.ref).sort());
  });

  test("GET /api/program.json partitions the Project pool and carries truthful freshness", async () => {
    const handler = createReadApiHandler({ db: await seededDb(), now: () => T2 });
    const response = await handler(new Request("https://pm.test/api/program.json"));
    expect(response.status).toBe(200);

    const body = (await response.json()) as {
      generatedAt: string;
      freshness: { latestSnapshotAt: string | null; snapshots: { boardId: string; snapshotId: string }[] };
      epics: { ref: string; headers: ProjectPoolItem[]; items: ProjectPoolItem[] }[];
      unassigned: ProjectPoolItem[];
    };
    expect(body.generatedAt).toBe(T2);
    expect(body.freshness).toEqual({
      latestSnapshotAt: T3,
      snapshots: [
        { boardId: BOARD.id, snapshotId: "program-snap-1", generatedAt: T1, writer: "hosted", epoch: 0 },
        { boardId: FORKS_BOARD.id, snapshotId: "program-snap-2", generatedAt: T3, writer: "hosted", epoch: 0 },
      ],
    });
    expect(body.epics).toEqual([
      { ref: ref(200), headers: [POOL[0], FORKS_EPIC], items: [POOL[1], FORKS_CHILD] },
      { ref: ref(204), headers: [POOL[4]], items: [] },
      { ref: ref(999), headers: [], items: [POOL[3]] },
    ]);
    expect(body.unassigned).toEqual([POOL[2]]);
    const returnedRefs = [
      ...body.epics.flatMap((group) => [
        ...group.headers.map((item) => item.ref),
        ...group.items.map((item) => item.ref),
      ]),
      ...body.unassigned.map((item) => item.ref),
    ].filter((item): item is string => item !== undefined);
    expect(returnedRefs.slice().sort()).toEqual([...POOL, FORKS_EPIC, FORKS_CHILD].map((item) => item.ref).sort());
  });
});
