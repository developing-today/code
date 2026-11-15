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
 * deletion event (projector precedent, #4210). The deletion also
 * leaves a tombstone (github_mirror_tombstones, keyed by repo +
 * number, carrying the deleted issue's updated_at): a delayed
 * pre-deletion snapshot or comment arriving after the deletion is
 * absorbed by the marker instead of resurrecting the row, in both
 * insertion paths — the snapshot upsert and the comment seed — as
 * the projector's deletionBlocks does (Codex P2, PR #8604). A
 * snapshot strictly newer than the marker (the issue genuinely
 * recreated or restored afterwards) is not blocked.
 *
 * The agent-note pointer keeps the same per-comment discipline for
 * deletions: clearing the pointer on a note's "deleted" delivery is
 * not enough, because the clear erases the only ordering state and a
 * delayed created/edited redelivery of that same comment would be
 * accepted through the empty-pointer branch and resurrect it. A
 * deleted delivery therefore also records the comment id in
 * github_mirror_deleted_notes, and the pointer CASE refuses any id
 * with a marker — statement-local, like the tombstone guard, so the
 * check is atomic with the update it guards (Codex P2, PR #8604).
 * (A genuinely recreated note always carries a new, higher comment
 * id, so the marker can never suppress a live note.)
 *
 * Comment counts carry their own version, stored separately from the
 * snapshot version (comments_gh_updated_at, NOT gh_updated_at): an
 * issue_comment delivery sets the count only when the carrying
 * object's updated_at is at least the stored count version, and
 * advances only that column. A comment event therefore never moves
 * the snapshot version, so a delayed older full snapshot (e.g. a
 * close) is never refused because a newer comment landed first, and
 * a comment-seeded row still accepts its first full snapshot. The
 * snapshot upserts move the count (and its version) under the same
 * count-version guard, so neither writer can regress the other's
 * count. (Same-second deliveries share a timestamp and stay
 * arrival-ordered; the backfill sweep is the repair path for that
 * residual, as for every mirror field.)
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
     comments_gh_updated_at TEXT,
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
     comments_gh_updated_at TEXT,
     latest_agent_note_id INTEGER, latest_agent_note_url TEXT,
     author TEXT, html_url TEXT,
     gh_created_at TEXT, gh_updated_at TEXT,
     mirrored_at TEXT NOT NULL,
     PRIMARY KEY (repo, number))`,
  `CREATE TABLE IF NOT EXISTS github_mirror_meta (
     repo TEXT PRIMARY KEY,
     last_webhook_at TEXT, last_webhook_event TEXT,
     last_backfill_at TEXT)`,
  `CREATE TABLE IF NOT EXISTS github_mirror_tombstones (
     repo TEXT NOT NULL, number INTEGER NOT NULL,
     gh_updated_at TEXT,
     mirrored_at TEXT NOT NULL,
     PRIMARY KEY (repo, number))`,
  `CREATE TABLE IF NOT EXISTS github_mirror_deleted_notes (
     repo TEXT NOT NULL, number INTEGER NOT NULL,
     comment_id INTEGER NOT NULL,
     mirrored_at TEXT NOT NULL,
     PRIMARY KEY (repo, number, comment_id))`,
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
  const logins = many.map((a) => str((a ?? {}) as Obj, "login")).filter((n): n is string => !!n);
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

export function buildIssueRow(payload: Obj, repo: string, bodyHash: string | null): IssueRow {
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
  /**
   * The version of the comments count, kept separate from the
   * snapshot version (gh_updated_at): the backfill sweep joins the
   * count from the issues-list object, whose updated_at is the
   * count's own version. Webhook payloads carry no separate field,
   * so buildPrRow falls back to the PR's updated_at — the count and
   * the snapshot then share one object and one version, as before.
   */
  comments_gh_updated_at: string | null;
}

export function buildPrRow(payload: Obj, repo: string, bodyHash: string | null): PrRow {
  const pr = (payload.pull_request ?? {}) as Obj;
  const base = buildIssueRow({ issue: pr }, repo, bodyHash);
  return {
    ...base,
    comments_gh_updated_at: str(pr, "comments_updated_at") ?? base.gh_updated_at,
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
  /** The carrying issue/PR object's updated_at: the count's version. */
  ghUpdatedAt: string | null;
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
  return body.includes("<!-- agent-note:v1") || body.startsWith("🤖 **Agent note**");
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
    ghUpdatedAt: str(issue, "updated_at"),
    agentNoteId: note && !deleted ? num(comment, "id") : null,
    agentNoteUrl: note && !deleted ? str(comment, "html_url") : null,
    deletedNoteId: note && deleted ? num(comment, "id") : null,
  };
}

/** SHA-256 hex of the issue/PR body (change detection for the sweep). */
export async function bodyHashOf(obj: Obj): Promise<string | null> {
  const body = str(obj, "body");
  if (body === null) return null;
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(body));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
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

const MIRROR_TABLES = ["github_mirror_issues", "github_mirror_pull_requests"];

/**
 * Idempotent column migration for databases created before
 * comments_gh_updated_at existed: CREATE TABLE IF NOT EXISTS cannot
 * add a column to an existing table, so probe pragma_table_info and
 * ALTER only when a column is absent. A null probe (no COUNT row)
 * means the table state is unreadable — skip and let the writes
 * surface any real error rather than guessing.
 */
async function ensureMirrorColumns(q: Turso): Promise<void> {
  const present = await q.queryText(
    `SELECT (SELECT COUNT(*) FROM pragma_table_info('github_mirror_issues') WHERE name = 'comments_gh_updated_at')
          + (SELECT COUNT(*) FROM pragma_table_info('github_mirror_pull_requests') WHERE name = 'comments_gh_updated_at')`,
  );
  // "2" = both columns present (the steady state); null = the probe
  // returned no row, so the table state is unreadable — skip and let
  // the writes surface any real error rather than guessing.
  if (present === null || present === "2") return;
  for (const table of MIRROR_TABLES) {
    try {
      await q.execute(`ALTER TABLE ${table} ADD COLUMN comments_gh_updated_at TEXT`);
    } catch (e) {
      // One table may already carry the column (mixed state); only a
      // duplicate-column error is ignorable here.
      if (!String(e).includes("duplicate column")) throw e;
    }
  }
}

/**
 * The tombstone rule (projector deletionBlocks, #6782, carried to
 * the mirror by Codex P2 on PR #8604): a deletion marker whose
 * version is at or after the incoming item's source version absorbs
 * the item. `markerVersion` is null when no marker row exists, and
 * the empty string when a marker row exists without a recorded
 * version — that marker still absorbs, because the deletion is a
 * fact even when its timestamp was not captured. An item whose own
 * version is unknown cannot prove it post-dates the deletion, so a
 * marker absorbs it too; only a strictly newer source version
 * passes.
 */
export function deletionBlocks(markerVersion: string | null, sourceUpdatedAt: string | null): boolean {
  if (markerVersion === null) return false;
  if (sourceUpdatedAt === null) return true;
  if (markerVersion === "") return true;
  return markerVersion >= sourceUpdatedAt;
}

/**
 * The tombstone guard: the SQL spelling of deletionBlocks, embedded
 * in every insertion path (the issue and PR snapshot upserts, the
 * comment seed, and the comment update). It binds, in order: repo,
 * number, source version, source version. Encoding the check in the
 * statement itself keeps it atomic with the write it guards — no
 * read-then-write window, and no extra pipeline round-trip.
 */
const TOMBSTONE_GUARD_SQL = `NOT EXISTS (
              SELECT 1 FROM github_mirror_tombstones m
              WHERE m.repo = ? AND m.number = ?
                AND (? IS NULL OR m.gh_updated_at IS NULL
                     OR m.gh_updated_at >= ?))`;

async function mirrorBatch(env: SinkEnv, event: string, repo: string, stmts: Stmt[]): Promise<void> {
  const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
  const at = queueNow();
  await q.batch(MIRROR_DDL.map((sql) => ({ sql })));
  await ensureMirrorColumns(q);
  await q.batch([...stmts, metaStamp(repo, event, at)]);
}

/**
 * Sweep support (#8580 remainder, task 16834): the backfill/repair
 * sweep (scripts/backfill.ts) reconciles the mirror from the GitHub
 * API. It reuses the same statement builders as the webhook paths —
 * one definition of every guard — but writes WITHOUT the webhook
 * meta stamp: last_webhook_at must keep meaning "a webhook landed",
 * so the sweep stamps only last_backfill_at, once per repo per run.
 */

/** Ensure the mirror schema without stamping any meta row. */
export async function ensureMirrorSchema(env: SinkEnv): Promise<void> {
  const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
  await q.batch(MIRROR_DDL.map((sql) => ({ sql })));
  await ensureMirrorColumns(q);
}

/** Run sweep statements (no meta stamp; see backfillStampStmt). */
export async function mirrorSweepBatch(env: SinkEnv, stmts: Stmt[]): Promise<void> {
  if (stmts.length === 0) return;
  const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
  await q.batch(stmts);
}

/** The sweep's meta stamp: last_backfill_at only, never the webhook fields. */
export function backfillStampStmt(repo: string, at: string): Stmt {
  return {
    sql: `INSERT INTO github_mirror_meta (repo, last_backfill_at)
          VALUES (?, ?)
          ON CONFLICT(repo) DO UPDATE SET last_backfill_at = excluded.last_backfill_at`,
    args: [tArg(repo), tArg(at)],
  };
}

/**
 * Remove an item the sweep found missing on GitHub: tombstone first
 * (a version-less marker absorbs every later insertion — GitHub never
 * reuses a number, so no genuine snapshot can post-date it), then the
 * row deletes in both tables (numbers share one namespace per repo).
 */
export function tombstoneDeleteStmts(repo: string, number: number): Stmt[] {
  return [
    {
      sql: `INSERT INTO github_mirror_tombstones (repo, number, gh_updated_at, mirrored_at)
            VALUES (?, ?, NULL, ?)
            ON CONFLICT(repo, number) DO UPDATE SET
              gh_updated_at = excluded.gh_updated_at,
              mirrored_at = excluded.mirrored_at`,
      args: [tArg(repo), iArg(number), tArg(queueNow())],
    },
    {
      sql: "DELETE FROM github_mirror_issues WHERE repo = ? AND number = ?",
      args: [tArg(repo), iArg(number)],
    },
    {
      sql: "DELETE FROM github_mirror_pull_requests WHERE repo = ? AND number = ?",
      args: [tArg(repo), iArg(number)],
    },
  ];
}

/**
 * Clear a stored agent-note pointer the sweep proved stale — the
 * pointed-at comment was deleted, or edited so it is no longer an
 * agent note. The clear is CONDITIONAL on the pointer still naming
 * the inspected comment: a webhook may have installed a newer note
 * after the sweep listed comments, and stale sweep work must never
 * erase it. When the comment is gone (not merely edited), its
 * deletion is also recorded in github_mirror_deleted_notes, so a
 * delayed created/edited delivery of that comment cannot resurrect
 * the pointer through the comment path's empty-pointer branch.
 */
/**
 * Sweep-only repair: set a row's comments count + version directly.
 * The version guard on the upserts is the right rule for webhooks,
 * but it cannot repair a count that a previous sweep wrote with a
 * WRONG count under a NEWER version stamp (the 2026-11-14 truncated
 * list run stamped pulls-list versions onto zeroed counts). The
 * write is conditional on the stored comments version still being
 * the exact stamp the sweep diffed against (guardVersion): if a
 * webhook has updated the row since, its version differs and this
 * statement is a no-op — a repair never overwrites a newer webhook
 * count with an older API count. The sweep re-reads the API count
 * immediately before repairing, so the value written is current.
 * Never used by a webhook path.
 */
export function forceCommentsStmt(
  repo: string,
  number: number,
  isPr: boolean,
  comments: number,
  commentsGhUpdatedAt: string | null,
  guardVersion: string | null,
): Stmt {
  const table = isPr ? "github_mirror_pull_requests" : "github_mirror_issues";
  return {
    sql: `UPDATE ${table} SET comments = ?, comments_gh_updated_at = ?, mirrored_at = ?
          WHERE repo = ? AND number = ? AND comments_gh_updated_at IS ?`,
    args: [iArg(comments), tArg(commentsGhUpdatedAt), tArg(queueNow()), tArg(repo), iArg(number), tArg(guardVersion)],
  };
}

export function clearAgentNotePointerStmts(
  repo: string,
  number: number,
  isPr: boolean,
  noteId: number,
  recordDeletion: boolean,
): Stmt[] {
  const table = isPr ? "github_mirror_pull_requests" : "github_mirror_issues";
  return [
    ...(recordDeletion
      ? [
          {
            sql: `INSERT INTO github_mirror_deleted_notes
                    (repo, number, comment_id, mirrored_at)
                  VALUES (?, ?, ?, ?)
                  ON CONFLICT(repo, number, comment_id) DO NOTHING`,
            args: [tArg(repo), iArg(number), iArg(noteId), tArg(queueNow())],
          },
        ]
      : []),
    {
      sql: `UPDATE ${table} SET latest_agent_note_id = NULL,
              latest_agent_note_url = NULL, mirrored_at = ?
            WHERE repo = ? AND number = ? AND latest_agent_note_id = ?`,
      args: [tArg(queueNow()), tArg(repo), iArg(number), iArg(noteId)],
    },
  ];
}

/** The issue snapshot upsert as a statement (shared by the webhook path and the sweep). */
export function issueUpsertStmt(row: IssueRow): Stmt {
  return {
    sql: `INSERT INTO github_mirror_issues (repo, number, title, body_hash, state,
            labels, assignees, comments, comments_gh_updated_at, author,
            html_url, gh_created_at, gh_updated_at, mirrored_at)
          SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
          WHERE ${TOMBSTONE_GUARD_SQL}
          ON CONFLICT(repo, number) DO UPDATE SET
            title = excluded.title, body_hash = excluded.body_hash,
            state = excluded.state, labels = excluded.labels,
            assignees = excluded.assignees,
            comments = CASE
              WHEN excluded.gh_updated_at IS NULL
                   OR github_mirror_issues.comments_gh_updated_at IS NULL
                   OR excluded.gh_updated_at >= github_mirror_issues.comments_gh_updated_at
              THEN excluded.comments ELSE github_mirror_issues.comments END,
            comments_gh_updated_at = CASE
              WHEN excluded.gh_updated_at IS NULL
                   OR github_mirror_issues.comments_gh_updated_at IS NULL
                   OR excluded.gh_updated_at >= github_mirror_issues.comments_gh_updated_at
              THEN COALESCE(excluded.gh_updated_at, github_mirror_issues.comments_gh_updated_at)
              ELSE github_mirror_issues.comments_gh_updated_at END,
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
      tArg(row.gh_updated_at),
      tArg(row.author),
      tArg(row.html_url),
      tArg(row.gh_created_at),
      tArg(row.gh_updated_at),
      tArg(queueNow()),
      tArg(row.repo),
      iArg(row.number),
      tArg(row.gh_updated_at),
      tArg(row.gh_updated_at),
    ],
  };
}

