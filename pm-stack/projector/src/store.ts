// Database wiring: connection + schema migration + a small store layer.
// Uses @libsql/client, which speaks both a local SQLite file (file: URL) and
// Turso/libSQL over HTTP or WebSocket (libsql:// or https:// + auth token).
import { createClient, type Client } from "@libsql/client";

export interface DbEnv {
  /** libSQL connection URL. Defaults to a local file DB. */
  DATABASE_URL?: string;
  /** Auth token for a remote Turso/libSQL endpoint (referenced by name, never committed). */
  TURSO_AUTH_TOKEN?: string;
}

export function dbUrl(env: DbEnv): string {
  return env.DATABASE_URL || "file:data/projector.db";
}

export function openDb(env: DbEnv): Client {
  const url = dbUrl(env);
  if (url.startsWith("file:")) {
    return createClient({ url });
  }
  return createClient({ url, authToken: env.TURSO_AUTH_TOKEN });
}

/** Run schema.sql. Idempotent — every statement is CREATE ... IF NOT EXISTS. */
export async function migrate(db: Client, schemaPath?: string): Promise<void> {
  const columns = await db.execute("PRAGMA table_info(pmp_comments)");
  if (columns.rows.length > 0) {
    const pk = columns.rows.filter((row) => Number(row.pk) > 0).sort((a, b) => Number(a.pk) - Number(b.pk));
    if (pk.length !== 2 || pk[0].name !== "id" || pk[1].name !== "kind") {
      throw new Error("Legacy pmp_comments schema: rebuild the comment projection before starting; preserve pmp_links and other tables. See the projector README.");
    }
  }
  const path = schemaPath ?? new URL("../schema.sql", import.meta.url).pathname;
  const sql = await Bun.file(path).text();
  // libSQL executes one statement per call. Strip full-line comments first,
  // then split on the statement boundary. schema.sql deliberately contains no
  // semicolons inside string literals.
  const noComments = sql
    .split("\n")
    .map((line) => line.replace(/--.*$/, ""))
    .join("\n");
  const statements = noComments
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  for (const stmt of statements) {
    await db.execute(stmt);
  }
}

export type Json = string | number | boolean | null | Json[] | { [k: string]: Json };

export interface ItemRow {
  ref: string;
  repo: string;
  number: number;
  kind: "issue" | "pr";
  url: string;
  api_url: string;
  title: string;
  state: "open" | "closed";
  state_reason: string | null;
  draft: number;
  merged: number;
  author: string | null;
  labels: string;
  assignees: string;
  created_at: string;
  updated_at: string;
  closed_at: string | null;
  merged_at: string | null;
  source_updated_at: string;
  fetched_at: string;
}

export interface CommentRow {
  id: string;
  ref: string;
  kind: "issue_comment" | "review" | "review_comment";
  author: string | null;
  body: string | null;
  created_at: string;
  updated_at: string | null;
  url: string | null;
  path: string | null;
  line: number | null;
  diff_hunk: string | null;
  review_state: string | null;
  submitted_at: string | null;
  commit_id: string | null;
  deleted: number;
  source_updated_at: string;
  fetched_at: string;
}

export interface LinkRow {
  subject: string;
  ref: string;
  relation: string;
  note: string | null;
  created_by: string | null;
  created_at: string;
}

const now = () => new Date().toISOString();

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/** Upsert an item. Monotonic: a stale source (older updated_at) never overwrites. */
export async function upsertItem(db: Client, i: Omit<ItemRow, "fetched_at">): Promise<void> {
  await db.execute({
    sql: `INSERT INTO pmp_items (
            ref, repo, number, kind, url, api_url, title, state, state_reason,
            draft, merged, author, labels, assignees, created_at, updated_at,
            closed_at, merged_at, source_updated_at, fetched_at
          ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
          ON CONFLICT(ref) DO UPDATE SET
            repo = excluded.repo,
            number = excluded.number,
            kind = excluded.kind,
            url = excluded.url,
            api_url = excluded.api_url,
            title = excluded.title,
            state = excluded.state,
            state_reason = excluded.state_reason,
            draft = excluded.draft,
            merged = excluded.merged,
            author = excluded.author,
            labels = excluded.labels,
            assignees = excluded.assignees,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            closed_at = excluded.closed_at,
            merged_at = excluded.merged_at,
            source_updated_at = excluded.source_updated_at,
            fetched_at = excluded.fetched_at
          WHERE excluded.source_updated_at >= pmp_items.source_updated_at`,
    args: [
      i.ref,
      i.repo,
      i.number,
      i.kind,
      i.url,
      i.api_url,
      i.title,
      i.state,
      i.state_reason,
      i.draft,
      i.merged,
      i.author,
      i.labels,
      i.assignees,
      i.created_at,
      i.updated_at,
      i.closed_at,
      i.merged_at,
      i.source_updated_at,
      now(),
    ],
  });
}

