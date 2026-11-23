// Turso-backed implementations of the remote store interfaces
// added in #6815, plus the comments interface added in #1744
// (Move 1, step 2). They speak the shared SqlClient contract
// (src/sql-client.ts), served by the Turso store client
// (src/turso-stores.ts), and write only the tables from
// shared-schema/migrations/20261116_0001_aggregator_remote_stores.up.sql
// plus the pre-existing agg_actions contract (20261105_0001). The SQL
// below is written once and translated to SQLite by storeStatement().
//
// The caller selects these only when PM_AGG_STORE_BACKEND=turso. A
// missing client is an error here: silently writing a local canonical
// copy would recreate exactly the laptop-only state #6815 removes.
import {
  ACTION_STATES,
  type ActionsRemote,
  type DashAction,
} from "./actions.ts";
import type {
  ActivityRemote,
  ActivitySourceStatus,
  GapRecord,
  IngestionFile,
  SourceCheckpoint,
} from "./activity.ts";
import type {
  AppendOnlyDocumentRemote,
  DocumentRemote,
} from "./aa-last-good.ts";
import type { SnapshotsRemote } from "./snapshots.ts";
import type { DashComment, SnapshotRecord } from "./model.ts";
import type { CommentsRemote } from "./comments.ts";
import { snapshotKey } from "./snapshots.ts";
import { storeWriterIdentity } from "./store-backend.ts";
import { sqlJson, sqlStr, type SqlClient } from "./sql-client.ts";
import { getTursoStoreClient } from "./turso-stores.ts";

export interface StoreRemotes {
  actions: ActionsRemote;
  activity: ActivityRemote;
  usage: SnapshotsRemote;
  aaLastGood: DocumentRemote & AppendOnlyDocumentRemote;
  comments: CommentsRemote;
}

type GetClient = () => Promise<SqlClient | null>;

const WRITE_CHUNK = 200;

// Use a single set of mutable columns for assignment and comparison. The
// null-safe check runs against the actual conflicting row inside the UPSERT,
// not against a cache that could be stale after another write or lost response.
function updateChanged(
  table: string,
  columns: readonly string[],
  touchUpdatedAt = false,
): string {
  const assignments = columns.map((column) => `${column} = excluded.${column}`);
  if (touchUpdatedAt) assignments.push("updated_at = now()");
  const changed = columns.map(
    (column) => `${table}.${column} is not excluded.${column}`,
  );
  return `do update set ${assignments.join(", ")} where ${changed.join(" or ")}`;
}

function chunks<T>(rows: readonly T[], size = WRITE_CHUNK): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < rows.length; i += size) out.push(rows.slice(i, i + size));
  return out;
}

async function requireClient(getClient: GetClient): Promise<SqlClient> {
  const db = await getClient();
  if (!db) {
    throw new Error(
      "PM_AGG_STORE_BACKEND=turso but the Turso store client is unavailable " +
        "(store token missing); refusing to fall back to a local canonical copy",
    );
  }
  return db;
}

function optStr(v: string | null | undefined): string {
  return v === null || v === undefined || v === "" ? "null" : sqlStr(v);
}

function optTs(v: string | null | undefined): string {
  return v === null || v === undefined || v === ""
    ? "null"
    : `${sqlStr(v)}::timestamptz`;
}

function sqlNum(v: number | null | undefined): string {
  if (v === null || v === undefined) return "null";
  if (!Number.isFinite(v)) throw new Error(`not a finite number: ${v}`);
  return String(v);
}

function isoOrUndefined(v: unknown): string | undefined {
  if (typeof v !== "string") return undefined;
  const t = Date.parse(v);
  return Number.isFinite(t) ? new Date(t).toISOString() : undefined;
}

function isoRequired(v: unknown, what: string): string {
  const out = isoOrUndefined(v);
  if (!out)
    throw new Error(
      `agg store row has an invalid ${what}: ${JSON.stringify(v)}`,
    );
  return out;
}

function textArray(values: readonly string[]): string {
  return values.length === 0
    ? "'{}'::text[]"
    : `array[${values.map(sqlStr).join(", ")}]::text[]`;
}

// ---------------------------------------------------------------------
// actions.json -> public.agg_actions
// ---------------------------------------------------------------------

