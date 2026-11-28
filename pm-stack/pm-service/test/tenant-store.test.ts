/**
 * PM tenant store conformance suite (#12825).
 *
 * Oracle: the merged component A design (§3 data homes, §5 lease),
 * #9060's design shapes (§1 BoardDef, §4 milestones, §11 link
 * records, §12 Metadata), and the laptop collector's landed output
 * types (pm-stack/aggregator/src/model.ts ProjectPoolItem,
 * board-registry.ts PROJECT_BOARDS). Fixtures below are those shapes;
 * the suite fails if the schema or the validators drift from them.
 */
import { describe, expect, test } from "bun:test";
import { tenantStoreMigrations } from "../src/tenant-store/load.ts";
import { applyMigrations, currentSchemaVersion, type Migration } from "../src/tenant-store/migrate.ts";
import {
  validateBoardDef,
  validateLinkRecord,
  validateMetadataBlock,
  validateWriterLease,
  type BoardDef,
} from "../src/tenant-store/schema.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";

const NOW = "2026-11-23T06:00:00.000Z";
const now = () => NOW;

async function migratedDb() {
  const { db, sql } = openTenantDb();
  const migrations = await tenantStoreMigrations();
  const result = applyMigrations(sql, migrations, now);
  return { db, sql, migrations, result };
}

/** The laptop collector's landed registry (board-registry.ts PROJECT_BOARDS). */
const LANDED_BOARDS: BoardDef[] = [
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
  {
    id: "lab-cluster",
    org: "Clustered-Computing",
    number: 1,
    nodeId: "PVT_kwDOAlO1HM4BrYAm",
    url: "https://github.com/orgs/Clustered-Computing/projects/1",
    role: "project",
    parentBoardId: "devtoday-main",
    siblingOrder: 2,
    title: "Lab Cluster",
  },
];

function insertBoard(sql: ReturnType<typeof openTenantDb>["sql"], board: BoardDef) {
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
      NOW,
    ],
  );
}

