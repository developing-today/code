/**
 * Hosted collector acceptance suite (#12826).
 *
 * Oracle: issue #12826's acceptance criteria and component A design
 * §3 — a cron run ingests every registry board into the tenant DB;
 * a forced failure on one board leaves the other boards' pools
 * intact and records a finding; ingest cursors make a second run
 * incremental (no full re-fetch when nothing changed); no token
 * value persists anywhere after a run. Plus the design's §3/§7
 * rules the criteria rest on: fail-soft per board, merged pool with
 * cross-board ancestry resolution, per-board IngestDO serialization
 * with rate-budget deferral, and the same GraphQL pool query as the
 * laptop collector.
 *
 * Everything is faked at the seams the collector injects (fetch,
 * token provider, IngestDO storage); the tenant DB is bun:sqlite
 * in-memory with slice 1's real migrations applied.
 */
import { describe, expect, test } from "bun:test";
import { runHostedCollector, type CollectorDeps } from "../src/collector/collector.ts";
import { IngestDO, MemoryIngestStorage, type IngestState } from "../src/collector/ingest-do.ts";
import { PROJECT_QUERY } from "../src/collector/pool.ts";
import { tenantStoreMigrations } from "../src/tenant-store/load.ts";
import { applyMigrations, type SqlDb } from "../src/tenant-store/migrate.ts";
import type { BoardDef } from "../src/tenant-store/schema.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";

const T1 = "2026-11-23T06:00:00.000Z";
const T2 = "2026-11-23T07:00:00.000Z";
const RESET_FUTURE = 1893456000; // 2030-01-01, unix seconds

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

async function migratedDb() {
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
  return sql;
}

// ---------------------------------------------------------------------------
// GraphQL fixtures: the wire shape of the laptop collector's query.
// ---------------------------------------------------------------------------

interface ItemSpec {
  id: string;
  repo: string;
  number: number;
  title: string;
  kind?: string | null;
  status?: string | null;
  parent?: { repo: string; number: number } | null;
  body?: string;
  pr?: boolean;
  labels?: string[];
  subIssuesSummary?: { total: number; completed: number };
}

function gqlItem(spec: ItemSpec) {
  const fieldValues: unknown[] = [];
  if (spec.status !== undefined) fieldValues.push({ name: spec.status, field: { name: "Status" } });
  if (spec.kind !== undefined) fieldValues.push({ name: spec.kind, field: { name: "Kind" } });
  const url = `https://github.com/${spec.repo}/${spec.pr ? "pull" : "issues"}/${spec.number}`;
  return {
    id: spec.id,
    databaseId: spec.number * 10,
    type: spec.pr ? "PULL_REQUEST" : "ISSUE",
    content: {
      number: spec.number,
      title: spec.title,
      ...(spec.pr ? {} : { body: spec.body ?? "" }),
      state: "OPEN",
      url,
      repository: { nameWithOwner: spec.repo },
      updatedAt: "2026-11-20T10:00:00.000Z",
      closedAt: null,
      ...(spec.pr ? { mergedAt: null, isDraft: false } : {}),
      milestone: null,
      ...(spec.pr ? {} : { subIssuesSummary: spec.subIssuesSummary ?? null }),
      author: { login: "dezren39" },
      ...(spec.pr
        ? {}
        : {
            labels: { nodes: (spec.labels ?? []).map((name) => ({ name })) },
            parent: spec.parent
              ? { number: spec.parent.number, url: "", repository: { nameWithOwner: spec.parent.repo } }
              : null,
            blockedBy: { nodes: [] },
            blocking: { nodes: [] },
          }),
    },
    fieldValues: { nodes: fieldValues },
  };
}

function gqlPage(board: BoardDef, items: ReturnType<typeof gqlItem>[]) {
  return {
    data: {
      node: {
        title: board.title,
        number: board.number,
        url: board.url,
        fields: {
          nodes: [
            { id: "F1", name: "Status", dataType: "SINGLE_SELECT", options: [{ id: "o1", name: "In Progress" }] },
            { id: "F2", name: "Area", dataType: "SINGLE_SELECT", options: [] },
            { id: "F3", name: "Horizon", dataType: "SINGLE_SELECT", options: [] },
            { id: "F4", name: "Kind", dataType: "SINGLE_SELECT", options: [] },
          ],
        },
        views: { nodes: [{ id: "V1", name: "Board", layout: "BOARD_LAYOUT", filter: null }] },
        items: {
          totalCount: items.length,
          pageInfo: { hasNextPage: false, endCursor: null },
          nodes: items,
        },
      },
    },
  };
}