const ACTION_COLUMNS =
  "id, client_token, kind, verb, object_ref, target_ref, session_id, session_source, payload, author, " +
  "created_at, state, applied_at, applied_by_run, result_note, supersedes_action_id, duplicate_of_action_id";

export function actionRowValues(a: DashAction): string {
  return `(${[
    sqlStr(a.id),
    optStr(a.clientToken),
    sqlStr(a.kind),
    sqlStr(a.verb),
    sqlStr(a.objectRef),
    optStr(a.targetRef),
    optStr(a.sessionId),
    optStr(a.sessionSource),
    sqlJson(a.payload),
    sqlJson(a.author),
    `${sqlStr(a.createdAt)}::timestamptz`,
    sqlStr(a.state),
    optTs(a.appliedAt),
    optStr(a.appliedByRun),
    optStr(a.resultNote),
    optStr(a.supersedes),
    optStr(a.duplicateOf),
  ].join(", ")})`;
}

export function actionFromRow(row: Record<string, unknown>): DashAction {
  const state = row.state;
  if (typeof state !== "string" || !ACTION_STATES.has(state)) {
    throw new Error(
      `agg_actions row ${String(row.id)} has an invalid state: ${JSON.stringify(state)}`,
    );
  }
  const action = {
    id: String(row.id),
    kind: String(row.kind),
    verb: String(row.verb),
    objectRef: String(row.object_ref),
    payload: (row.payload ?? {}) as DashAction["payload"],
    author: row.author as DashAction["author"],
    createdAt: isoRequired(row.created_at, "created_at"),
    state: state as DashAction["state"],
  } as DashAction;
  // Pre-token audit rows are readable with no clientToken (#6103 contract).
  if (typeof row.client_token === "string")
    action.clientToken = row.client_token;
  if (typeof row.target_ref === "string") action.targetRef = row.target_ref;
  if (typeof row.session_id === "string") action.sessionId = row.session_id;
  if (typeof row.session_source === "string")
    action.sessionSource = row.session_source as DashAction["sessionSource"];
  const appliedAt = isoOrUndefined(row.applied_at);
  if (appliedAt) action.appliedAt = appliedAt;
  if (typeof row.applied_by_run === "string")
    action.appliedByRun = row.applied_by_run;
  if (typeof row.result_note === "string") action.resultNote = row.result_note;
  if (typeof row.supersedes_action_id === "string")
    action.supersedes = row.supersedes_action_id;
  if (typeof row.duplicate_of_action_id === "string")
    action.duplicateOf = row.duplicate_of_action_id;
  return action;
}

export function createTursoActionsRemote(
  getClient: GetClient = getTursoStoreClient,
): ActionsRemote {
  return {
    async loadAll(): Promise<DashAction[]> {
      const db = await requireClient(getClient);
      const rows = await db.query<Record<string, unknown>>(
        `select ${ACTION_COLUMNS} from public.agg_actions order by created_at asc`,
      );
      return rows.map(actionFromRow);
    },

    async saveAll(actions: readonly DashAction[]): Promise<void> {
      if (actions.length === 0) return;
      const db = await requireClient(getClient);
      // Insertion order matters: supersedes/duplicate_of are self-FKs, so
      // parents must land before children. The store keeps insertion order;
      // createdAt ascending preserves it.
      const ordered = [...actions].sort((a, b) =>
        a.createdAt.localeCompare(b.createdAt),
      );
      for (const chunk of chunks(ordered)) {
        const values = chunk.map(actionRowValues).join(",\n");
        await db.query(
          `insert into public.agg_actions (${ACTION_COLUMNS}) values\n${values}\n` +
            `on conflict (id) ${updateChanged("agg_actions", [
              "state",
              "applied_at",
              "applied_by_run",
              "result_note",
              "supersedes_action_id",
              "duplicate_of_action_id",
            ])}`,
        );
      }
    },
  };
}

// ---------------------------------------------------------------------
// activity-ingestion.json -> public.agg_activity_ingestion + gaps
// ---------------------------------------------------------------------

