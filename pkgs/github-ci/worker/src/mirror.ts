/**
 * GitHub mirror (#8580, fleet unification slice 2): the read model of
 * GitHub issue / PR state inside the fleet-queue database.
 *
 * The mirror is a READ MODEL, rebuildable from GitHub at any time (the
 * backfill/repair sweep reconciles it; webhooks only keep it warm). It
 * is never a second truth: no write path reads it to decide anything,
 * and staleness never gates work (#8575 ruling) — github_mirror_meta
 * exists so readers can SEE the staleness (last_webhook_at stamped
 * here, last_backfill_at stamped by the sweep), not act on it.
 *
 * Writes reuse the typed-args Turso pipeline client and the
 * fleet-queue credentials the Worker already holds for snapshots and
 * watermarks — no new secrets. Schema is ensured idempotently in the
 * same batch as each upsert (CREATE TABLE IF NOT EXISTS; the DDL also
 * ships as mirror.sql) so a fresh or rebuilt database self-heals from
 * any single webhook delivery.
 *
 * Webhook delivery order can differ from event order, so the issue/PR
 * upserts refuse a snapshot whose gh_updated_at is older than the
 * stored row's (an equal timestamp re-applies, keeping redelivery
 * idempotent; a NULL stored timestamp — a comment-seeded row — always
 * accepts the first full snapshot).
 *
 * The agent-note pointer follows the same discipline by comment id
 * (GitHub comment ids are issued in creation order): an older or
 * redelivered note never replaces a newer stored pointer. An issues
 * "deleted" delivery removes the mirror row outright: a deleted
 * issue must leave the read model, not be re-upserted by its own
 * deletion event (projector precedent, #4210).
 */
import { Turso, iArg, tArg, type TursoArg } from "./turso";
import { queueNow, type SinkEnv } from "./sink";

export const MIRROR_DDL: string[] = [
  `CREATE TABLE IF NOT EXISTS github_mirror_issues (
     repo TEXT NOT NULL, number INTEGER NOT NULL,
     title TEXT, body_hash TEXT, state TEXT,
     labels TEXT NOT NULL DEFAULT '[]',
     assignees TEXT NOT NULL DEFAULT '[]',
     comments INTEGER NOT NULL DEFAULT 0,
     latest_agent_note_id INTEGER, latest_agent_note_url TEXT,
     author TEXT, html_url TEXT,
     gh_created_at TEXT, gh_updated_at TEXT,
     mirrored_at TEXT NOT NULL,
     PRIMARY KEY (repo, number))`,
  `CREATE TABLE IF NOT EXISTS github_mirror_pull_requests (
     repo TEXT NOT NULL, number INTEGER NOT NULL,
     title TEXT, body_hash TEXT, state TEXT,
     merged INTEGER NOT NULL DEFAULT 0,
     draft INTEGER NOT NULL DEFAULT 0,
     head_sha TEXT, head_ref TEXT, base_ref TEXT,
     labels TEXT NOT NULL DEFAULT '[]',
     assignees TEXT NOT NULL DEFAULT '[]',
     comments INTEGER NOT NULL DEFAULT 0,
     latest_agent_note_id INTEGER, latest_agent_note_url TEXT,
     author TEXT, html_url TEXT,
     gh_created_at TEXT, gh_updated_at TEXT,
     mirrored_at TEXT NOT NULL,
     PRIMARY KEY (repo, number))`,
  `CREATE TABLE IF NOT EXISTS github_mirror_meta (
     repo TEXT PRIMARY KEY,
     last_webhook_at TEXT, last_webhook_event TEXT,
     last_backfill_at TEXT)`,
];

type Obj = Record<string, unknown>;

function str(o: Obj, k: string): string | null {
  const v = o[k];
  return typeof v === "string" ? v : null;
}

function num(o: Obj, k: string): number | null {
  const v = Number(o[k] ?? 0);
  return Number.isFinite(v) && v !== 0 ? v : null;
}

