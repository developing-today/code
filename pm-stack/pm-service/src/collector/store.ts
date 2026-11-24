/**
 * The collector's tenant-DB surface: read the board registry, land a
 * board snapshot (header + merged pool rows), record ingest
 * findings, and upsert Metadata copies — all against slice 1's
 * SqlDb surface and schema (pm-stack/pm-service/src/tenant-store).
 *
 * The pool is "current state per board": landing a snapshot replaces
 * the board's pm_items rows with the snapshot's rows (each row names
 * its snapshot), so a board whose fetch failed keeps its previous
 * rows untouched — fail-soft is a property of the write path, not
 * just the fetch loop (design §3, #9060 §1). Snapshot headers are
 * append-only history. Writes carry writer + epoch provenance from
 * the board's writer-of-record lease row (design §5/§7 invariant 2);
 * with no lease row the hosted writer records epoch 0.
 */
import type { SqlDb } from "../tenant-store/migrate.ts";
import {
  validateBoardDef,
  validateMetadataBlock,
  type BoardDef,
  type PmMetadataBlock,
} from "../tenant-store/schema.ts";
import { metadataBlockFromBody, type ProjectAudit, type ProjectPoolItem } from "./pool.ts";

interface BoardRow {
  board_id: string;
  org: string;
  number: number;
  node_id: string;
  url: string;
  role: BoardDef["role"];
  parent_board_id: string | null;
  sibling_order: number;
  title: string;
}

/** Every registry board, in sibling order. A row that fails the
 * slice-1 BoardDef validator is a hard error: a corrupt registry
 * must fail the run loudly, never silently shrink the pool. */
export function loadRegistry(db: SqlDb): BoardDef[] {
  const rows = db.all<BoardRow>(
    `SELECT board_id, org, number, node_id, url, role, parent_board_id, sibling_order, title
     FROM pm_boards ORDER BY sibling_order, board_id`,
  );
  return rows.map((row) => {
    const def: BoardDef = {
      id: row.board_id,
      org: row.org,
      number: row.number,
      nodeId: row.node_id,
      url: row.url,
      role: row.role,
      ...(row.parent_board_id !== null ? { parentBoardId: row.parent_board_id } : {}),
      siblingOrder: row.sibling_order,
      title: row.title,
    };
    const problems = validateBoardDef(def);
    if (problems.length) throw new Error(`invalid board registry row ${row.board_id}: ${problems.join("; ")}`);
    return def;
  });
}

/** The board's current writer-of-record lease epoch (0 when no lease
 * row exists yet — the flip procedure of design §5 creates it). */
export function writerEpoch(db: SqlDb, boardId: string): number {
  const rows = db.all<{ epoch: number }>("SELECT epoch FROM pm_writer_leases WHERE board_id = ?", [boardId]);
  return rows[0]?.epoch ?? 0;
}

export interface LandedSnapshot {
  snapshotId: string;
  boardId: string;
  generatedAt: string;
  audit: ProjectAudit;
  counts: { reportedTotal: number; emittedEntries: number };
  items: readonly ProjectPoolItem[];
  /** Issue bodies by ref, parsed for §12 Metadata copies. */
  bodies?: ReadonlyMap<string, string>;
  now: string;
}

/**
 * Land one board's snapshot in a single transaction: append the
 * header, replace the board's pool rows with this snapshot's rows,
 * and refresh the Metadata copies for items whose bodies carry a
 * valid §12 block. Items arrive already epic/spawn-resolved over
 * the merged pool (cross-board ancestry is a property of the union,
 * so resolution happens in the orchestrator, not here).
 */
export function landSnapshot(db: SqlDb, landed: LandedSnapshot): void {
  const epoch = writerEpoch(db, landed.boardId);
  db.transaction(() => {
    db.run(
      `INSERT INTO pm_snapshots (snapshot_id, board_id, generated_at, reported_total, emitted_entries, writer, epoch, audit_json, recorded_at)
       VALUES (?, ?, ?, ?, ?, 'hosted', ?, ?, ?)`,
      [
        landed.snapshotId,
        landed.boardId,
        landed.generatedAt,
        landed.counts.reportedTotal,
        landed.counts.emittedEntries,
        epoch,
        JSON.stringify(landed.audit),
        landed.now,
      ],
    );
    db.run("DELETE FROM pm_items WHERE board_id = ?", [landed.boardId]);
    for (const item of landed.items) insertPoolItem(db, landed, item);
    if (landed.bodies) {
      for (const item of landed.items) {
        const body = landed.bodies.get(item.ref);
        if (body !== undefined) upsertMetadataCopy(db, item, body, landed.now);
      }
    }
  });
}

