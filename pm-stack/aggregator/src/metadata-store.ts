/** Versioned PM Metadata storage contract for issue #9074. */
import type { Database } from "bun:sqlite";

export type JsonObject = Record<string, unknown>;

export interface MetadataIndexes {
  directParentRepo?: string;
  directParentNumber?: number;
  sessionId?: string;
  lane?: string;
  host?: string;
  queueId?: string;
  links: { targetRepo?: string; targetNumber?: number; relation?: string; status?: string }[];
}

export interface ParsedPmMetadata {
  schemaVersion: number;
  /** Original, complete object; stored verbatim as JSON by the DB layer. */
  metadata: JsonObject;
  /** Unknown keys and values with a registered field's wrong shape. */
  extras: JsonObject;
  indexes: MetadataIndexes;
}

export type ParsePmMetadataResult = { ok: true; value: ParsedPmMetadata } | { ok: false; error: string };

export interface PmMetadataRecord {
  artifactId: string;
  repo: string;
  itemNumber: number;
  itemKind: string;
  sourceUrl: string;
  parsed: ParsedPmMetadata;
  sourceUpdatedAt?: string;
}

export interface PmMetadataRow {
  artifactId: string;
  repo: string;
  itemNumber: number;
  itemKind: string;
  sourceUrl: string;
  schemaVersion: number;
  metadata: JsonObject;
  extras: JsonObject;
  indexes: MetadataIndexes;
  sourceUpdatedAt?: string;
}

export const PM_METADATA_SCHEMA_VERSION = 1;

export const PM_METADATA_STORE_DDL: readonly string[] = [
  `CREATE TABLE IF NOT EXISTS pm_metadata_items (
    artifact_id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    item_number INTEGER NOT NULL,
    item_kind TEXT NOT NULL,
    source_url TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    metadata_json TEXT NOT NULL,
    extras_json TEXT NOT NULL,
    direct_parent_repo TEXT,
    direct_parent_number INTEGER,
    session_id TEXT,
    lane TEXT,
    host TEXT,
    queue_id TEXT,
    links_json TEXT NOT NULL,
    source_updated_at TEXT
  )`,
  "CREATE INDEX IF NOT EXISTS pm_metadata_parent_idx ON pm_metadata_items (direct_parent_repo, direct_parent_number)",
  "CREATE INDEX IF NOT EXISTS pm_metadata_session_idx ON pm_metadata_items (session_id)",
  "CREATE INDEX IF NOT EXISTS pm_metadata_lane_idx ON pm_metadata_items (lane)",
  "CREATE INDEX IF NOT EXISTS pm_metadata_host_idx ON pm_metadata_items (host)",
  "CREATE INDEX IF NOT EXISTS pm_metadata_queue_idx ON pm_metadata_items (queue_id)",
  `CREATE TABLE IF NOT EXISTS pm_metadata_links (
    artifact_id TEXT NOT NULL,
    link_index INTEGER NOT NULL,
    target_repo TEXT,
    target_number INTEGER,
    relation TEXT,
    status TEXT,
    link_json TEXT NOT NULL,
    PRIMARY KEY (artifact_id, link_index)
  )`,
  "CREATE INDEX IF NOT EXISTS pm_metadata_link_status_idx ON pm_metadata_links (status, relation)",
  "CREATE INDEX IF NOT EXISTS pm_metadata_link_target_idx ON pm_metadata_links (target_repo, target_number)",
];

/** Create the metadata tables and their rebuildable query indexes. */
export function createPmMetadataStore(db: Database): void {
  for (const statement of PM_METADATA_STORE_DDL) db.exec(statement);
}

/**
 * Persist one parsed source row. The artifact ID is the identity boundary:
 * replaying an artifact replaces its row and rebuilds its derived link rows.
 * Oracle: one source artifact must have one current row and exactly its
 * current derived links, regardless of replay count.
 */