// ---------------------------------------------------------------------------
// Fake seams: fetch routed by board node id, per-org token minter,
// per-board IngestDOs over memory storage — all recording their use.
// ---------------------------------------------------------------------------

interface Harness {
  deps: CollectorDeps;
  requests: { boardId: string; authorization: string | null; ifNoneMatch: string | null }[];
  mintedOrgs: string[];
  storages: Map<string, MemoryIngestStorage>;
  setHandler: (boardId: string, handler: ((ifNoneMatch: string | null) => Response) | null) => void;
  setNow: (iso: string) => void;
}

function harness(sql: SqlDb, pages: Map<string, ReturnType<typeof gqlItem>[]>): Harness {
  const requests: Harness["requests"] = [];
  const mintedOrgs: string[] = [];
  const storages = new Map<string, MemoryIngestStorage>();
  const dos = new Map<string, IngestDO>();
  const handlers = new Map<string, ((ifNoneMatch: string | null) => Response) | null>();
  let nowValue = T1;

  const boardByNode = new Map(BOARDS.map((b) => [b.nodeId, b]));
  const fetchFn = async (_input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    const variables = JSON.parse(String(init?.body)).variables as { id: string };
    const board = boardByNode.get(variables.id) as BoardDef;
    const headers = (init?.headers ?? {}) as Record<string, string>;
    const ifNoneMatch = headers["If-None-Match"] ?? null;
    requests.push({ boardId: board.id, authorization: headers["Authorization"] ?? null, ifNoneMatch });
    const handler = handlers.get(board.id);
    if (handler) return handler(ifNoneMatch);
    if (handler === null) throw new Error(`forced failure for board ${board.id}`);
    return new Response(JSON.stringify(gqlPage(board, pages.get(board.id) ?? [])), {
      status: 200,
      headers: {
        "content-type": "application/json",
        etag: `"etag-${board.id}-1"`,
        "x-ratelimit-limit": "5000",
        "x-ratelimit-remaining": "4999",
        "x-ratelimit-reset": String(RESET_FUTURE),
      },
    });
  };

  return {
    deps: {
      db: sql,
      tokens: async (org) => {
        mintedOrgs.push(org);
        return `test-token-for-${org}`;
      },
      fetchFn,
      ingestDoFor: (boardId) => {
        let entry = dos.get(boardId);
        if (!entry) {
          const storage = new MemoryIngestStorage();
          storages.set(boardId, storage);
          entry = new IngestDO(storage);
          dos.set(boardId, entry);
        }
        return entry;
      },
      now: () => nowValue,
    },
    requests,
    mintedOrgs,
    storages,
    setHandler: (boardId, handler) => handlers.set(boardId, handler),
    setNow: (iso) => {
      nowValue = iso;
    },
  };
}

const MAIN_ITEMS = [
  gqlItem({
    id: "PVTI_main_1",
    repo: "developing-today/code",
    number: 3447,
    title: "epic: id crate split",
    kind: "Task",
    status: "In Progress",
    labels: ["epic"],
  }),
  gqlItem({ id: "PVTI_main_2", repo: "developing-today/code", number: 200, title: "a pull request", pr: true }),
];
const FORKS_ITEMS = [
  gqlItem({
    id: "PVTI_forks_1",
    repo: "developing-today-forks/lab",
    number: 7,
    title: "satellite work",
    kind: "Task",
    status: "Todo",
  }),
];

function count(sql: SqlDb, query: string, params: (string | number | null)[] = []): number {
  return sql.all<{ n: number }>(query, params)[0]?.n ?? 0;
}

