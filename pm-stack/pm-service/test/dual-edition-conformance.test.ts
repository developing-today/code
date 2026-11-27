/**
 * Dual-edition read-contract conformance (#12833).
 *
 * Oracle: #9079 invariant 1 and #9060 §§4, 6, 8, 12 — the same board
 * registry, pool snapshot, roadmap row, milestone and Metadata block must
 * produce identical shared API projections in the laptop aggregator and
 * hosted read API. The #9060 §8 crate-split chain is the worked fixture;
 * #9074's versioned Metadata DB is the shape oracle for its persisted copy.
 * Edition-specific response envelope fields are outside this shared
 * projection; the domain objects served by both editions must match.
 */
import { Database } from "bun:sqlite";
import { deepStrictEqual } from "node:assert/strict";
import { describe, expect, test } from "bun:test";
import {
  PROJECT_BOARDS,
  historicalParentsFromMetadata as aggregatorHistoricalParents,
  resolveEpicRefs,
} from "../../aggregator/src/projects.ts";
import { deriveGenealogy, enrichRoadmapRows } from "../../aggregator/src/genealogy.ts";
import { buildReleaseSnapshots } from "../../aggregator/src/releases.ts";
import {
  createPmMetadataStore,
  getPmMetadata,
  parsePmMetadata,
  upsertPmMetadata,
} from "../../aggregator/src/metadata-store.ts";
import type {
  ProjectAudit,
  ProjectPoolItem as AggregatorPoolItem,
  RoadmapRow as AggregatorRoadmapRow,
} from "../../aggregator/src/model.ts";
import { itemJson } from "../src/read-api/api.ts";
import { loadBoardSummaries, loadPool, loadReleaseSnapshots, loadRoadmapRows } from "../src/read-api/store.ts";
import { historicalParentsFromMetadata } from "../src/collector/pool.ts";
import { landSnapshot } from "../src/collector/store.ts";
import { tenantStoreMigrations } from "../src/tenant-store/load.ts";
import { applyMigrations, type SqlDb } from "../src/tenant-store/migrate.ts";
import { validateMetadataBlock, type BoardDef, type PmMetadataBlock } from "../src/tenant-store/schema.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";

const REPO = "developing-today/code";
const ref = (number: number) => `${REPO}#${number}`;
const SNAPSHOT_AT = "2026-11-23T06:00:00.000Z";
const GENERATED_AT = "2026-11-23T05:59:00.000Z";
const NOW = "2026-11-23T07:00:00.000Z";

const METADATA = {
  pm_meta: 1,
  direct_parent: { repo: REPO, number: 3447, kind: "Epic" },
  links: [
    { target: { repo: REPO, number: 3447 }, relation: "parent", status: "active" },
    {
      target: { repo: REPO, number: 1999 },
      relation: "parent",
      status: "historical",
      until: "2026-11-20",
      reason: "split",
    },
  ],
  lineage: {
    design: ref(3195),
    roadmap_item: ref(2056),
    priority: ref(2056),
  },
  provenance: {
    session_id: "ses_conformance_fixture",
    lane: "agent-next-23-local",
    host: "fleet-runner",
    queue_id: "fixture-9074",
    source_note: "preserved as an unindexed extension field",
  },
  extras: { retained: true },
} satisfies PmMetadataBlock & { extras: { retained: boolean } };

const METADATA_BODY = `Fixture body\n\n<!-- pm-metadata:start -->\n\`\`\`json\n${JSON.stringify(METADATA)}\n\`\`\`\n<!-- pm-metadata:end -->`;

function poolItem(partial: Partial<AggregatorPoolItem> & { number: number; title: string }): AggregatorPoolItem {
  const url = `https://github.com/${REPO}/issues/${partial.number}`;
  return {
    boardId: "devtoday-main",
    id: `PVTI_${partial.number}`,
    databaseId: partial.number * 10,
    url,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: ref(partial.number),
    repo: REPO,
    number: partial.number,
    issueUrl: url,
    title: partial.title,
    state: "open",
    contentKind: "issue",
    labels: [],
    author: "fixture",
    updatedAt: SNAPSHOT_AT,
    status: "Todo",
    area: "ID",
    horizon: null,
    kind: null,
    ...partial,
    ref: ref(partial.number),
  };
}