/** Label names, sorted — stable JSON so backfill diffs stay clean. */
export function labelsOf(obj: Obj): string[] {
  const raw = Array.isArray(obj.labels) ? obj.labels : [];
  return raw
    .map((l) => (typeof l === "string" ? l : str((l ?? {}) as Obj, "name")))
    .filter((n): n is string => !!n)
    .sort();
}

/** Assignee logins: the assignees array, falling back to the single assignee. */
export function assigneesOf(obj: Obj): string[] {
  const many = Array.isArray(obj.assignees) ? obj.assignees : [];
  const logins = many
    .map((a) => str((a ?? {}) as Obj, "login"))
    .filter((n): n is string => !!n);
  if (logins.length) return logins.sort();
  const one = str((obj.assignee ?? {}) as Obj, "login");
  return one ? [one] : [];
}

export interface IssueRow {
  repo: string;
  number: number | null;
  title: string | null;
  body_hash: string | null;
  state: string | null;
  labels: string;
  assignees: string;
  comments: number;
  author: string | null;
  html_url: string | null;
  gh_created_at: string | null;
  gh_updated_at: string | null;
}

export function buildIssueRow(
  payload: Obj,
  repo: string,
  bodyHash: string | null,
): IssueRow {
  const issue = (payload.issue ?? {}) as Obj;
  return {
    repo,
    number: num(issue, "number"),
    title: str(issue, "title"),
    body_hash: bodyHash,
    state: str(issue, "state"),
    labels: JSON.stringify(labelsOf(issue)),
    assignees: JSON.stringify(assigneesOf(issue)),
    comments: Number(issue.comments ?? 0) || 0,
    author: str((issue.user ?? {}) as Obj, "login"),
    html_url: str(issue, "html_url"),
    gh_created_at: str(issue, "created_at"),
    gh_updated_at: str(issue, "updated_at"),
  };
}

export interface PrRow extends IssueRow {
  merged: number;
  draft: number;
  head_sha: string | null;
  head_ref: string | null;
  base_ref: string | null;
}

export function buildPrRow(
  payload: Obj,
  repo: string,
  bodyHash: string | null,
): PrRow {
  const pr = (payload.pull_request ?? {}) as Obj;
  const base = buildIssueRow({ issue: pr }, repo, bodyHash);
  return {
    ...base,
    merged: pr.merged === true || typeof pr.merged_at === "string" ? 1 : 0,
    draft: pr.draft === true ? 1 : 0,
    head_sha: str((pr.head ?? {}) as Obj, "sha"),
    head_ref: str((pr.head ?? {}) as Obj, "ref"),
    base_ref: str((pr.base ?? {}) as Obj, "ref"),
  };
}

export interface CommentTouch {
  repo: string;
  number: number | null;
  isPr: boolean;
  comments: number;
  /** Set only when the comment carries the fleet agent-note marker. */
  agentNoteId: number | null;
  agentNoteUrl: string | null;
  /**
   * Set only on a `deleted` delivery whose comment was an agent note:
   * the pointer must never be assigned from a deletion, and a stored
   * pointer naming this comment is cleared (the sweep re-derives the
   * latest surviving note on its next pass).
   */
  deletedNoteId: number | null;
}

/** Agent notes carry the machine marker; the emoji header is the fallback. */
export function isAgentNote(body: string | null | undefined): boolean {
  if (!body) return false;
  return (
    body.includes("<!-- agent-note:v1") || body.startsWith("🤖 **Agent note**")
  );
}

export function buildCommentTouch(payload: Obj, repo: string): CommentTouch {
  const issue = (payload.issue ?? {}) as Obj;
  const comment = (payload.comment ?? {}) as Obj;
  const note = isAgentNote(str(comment, "body"));
  const deleted = payload.action === "deleted";
  return {
    repo,
    number: num(issue, "number"),
    isPr: !!issue.pull_request,
    comments: Number(issue.comments ?? 0) || 0,
    agentNoteId: note && !deleted ? num(comment, "id") : null,
    agentNoteUrl: note && !deleted ? str(comment, "html_url") : null,
    deletedNoteId: note && deleted ? num(comment, "id") : null,
  };
}

