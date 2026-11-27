/**
 * CF-5d (#14970) first read wave — overview + queue views.
 *
 * Oracle: #14970's acceptance for these two surfaces — they serve
 * over the tenant-DB projections with data-plane freshness visible.
 * The expected totals, breakdowns, freshness stamp, and the
 * blocked/ready split below are hand-computed from the fixture
 * pool, not derived by the code under test. A blocker that is
 * closed or absent from the pool must NOT hold an item (it is not
 * part of the live queue's wait); an empty store serves honest
 * zeros and a null freshness stamp, never fabricated data.
 */
import { describe, expect, test } from "bun:test";
import { createReadApiHandler } from "../src/read-api/api.ts";
import { landSnapshot } from "../src/collector/store.ts";
import type { ProjectAudit, ProjectPoolItem } from "../src/collector/pool.ts";
import { tenantStoreMigrations } from "../src/tenant-store/load.ts";
import { applyMigrations, type SqlDb } from "../src/tenant-store/migrate.ts";
import type { BoardDef } from "../src/tenant-store/schema.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";
import { freshnessFromSummaries, overviewFromPool, queueFromPool } from "../src/read-api/waves.ts";
import type { BoardSummary } from "../src/read-api/store.ts";

const REPO = "developing-today/code";
const ref = (number: number) => `${REPO}#${number}`;

function poolItem(partial: Partial<ProjectPoolItem> & { number: number }): ProjectPoolItem {
  const url = `https://github.com/${REPO}/issues/${partial.number}`;
  return {
    id: `PVTI_${partial.number}`,
    url,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    repo: REPO,
    issueUrl: url,
    title: `item ${partial.number}`,
    state: "open",
    contentKind: "issue",
    status: "Todo",
    area: "ID",
    horizon: null,
    kind: null,
    ...partial,
    ref: ref(partial.number),
  };
}

// Hand-computed expectations for this pool:
// - open: 100 (blocked by open 101), 101, 102 (blocked only by
//   closed 103 and absent 999 — both non-live), 104 (draft PR)
// - closed: 103; merged: 105
const POOL: ProjectPoolItem[] = [
  poolItem({ number: 100, blockedBy: [ref(101)] }),
  poolItem({ number: 101, status: "In Progress" }),
  poolItem({ number: 102, blockedBy: [ref(103), ref(999)] }),
  poolItem({ number: 103, state: "closed" }),
  poolItem({ number: 104, contentKind: "pr", draft: true, status: null, area: null }),
  poolItem({ number: 105, state: "merged", contentKind: "pr" }),
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

const SUMMARY_WITH_SNAPSHOT: BoardSummary = {
  board: BOARD,
  snapshot: {
    snapshotId: "snap-1",
    generatedAt: "2026-11-25T08:00:00.000Z",
    reportedTotal: 6,
    emittedEntries: 6,
    writer: "hosted",
    epoch: 3,
  },
  audit: null,
};
const SUMMARY_NO_SNAPSHOT: BoardSummary = {
  board: { ...BOARD, id: "forks-main", title: "Forks" },
  snapshot: null,
  audit: null,
};

describe("CF-5d wave derivations", () => {
  test("freshness reports the newest snapshot and skips boards without one", () => {
    const freshness = freshnessFromSummaries([SUMMARY_NO_SNAPSHOT, SUMMARY_WITH_SNAPSHOT]);
    expect(freshness.latestSnapshotAt).toBe("2026-11-25T08:00:00.000Z");
    expect(freshness.snapshots).toEqual([
      {
        boardId: "devtoday-main",
        snapshotId: "snap-1",
        generatedAt: "2026-11-25T08:00:00.000Z",
        writer: "hosted",
        epoch: 3,
      },
    ]);
    expect(freshnessFromSummaries([SUMMARY_NO_SNAPSHOT]).latestSnapshotAt).toBeNull();
  });

  test("overview totals and breakdowns match the hand-computed pool accounting", () => {
    const overview = overviewFromPool(POOL, [SUMMARY_WITH_SNAPSHOT, SUMMARY_NO_SNAPSHOT]);
    expect(overview.totals).toEqual({
      boards: 2,
      boardsWithSnapshots: 1,
      items: 6,
      open: 4,
      closed: 1,
      merged: 1,
      drafts: 1,
    });
    expect(overview.counts.state).toEqual({ closed: 1, merged: 1, open: 4 });
    expect(overview.counts.status).toEqual({ "(unset)": 1, "In Progress": 1, Todo: 4 });
    expect(overview.counts.contentKind).toEqual({ issue: 4, pr: 2 });
  });

  test("queue splits open items by live blockers only", () => {
    const queue = queueFromPool(POOL);
    expect(queue.totalOpen).toBe(4);
    expect(queue.blocked).toEqual([{ ref: ref(100), blockedBy: [ref(101)] }]);
    expect(queue.readyRefs).toEqual([ref(101), ref(102), ref(104)]);
    expect(queue.byStatus).toEqual([
      { status: "(unset)", refs: [ref(104)] },
      { status: "In Progress", refs: [ref(101)] },
      { status: "Todo", refs: [ref(100), ref(102)] },
    ]);
    expect(queue.items.map((item) => item.ref)).toEqual([ref(100), ref(101), ref(102), ref(104)]);
  });

  test("an empty pool serves honest zeros", () => {
    const queue = queueFromPool([]);
    expect(queue.totalOpen).toBe(0);
    expect(queue.readyRefs).toEqual([]);
    expect(overviewFromPool([], []).totals.items).toBe(0);
  });
});

describe("CF-5d wave routes over a seeded tenant DB", () => {
  const T1 = "2026-11-25T07:00:00.000Z";
  const T2 = "2026-11-25T09:00:00.000Z";

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
      generatedAt: "2026-11-25T08:00:00.000Z",
      totalItems: POOL.length,
      fields: [],
      views: [],
      counts: { status: {}, area: {}, repo: {} },
    };
    landSnapshot(sql, {
      snapshotId: "snap-1",
      boardId: BOARD.id,
      generatedAt: audit.generatedAt,
      audit,
      counts: { reportedTotal: POOL.length, emittedEntries: POOL.length },
      items: POOL,
      now: T1,
    });
    return sql;
  }

  test("GET /api/overview.json serves totals with freshness visible", async () => {
    const handler = createReadApiHandler({ db: await seededDb(), now: () => T2 });
    const res = await handler(new Request("https://pm.test/api/overview.json"));
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      generatedAt: string;
      freshness: { latestSnapshotAt: string | null };
      totals: { items: number; open: number };
    };
    expect(body.generatedAt).toBe(T2);
    expect(body.freshness.latestSnapshotAt).toBe("2026-11-25T08:00:00.000Z");
    expect(body.totals.items).toBe(6);
    expect(body.totals.open).toBe(4);
  });

  test("GET /api/queue.json serves the open pool split by live blockers", async () => {
    const handler = createReadApiHandler({ db: await seededDb(), now: () => T2 });
    const res = await handler(new Request("https://pm.test/api/queue.json"));
    expect(res.status).toBe(200);
    const body = (await res.json()) as { queue: { totalOpen: number; readyRefs: string[] } };
    expect(body.queue.totalOpen).toBe(4);
    expect(body.queue.readyRefs).toEqual([ref(101), ref(102), ref(104)]);
  });
});
