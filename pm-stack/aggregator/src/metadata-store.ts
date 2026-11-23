/** Versioned PM Metadata storage contract for issue #9074. */
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

export type ParsePmMetadataResult =
  | { ok: true; value: ParsedPmMetadata }
  | { ok: false; error: string };

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
      "queue_id", "session_id", "lane", "host", "worktree", "picked_up_at",
      "work_started_at", "created_at", "model", "lane_start_issues",
    ]);
    for (const [key, value] of Object.entries(provenance)) {
      if (!knownProvenance.has(key)) extras[`provenance.${key}`] = value;
    }
    for (const field of fields) {
      const value = provenance[field];
      if (value === undefined || value === null) continue;
      if (nonempty(value)) {
        const index = { session_id: "sessionId", lane: "lane", host: "host", queue_id: "queueId" }[field] as
          | "sessionId" | "lane" | "host" | "queueId";
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
      if (!isObject(raw.target) || !nonempty(target.repo) || !positiveInteger(target.number) || !nonempty(raw.relation) || !nonempty(raw.status)) {
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