export function upsertPmMetadata(db: Database, record: PmMetadataRecord): void {
  if (!nonempty(record.artifactId)) throw new Error("artifactId must be non-empty");
  if (!nonempty(record.repo)) throw new Error("repo must be non-empty");
  if (!positiveInteger(record.itemNumber)) throw new Error("itemNumber must be a positive integer");
  if (!nonempty(record.itemKind)) throw new Error("itemKind must be non-empty");
  if (!nonempty(record.sourceUrl)) throw new Error("sourceUrl must be non-empty");

  const { parsed } = record;
  db.exec("BEGIN IMMEDIATE");
  try {
    db.query(`
    INSERT INTO pm_metadata_items (
      artifact_id, repo, item_number, item_kind, source_url, schema_version,
      metadata_json, extras_json, direct_parent_repo, direct_parent_number,
      session_id, lane, host, queue_id, links_json, source_updated_at
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(artifact_id) DO UPDATE SET
      repo = excluded.repo,
      item_number = excluded.item_number,
      item_kind = excluded.item_kind,
      source_url = excluded.source_url,
      schema_version = excluded.schema_version,
      metadata_json = excluded.metadata_json,
      extras_json = excluded.extras_json,
      direct_parent_repo = excluded.direct_parent_repo,
      direct_parent_number = excluded.direct_parent_number,
      session_id = excluded.session_id,
      lane = excluded.lane,
      host = excluded.host,
      queue_id = excluded.queue_id,
      links_json = excluded.links_json,
      source_updated_at = excluded.source_updated_at
    WHERE pm_metadata_items.repo IS NOT excluded.repo
      OR pm_metadata_items.item_number IS NOT excluded.item_number
      OR pm_metadata_items.item_kind IS NOT excluded.item_kind
      OR pm_metadata_items.source_url IS NOT excluded.source_url
      OR pm_metadata_items.schema_version IS NOT excluded.schema_version
      OR pm_metadata_items.metadata_json IS NOT excluded.metadata_json
      OR pm_metadata_items.extras_json IS NOT excluded.extras_json
      OR pm_metadata_items.direct_parent_repo IS NOT excluded.direct_parent_repo
      OR pm_metadata_items.direct_parent_number IS NOT excluded.direct_parent_number
      OR pm_metadata_items.session_id IS NOT excluded.session_id
      OR pm_metadata_items.lane IS NOT excluded.lane
      OR pm_metadata_items.host IS NOT excluded.host
      OR pm_metadata_items.queue_id IS NOT excluded.queue_id
      OR pm_metadata_items.links_json IS NOT excluded.links_json
      OR pm_metadata_items.source_updated_at IS NOT excluded.source_updated_at
    `).run(
      record.artifactId,
      record.repo,
      record.itemNumber,
      record.itemKind,
      record.sourceUrl,
      parsed.schemaVersion,
      JSON.stringify(parsed.metadata),
      JSON.stringify(parsed.extras),
      parsed.indexes.directParentRepo ?? null,
      parsed.indexes.directParentNumber ?? null,
      parsed.indexes.sessionId ?? null,
      parsed.indexes.lane ?? null,
      parsed.indexes.host ?? null,
      parsed.indexes.queueId ?? null,
      JSON.stringify(parsed.indexes.links),
      record.sourceUpdatedAt ?? null,
    );

    // Upsert only changed derived rows. A source replay must not delete and
    // recreate every link for an artifact; remove only trailing indexes left
    // behind when the source has fewer links than before.
    const insertLink = db.query(`
    INSERT INTO pm_metadata_links (
      artifact_id, link_index, target_repo, target_number, relation, status, link_json
    ) VALUES (?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(artifact_id, link_index) DO UPDATE SET
      target_repo = excluded.target_repo,
      target_number = excluded.target_number,
      relation = excluded.relation,
      status = excluded.status,
      link_json = excluded.link_json
    WHERE pm_metadata_links.target_repo IS NOT excluded.target_repo
      OR pm_metadata_links.target_number IS NOT excluded.target_number
      OR pm_metadata_links.relation IS NOT excluded.relation
      OR pm_metadata_links.status IS NOT excluded.status
      OR pm_metadata_links.link_json IS NOT excluded.link_json
    `);
    for (const [linkIndex, link] of parsed.indexes.links.entries()) {
      insertLink.run(
        record.artifactId,
        linkIndex,
        link.targetRepo ?? null,
        link.targetNumber ?? null,
        link.relation ?? null,
        link.status ?? null,
        JSON.stringify(link),
      );
    }
    db.query("DELETE FROM pm_metadata_links WHERE artifact_id = ? AND link_index >= ?").run(
      record.artifactId,
      parsed.indexes.links.length,
    );
    db.exec("COMMIT");
  } catch (error) {
    db.exec("ROLLBACK");
    throw error;
  }
}

const stringOrUndefined = (value: unknown): string | undefined => (typeof value === "string" ? value : undefined);
const numberOrUndefined = (value: unknown): number | undefined => (typeof value === "number" ? value : undefined);

/** Read a source row with its extracted query projection. */
export function getPmMetadata(db: Database, artifactId: string): PmMetadataRow | null {
  const row = db
    .query(`
    SELECT artifact_id, repo, item_number, item_kind, source_url, schema_version,
      metadata_json, extras_json, direct_parent_repo, direct_parent_number,
      session_id, lane, host, queue_id, source_updated_at
    FROM pm_metadata_items WHERE artifact_id = ?
  `)
    .get(artifactId) as Record<string, unknown> | null;
  if (!row) return null;

  const links = db
    .query(`
    SELECT target_repo, target_number, relation, status
    FROM pm_metadata_links WHERE artifact_id = ? ORDER BY link_index
  `)
    .all(artifactId) as Record<string, unknown>[];
  return {
    artifactId: String(row.artifact_id),
    repo: String(row.repo),
    itemNumber: Number(row.item_number),
    itemKind: String(row.item_kind),
    sourceUrl: String(row.source_url),
    schemaVersion: Number(row.schema_version),
    metadata: JSON.parse(String(row.metadata_json)) as JsonObject,
    extras: JSON.parse(String(row.extras_json)) as JsonObject,
    indexes: {
      ...(stringOrUndefined(row.direct_parent_repo) ? { directParentRepo: String(row.direct_parent_repo) } : {}),
      ...(numberOrUndefined(row.direct_parent_number) !== undefined
        ? { directParentNumber: Number(row.direct_parent_number) }
        : {}),
      ...(stringOrUndefined(row.session_id) ? { sessionId: String(row.session_id) } : {}),
      ...(stringOrUndefined(row.lane) ? { lane: String(row.lane) } : {}),
      ...(stringOrUndefined(row.host) ? { host: String(row.host) } : {}),
      ...(stringOrUndefined(row.queue_id) ? { queueId: String(row.queue_id) } : {}),
      links: links.map((link) => ({
        ...(stringOrUndefined(link.target_repo) ? { targetRepo: String(link.target_repo) } : {}),
        ...(numberOrUndefined(link.target_number) !== undefined ? { targetNumber: Number(link.target_number) } : {}),
        ...(stringOrUndefined(link.relation) ? { relation: String(link.relation) } : {}),
        ...(stringOrUndefined(link.status) ? { status: String(link.status) } : {}),
      })),
    },
    ...(stringOrUndefined(row.source_updated_at) ? { sourceUpdatedAt: String(row.source_updated_at) } : {}),
  };
}