export function createTursoActivityRemote(
  getClient: GetClient = getTursoStoreClient,
): ActivityRemote {
  return {
    async load(): Promise<IngestionFile | null> {
      const db = await requireClient(getClient);
      const rows = await db.query<Record<string, unknown>>(
        `select source, checkpoint, seen_ids, source_status from public.agg_activity_ingestion`,
      );
      const gapRows = await db.query<Record<string, unknown>>(
        `select id, source, gap_from, gap_to, status, detected_at, resolved_at from public.agg_activity_gaps`,
      );
      if (rows.length === 0 && gapRows.length === 0) return null;
      const data: IngestionFile = {
        version: 1,
        checkpoints: {},
        seen: {},
        gaps: [],
        sourceStatus: {},
      };
      for (const row of rows) {
        const source = String(row.source);
        if (row.checkpoint && typeof row.checkpoint === "object") {
          data.checkpoints[source] = row.checkpoint as SourceCheckpoint;
        }
        if (Array.isArray(row.seen_ids))
          data.seen[source] = row.seen_ids.map(String);
        if (row.source_status && typeof row.source_status === "object") {
          data.sourceStatus![source] =
            row.source_status as ActivitySourceStatus;
        }
      }
      for (const row of gapRows) {
        const status = String(row.status);
        if (
          status !== "open" &&
          status !== "backfilled" &&
          status !== "unrecoverable"
        ) {
          throw new Error(
            `agg_activity_gaps row ${String(row.id)} has an invalid status: ${status}`,
          );
        }
        const gap: GapRecord = {
          id: String(row.id),
          source: String(row.source),
          from: isoRequired(row.gap_from, "gap_from"),
          to: isoRequired(row.gap_to, "gap_to"),
          status,
          detectedAt: isoRequired(row.detected_at, "detected_at"),
        };
        const resolvedAt = isoOrUndefined(row.resolved_at);
        if (resolvedAt) gap.resolvedAt = resolvedAt;
        data.gaps.push(gap);
      }
      return data;
    },

    async save(data: IngestionFile): Promise<void> {
      const db = await requireClient(getClient);
      const sources = new Set<string>([
        ...Object.keys(data.checkpoints),
        ...Object.keys(data.seen),
        ...Object.keys(data.sourceStatus ?? {}),
      ]);
      const sourceRows = [...sources].map((source) => {
        const checkpoint = data.checkpoints[source];
        const status = data.sourceStatus?.[source];
        return (
          `(${sqlStr(source)}, ${checkpoint ? sqlJson(checkpoint) : "null"}, ` +
          `${textArray(data.seen[source] ?? [])}, ${status ? sqlJson(status) : "null"}, now())`
        );
      });
      for (const chunk of chunks(sourceRows)) {
        await db.query(
          `insert into public.agg_activity_ingestion (source, checkpoint, seen_ids, source_status, updated_at) values\n` +
            `${chunk.join(",\n")}\n` +
            `on conflict (source) ${updateChanged(
              "agg_activity_ingestion",
              ["checkpoint", "seen_ids", "source_status"],
              true,
            )}`,
        );
      }
      const gapRows = data.gaps.map(
        (gap) =>
          `(${sqlStr(gap.id)}, ${sqlStr(gap.source)}, ${sqlStr(gap.from)}::timestamptz, ` +
          `${sqlStr(gap.to)}::timestamptz, ${sqlStr(gap.status)}, ${sqlStr(gap.detectedAt)}::timestamptz, ` +
          `${optTs(gap.resolvedAt)}, now())`,
      );
      for (const chunk of chunks(gapRows)) {
        await db.query(
          `insert into public.agg_activity_gaps (id, source, gap_from, gap_to, status, detected_at, resolved_at, updated_at) values\n` +
            `${chunk.join(",\n")}\n` +
            `on conflict (id) ${updateChanged("agg_activity_gaps", ["status", "gap_to", "resolved_at"], true)}`,
        );
      }
    },
  };
}

// ---------------------------------------------------------------------
// usage-YYYY-MM.jsonl -> public.agg_usage_snapshots
// ---------------------------------------------------------------------

export function snapshotFromRow(row: Record<string, unknown>): SnapshotRecord {
  const value = row.value;
  if (
    typeof value !== "number" &&
    (typeof value !== "object" || value === null)
  ) {
    throw new Error(
      `agg_usage_snapshots row ${String(row.snapshot_key)} has an invalid value`,
    );
  }
  const record: SnapshotRecord = {
    ts: isoRequired(row.ts, "ts"),
    serviceId: String(row.service_id),
    metricId: String(row.metric_id),
    value: value as SnapshotRecord["value"],
    unit: String(row.unit) as SnapshotRecord["unit"],
    source: String(row.source) as SnapshotRecord["source"],
    observedBy: String(row.observed_by),
  };
  if (row.limit_value !== null && row.limit_value !== undefined)
    record.limit = Number(row.limit_value);
  const resetsAt = isoOrUndefined(row.resets_at);
  if (resetsAt) record.resetsAt = resetsAt;
  return record;
}