describe("hosted collector (#12826)", () => {
  test("the pool query is the laptop collector's query (fields, views, parent, blockedBy/blocking, labels, milestones, subIssuesSummary)", () => {
    for (const fragment of [
      "fields(first: 100)",
      "views(first: 100)",
      "parent {",
      "blockedBy(first: 50)",
      "blocking(first: 50)",
      "labels(first: 20)",
      "milestone {",
      "subIssuesSummary {",
    ]) {
      expect(PROJECT_QUERY).toContain(fragment);
    }
  });

  test("a cron run ingests every registry board into the tenant DB", async () => {
    const sql = await migratedDb();
    const h = harness(
      sql,
      new Map([
        ["devtoday-main", MAIN_ITEMS],
        ["forks-main", FORKS_ITEMS],
      ]),
    );
    const report = await runHostedCollector(h.deps);

    expect(report.landedBoardIds).toEqual(["devtoday-main", "forks-main"]);
    expect(report.failedBoardIds).toEqual([]);
    // Tokens are minted per run, per board org, through the tenancy seam.
    expect(h.mintedOrgs).toEqual(["developing-today", "developing-today-forks"]);
    expect(h.requests.map((r) => r.authorization)).toEqual([
      "Bearer test-token-for-developing-today",
      "Bearer test-token-for-developing-today-forks",
    ]);
    // Both boards' pools landed, under hosted-writer snapshot headers.
    expect(count(sql, "SELECT COUNT(*) AS n FROM pm_items WHERE board_id = 'devtoday-main'")).toBe(2);
    expect(count(sql, "SELECT COUNT(*) AS n FROM pm_items WHERE board_id = 'forks-main'")).toBe(1);
    const snapshots = sql.all<{ board_id: string; writer: string; emitted_entries: number }>(
      "SELECT board_id, writer, emitted_entries FROM pm_snapshots ORDER BY board_id",
    );
    expect(snapshots).toEqual([
      { board_id: "devtoday-main", writer: "hosted", emitted_entries: 2 },
      { board_id: "forks-main", writer: "hosted", emitted_entries: 1 },
    ]);
    // The stored pool row is the parsed pool item, byte-faithful.
    const row = sql.all<{ ref: string; status: string; item_json: string }>(
      "SELECT ref, status, item_json FROM pm_items WHERE board_id = 'devtoday-main' AND number = 3447",
    )[0];
    expect(row?.ref).toBe("developing-today/code#3447");
    expect(row?.status).toBe("In Progress");
    expect(JSON.parse(row?.item_json as string).title).toBe("epic: id crate split");
    // Cursors advanced: etag + watermark recorded per board.
    const state = await h.deps.ingestDoFor("devtoday-main").state();
    expect(state.etag).toBe('"etag-devtoday-main-1"');
    expect(state.lastSnapshotAt).toBe(T1);
    expect(state.lastOutcome).toBe("landed");
  });

  test("a forced failure on one board leaves the other boards' pools intact and records a finding", async () => {
    const sql = await migratedDb();
    const h = harness(
      sql,
      new Map([
        ["devtoday-main", MAIN_ITEMS],
        ["forks-main", FORKS_ITEMS],
      ]),
    );
    await runHostedCollector(h.deps);

    // Second run: the main board's export changed (new etag, one more
    // item); the forks board's fetch is forced to fail.
    h.setNow(T2);
    h.setHandler(
      "devtoday-main",
      () =>
        new Response(
          JSON.stringify(
            gqlPage(BOARDS[0] as BoardDef, [
              ...MAIN_ITEMS,
              gqlItem({
                id: "PVTI_main_3",
                repo: "developing-today/code",
                number: 4000,
                title: "new work",
                kind: "Task",
                status: "Todo",
              }),
            ]),
          ),
          {
            status: 200,
            headers: {
              etag: '"etag-devtoday-main-2"',
              "x-ratelimit-remaining": "4990",
              "x-ratelimit-reset": String(RESET_FUTURE),
            },
          },
        ),
    );
    h.setHandler("forks-main", null);
    const report = await runHostedCollector(h.deps);

    expect(report.landedBoardIds).toEqual(["devtoday-main"]);
    expect(report.failedBoardIds).toEqual(["forks-main"]);
    // Main pool moved on; forks pool is exactly the first run's rows.
    expect(count(sql, "SELECT COUNT(*) AS n FROM pm_items WHERE board_id = 'devtoday-main'")).toBe(3);
    expect(count(sql, "SELECT COUNT(*) AS n FROM pm_items WHERE board_id = 'forks-main'")).toBe(1);
    expect(count(sql, "SELECT COUNT(*) AS n FROM pm_snapshots WHERE board_id = 'forks-main'")).toBe(1);
    // The failure is on the findings surface, with the board named.
    const findings = sql.all<{ board_id: string; kind: string; severity: string; summary: string }>(
      "SELECT board_id, kind, severity, summary FROM pm_reconcile_findings",
    );
    expect(findings.length).toBe(1);
    expect(findings[0]?.board_id).toBe("forks-main");
    expect(findings[0]?.kind).toBe("ingest-failure");
    expect(findings[0]?.severity).toBe("error");
    expect(findings[0]?.summary).toContain("forks-main");
  });

  test("ingest cursors make a second run incremental: a 304 ends the board's run with no re-fetch and no writes", async () => {
    const sql = await migratedDb();
    const h = harness(
      sql,
      new Map([
        ["devtoday-main", MAIN_ITEMS],
        ["forks-main", FORKS_ITEMS],
      ]),
    );
    await runHostedCollector(h.deps);
    const snapshotsAfterFirst = count(sql, "SELECT COUNT(*) AS n FROM pm_snapshots");
    const itemsAfterFirst = count(sql, "SELECT COUNT(*) AS n FROM pm_items");

    h.setNow(T2);
    h.requests.length = 0;
    for (const board of BOARDS) {
      h.setHandler(board.id, (ifNoneMatch) =>
        ifNoneMatch === `"etag-${board.id}-1"`
          ? new Response(null, {
              status: 304,
              headers: { "x-ratelimit-remaining": "4995", "x-ratelimit-reset": String(RESET_FUTURE) },
            })
          : new Response(JSON.stringify(gqlPage(board, [])), { status: 200 }),
      );
    }
    const report = await runHostedCollector(h.deps);

    expect(report.boards.map((b) => b.outcome)).toEqual(["unchanged", "unchanged"]);
    // Exactly one conditional request per board — no page fetches.
    expect(h.requests.length).toBe(2);
    expect(h.requests.every((r) => r.ifNoneMatch !== null)).toBe(true);
    // Nothing was written.
    expect(count(sql, "SELECT COUNT(*) AS n FROM pm_snapshots")).toBe(snapshotsAfterFirst);
    expect(count(sql, "SELECT COUNT(*) AS n FROM pm_items")).toBe(itemsAfterFirst);
    const state = await h.deps.ingestDoFor("forks-main").state();
    expect(state.lastOutcome).toBe("unchanged");
    expect(state.lastSnapshotAt).toBe(T1);
  });

  test("no token value persists anywhere after a run (E §2.3, test-pinned)", async () => {
    const sql = await migratedDb();
    const h = harness(
      sql,
      new Map([
        ["devtoday-main", MAIN_ITEMS],
        ["forks-main", FORKS_ITEMS],
      ]),
    );
    const report = await runHostedCollector(h.deps);
    // The tokens really were used — the pin below is not vacuous.
    expect(h.requests.map((r) => r.authorization)).toContain("Bearer test-token-for-developing-today");

    const surfaces: string[] = [JSON.stringify(report)];
    const tables = sql
      .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table'")
      .map((row) => row.name);
    for (const table of tables) {
      surfaces.push(JSON.stringify(sql.all<Record<string, unknown>>(`SELECT * FROM ${table}`)));
    }
    for (const storage of h.storages.values()) surfaces.push(JSON.stringify(storage.snapshot()));
    for (const surface of surfaces) {
      expect(surface).not.toContain("test-token-for-");
    }
  });

  test("cross-board ancestry resolves in the merged pool (epic on one board, descendants on another)", async () => {
    const sql = await migratedDb();
    const epic = gqlItem({
      id: "PVTI_forks_epic",
      repo: "developing-today/code",
      number: 100,
      title: "the epic",
      kind: "Epic",
      status: "In Progress",
      subIssuesSummary: { total: 2, completed: 0 },
    });
    const mid = gqlItem({
      id: "PVTI_main_mid",
      repo: "developing-today/code",
      number: 101,
      title: "mid task",
      kind: "Task",
      status: "Todo",
      parent: { repo: "developing-today/code", number: 100 },
    });
    const leaf = gqlItem({
      id: "PVTI_main_leaf",
      repo: "developing-today/code",
      number: 102,
      title: "leaf task",
      kind: "Task",
      status: "Todo",
      parent: { repo: "developing-today/code", number: 101 },
    });
    const h = harness(
      sql,
      new Map([
        ["devtoday-main", [mid, leaf]],
        ["forks-main", [epic]],
      ]),
    );
    const report = await runHostedCollector(h.deps);
    expect(report.failedBoardIds).toEqual([]);

    // The leaf sits on the main board; its epic sits on the forks
    // board. Only the merged pool can resolve the chain.
    const rows = sql.all<{ number: number; epic_ref: string | null }>(
      "SELECT number, epic_ref FROM pm_items ORDER BY number",
    );
    expect(rows).toEqual([
      { number: 100, epic_ref: null },
      { number: 101, epic_ref: "developing-today/code#100" },
      { number: 102, epic_ref: "developing-today/code#100" },
    ]);
  });

  test("Metadata copies land for items carrying a valid §12 block; invalid blocks store nothing", async () => {
    const sql = await migratedDb();
    const block = {
      pm_meta: 1,
      direct_parent: { repo: "developing-today/code", number: 3447, kind: "epic" },
      links: [],
      lineage: { design: null, roadmap_item: null, priority: null },
      provenance: {
        queue_id: null,
        session_id: "sess-1",
        lane: "xaltha",
        host: null,
        worktree: null,
        picked_up_at: null,
        work_started_at: null,
        created_at: null,
        model: null,
        lane_start_issues: [],
      },
    };
    const body = `work\n\n<!-- pm-metadata:start -->\n\`\`\`json\n${JSON.stringify(block)}\n\`\`\`\n<!-- pm-metadata:end -->`;
    const withMeta = gqlItem({
      id: "PVTI_main_meta",
      repo: "developing-today/code",
      number: 4403,
      title: "carries metadata",
      kind: "Task",
      status: "Todo",
      body,
    });
    const withBadMeta = gqlItem({
      id: "PVTI_main_badmeta",
      repo: "developing-today/code",
      number: 4404,
      title: "carries a bad block",
      kind: "Task",
      status: "Todo",
      body: body.replace('"pm_meta":1', '"pm_meta":99'),
    });
    const h = harness(
      sql,
      new Map([
        ["devtoday-main", [withMeta, withBadMeta]],
        ["forks-main", FORKS_ITEMS],
      ]),
    );
    await runHostedCollector(h.deps);

    const rows = sql.all<{ ref: string; direct_parent_ref: string; provenance_session_id: string }>(
      "SELECT ref, direct_parent_ref, provenance_session_id FROM pm_item_metadata",
    );
    expect(rows).toEqual([
      {
        ref: "developing-today/code#4403",
        direct_parent_ref: "developing-today/code#3447",
        provenance_session_id: "sess-1",
      },
    ]);
  });

  test("a board whose rate budget is spent defers without a fetch and records a finding", async () => {
    const sql = await migratedDb();
    const h = harness(
      sql,
      new Map([
        ["devtoday-main", MAIN_ITEMS],
        ["forks-main", FORKS_ITEMS],
      ]),
    );
    const storage = new MemoryIngestStorage();
    const spent: IngestState = {
      etag: null,
      lastSnapshotAt: null,
      lastSnapshotId: null,
      rate: { limit: 5000, remaining: 0, resetAt: RESET_FUTURE },
      lastOutcome: null,
      lastRunAt: null,
    };
    await storage.put("ingest-state", spent);
    h.storages.set("forks-main", storage);
    const forksDo = new IngestDO(storage);
    const baseFor = h.deps.ingestDoFor;
    h.deps.ingestDoFor = (boardId) => (boardId === "forks-main" ? forksDo : baseFor(boardId));

    const report = await runHostedCollector(h.deps);
    expect(report.boards.find((b) => b.boardId === "forks-main")?.outcome).toBe("deferred");
    expect(report.landedBoardIds).toEqual(["devtoday-main"]);
    expect(h.requests.map((r) => r.boardId)).toEqual(["devtoday-main"]);
    const findings = sql.all<{ kind: string; severity: string }>("SELECT kind, severity FROM pm_reconcile_findings");
    expect(findings).toEqual([{ kind: "ingest-deferred", severity: "warning" }]);
  });

  test("IngestDO serializes runs for its board: a queued run sees the previous run's cursors", async () => {
    const storage = new MemoryIngestStorage();
    const ingestDo = new IngestDO(storage);
    const order: string[] = [];
    const first = ingestDo.runSerialized(async (state) => {
      order.push(`first:${state.etag}`);
      await new Promise((resolve) => setTimeout(resolve, 20));
      return { result: "one", next: { ...state, etag: '"e2"' } };
    });
    const second = ingestDo.runSerialized(async (state) => {
      order.push(`second:${state.etag}`);
      return { result: "two", next: state };
    });
    expect(await Promise.all([first, second])).toEqual(["one", "two"]);
    expect(order).toEqual(["first:null", 'second:"e2"']);
  });
});