export async function getItems(db: Client, refs: string[]): Promise<ItemRow[]> {
  if (refs.length === 0) return [];
  const placeholders = refs.map(() => "?").join(",");
  const r = await db.execute({
    sql: `SELECT * FROM pmp_items WHERE ref IN (${placeholders}) ORDER BY updated_at DESC`,
    args: refs,
  });
  return r.rows as unknown as ItemRow[];
}

export async function listItems(
  db: Client,
  opts: { repo?: string; state?: string; kind?: string; limit?: number },
): Promise<ItemRow[]> {
  const clauses: string[] = [];
  const args: (string | number)[] = [];
  if (opts.repo) {
    clauses.push("repo = ?");
    args.push(opts.repo);
  }
  if (opts.state) {
    clauses.push("state = ?");
    args.push(opts.state);
  }
  if (opts.kind) {
    clauses.push("kind = ?");
    args.push(opts.kind);
  }
  const where = clauses.length ? `WHERE ${clauses.join(" AND ")}` : "";
  const limit = opts.limit ?? 200;
  const r = await db.execute({
    sql: `SELECT * FROM pmp_items ${where} ORDER BY updated_at DESC LIMIT ?`,
    args: [...args, limit],
  });
  return r.rows as unknown as ItemRow[];
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

export async function upsertComment(db: Client, c: Omit<CommentRow, "fetched_at">): Promise<void> {
  await db.execute({
    sql: `INSERT INTO pmp_comments (
            id, ref, kind, author, body, created_at, updated_at, url, path,
            line, diff_hunk, review_state, submitted_at, commit_id, deleted,
            source_updated_at, fetched_at
          ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
          ON CONFLICT(id, kind) DO UPDATE SET
            ref = excluded.ref,
            kind = excluded.kind,
            author = excluded.author,
            body = excluded.body,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            url = excluded.url,
            path = excluded.path,
            line = excluded.line,
            diff_hunk = excluded.diff_hunk,
            review_state = excluded.review_state,
            submitted_at = excluded.submitted_at,
            commit_id = excluded.commit_id,
            deleted = excluded.deleted,
            source_updated_at = excluded.source_updated_at,
            fetched_at = excluded.fetched_at
          WHERE excluded.source_updated_at >= pmp_comments.source_updated_at`,
    args: [
      c.id,
      c.ref,
      c.kind,
      c.author,
      c.body,
      c.created_at,
      c.updated_at,
      c.url,
      c.path,
      c.line,
      c.diff_hunk,
      c.review_state,
      c.submitted_at,
      c.commit_id,
      c.deleted,
      c.source_updated_at,
      now(),
    ],
  });
}

export async function markCommentDeleted(db: Client, id: string, kind: CommentRow["kind"]): Promise<void> {
  await db.execute({
    sql: `UPDATE pmp_comments SET deleted = 1, fetched_at = ? WHERE id = ? AND kind = ?`,
    args: [now(), id, kind],
  });
}

export async function getComments(
  db: Client,
  refs: string[],
  opts: { limit?: number; includeDeleted?: boolean } = {},
): Promise<CommentRow[]> {
  if (refs.length === 0) return [];
  const placeholders = refs.map(() => "?").join(",");
  const deleted = opts.includeDeleted ? "" : "AND deleted = 0";
  const r = await db.execute({
    sql: `SELECT * FROM pmp_comments
          WHERE ref IN (${placeholders}) ${deleted}
          ORDER BY created_at DESC, id DESC
          LIMIT ?`,
    args: [...refs, opts.limit ?? 500],
  });
  return r.rows as unknown as CommentRow[];
}

export async function getCommentsForSubject(
  db: Client,
  subject: string,
  opts: { limit?: number } = {},
): Promise<CommentRow[]> {
  const r = await db.execute({
    sql: `SELECT c.* FROM pmp_comments c
          JOIN pmp_links l ON l.ref = c.ref
          WHERE l.subject = ? AND c.deleted = 0
          ORDER BY c.created_at DESC, c.id DESC
          LIMIT ?`,
    args: [subject, opts.limit ?? 500],
  });
  return r.rows as unknown as CommentRow[];
}

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------

export async function upsertLink(
  db: Client,
  link: { subject: string; ref: string; relation: string; note?: string | null; createdBy?: string | null },
): Promise<void> {
  await db.execute({
    sql: `INSERT INTO pmp_links (subject, ref, relation, note, created_by, created_at)
          VALUES (?,?,?,?,?,?)
          ON CONFLICT(subject, ref, relation) DO UPDATE SET
            note = excluded.note,
            created_by = excluded.created_by,
            created_at = excluded.created_at`,
    args: [link.subject, link.ref, link.relation, link.note ?? null, link.createdBy ?? null, now()],
  });
}

export async function deleteLink(db: Client, subject: string, ref: string, relation: string): Promise<void> {
  await db.execute({
    sql: `DELETE FROM pmp_links WHERE subject = ? AND ref = ? AND relation = ?`,
    args: [subject, ref, relation],
  });
}

export async function getLinks(db: Client, opts: { subject?: string; ref?: string } = {}): Promise<LinkRow[]> {
  const clauses: string[] = [];
  const args: string[] = [];
  if (opts.subject) {
    clauses.push("subject = ?");
    args.push(opts.subject);
  }
  if (opts.ref) {
    clauses.push("ref = ?");
    args.push(opts.ref);
  }
  const where = clauses.length ? `WHERE ${clauses.join(" AND ")}` : "";
  const r = await db.execute({
    sql: `SELECT * FROM pmp_links ${where} ORDER BY created_at DESC`,
    args,
  });
  return r.rows as unknown as LinkRow[];
}

// ---------------------------------------------------------------------------
// Deliveries + sync state + push events
// ---------------------------------------------------------------------------

export async function deliveryApplied(db: Client, deliveryId: string): Promise<boolean> {
  const r = await db.execute({
    sql: `SELECT 1 AS x FROM pmp_deliveries WHERE delivery_id = ?`,
    args: [deliveryId],
  });
  return r.rows.length > 0;
}

export async function recordDelivery(
  db: Client,
  d: {
    deliveryId: string;
    event: string;
    action?: string | null;
    ref?: string | null;
    outcome: string;
    detail?: string | null;
  },
): Promise<void> {
  await db.execute({
    sql: `INSERT INTO pmp_deliveries (delivery_id, event, action, ref, received_at, outcome, detail)
          VALUES (?,?,?,?,?,?,?)`,
    args: [d.deliveryId, d.event, d.action ?? null, d.ref ?? null, now(), d.outcome, d.detail ?? null],
  });
}

export async function getSyncCursor(db: Client, repo: string, stream: string): Promise<string | null> {
  const r = await db.execute({
    sql: `SELECT cursor FROM pmp_sync_state WHERE repo = ? AND stream = ?`,
    args: [repo, stream],
  });
  return r.rows.length ? (r.rows[0] as unknown as { cursor: string | null }).cursor : null;
}

export async function setSyncCursor(db: Client, repo: string, stream: string, cursor: string | null): Promise<void> {
  await db.execute({
    sql: `INSERT INTO pmp_sync_state (repo, stream, cursor, updated_at)
          VALUES (?,?,?,?)
          ON CONFLICT(repo, stream) DO UPDATE SET cursor = excluded.cursor, updated_at = excluded.updated_at`,
    args: [repo, stream, cursor, now()],
  });
}

export async function recordPush(
  db: Client,
  p: {
    deliveryId: string;
    repo: string;
    ref: string | null;
    headSha: string | null;
    beforeSha: string | null;
    pusher: string | null;
  },
): Promise<void> {
  await db.execute({
    sql: `INSERT INTO pmp_push_events (delivery_id, repo, ref, head_sha, before_sha, pusher, created_at)
          VALUES (?,?,?,?,?,?,?)
          ON CONFLICT(delivery_id) DO NOTHING`,
    args: [p.deliveryId, p.repo, p.ref, p.headSha, p.beforeSha, p.pusher, now()],
  });
}

// ---------------------------------------------------------------------------
// Joined subject view (aggregator requirement #1 + #2 in one call)
// ---------------------------------------------------------------------------

export interface SubjectView {
  subject: string;
  links: (LinkRow & { item: ItemRow | null })[];
  activity: CommentRow[];
}

export async function getSubject(db: Client, subject: string): Promise<SubjectView | null> {
  const links = await getLinks(db, { subject });
  if (links.length === 0) return null;
  const refs = [...new Set(links.map((l) => l.ref))];
  const items = await getItems(db, refs);
  const itemByRef = new Map(items.map((i) => [i.ref, i]));
  const activity = await getCommentsForSubject(db, subject);
  return {
    subject,
    links: links.map((l) => ({ ...l, item: itemByRef.get(l.ref) ?? null })),
    activity,
  };
}