export function snapshotRowValues(r: SnapshotRecord): string {
  return (
    `(${sqlStr(snapshotKey(r))}, ${sqlStr(r.ts)}::timestamptz, ${sqlStr(r.serviceId)}, ${sqlStr(r.metricId)}, ` +
    `${sqlStr(r.unit)}, ${sqlStr(r.source)}, ${sqlJson(r.value)}, ${sqlNum(r.limit)}, ${optTs(r.resetsAt)}, ` +
    `${sqlStr(r.observedBy)})`
  );
}

export function createTursoSnapshotsRemote(
  getClient: GetClient = getTursoStoreClient,
): SnapshotsRemote {
  return {
    async readAll(): Promise<SnapshotRecord[]> {
      const db = await requireClient(getClient);
      const rows = await db.query<Record<string, unknown>>(
        `select snapshot_key, ts, service_id, metric_id, unit, source, value, limit_value, resets_at, observed_by ` +
          `from public.agg_usage_snapshots order by ts asc`,
      );
      return rows.map(snapshotFromRow);
    },

    async append(records: readonly SnapshotRecord[]): Promise<number> {
      if (records.length === 0) return 0;
      const db = await requireClient(getClient);
      let inserted = 0;
      for (const chunk of chunks(records)) {
        const rows = await db.query<Record<string, unknown>>(
          `insert into public.agg_usage_snapshots ` +
            `(snapshot_key, ts, service_id, metric_id, unit, source, value, limit_value, resets_at, observed_by) values\n` +
            `${chunk.map(snapshotRowValues).join(",\n")}\n` +
            `on conflict (snapshot_key) do nothing returning snapshot_key`,
        );
        inserted += rows.length;
      }
      return inserted;
    },
  };
}

// ---------------------------------------------------------------------
// namespaced PM documents -> public.agg_store_documents
// ---------------------------------------------------------------------

export function createTursoDocumentRemote(
  getClient: GetClient = getTursoStoreClient,
  writer: string = storeWriterIdentity(),
): DocumentRemote & AppendOnlyDocumentRemote {
  return {
    async load(
      store: string,
    ): Promise<{ version: number; document: unknown } | null> {
      const db = await requireClient(getClient);
      const rows = await db.query<Record<string, unknown>>(
        `select version, document from public.agg_store_documents where store = ${sqlStr(store)}`,
      );
      const row = rows[0];
      if (!row) return null;
      return { version: Number(row.version), document: row.document };
    },

    async save(
      store: string,
      version: number,
      document: unknown,
    ): Promise<void> {
      const db = await requireClient(getClient);
      await db.query(
        `insert into public.agg_store_documents (store, version, document, writer, updated_at) ` +
          `values (${sqlStr(store)}, ${version}, ${sqlJson(document)}, ${sqlStr(writer)}, now()) ` +
          `on conflict (store) do update set version = excluded.version, document = excluded.document, ` +
          `writer = excluded.writer, updated_at = now()`,
      );
    },

    async insertIfAbsent(
      store: string,
      version: number,
      document: unknown,
    ): Promise<boolean> {
      const db = await requireClient(getClient);
      const rows = await db.query<Record<string, unknown>>(
        `insert into public.agg_store_documents (store, version, document, writer, updated_at) ` +
          `values (${sqlStr(store)}, ${version}, ${sqlJson(document)}, ${sqlStr(writer)}, now()) ` +
          `on conflict (store) do nothing returning store`,
      );
      return rows.length > 0;
    },

    async listPrefix(
      prefix: string,
    ): Promise<Array<{ store: string; version: number; document: unknown }>> {
      const db = await requireClient(getClient);
      return db.query<{ store: string; version: number; document: unknown }>(
        `select store, version, document from public.agg_store_documents ` +
          `where substr(store, 1, length(${sqlStr(prefix)})) = ${sqlStr(prefix)} ` +
          `order by store asc`,
      );
    },
  };
}