const isObject = (value: unknown): value is JsonObject =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const nonempty = (value: unknown): value is string => typeof value === "string" && value.trim().length > 0;
const positiveInteger = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) > 0;

/**
 * Read a PM Metadata object through its version adapter and extract only
 * registered, correctly shaped query fields. The source object is retained
 * untouched; malformed/unknown fields are preserved in `extras` rather than
 * discarded or guessed. Oracle: the section-12 JSON fixture and #9095's
 * versioned-reader / type-mismatch preservation rules.
 */
export function parsePmMetadata(input: unknown): ParsePmMetadataResult {
  if (!isObject(input)) return { ok: false, error: "Metadata must be a JSON object" };
  const version = input.pm_meta;
  if (!Number.isSafeInteger(version) || (version as number) < 1 || (version as number) > PM_METADATA_SCHEMA_VERSION) {
    return { ok: false, error: `unsupported PM Metadata schema version: ${String(version)}` };
  }

  const extras: JsonObject = {};
  const registered = new Set(["pm_meta", "direct_parent", "links", "lineage", "provenance"]);
  for (const [key, value] of Object.entries(input)) if (!registered.has(key)) extras[key] = value;

  const indexes: MetadataIndexes = { links: [] };
  const parent = input.direct_parent;
  if (parent === undefined || parent === null) {
    // Optional and absent/null are intentionally not collapsed into false.
  } else if (isObject(parent) && nonempty(parent.repo) && positiveInteger(parent.number) && nonempty(parent.kind)) {
    indexes.directParentRepo = parent.repo.trim();
    indexes.directParentNumber = parent.number;
  } else {
    extras.direct_parent = parent;
  }

  const provenance = input.provenance;
  if (provenance === undefined || provenance === null) {
    // Optional forever; absence remains absence.
  } else if (isObject(provenance)) {
    const fields = ["session_id", "lane", "host", "queue_id"] as const;
    const knownProvenance = new Set([
      "queue_id",
      "session_id",
      "lane",
      "host",
      "worktree",
      "picked_up_at",
      "work_started_at",
      "created_at",
      "model",
      "lane_start_issues",
    ]);
    for (const [key, value] of Object.entries(provenance)) {
      if (!knownProvenance.has(key)) extras[`provenance.${key}`] = value;
    }
    for (const field of fields) {
      const value = provenance[field];
      if (value === undefined || value === null) continue;
      if (nonempty(value)) {
        const index = { session_id: "sessionId", lane: "lane", host: "host", queue_id: "queueId" }[field] as
          | "sessionId"
          | "lane"
          | "host"
          | "queueId";
        indexes[index] = value.trim();
      } else {
        extras[`provenance.${field}`] = value;
      }
    }
  } else {
    extras.provenance = provenance;
  }

  const links = input.links;
  if (links === undefined || links === null) {
    // As above, preserve absent versus present values in metadata_json.
  } else if (Array.isArray(links)) {
    indexes.links = links.map((raw, position) => {
      if (!isObject(raw)) {
        extras[`links.${position}`] = raw;
        return {};
      }
      const target = isObject(raw.target) ? raw.target : {};
      if (
        !isObject(raw.target) ||
        !nonempty(target.repo) ||
        !positiveInteger(target.number) ||
        !nonempty(raw.relation) ||
        !nonempty(raw.status)
      ) {
        extras[`links.${position}`] = raw;
      }
      return {
        ...(nonempty(target.repo) ? { targetRepo: target.repo.trim() } : {}),
        ...(positiveInteger(target.number) ? { targetNumber: target.number } : {}),
        ...(nonempty(raw.relation) ? { relation: raw.relation.trim() } : {}),
        ...(nonempty(raw.status) ? { status: raw.status.trim() } : {}),
      };
    });
  } else {
    extras.links = links;
  }

  return { ok: true, value: { schemaVersion: version as number, metadata: input, extras, indexes } };
}