// #9060 §8: #4403 → #3447 → #3195 → #2056. The extra predecessor
// #1999 and milestone exercise afterRefs, release, and historical metadata.
const STEP = poolItem({
  number: 4403,
  title: "crate-split step: extract the id-events crate",
  kind: "Task",
  status: "In Progress",
  labels: ["primary-parent/id-crate-split"],
  parentRef: ref(3447),
  historicalParentRefs: [ref(1999)],
  milestone: { title: "ID platform — release 2026-12-01", dueOn: "2026-12-01", closedAt: null },
});
const EPIC = poolItem({
  number: 3447,
  title: "epic: ID crate split",
  kind: "Epic",
  labels: ["epic", "epic/id-crate-split", "design/id-crate-split"],
  parentRef: ref(3195),
});
const DESIGN = poolItem({
  number: 3195,
  title: "design: ID crate split",
  labels: ["kind/design", "design/id-crate-split"],
  parentRef: ref(2056),
});
const ROADMAP_ITEM = poolItem({
  number: 2056,
  title: "ID platform roadmap",
  labels: ["kind/roadmap-item"],
  milestone: { title: "ID platform — release 2026-12-01", dueOn: "2026-12-01", closedAt: null },
  blockedBy: [ref(1999)],
});
const PREDECESSOR = poolItem({
  number: 1999,
  title: "prior ID milestone",
  state: "closed",
  status: "Done",
  kind: "Task",
});
const POOL = resolveEpicRefs([STEP, EPIC, DESIGN, ROADMAP_ITEM, PREDECESSOR]);
const STEP_IN_POOL = POOL.find((item) => item.ref === STEP.ref) as AggregatorPoolItem;

const AUDIT: ProjectAudit = {
  title: "Developing Today",
  number: 4,
  url: "https://github.com/orgs/developing-today/projects/4",
  generatedAt: GENERATED_AT,
  totalItems: POOL.length,
  fields: [{ id: "PVTF_status", name: "Status", dataType: "SINGLE_SELECT" }],
  views: [{ id: "PVTV_1", name: "ID Program", layout: "TABLE_LAYOUT", filter: null }],
  counts: { status: { Todo: 3, "In Progress": 1, Done: 1 }, area: { ID: 5 }, repo: { [REPO]: 5 } },
};

const ROADMAP_ROW: AggregatorRoadmapRow = {
  id: 1,
  subject: "grist:docA/tblA/1",
  title: "ID crate split",
  area: "ID",
  status: "Active",
  horizon: "",
  source: "grist",
  notes: "",
  links: [{ ref: ref(3447), relation: "tracks" }],
};

async function seededDb(): Promise<SqlDb> {
  const { sql } = openTenantDb();
  applyMigrations(sql, await tenantStoreMigrations(), () => SNAPSHOT_AT);
  for (const board of PROJECT_BOARDS) {
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
        SNAPSHOT_AT,
      ],
    );
  }
  landSnapshot(sql, {
    snapshotId: "shared-fixture-1",
    boardId: "devtoday-main",
    generatedAt: GENERATED_AT,
    audit: AUDIT,
    counts: { reportedTotal: POOL.length, emittedEntries: POOL.length },
    items: POOL,
    bodies: new Map([[STEP.ref, METADATA_BODY]]),
    now: SNAPSHOT_AT,
  });
  sql.run(
    `INSERT INTO pm_roadmap_rows (row_id, subject, title, area, status, horizon, horizon_error, source, notes,
       links_json, priority_ref, after_refs_json, release_ref, grist_url, file_url, recorded_at)
     VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, NULL, '[]', NULL, NULL, NULL, ?)`,
    [
      ROADMAP_ROW.id,
      ROADMAP_ROW.subject,
      ROADMAP_ROW.title,
      ROADMAP_ROW.area,
      ROADMAP_ROW.status,
      ROADMAP_ROW.horizon,
      ROADMAP_ROW.source,
      ROADMAP_ROW.notes,
      JSON.stringify(ROADMAP_ROW.links),
      SNAPSHOT_AT,
    ],
  );
  return sql;
}