/** SHA-256 hex of the issue/PR body (change detection for the sweep). */
export async function bodyHashOf(obj: Obj): Promise<string | null> {
  const body = str(obj, "body");
  if (body === null) return null;
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(body),
  );
  return [...new Uint8Array(digest)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

interface Stmt {
  sql: string;
  args?: TursoArg[];
}

function metaStamp(repo: string, event: string, at: string): Stmt {
  return {
    sql: `INSERT INTO github_mirror_meta (repo, last_webhook_at, last_webhook_event)
          VALUES (?, ?, ?)
          ON CONFLICT(repo) DO UPDATE SET last_webhook_at = excluded.last_webhook_at,
            last_webhook_event = excluded.last_webhook_event`,
    args: [tArg(repo), tArg(at), tArg(event)],
  };
}

async function mirrorBatch(
  env: SinkEnv,
  event: string,
  repo: string,
  stmts: Stmt[],
): Promise<void> {
  const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
  const at = queueNow();
  await q.batch([
    ...MIRROR_DDL.map((sql) => ({ sql })),
    ...stmts,
    metaStamp(repo, event, at),
  ]);
}

/** Upsert the issue mirror row from an `issues` webhook payload. */
export async function mirrorIssueEvent(
  env: SinkEnv,
  payload: Obj,
  repo: string,
): Promise<void> {
  if (payload.action === "deleted") {
    const issue = (payload.issue ?? {}) as Obj;
    await mirrorBatch(env, "issues", repo, [
      {
        sql: "DELETE FROM github_mirror_issues WHERE repo = ? AND number = ?",
        args: [tArg(repo), iArg(num(issue, "number"))],
      },
    ]);
    return;
  }
  const row = buildIssueRow(
    payload,
    repo,
    await bodyHashOf((payload.issue ?? {}) as Obj),
  );
  await mirrorBatch(env, "issues", repo, [
    {
      sql: `INSERT INTO github_mirror_issues (repo, number, title, body_hash, state,
              labels, assignees, comments, author, html_url,
              gh_created_at, gh_updated_at, mirrored_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(repo, number) DO UPDATE SET
              title = excluded.title, body_hash = excluded.body_hash,
              state = excluded.state, labels = excluded.labels,
              assignees = excluded.assignees, comments = excluded.comments,
              author = excluded.author, html_url = excluded.html_url,
              gh_created_at = excluded.gh_created_at,
              gh_updated_at = excluded.gh_updated_at,
              mirrored_at = excluded.mirrored_at
            WHERE excluded.gh_updated_at IS NULL
               OR github_mirror_issues.gh_updated_at IS NULL
               OR excluded.gh_updated_at >= github_mirror_issues.gh_updated_at`,
      args: [
        tArg(row.repo),
        iArg(row.number),
        tArg(row.title),
        tArg(row.body_hash),
        tArg(row.state),
        tArg(row.labels),
        tArg(row.assignees),
        iArg(row.comments),
        tArg(row.author),
        tArg(row.html_url),
        tArg(row.gh_created_at),
        tArg(row.gh_updated_at),
        tArg(queueNow()),
      ],
    },
  ]);
}

/** Upsert the PR mirror row from a `pull_request` webhook payload. */
export async function mirrorPrEvent(
  env: SinkEnv,
  payload: Obj,
  repo: string,
): Promise<void> {
  const row = buildPrRow(
    payload,
    repo,
    await bodyHashOf((payload.pull_request ?? {}) as Obj),
  );
  await mirrorBatch(env, "pull_request", repo, [
    {
      sql: `INSERT INTO github_mirror_pull_requests (repo, number, title, body_hash,
              state, merged, draft, head_sha, head_ref, base_ref, labels,
              assignees, comments, author, html_url, gh_created_at,
              gh_updated_at, mirrored_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(repo, number) DO UPDATE SET
              title = excluded.title, body_hash = excluded.body_hash,
              state = excluded.state, merged = excluded.merged,
              draft = excluded.draft, head_sha = excluded.head_sha,
              head_ref = excluded.head_ref, base_ref = excluded.base_ref,
              labels = excluded.labels, assignees = excluded.assignees,
              comments = excluded.comments, author = excluded.author,
              html_url = excluded.html_url,
              gh_created_at = excluded.gh_created_at,
              gh_updated_at = excluded.gh_updated_at,
              mirrored_at = excluded.mirrored_at
            WHERE excluded.gh_updated_at IS NULL
               OR github_mirror_pull_requests.gh_updated_at IS NULL
               OR excluded.gh_updated_at >= github_mirror_pull_requests.gh_updated_at`,
      args: [
        tArg(row.repo),
        iArg(row.number),
        tArg(row.title),
        tArg(row.body_hash),
        tArg(row.state),
        iArg(row.merged),
        iArg(row.draft),
        tArg(row.head_sha),
        tArg(row.head_ref),
        tArg(row.base_ref),
        tArg(row.labels),
        tArg(row.assignees),
        iArg(row.comments),
        tArg(row.author),
        tArg(row.html_url),
        tArg(row.gh_created_at),
        tArg(row.gh_updated_at),
        tArg(queueNow()),
      ],
    },
  ]);
}

/**
 * Apply an `issue_comment` payload: refresh the comments count and, when
 * the comment is an agent note, the latest-agent-note pointer — replaced
 * only when the incoming note is at least as new as the stored one
 * (comment ids are creation order), so a delayed or redelivered older
 * note never overwrites the pointer to a newer note. Only the
 * comment fields move — title/state stay with the issues/PR events and
 * the backfill sweep. A row the mirror has never seen is seeded minimal
 * (the sweep fills the rest).
 */
export async function mirrorCommentEvent(
  env: SinkEnv,
  payload: Obj,
  repo: string,
): Promise<void> {
  const touch = buildCommentTouch(payload, repo);
  const table = touch.isPr
    ? "github_mirror_pull_requests"
    : "github_mirror_issues";
  await mirrorBatch(env, "issue_comment", repo, [
    {
      sql: `INSERT INTO ${table} (repo, number, mirrored_at) VALUES (?, ?, ?)
            ON CONFLICT(repo, number) DO NOTHING`,
      args: [tArg(touch.repo), iArg(touch.number), tArg(queueNow())],
    },
    {
      sql: `UPDATE ${table} SET comments = ?, mirrored_at = ?,
              latest_agent_note_id = CASE
                WHEN ? IS NOT NULL
                 AND (latest_agent_note_id IS NULL
                      OR ? >= latest_agent_note_id)
                THEN ? ELSE latest_agent_note_id END,
              latest_agent_note_url = CASE
                WHEN ? IS NOT NULL
                 AND (latest_agent_note_id IS NULL
                      OR ? >= latest_agent_note_id)
                THEN ? ELSE latest_agent_note_url END
            WHERE repo = ? AND number = ?`,
      args: [
        iArg(touch.comments),
        tArg(queueNow()),
        iArg(touch.agentNoteId),
        iArg(touch.agentNoteId),
        iArg(touch.agentNoteId),
        iArg(touch.agentNoteId),
        iArg(touch.agentNoteId),
        tArg(touch.agentNoteUrl),
        tArg(touch.repo),
        iArg(touch.number),
      ],
    },
    // A deleted agent note never becomes the pointer (above), and a
    // stored pointer naming the deleted comment is cleared here.
    ...(touch.deletedNoteId !== null
      ? [
          {
            sql: `UPDATE ${table} SET latest_agent_note_id = NULL,
                    latest_agent_note_url = NULL, mirrored_at = ?
                  WHERE repo = ? AND number = ? AND latest_agent_note_id = ?`,
            args: [
              tArg(queueNow()),
              tArg(touch.repo),
              iArg(touch.number),
              iArg(touch.deletedNoteId),
            ],
          },
        ]
      : []),
  ]);
}
