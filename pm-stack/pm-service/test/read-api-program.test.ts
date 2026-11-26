/**
 * CF-5d program read wave.
 *
 * Oracle: every Project pool item appears exactly once: an Epic item is
 * its group's header, a non-Epic item with epicRef belongs to that group,
 * and a non-Epic item without epicRef is unassigned. A reference to an
 * Epic absent from this projection remains a group with epic: null.
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

const T1 = "2026-11-25T08:00:00.000Z";
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
  poolItem(201, { epicRef: ref(200) }),
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

async function seededDb(): Promise<SqlDb> {
  const { sql } = openTenantDb();
  applyMigrations(sql, await tenantStoreMigrations(), () => T1);
  sql.run(
    `INSERT INTO pm_boards (board_id, org, number, node_id, url, role, parent_board_id, sibling_order, title, recorded_at)
     VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)`,
    [BOARD.id, BOARD.org, BOARD.number, BOARD.nodeId, BOARD.url, BOARD.role, BOARD.siblingOrder, BOARD.title, T1],
  );
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
  return sql;
}

describe("CF-5d program read wave", () => {
  test("pool grouping is a deterministic, lossless partition against a filter oracle", () => {
    const generated: ProjectPoolItem[] = [
      poolItem(300, { kind: "Epic" }),
      poolItem(301, { kind: "Epic" }),
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
      epic: generated.find((item) => isEpic(item) && item.ref === epicRef) ?? null,
      items: generated.filter((item) => !isEpic(item) && item.epicRef === epicRef),
    }));
    const expectedUnassigned = generated.filter((item) => !isEpic(item) && !item.epicRef);
    const program = programFromPool(generated);
    expect(program).toEqual({ epics: expectedEpics, unassigned: expectedUnassigned });

    const returnedRefs = [
      ...program.epics.flatMap((group) => [group.epic?.ref, ...group.items.map((item) => item.ref)]),
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
      epics: { ref: string; epic: ProjectPoolItem | null; items: ProjectPoolItem[] }[];
      unassigned: ProjectPoolItem[];
    };
    expect(body.generatedAt).toBe(T2);
    expect(body.freshness).toEqual({
      latestSnapshotAt: T1,
      snapshots: [{ boardId: BOARD.id, snapshotId: "program-snap-1", generatedAt: T1, writer: "hosted", epoch: 0 }],
    });
    expect(body.epics).toEqual([
      { ref: ref(200), epic: POOL[0], items: [POOL[1]] },
      { ref: ref(204), epic: POOL[4], items: [] },
      { ref: ref(999), epic: null, items: [POOL[3]] },
    ]);
    expect(body.unassigned).toEqual([POOL[2]]);
    const returnedRefs = [
      ...body.epics.flatMap((group) => [group.epic?.ref, ...group.items.map((item) => item.ref)]),
      ...body.unassigned.map((item) => item.ref),
    ].filter((item): item is string => item !== undefined);
    expect(returnedRefs.slice().sort()).toEqual(POOL.map((item) => item.ref).sort());
  });
});