describe("dual-edition PM read-contract projections", () => {
  test("the board registry and Project pool produce identical JSON", async () => {
    const db = await seededDb();
    const hostedProjects = loadBoardSummaries(db);
    const hostedRegistry = hostedProjects.map((entry) => entry.board);
    const hostedItems = loadPool(db);
    const registry: readonly BoardDef[] = PROJECT_BOARDS;

    deepStrictEqual(hostedRegistry, registry);
    deepStrictEqual(
      hostedItems,
      [...POOL].sort((a, b) => a.ref.localeCompare(b.ref)),
    );
  });

  test("the same #9060 §8 item produces identical genealogy JSON", async () => {
    const db = await seededDb();
    const hosted = itemJson({ db, now: () => NOW }, STEP.ref);
    const genealogy = deriveGenealogy(STEP_IN_POOL, POOL);
    const laptopProjection = {
      item: STEP_IN_POOL,
      ancestors: genealogy.ancestors,
      release: genealogy.release,
      genealogySentence: genealogy.sentence,
    };

    expect(hosted.status).toBe(200);
    deepStrictEqual(hosted.body, laptopProjection);
    expect((hosted.body as { genealogySentence: string }).genealogySentence).toContain("Formerly part of");
  });

  test("the same roadmap row and pool produce identical overlay JSON", async () => {
    const db = await seededDb();
    const hosted = loadRoadmapRows(db, loadPool(db));
    const laptop = enrichRoadmapRows([ROADMAP_ROW], POOL);
    deepStrictEqual(hosted, laptop);
  });

  test("the same milestone snapshot produces identical release JSON", async () => {
    const db = await seededDb();
    const hosted = loadReleaseSnapshots(db, loadPool(db));
    const laptop = buildReleaseSnapshots(POOL);
    deepStrictEqual(hosted, laptop);
  });

  test("the same §12 Metadata fixture pins §9074 storage and extracted fields", async () => {
    const db = await seededDb();
    const parsedFleetMetadata = parsePmMetadata(METADATA);
    expect(parsedFleetMetadata.ok).toBe(true);
    expect(validateMetadataBlock(METADATA)).toEqual([]);
    deepStrictEqual(aggregatorHistoricalParents(METADATA_BODY), historicalParentsFromMetadata(METADATA_BODY));

    if (!parsedFleetMetadata.ok) throw new Error(parsedFleetMetadata.error);
    const metadataDb = new Database(":memory:");
    createPmMetadataStore(metadataDb);
    upsertPmMetadata(metadataDb, {
      artifactId: `issue:${STEP.ref}`,
      repo: STEP.repo,
      itemNumber: STEP.number,
      itemKind: STEP.contentKind,
      sourceUrl: STEP.issueUrl,
      parsed: parsedFleetMetadata.value,
      sourceUpdatedAt: STEP.updatedAt,
    });
    const fleetRow = getPmMetadata(metadataDb, `issue:${STEP.ref}`);
    const hostedRow = db.all<{
      pm_meta: number;
      direct_parent_ref: string | null;
      direct_parent_kind: string | null;
      lineage_design_ref: string | null;
      lineage_roadmap_item_ref: string | null;
      lineage_priority_ref: string | null;
      provenance_session_id: string | null;
      provenance_lane: string | null;
      provenance_queue_id: string | null;
      metadata_json: string;
    }>(
      "SELECT pm_meta, direct_parent_ref, direct_parent_kind, lineage_design_ref, lineage_roadmap_item_ref, lineage_priority_ref, provenance_session_id, provenance_lane, provenance_queue_id, metadata_json FROM pm_item_metadata WHERE ref = ?",
      [STEP.ref],
    )[0];

    expect(fleetRow).not.toBeNull();
    expect(hostedRow).toBeDefined();
    expect(hostedRow?.pm_meta).toBe(fleetRow?.schemaVersion);
    expect(hostedRow?.metadata_json).toBe(JSON.stringify(fleetRow?.metadata));
    expect(hostedRow?.direct_parent_ref).toBe(
      `${fleetRow?.indexes.directParentRepo}#${fleetRow?.indexes.directParentNumber}`,
    );
    expect(hostedRow?.direct_parent_kind).toBe(METADATA.direct_parent?.kind);
    expect(hostedRow?.lineage_design_ref).toBe(METADATA.lineage.design);
    expect(hostedRow?.lineage_roadmap_item_ref).toBe(METADATA.lineage.roadmap_item);
    expect(hostedRow?.lineage_priority_ref).toBe(METADATA.lineage.priority);
    expect(hostedRow?.provenance_session_id).toBe(fleetRow?.indexes.sessionId);
    expect(hostedRow?.provenance_lane).toBe(fleetRow?.indexes.lane);
    expect(hostedRow?.provenance_queue_id).toBe(fleetRow?.indexes.queueId);
    expect(fleetRow?.extras).toEqual({
      extras: { retained: true },
      "provenance.source_note": "preserved as an unindexed extension field",
    });
    expect(fleetRow?.indexes.links).toEqual(
      METADATA.links.map((link) => ({
        targetRepo: link.target.repo,
        targetNumber: link.target.number,
        relation: link.relation,
        status: link.status,
      })),
    );
  });

  test("the strict JSON comparator rejects a one-edition shape change", () => {
    const laptop = { item: { ref: STEP.ref, title: STEP.title } };
    const hosted = { item: { ref: STEP.ref, title: STEP.title }, unexpected: true };
    expect(() => deepStrictEqual(laptop, hosted)).toThrow();
  });
});