describe("migrations", () => {
  test("apply to an empty tenant DB, record the schema version, and are idempotent", async () => {
    const { sql, result } = await migratedDb();
    expect(result.applied).toEqual([1, 2]);
    expect(currentSchemaVersion(sql)).toEqual([1, 2]);
    const rows = sql.all<{ version: number; name: string; applied_at: string }>(
      "SELECT version, name, applied_at FROM pm_schema_migrations",
    );
    expect(rows).toEqual([
      { version: 1, name: "pm_tenant_store", applied_at: NOW },
      { version: 2, name: "pm_search_index", applied_at: NOW },
    ]);

    const migrations = await tenantStoreMigrations();
    expect(applyMigrations(sql, migrations, now).applied).toEqual([]);
    expect(currentSchemaVersion(sql)).toEqual([1, 2]);
  });

  test("create the full PM store schema (design §3 + §5)", async () => {
    const { sql } = await migratedDb();
    const tables = sql
      .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
      .map((row) => row.name);
    expect(tables).toEqual([
      "pm_boards",
      "pm_item_metadata",
      "pm_items",
      "pm_items_fts",
      "pm_items_fts_config",
      "pm_items_fts_data",
      "pm_items_fts_docsize",
      "pm_items_fts_idx",
      "pm_links",
      "pm_milestones",
      "pm_reconcile_findings",
      "pm_release_snapshots",
      "pm_roadmap_rows",
      "pm_schema_migrations",
      "pm_snapshots",
      "pm_writer_leases",
    ]);
  });

  test("fail loudly when a recorded migration's name drifts from the file on disk", async () => {
    const { sql, migrations } = await migratedDb();
    sql.run("UPDATE pm_schema_migrations SET name = 'renamed_elsewhere' WHERE version = 1");
    expect(() => applyMigrations(sql, migrations, now)).toThrow(/migration drift/);
  });

  test("no table holds a token, key, secret, password, or credential column (E §2.3)", async () => {
    const { sql } = await migratedDb();
    const tables = sql
      .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table'")
      .map((row) => row.name);
    const offenders: string[] = [];
    for (const table of tables) {
      for (const column of sql.all<{ name: string }>(`PRAGMA table_info(${table})`)) {
        if (/(token|secret|password|credential|key)/i.test(column.name)) offenders.push(`${table}.${column.name}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  test("the shipped migration files load in order with parsed versions", async () => {
    const migrations: Migration[] = await tenantStoreMigrations();
    expect(migrations.map((m) => [m.version, m.name])).toEqual([
      [1, "pm_tenant_store"],
      [2, "pm_search_index"],
    ]);
    expect(migrations[0]?.sql).toContain("CREATE TABLE pm_writer_leases");
    expect(migrations[1]?.sql).toContain("CREATE VIRTUAL TABLE pm_items_fts");
  });
});

describe("board registry (#9060 §1)", () => {
  test("the collector's landed registry validates and round-trips as rows", async () => {
    const { sql } = await migratedDb();
    for (const board of LANDED_BOARDS) {
      expect(validateBoardDef(board)).toEqual([]);
      insertBoard(sql, board);
    }
    const rows = sql.all<{ board_id: string; role: string; parent_board_id: string | null }>(
      "SELECT board_id, role, parent_board_id FROM pm_boards ORDER BY sibling_order",
    );
    expect(rows).toEqual([
      { board_id: "devtoday-main", role: "overall", parent_board_id: null },
      { board_id: "forks-main", role: "project", parent_board_id: "devtoday-main" },
      { board_id: "lab-cluster", role: "project", parent_board_id: "devtoday-main" },
    ]);
  });

  test("role is a closed vocabulary and (org, number) is unique", async () => {
    const { sql } = await migratedDb();
    insertBoard(sql, LANDED_BOARDS[0] as BoardDef);
    expect(() =>
      insertBoard(sql, { ...(LANDED_BOARDS[0] as BoardDef), id: "other", role: "bogus" as BoardDef["role"] }),
    ).toThrow();
    expect(() => insertBoard(sql, { ...(LANDED_BOARDS[0] as BoardDef), id: "dupe" })).toThrow();
    expect(validateBoardDef({ ...(LANDED_BOARDS[0] as BoardDef), role: "bogus" }).length).toBeGreaterThan(0);
  });
});

/** A collector ProjectPoolItem (aggregator model.ts), as emitted for the crate-split epic. */
const POOL_ITEM = {
  boardId: "devtoday-main",
  id: "PVTI_lADOAZNvF84Bouayzg12345",
  databaseId: 12345,
  url: "https://github.com/orgs/developing-today/projects/4/views/1?pane=issue&itemId=12345",
  projectUrl: "https://github.com/orgs/developing-today/projects/4",
  ref: "developing-today/code#3447",
  repo: "developing-today/code",
  number: 3447,
  issueUrl: "https://github.com/developing-today/code/issues/3447",
  title: "epic: id crate split",
  state: "open",
  contentKind: "issue",
  labels: ["epic", "epic/id-crate-split"],
  author: "dezren39",
  updatedAt: "2026-11-12T10:00:00.000Z",
  milestone: { title: "ID platform — release 2026-12-01", dueOn: "2026-12-01", closedAt: null },
  subIssuesSummary: { total: 9, completed: 4 },
  status: "In Progress",
  area: "ID",
  horizon: "Now",
  kind: "Epic",
  parentRef: "developing-today/code#3195",
  epicRef: "developing-today/code#2056",
  blockedBy: [],
  blocks: ["developing-today/code#4403"],
};

describe("merged item pool (collector ProjectPoolItem shape)", () => {
  test("a snapshot header + pool item store byte-faithful JSON with extracted keys", async () => {
    const { sql } = await migratedDb();
    insertBoard(sql, LANDED_BOARDS[0] as BoardDef);
    sql.run(
      `INSERT INTO pm_snapshots (snapshot_id, board_id, generated_at, reported_total, emitted_entries, writer, epoch, audit_json, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      ["snap-1", "devtoday-main", NOW, 1229, 1200, "laptop", 1, "{}", NOW],
    );
    sql.run(
      `INSERT INTO pm_items (board_id, item_id, snapshot_id, ref, repo, number, content_kind, state, draft, title,
         url, project_url, issue_url, database_id, author, labels_json, status, area, horizon, kind,
         milestone_title, milestone_due_on, milestone_closed_at, sub_issues_total, sub_issues_completed,
         parent_ref, epic_ref, blocked_by_json, blocks_json, item_updated_at, item_json, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        POOL_ITEM.boardId,
        POOL_ITEM.id,
        "snap-1",
        POOL_ITEM.ref,
        POOL_ITEM.repo,
        POOL_ITEM.number,
        POOL_ITEM.contentKind,
        POOL_ITEM.state,
        0,
        POOL_ITEM.title,
        POOL_ITEM.url,
        POOL_ITEM.projectUrl,
        POOL_ITEM.issueUrl,
        POOL_ITEM.databaseId,
        POOL_ITEM.author,
        JSON.stringify(POOL_ITEM.labels),
        POOL_ITEM.status,
        POOL_ITEM.area,
        POOL_ITEM.horizon,
        POOL_ITEM.kind,
        POOL_ITEM.milestone.title,
        POOL_ITEM.milestone.dueOn,
        POOL_ITEM.milestone.closedAt,
        POOL_ITEM.subIssuesSummary.total,
        POOL_ITEM.subIssuesSummary.completed,
        POOL_ITEM.parentRef,
        POOL_ITEM.epicRef,
        JSON.stringify(POOL_ITEM.blockedBy),
        JSON.stringify(POOL_ITEM.blocks),
        POOL_ITEM.updatedAt,
        JSON.stringify(POOL_ITEM),
        NOW,
      ],
    );
    const row = sql.all<{ ref: string; status: string; kind: string; epic_ref: string; item_json: string }>(
      "SELECT ref, status, kind, epic_ref, item_json FROM pm_items WHERE board_id = 'devtoday-main'",
    )[0];
    expect(row?.ref).toBe("developing-today/code#3447");
    expect(row?.status).toBe("In Progress");
    expect(row?.kind).toBe("Epic");
    expect(row?.epic_ref).toBe("developing-today/code#2056");
    expect(JSON.parse(row?.item_json as string)).toEqual(POOL_ITEM);
  });

  test("pool rows require a known board (foreign keys enforced)", async () => {
    const { sql } = await migratedDb();
    expect(() =>
      sql.run(
        `INSERT INTO pm_snapshots (snapshot_id, board_id, generated_at, reported_total, emitted_entries, writer, epoch, audit_json, recorded_at)
         VALUES ('snap-x', 'no-such-board', ?, 0, 0, 'laptop', 0, '{}', ?)`,
        [NOW, NOW],
      ),
    ).toThrow();
  });
});

describe("link records (#9060 §11)", () => {
  const insertLink = (sql: ReturnType<typeof openTenantDb>["sql"], row: (string | number | null)[]) =>
    sql.run(
      `INSERT INTO pm_links (link_id, source_kind, source_ref, target_kind, target_ref, target_repo, target_number,
         relation, status, since, until, reason, provenance_json, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      row,
    );

  test("active, historical, removed, and board-parent records validate and store", async () => {
    const { sql } = await migratedDb();
    const records = [
      {
        sourceKind: "item",
        sourceRef: "developing-today/code#4403",
        target: { repo: "developing-today/code", number: 3447 },
        relation: "parent",
        status: "active",
        since: "2026-11-02",
      },
      {
        sourceKind: "item",
        sourceRef: "developing-today/code#4403",
        target: { repo: "developing-today/code", number: 123 },
        relation: "parent",
        status: "historical",
        since: "2026-11-02",
        until: "2026-11-12",
        reason: "split",
      },
      {
        sourceKind: "item",
        sourceRef: "developing-today/code#4403",
        target: { repo: "developing-today/code", number: 999 },
        relation: "parent",
        status: "removed",
        since: "2026-11-02",
        reason: "misclassification",
      },
      {
        sourceKind: "board",
        sourceRef: "forks-main",
        target: { boardId: "devtoday-main" },
        relation: "board-parent",
        status: "active",
      },
    ] as const;
    for (const [index, record] of records.entries()) {
      expect(validateLinkRecord(record)).toEqual([]);
      const target = record.target;
      insertLink(sql, [
        `link-${index}`,
        record.sourceKind,
        record.sourceRef,
        "boardId" in target ? "board" : "item",
        "boardId" in target ? target.boardId : `${target.repo}#${target.number}`,
        "boardId" in target ? null : target.repo,
        "boardId" in target ? null : target.number,
        record.relation,
        record.status,
        "since" in record ? record.since : null,
        "until" in record ? record.until : null,
        "reason" in record ? record.reason : null,
        "{}",
        NOW,
      ]);
    }
    expect(sql.all<{ n: number }>("SELECT COUNT(*) AS n FROM pm_links")[0]?.n).toBe(4);
    const historical = sql.all<{ n: number }>(
      "SELECT COUNT(*) AS n FROM pm_links WHERE status = 'historical' AND until IS NOT NULL AND reason = 'split'",
    )[0]?.n;
    expect(historical).toBe(1);
  });

  test("historical without until/reason and removed without reason are rejected by validator and schema", async () => {
    const { sql } = await migratedDb();
    const bad = {
      sourceKind: "item",
      sourceRef: "developing-today/code#4403",
      target: { repo: "developing-today/code", number: 123 },
      relation: "parent",
      status: "historical",
      since: "2026-11-02",
    } as const;
    expect(validateLinkRecord(bad).length).toBeGreaterThan(0);
    expect(() =>
      insertLink(sql, [
        "bad-1",
        "item",
        bad.sourceRef,
        "item",
        "developing-today/code#123",
        "developing-today/code",
        123,
        "parent",
        "historical",
        "2026-11-02",
        null,
        null,
        "{}",
        NOW,
      ]),
    ).toThrow();
    expect(() =>
      insertLink(sql, [
        "bad-2",
        "item",
        bad.sourceRef,
        "item",
        "developing-today/code#123",
        "developing-today/code",
        123,
        "parent",
        "removed",
        "2026-11-02",
        null,
        null,
        "{}",
        NOW,
      ]),
    ).toThrow();
  });
});

/** The #9060 §12 worked example block, verbatim shape. */
const METADATA_BLOCK = {
  pm_meta: 1,
  direct_parent: { repo: "developing-today/code", number: 3447, kind: "epic" },
  links: [
    {
      target: { repo: "developing-today/code", number: 123 },
      relation: "parent",
      status: "historical",
      since: "2026-11-02",
      until: "2026-11-12",
      reason: "split",
    },
  ],
  lineage: { design: null, roadmap_item: null, priority: null },
  provenance: {
    queue_id: null,
    session_id: null,
    lane: null,
    host: null,
    worktree: null,
    picked_up_at: null,
    work_started_at: null,
    created_at: null,
    model: null,
    lane_start_issues: [],
  },
};

describe("Metadata copies (#9060 §12)", () => {
  test("the §12 example block validates, stores byte-faithful, and extracts its direct parent", async () => {
    const { sql } = await migratedDb();
    expect(validateMetadataBlock(METADATA_BLOCK)).toEqual([]);
    expect(validateMetadataBlock({ ...METADATA_BLOCK, pm_meta: 2 }).length).toBeGreaterThan(0);
    sql.run(
      `INSERT INTO pm_item_metadata (ref, repo, number, kind, pm_meta, direct_parent_ref, direct_parent_kind,
         lineage_design_ref, lineage_roadmap_item_ref, lineage_priority_ref,
         provenance_session_id, provenance_lane, provenance_queue_id, provenance_json, metadata_json, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        "developing-today/code#4403",
        "developing-today/code",
        4403,
        "issue",
        1,
        "developing-today/code#3447",
        "epic",
        null,
        null,
        null,
        null,
        null,
        null,
        JSON.stringify(METADATA_BLOCK.provenance),
        JSON.stringify(METADATA_BLOCK),
        NOW,
      ],
    );
    const row = sql.all<{ direct_parent_ref: string; metadata_json: string }>(
      "SELECT direct_parent_ref, metadata_json FROM pm_item_metadata",
    )[0];
    expect(row?.direct_parent_ref).toBe("developing-today/code#3447");
    expect(JSON.parse(row?.metadata_json as string)).toEqual(METADATA_BLOCK);
  });
});

describe("writer-of-record lease (component A design §5)", () => {
  test("one lease row per board; writer and epoch are fenced by the schema", async () => {
    const { sql } = await migratedDb();
    insertBoard(sql, LANDED_BOARDS[0] as BoardDef);
    const lease = { boardId: "devtoday-main", writer: "laptop", epoch: 1, heartbeatAt: NOW } as const;
    expect(validateWriterLease(lease)).toEqual([]);
    expect(validateWriterLease({ ...lease, writer: "third-party" }).length).toBeGreaterThan(0);
    sql.run("INSERT INTO pm_writer_leases (board_id, writer, epoch, heartbeat_at, updated_at) VALUES (?, ?, ?, ?, ?)", [
      lease.boardId,
      lease.writer,
      lease.epoch,
      lease.heartbeatAt,
      NOW,
    ]);
    expect(() =>
      sql.run(
        "INSERT INTO pm_writer_leases (board_id, writer, epoch, heartbeat_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        ["devtoday-main", "hosted", 2, NOW, NOW],
      ),
    ).toThrow(); // second row for the same board: flips update the row and bump the epoch
    sql.run("UPDATE pm_writer_leases SET writer = 'hosted', epoch = epoch + 1, heartbeat_at = ? WHERE board_id = ?", [
      NOW,
      "devtoday-main",
    ]);
    const row = sql.all<{ writer: string; epoch: number }>("SELECT writer, epoch FROM pm_writer_leases")[0];
    expect(row).toEqual({ writer: "hosted", epoch: 2 });
  });
});

describe("milestones + release snapshots (#9060 §4)", () => {
  test("program milestones carry no date; release milestones require one; close-out accounting is consistent", async () => {
    const { sql } = await migratedDb();
    sql.run(
      `INSERT INTO pm_milestones (repo, number, title, kind, due_on, closed_at, state, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ["developing-today/code", 3, "ID program (ROLLING)", "program", null, null, "open", NOW],
    );
    expect(() =>
      sql.run(
        `INSERT INTO pm_milestones (repo, number, title, kind, due_on, closed_at, state, recorded_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
        ["developing-today/code", 16, "ID platform — release 2026-12-01", "release", null, null, "open", NOW],
      ),
    ).toThrow();
    sql.run(
      `INSERT INTO pm_milestones (repo, number, title, kind, due_on, closed_at, state, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        "developing-today/code",
        16,
        "ID platform — release 2026-12-01",
        "release",
        "2026-12-01",
        "2026-12-01T18:00:00.000Z",
        "closed",
        NOW,
      ],
    );
    // §4's worked example: "closed with the release covering 123 issues, 23 designs, 4 epics".
    sql.run(
      `INSERT INTO pm_release_snapshots (repo, milestone_number, release_date,
         issues_covered, issues_closed, issues_carried_over,
         designs_covered, designs_closed, designs_carried_over,
         epics_covered, epics_closed, epics_carried_over,
         carried_over_epics_json, computed_at, snapshot_json)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        "developing-today/code",
        16,
        "2026-12-01",
        123,
        100,
        23,
        23,
        20,
        3,
        4,
        3,
        1,
        JSON.stringify(["developing-today/code#2056"]),
        NOW,
        "{}",
      ],
    );
    expect(() =>
      sql.run(
        `INSERT INTO pm_release_snapshots (repo, milestone_number, release_date,
           issues_covered, issues_closed, issues_carried_over,
           designs_covered, designs_closed, designs_carried_over,
           epics_covered, epics_closed, epics_carried_over,
           carried_over_epics_json, computed_at, snapshot_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
        ["developing-today/code", 3, "2026-12-01", 10, 11, 0, 0, 0, 0, 0, 0, 0, "[]", NOW, "{}"],
      ),
    ).toThrow(); // closed (11) cannot exceed covered (10)
  });
});

describe("roadmap rows + reconcile findings", () => {
  test("a roadmap row stores its genealogy overlay; findings fence severity", async () => {
    const { sql } = await migratedDb();
    insertBoard(sql, LANDED_BOARDS[0] as BoardDef);
    sql.run(
      `INSERT INTO pm_roadmap_rows (row_id, subject, title, area, status, horizon, source, notes,
         links_json, priority_ref, after_refs_json, release_ref, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        28,
        "grist:doc1/Roadmap/28",
        "ID platform v1",
        "ID",
        "In Progress",
        "Now",
        "grist",
        "",
        "[]",
        "developing-today/code#2056",
        JSON.stringify(["developing-today/code#3195"]),
        "developing-today/code#2056",
        NOW,
      ],
    );
    const row = sql.all<{ priority_ref: string; after_refs_json: string }>(
      "SELECT priority_ref, after_refs_json FROM pm_roadmap_rows",
    )[0];
    expect(row?.priority_ref).toBe("developing-today/code#2056");
    expect(JSON.parse(row?.after_refs_json as string)).toEqual(["developing-today/code#3195"]);

    sql.run(
      `INSERT INTO pm_reconcile_findings (finding_id, board_id, kind, severity, subject_ref, summary, detail_json, writer, epoch, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        "f-1",
        "devtoday-main",
        "non-record-writer",
        "error",
        "developing-today/code#4403",
        "write arrived from a non-record writer",
        "{}",
        "laptop",
        2,
        NOW,
      ],
    );
    expect(() =>
      sql.run(
        `INSERT INTO pm_reconcile_findings (finding_id, board_id, kind, severity, summary, recorded_at)
         VALUES ('f-2', 'devtoday-main', 'drift', 'fatal', 'x', ?)`,
        [NOW],
      ),
    ).toThrow();
  });
});