// ---------------------------------------------------------------------
// comments.json -> public.shared_comments (#1744 Move 1, step 2)
// ---------------------------------------------------------------------

const COMMENT_COLUMNS =
  "id, object_ref, kind, author, author_verified, provenance, linked_identity, body, " +
  "created_at, triage_state, github_ref, github_state, github_url, github_error, github_failed_at";

export function commentRowValues(c: DashComment): string {
  return `(${[
    sqlStr(c.id),
    sqlStr(c.objectRef),
    sqlStr(c.kind),
    sqlJson(c.author),
    c.authorVerified === true ? "true" : "false",
    // The file store persists the #9607 provenance default on load;
    // the remote writes the same effective value so a row never
    // round-trips to a different record than the file would hold.
    sqlStr(c.provenance ?? "asserted-legacy"),
    c.linkedIdentity ? sqlJson(c.linkedIdentity) : "null",
    sqlStr(c.body),
    `${sqlStr(c.createdAt)}::timestamptz`,
    optStr(c.triageState),
    optStr(c.githubRef),
    optStr(c.githubState),
    optStr(c.githubUrl),
    optStr(c.githubError),
    optTs(c.githubFailedAt),
  ].join(", ")})`;
}

export function commentFromRow(row: Record<string, unknown>): DashComment {
  const comment = {
    id: String(row.id),
    objectRef: String(row.object_ref),
    kind: String(row.kind),
    author: row.author as DashComment["author"],
    // SQLite (the Turso dialect) returns the boolean as 1/0.
    authorVerified: row.author_verified === true || row.author_verified === 1,
    body: String(row.body),
    createdAt: isoRequired(row.created_at, "created_at"),
  } as DashComment;
  if (typeof row.provenance === "string")
    comment.provenance = row.provenance as DashComment["provenance"];
  if (row.linked_identity && typeof row.linked_identity === "object") {
    comment.linkedIdentity =
      row.linked_identity as DashComment["linkedIdentity"];
  }
  if (typeof row.triage_state === "string")
    comment.triageState = row.triage_state as DashComment["triageState"];
  if (typeof row.github_ref === "string")
    comment.githubRef = row.github_ref as DashComment["githubRef"];
  if (typeof row.github_state === "string")
    comment.githubState = row.github_state as DashComment["githubState"];
  if (typeof row.github_url === "string") comment.githubUrl = row.github_url;
  if (typeof row.github_error === "string")
    comment.githubError = row.github_error;
  const failedAt = isoOrUndefined(row.github_failed_at);
  if (failedAt) comment.githubFailedAt = failedAt;
  return comment;
}

export function createTursoCommentsRemote(
  getClient: GetClient = getTursoStoreClient,
): CommentsRemote {
  return {
    async loadAll(): Promise<DashComment[]> {
      const db = await requireClient(getClient);
      const rows = await db.query<Record<string, unknown>>(
        `select ${COMMENT_COLUMNS} from public.shared_comments order by created_at asc`,
      );
      return rows.map(commentFromRow);
    },

    async saveAll(comments: readonly DashComment[]): Promise<void> {
      if (comments.length === 0) return;
      const db = await requireClient(getClient);
      const ordered = [...comments].sort((a, b) =>
        a.createdAt.localeCompare(b.createdAt),
      );
      for (const chunk of chunks(ordered)) {
        const values = chunk.map(commentRowValues).join(",\n");
        await db.query(
          `insert into public.shared_comments (${COMMENT_COLUMNS}) values\n${values}\n` +
            // Only the state fields the store mutates after creation
            // ride the update: triage transitions and github posting
            // results. Identity, author and body are write-once.
            `on conflict (id) ${updateChanged("shared_comments", [
              "triage_state",
              "github_state",
              "github_url",
              "github_error",
              "github_failed_at",
            ])}`,
        );
      }
    },
  };
}

export function createTursoRemotes(
  getClient: GetClient = getTursoStoreClient,
  writer: string = storeWriterIdentity(),
): StoreRemotes {
  return {
    actions: createTursoActionsRemote(getClient),
    activity: createTursoActivityRemote(getClient),
    usage: createTursoSnapshotsRemote(getClient),
    aaLastGood: createTursoDocumentRemote(getClient, writer),
    comments: createTursoCommentsRemote(getClient),
  };
}