function insertPoolItem(db: SqlDb, landed: LandedSnapshot, item: ProjectPoolItem): void {
  db.run(
    `INSERT INTO pm_items (board_id, item_id, snapshot_id, ref, repo, number, content_kind, state, draft, title,
       url, project_url, issue_url, database_id, author, labels_json, status, area, horizon, kind,
       milestone_title, milestone_due_on, milestone_closed_at, sub_issues_total, sub_issues_completed,
       parent_ref, epic_ref, blocked_by_json, blocks_json, historical_parent_refs_json, spawn_rollup_json,
       edge_diagnostics_json, item_updated_at, closed_at, merged_at, item_json, recorded_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    [
      landed.boardId,
      item.id,
      landed.snapshotId,
      item.ref,
      item.repo,
      item.number,
      item.contentKind,
      item.state,
      item.draft ? 1 : 0,
      item.title,
      item.url,
      item.projectUrl,
      item.issueUrl,
      item.databaseId ?? null,
      item.author ?? null,
      JSON.stringify(item.labels ?? []),
      item.status,
      item.area,
      item.horizon,
      item.kind,
      item.milestone?.title ?? null,
      item.milestone?.dueOn ?? null,
      item.milestone?.closedAt ?? null,
      item.subIssuesSummary?.total ?? null,
      item.subIssuesSummary?.completed ?? null,
      item.parentRef ?? null,
      item.epicRef ?? null,
      JSON.stringify(item.blockedBy ?? []),
      JSON.stringify(item.blocks ?? []),
      JSON.stringify(item.historicalParentRefs ?? []),
      item.spawnRollup ? JSON.stringify(item.spawnRollup) : null,
      JSON.stringify(item.edgeDiagnostics ?? []),
      item.updatedAt || null,
      item.closedAt ?? null,
      item.mergedAt ?? null,
      JSON.stringify(item),
      landed.now,
    ],
  );
}

/** Store the tenant DB's copy of an item's §12 Metadata block when
 * the body carries a valid one; an invalid block stores nothing
 * (the canonical record lives on GitHub; a bad copy must not
 * displace a good one). */
function upsertMetadataCopy(db: SqlDb, item: ProjectPoolItem, body: string, now: string): void {
  const parsed = metadataBlockFromBody(body);
  if (parsed === null || validateMetadataBlock(parsed).length) return;
  const block = parsed as PmMetadataBlock;
  const parent = block.direct_parent;
  const provenance = block.provenance;
  const prov = (key: string): string | null => {
    const value = provenance[key];
    return typeof value === "string" && value ? value : null;
  };
  db.run(
    `INSERT INTO pm_item_metadata (ref, repo, number, kind, pm_meta, direct_parent_ref, direct_parent_kind,
       lineage_design_ref, lineage_roadmap_item_ref, lineage_priority_ref,
       provenance_session_id, provenance_lane, provenance_queue_id, provenance_json, metadata_json, recorded_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (ref) DO UPDATE SET
       pm_meta = excluded.pm_meta,
       direct_parent_ref = excluded.direct_parent_ref,
       direct_parent_kind = excluded.direct_parent_kind,
       lineage_design_ref = excluded.lineage_design_ref,
       lineage_roadmap_item_ref = excluded.lineage_roadmap_item_ref,
       lineage_priority_ref = excluded.lineage_priority_ref,
       provenance_session_id = excluded.provenance_session_id,
       provenance_lane = excluded.provenance_lane,
       provenance_queue_id = excluded.provenance_queue_id,
       provenance_json = excluded.provenance_json,
       metadata_json = excluded.metadata_json,
       recorded_at = excluded.recorded_at`,
    [
      item.ref,
      item.repo,
      item.number,
      item.contentKind,
      block.pm_meta,
      parent ? `${parent.repo}#${parent.number}` : null,
      parent?.kind ?? null,
      block.lineage.design,
      block.lineage.roadmap_item,
      block.lineage.priority,
      prov("session_id"),
      prov("lane"),
      prov("queue_id"),
      JSON.stringify(provenance),
      JSON.stringify(block),
      now,
    ],
  );
}

/** Record one ingest finding (design §3 acceptance: a board failure
 * "records a finding"). Findings are the enforcement/report surface
 * where GitHub cannot hard-block (#9060 §7); ingest failures land
 * here with the board's lease epoch as provenance. */
export function recordIngestFinding(
  db: SqlDb,
  input: {
    findingId: string;
    boardId: string;
    kind: "ingest-failure" | "ingest-deferred";
    severity: "warning" | "error";
    summary: string;
    detail?: Record<string, unknown>;
    now: string;
  },
): void {
  db.run(
    `INSERT INTO pm_reconcile_findings (finding_id, board_id, kind, severity, subject_ref, summary, detail_json, writer, epoch, recorded_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, 'hosted', ?, ?)`,
    [
      input.findingId,
      input.boardId,
      input.kind,
      input.severity,
      input.boardId,
      input.summary,
      JSON.stringify(input.detail ?? {}),
      writerEpoch(db, input.boardId),
      input.now,
    ],
  );
}