/** Upsert the issue mirror row from an `issues` webhook payload. */
export async function mirrorIssueEvent(env: SinkEnv, payload: Obj, repo: string): Promise<void> {
  if (payload.action === "deleted") {
    const issue = (payload.issue ?? {}) as Obj;
    // Order is the fix (Codex P2, PR #8604, two rounds): the
    // tombstone is written BEFORE the row is deleted. Turso.batch
    // is a sequence of separate executes, not a transaction, so a
    // DELETE-then-marker order leaves a window in which a delayed
    // pre-deletion delivery can insert after the DELETE but before
    // the marker exists — and the later marker then leaves that
    // resurrected row in place forever. Marker first closes the
    // window: once the marker lands, every insertion path is
    // guard-blocked (TOMBSTONE_GUARD_SQL), and the DELETE that
    // follows removes any row inserted before the marker landed.
    await mirrorBatch(env, "issues", repo, [
      {
        // The tombstone: evidence of the deletion, carrying the
        // deleted issue's version, so a delayed pre-deletion
        // delivery cannot resurrect the row (Codex P2, PR #8604).
        // A later deletion of a recreated issue refreshes it.
        sql: `INSERT INTO github_mirror_tombstones (repo, number, gh_updated_at, mirrored_at)
              VALUES (?, ?, ?, ?)
              ON CONFLICT(repo, number) DO UPDATE SET
                gh_updated_at = excluded.gh_updated_at,
                mirrored_at = excluded.mirrored_at`,
        args: [tArg(repo), iArg(num(issue, "number")), tArg(str(issue, "updated_at")), tArg(queueNow())],
      },
      {
        sql: "DELETE FROM github_mirror_issues WHERE repo = ? AND number = ?",
        args: [tArg(repo), iArg(num(issue, "number"))],
      },
    ]);
    return;
  }
  const row = buildIssueRow(payload, repo, await bodyHashOf((payload.issue ?? {}) as Obj));
  await mirrorBatch(env, "issues", repo, [issueUpsertStmt(row)]);
}
/** The PR snapshot upsert as a statement (shared by the webhook path and the sweep). */
export function prUpsertStmt(row: PrRow): Stmt {
  return {
    sql: `INSERT INTO github_mirror_pull_requests (repo, number, title, body_hash,
            state, merged, draft, head_sha, head_ref, base_ref, labels,
            assignees, comments, comments_gh_updated_at, author, html_url,
            gh_created_at, gh_updated_at, mirrored_at)
          SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
          WHERE ${TOMBSTONE_GUARD_SQL}
          ON CONFLICT(repo, number) DO UPDATE SET
            title = excluded.title, body_hash = excluded.body_hash,
            state = excluded.state, merged = excluded.merged,
            draft = excluded.draft, head_sha = excluded.head_sha,
            head_ref = excluded.head_ref, base_ref = excluded.base_ref,
            labels = excluded.labels, assignees = excluded.assignees,
            comments = CASE
              WHEN excluded.comments_gh_updated_at IS NULL
                   OR github_mirror_pull_requests.comments_gh_updated_at IS NULL
                   OR excluded.comments_gh_updated_at >= github_mirror_pull_requests.comments_gh_updated_at
              THEN excluded.comments ELSE github_mirror_pull_requests.comments END,
            comments_gh_updated_at = CASE
              WHEN excluded.comments_gh_updated_at IS NULL
                   OR github_mirror_pull_requests.comments_gh_updated_at IS NULL
                   OR excluded.comments_gh_updated_at >= github_mirror_pull_requests.comments_gh_updated_at
              THEN COALESCE(excluded.comments_gh_updated_at, github_mirror_pull_requests.comments_gh_updated_at)
              ELSE github_mirror_pull_requests.comments_gh_updated_at END,
            author = excluded.author,
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
      tArg(row.comments_gh_updated_at),
      tArg(row.author),
      tArg(row.html_url),
      tArg(row.gh_created_at),
      tArg(row.gh_updated_at),
      tArg(queueNow()),
      tArg(row.repo),
      iArg(row.number),
      tArg(row.gh_updated_at),
      tArg(row.gh_updated_at),
    ],
  };
}

/** Upsert the PR mirror row from a `pull_request` webhook payload. */
export async function mirrorPrEvent(env: SinkEnv, payload: Obj, repo: string): Promise<void> {
  const row = buildPrRow(payload, repo, await bodyHashOf((payload.pull_request ?? {}) as Obj));
  await mirrorBatch(env, "pull_request", repo, [prUpsertStmt(row)]);
}
/**
 * Apply an `issue_comment` payload: refresh the comments count and, when
 * the comment is an agent note, the latest-agent-note pointer — replaced
 * only when the incoming note is at least as new as the stored one
 * (comment ids are creation order), so a delayed or redelivered older
 * note never overwrites the pointer to a newer note. The comments
 * count is versioned separately (the carrying object's updated_at
 * against the stored comments_gh_updated_at — the snapshot version
 * gh_updated_at is never read or moved here), so a delayed delivery
 * cannot regress a newer count and a comment landing before a delayed
 * older snapshot cannot make that snapshot look stale. Only the
 * comment fields move — title/state stay with the issues/PR events and
 * the backfill sweep. A row the mirror has never seen is seeded minimal
 * (the sweep fills the rest).
 */
/** The comment-touch statements as a list (shared by the webhook path and the sweep). */
export function commentTouchStmts(touch: CommentTouch): Stmt[] {
  const table = touch.isPr ? "github_mirror_pull_requests" : "github_mirror_issues";
  return [
    {
      sql: `INSERT INTO ${table} (repo, number, mirrored_at) SELECT ?, ?, ?
          WHERE ${TOMBSTONE_GUARD_SQL}
          ON CONFLICT(repo, number) DO NOTHING`,
      args: [
        tArg(touch.repo),
        iArg(touch.number),
        tArg(queueNow()),
        tArg(touch.repo),
        iArg(touch.number),
        tArg(touch.ghUpdatedAt),
        tArg(touch.ghUpdatedAt),
      ],
    },
    {
      sql: `UPDATE ${table} SET
            comments = CASE
              WHEN ? IS NULL OR comments_gh_updated_at IS NULL
                   OR ? >= comments_gh_updated_at
              THEN ? ELSE comments END,
            comments_gh_updated_at = CASE
              WHEN ? IS NULL OR comments_gh_updated_at IS NULL
                   OR ? >= comments_gh_updated_at
              THEN COALESCE(?, comments_gh_updated_at)
              ELSE comments_gh_updated_at END,
            mirrored_at = ?,
            latest_agent_note_id = CASE
              WHEN ? IS NOT NULL
               AND NOT EXISTS (
                     SELECT 1 FROM github_mirror_deleted_notes d
                     WHERE d.repo = ? AND d.number = ? AND d.comment_id = ?)
               AND (latest_agent_note_id IS NULL
                    OR ? >= latest_agent_note_id)
              THEN ? ELSE latest_agent_note_id END,
            latest_agent_note_url = CASE
              WHEN ? IS NOT NULL
               AND NOT EXISTS (
                     SELECT 1 FROM github_mirror_deleted_notes d
                     WHERE d.repo = ? AND d.number = ? AND d.comment_id = ?)
               AND (latest_agent_note_id IS NULL
                    OR ? >= latest_agent_note_id)
              THEN ? ELSE latest_agent_note_url END
          WHERE repo = ? AND number = ? AND ${TOMBSTONE_GUARD_SQL}`,
      args: [
        tArg(touch.ghUpdatedAt),
        tArg(touch.ghUpdatedAt),
        iArg(touch.comments),
        tArg(touch.ghUpdatedAt),
        tArg(touch.ghUpdatedAt),
        tArg(touch.ghUpdatedAt),
        tArg(queueNow()),
        iArg(touch.agentNoteId),
        tArg(touch.repo),
        iArg(touch.number),
        iArg(touch.agentNoteId),
        iArg(touch.agentNoteId),
        iArg(touch.agentNoteId),
        iArg(touch.agentNoteId),
        tArg(touch.repo),
        iArg(touch.number),
        iArg(touch.agentNoteId),
        iArg(touch.agentNoteId),
        tArg(touch.agentNoteUrl),
        tArg(touch.repo),
        iArg(touch.number),
        tArg(touch.repo),
        iArg(touch.number),
        tArg(touch.ghUpdatedAt),
        tArg(touch.ghUpdatedAt),
      ],
    },
    // A deleted agent note never becomes the pointer (above), and a
    // stored pointer naming the deleted comment is cleared here.
    ...(touch.deletedNoteId !== null
      ? [
          {
            // Record the deletion itself: a delayed created/edited
            // redelivery of this comment must not resurrect the
            // pointer through the empty-pointer branch (Codex P2).
            sql: `INSERT INTO github_mirror_deleted_notes
                  (repo, number, comment_id, mirrored_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(repo, number, comment_id) DO NOTHING`,
            args: [tArg(touch.repo), iArg(touch.number), iArg(touch.deletedNoteId), tArg(queueNow())],
          },
          {
            sql: `UPDATE ${table} SET latest_agent_note_id = NULL,
                  latest_agent_note_url = NULL, mirrored_at = ?
                WHERE repo = ? AND number = ? AND latest_agent_note_id = ?`,
            args: [tArg(queueNow()), tArg(touch.repo), iArg(touch.number), iArg(touch.deletedNoteId)],
          },
        ]
      : []),
  ];
}

export async function mirrorCommentEvent(env: SinkEnv, payload: Obj, repo: string): Promise<void> {
  const touch = buildCommentTouch(payload, repo);
  await mirrorBatch(env, "issue_comment", repo, commentTouchStmts(touch));
}
