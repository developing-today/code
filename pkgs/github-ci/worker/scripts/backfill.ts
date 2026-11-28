/**
 * Backfill / repair sweep for the GitHub mirror (#8580, task 16834).
 *
 * The mirror (src/mirror.ts) is a read model: webhooks keep it warm,
 * this sweep reconciles it from the GitHub API. One run per repo:
 *
 *   1. List every issue and PR (all states) from the REST API. The
 *      issues list also carries each PR's comment count, which the
 *      pulls list omits — the two are joined on number.
 *   2. Diff against the stored rows on (gh_updated_at, body_hash,
 *      comments, parent). Changed or missing items are re-upserted
 *      through the SAME statement builders the webhook paths use, so
 *      every guard (tombstone, version, comment-count versioning)
 *      applies identically. Unchanged items are not rewritten. For
 *      items that already had a stored row, the diff also produces
 *      the #14170 Q7 history rows (body edits, count deltas, parent
 *      changes) a missed webhook would have written — actor
 *      "backfill" — flushed fail-open after the snapshots: a failed
 *      history write never fails the sweep.
 *   3. Stored numbers the API no longer returns are tombstoned and
 *      deleted (GitHub deletes are the one event class a missed
 *      webhook can never replay).
 *   4. For every changed item with comments, the item's comments are
 *      listed and the latest agent-note pointer is re-derived: a
 *      surviving latest note is applied through the comment-touch
 *      statements; a stored pointer naming a comment that no longer
 *      exists is cleared.
 *   5. github_mirror_meta.last_backfill_at is stamped for the repo.
 *      The webhook fields are never touched by the sweep.
 *
 * Usage (from pkgs/github-ci/worker):
 *   GH_TOKEN=$(gh auth token) \
 *   TURSO_QUEUE_TOKEN=$(fleet secrets get turso.fleet-queue-rw-token) \
 *   npm run backfill -- [--max-pointer-repairs=N] [owner/repo ...]
 * With no arguments every FLEET_REPOS repo is swept; a repo that
 * errors (renamed, no access) is reported and the sweep continues.
 * Output is one JSON summary line per repo plus a totals line.
 */
import {
  backfillStampStmt,
  bodyHashOf,
  buildCommentTouch,
  buildIssueRow,
  buildPrRow,
  clearAgentNotePointerStmts,
  commentTouchStmts,
  forceCommentsStmt,
  ensureMirrorSchema,
  historyInsertStmt,
  isAgentNote,
  labelStartStmt,
  labelHistoryInsertStmt,
  snapshotLabelHistory,
  issueUpsertStmt,
  mirrorSweepBatch,
  parentOf,
  prUpsertStmt,
  snapshotHistory,
  tombstoneDeleteStmts,
  type IssueRow,
  type PrRow,
  type StoredMirrorRow,
} from "../src/mirror";
import { FLEET_REPOS } from "../src/policy";
import { queueNow, type SinkEnv } from "../src/sink";
import { Turso } from "../src/turso";

type Obj = Record<string, unknown>;

const TURSO_QUEUE_URL = process.env.TURSO_QUEUE_URL ?? "https://d1-gateway.developing-today.workers.dev/db/fleet-queue";

const env: SinkEnv = {
  TURSO_QUEUE_URL,
  TURSO_QUEUE_TOKEN: process.env.TURSO_QUEUE_TOKEN ?? "",
};

const GH_TOKEN = process.env.GH_TOKEN ?? "";
if (!GH_TOKEN) throw new Error("GH_TOKEN is unset (see usage in the file header)");
if (!env.TURSO_QUEUE_TOKEN) throw new Error("TURSO_QUEUE_TOKEN is unset (see usage in the file header)");

/**
 * Retry transient transport failures (socket termination, 5xx) with
 * backoff. The fleet's laptop network drops long request sequences
 * regularly; a sweep is exactly that, so every call it makes goes
 * through here or through sweepBatch. HTTP 4xx is not retried.
 */
/** An HTTP error response; retry() never retries a 4xx. */
class HttpStatusError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

const retry = async <T>(label: string, fn: () => Promise<T>): Promise<T> => {
  let lastErr: unknown;
  for (let attempt = 1; attempt <= 5; attempt++) {
    try {
      return await fn();
    } catch (e) {
      if (e instanceof HttpStatusError && e.status < 500) throw e;
      lastErr = e;
      if (attempt < 5) await new Promise((r) => setTimeout(r, 500 * 2 ** attempt));
    }
  }
  throw new Error(`${label}: failed after 5 attempts: ${String(lastErr)}`);
};

const sweepBatch: typeof mirrorSweepBatch = (e, stmts) =>
  retry(`turso batch (${stmts.length} stmts)`, () => mirrorSweepBatch(e, stmts));

const api = async (path: string): Promise<unknown> =>
  retry(`github api ${path}`, async () => {
    const res = await fetch(`https://api.github.com${path}`, {
      // A wedged connection must fail into the retry loop in
      // seconds, not stall the sweep for minutes.
      signal: AbortSignal.timeout(60_000),
      headers: {
        Authorization: `Bearer ${GH_TOKEN}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "github-ci-mirror-backfill",
      },
    });
    if (res.status >= 500) throw new Error(`github api ${path}: HTTP ${res.status}`);
    if (!res.ok) throw new HttpStatusError(res.status, `github api ${path}: HTTP ${res.status}`);
    return res.json();
  });

/**
 * Fetch every page of a list endpoint by following Link rel="next"
 * URLs verbatim. The issues endpoint is cursor-paginated (the Link
 * header carries only an `after` cursor, no rel="last"), while the
 * pulls endpoint still uses page numbers — following `next` is the
 * one scheme that is correct for both. (The first version of this
 * sweep parsed rel="last" and silently stopped after page 1 of the
 * issues list: 2 pure issues out of 5430. The summary's seen counts
 * are what caught it.)
 */
const apiAll = async (path: string): Promise<Obj[]> => {
  const out: Obj[] = [];
  let url: string | null = `https://api.github.com${path}${path.includes("?") ? "&" : "?"}per_page=100`;
  while (url) {
    const page: { items: Obj[]; next: string | null } = await retry(`github api ${url}`, async () => {
      const res = await fetch(url, {
        signal: AbortSignal.timeout(60_000),
        headers: {
          Authorization: `Bearer ${GH_TOKEN}`,
          Accept: "application/vnd.github+json",
          "X-GitHub-Api-Version": "2022-11-28",
          "User-Agent": "github-ci-mirror-backfill",
        },
      });
      if (!res.ok) throw new HttpStatusError(res.status, `github api ${url}: HTTP ${res.status}`);
      const link = res.headers.get("link") ?? "";
      const next = /<([^>]+)>; rel="next"/.exec(link);
      return {
        items: (await res.json()) as Obj[],
        next: next ? next[1] : null,
      };
    });
    out.push(...page.items);
    url = page.next;
  }
  return out;
};

/** HTTP status of a single GET (404 is an answer, not an error). */
const apiStatus = async (path: string): Promise<number> =>
  retry(`github api ${path}`, async () => {
    const res = await fetch(`https://api.github.com${path}`, {
      signal: AbortSignal.timeout(60_000),
      headers: {
        Authorization: `Bearer ${GH_TOKEN}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "github-ci-mirror-backfill",
      },
    });
    if (res.status >= 500) throw new Error(`github api ${path}: HTTP ${res.status}`);
    return res.status;
  });

interface StoredRow {
  number: number;
  ghUpdatedAt: string | null;
  bodyHash: string | null;
  comments: number;
  commentsGhUpdatedAt: string | null;
  agentNoteId: number | null;
  /** The sub-issue parent projection; issues table only (null for PRs). */
  parentNumber: number | null;
  /** The stored label set as JSON text (#14170 Q5). */
  labels: string | null;
}

const storedRows = async (
  q: Turso,
  table: string,
  repo: string,
  withParent = false,
): Promise<Map<number, StoredRow>> => {
  const r = await retry(`read ${table}`, () =>
    q.execute(
      `SELECT number, gh_updated_at, body_hash, comments, comments_gh_updated_at, latest_agent_note_id,
              ${withParent ? "parent_number" : "NULL"}, labels
         FROM ${table} WHERE repo = ?`,
      [{ type: "text", value: repo }],
    ),
  );
  const map = new Map<number, StoredRow>();
  for (const row of r.response?.result?.rows ?? []) {
    const cell = (i: number): string | null => {
      const c = row[i];
      return !c || c.type === "null" ? null : "value" in c ? String(c.value) : null;
    };
    const number = Number(cell(0));
    map.set(number, {
      number,
      ghUpdatedAt: cell(1),
      bodyHash: cell(2),
      comments: Number(cell(3) ?? 0) || 0,
      commentsGhUpdatedAt: cell(4),
      agentNoteId: cell(5) === null ? null : Number(cell(5)),
      parentNumber: cell(6) === null ? null : Number(cell(6)),
      labels: cell(7),
    });
  }
  return map;
};

interface RepoSummary {
  repo: string;
  issues_seen: number;
  prs_seen: number;
  upserted: number;
  vanished_deleted: number;
  pointers_set: number;
  pointers_cleared: number;
  pointers_deferred: number;
  pointers_failed: number;
  counts_forced: number;
  counts_repair_failed: number;
  history_rows: number;
  history_failed: number;
  /** #14170 Q5 label-diff rows written (start rows are set-based and not counted). */
  label_history_rows: number;
  incomplete?: boolean;
  vanished_skipped_live: number;
  unchanged: number;
  error?: string;
}

const sweepRepo = async (repo: string): Promise<RepoSummary> => {
  const summary: RepoSummary = {
    repo,
    issues_seen: 0,
    prs_seen: 0,
    upserted: 0,
    vanished_deleted: 0,
    vanished_skipped_live: 0,
    pointers_set: 0,
    pointers_cleared: 0,
    pointers_deferred: 0,
    pointers_failed: 0,
    counts_forced: 0,
    counts_repair_failed: 0,
    history_rows: 0,
    history_failed: 0,
    label_history_rows: 0,
    unchanged: 0,
  };
  const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
  const issueList = await apiAll(`/repos/${repo}/issues?state=all&sort=created&direction=asc`);
  const pullList = await apiAll(`/repos/${repo}/pulls?state=all&sort=created&direction=asc`);
  const issueByNumber = new Map<number, Obj>();
  for (const item of issueList) issueByNumber.set(Number(item.number), item);
  const commentsByNumber = new Map<number, number>();
  for (const item of issueList) {
    commentsByNumber.set(Number(item.number), Number(item.comments ?? 0) || 0);
  }
  const pureIssues = issueList.filter((i) => !i.pull_request);
  const storedIssues = await storedRows(q, "github_mirror_issues", repo, true);
  const storedPrs = await storedRows(q, "github_mirror_pull_requests", repo);
  summary.issues_seen = pureIssues.length;
  summary.prs_seen = pullList.length;

  interface Pending {
    number: number;
    isPr: boolean;
    item: Obj;
    stmt: { sql: string; args?: import("../src/turso").TursoArg[] };
    /** Q7 history rows this item's diff implies (empty on first sight). */
    history: { sql: string; args?: import("../src/turso").TursoArg[] }[];
    /** Q5 label-diff rows (empty on first sight; start rows are set-based). */
    labelHistory: { sql: string; args?: import("../src/turso").TursoArg[] }[];
  }
  interface CountRepair {
    number: number;
    isPr: boolean;
    /** The stored comments version at diff time — the stamp the repair replaces. */
    guardVersion: string | null;
  }
  const countRepairs: CountRepair[] = [];
  const pending: Pending[] = [];
  const consider = async (item: Obj, isPr: boolean, stored: Map<number, StoredRow>): Promise<void> => {
    const number = Number(item.number);
    const prev = stored.get(number);
    const hash = await bodyHashOf(item);
    // The diff key is (gh_updated_at, body_hash, comments, parent):
    // the count must be in the key because a comment does not move
    // a PR's updated_at, and a count written wrong under a newer
    // version stamp is invisible to the other two fields; the
    // parent (issues only, #14170 Q7) is in the key because a
    // re-parenting can be an issue's only change, and the first
    // sweep after Q7 must populate the projection on rows whose
    // parent was never observed.
    const apiComments = isPr
      ? (commentsByNumber.get(number) ?? Number(item.comments ?? 0))
      : Number(item.comments ?? 0);
    const parent = isPr ? { known: false, parent: null } : parentOf(item);
    const parentMoved = !!prev && parent.known && prev.parentNumber !== parent.parent;
    if (
      prev &&
      prev.ghUpdatedAt === String(item.updated_at ?? "") &&
      prev.bodyHash === hash &&
      prev.comments === apiComments &&
      !parentMoved
    ) {
      summary.unchanged++;
      return;
    }
    if (prev && prev.comments !== apiComments) {
      countRepairs.push({
        number,
        isPr,
        guardVersion: prev.commentsGhUpdatedAt,
      });
    }
    let row: IssueRow | PrRow;
    let stmt: Pending["stmt"];
    if (isPr) {
      // Join the comment count from the issues-list object and keep
      // THAT object's updated_at as the count's version: the two
      // lists are fetched seconds apart, and pairing the count with
      // the pulls-list version would let a stale count win the
      // version guard (review finding, PR #10062).
      const issueObj = issueByNumber.get(number);
      const withComments = {
        ...item,
        comments: commentsByNumber.get(number) ?? Number(item.comments ?? 0),
        comments_updated_at: issueObj ? String(issueObj.updated_at ?? "") : undefined,
      };
      row = buildPrRow({ pull_request: withComments }, repo, hash);
      stmt = prUpsertStmt(row as PrRow);
    } else {
      row = buildIssueRow({ issue: item }, repo, hash);
      stmt = issueUpsertStmt(row);
    }
    // Q7 history for the changes this upsert lands, diffed against
    // the stored row exactly as the webhook path diffs (the shared
    // snapshotHistory applies the same acceptance rules, so a sweep
    // item the upsert's guards would refuse yields no rows). First
    // sights (no prev) write none.
    let history: Pending["history"] = [];
    let labelHistory: Pending["labelHistory"] = [];
    if (prev) {
      const storedView: StoredMirrorRow = {
        bodyHash: prev.bodyHash,
        comments: prev.comments,
        commentsGhUpdatedAt: prev.commentsGhUpdatedAt,
        ghUpdatedAt: prev.ghUpdatedAt,
        parentNumber: prev.parentNumber,
        tombstoneVersion: null,
        labels: prev.labels,
      };
      const countVersion = isPr ? (row as PrRow).comments_gh_updated_at : row.gh_updated_at;
      history = snapshotHistory(storedView, row, countVersion, isPr, "backfill").map(historyInsertStmt);
      // Q5: the label diff a missed labeled/unlabeled webhook would
      // have written. A row whose labels were never observed gets
      // its start row from the set-based pass after the flush.
      labelHistory = snapshotLabelHistory(storedView, row, isPr, "backfill", "backfill").map(labelHistoryInsertStmt);
    }
    pending.push({ number, isPr, item, stmt, history, labelHistory });
  };
  for (const issue of pureIssues) await consider(issue, false, storedIssues);
  for (const pr of pullList) await consider(pr, true, storedPrs);

  // Vanished: stored numbers (one namespace per repo) the API lists
  // did not return. Absence from a paginated list is NOT proof of
  // deletion — an item created after the list was fetched but before
  // the stored rows were read would look identical, and a NULL
  // tombstone is permanent. Confirm each candidate against the item
  // endpoint first; only a 404 tombstones.
  const seen = new Set<number>([...issueList.map((i) => Number(i.number)), ...pullList.map((p) => Number(p.number))]);
  const flush: Pending["stmt"][] = [];
  for (const number of new Set([...storedIssues.keys(), ...storedPrs.keys()])) {
    if (seen.has(number)) continue;
    if ((await apiStatus(`/repos/${repo}/issues/${number}`)) === 404) {
      flush.push(...tombstoneDeleteStmts(repo, number));
      summary.vanished_deleted++;
    } else {
      summary.vanished_skipped_live++;
    }
  }

  // Pointer re-derivation runs BEFORE the snapshot flush, and a
  // failed repair keeps its item out of the flush: the item then
  // still diffs as changed on the next run, so the repair stays
  // eligible instead of being hidden by snapshot equality.
  const failed = new Set<number>();
  const candidates = pending
    .filter((p) => {
      const stored = (p.isPr ? storedPrs : storedIssues).get(p.number);
      return (commentsByNumber.get(p.number) ?? 0) > 0 || stored?.agentNoteId != null;
    })
    .sort((a, b) => String(b.item.updated_at ?? "").localeCompare(String(a.item.updated_at ?? "")));
  let pointerBudget = MAX_POINTER_REPAIRS;
  for (const cand of candidates) {
    const { number, isPr, item } = cand;
    if (pointerBudget <= 0) {
      summary.pointers_deferred++;
      continue;
    }
    pointerBudget--;
    try {
      const comments = await apiAll(`/repos/${repo}/issues/${number}/comments?sort=created&direction=asc`);
      const stored = (isPr ? storedPrs : storedIssues).get(number);
      const isNote = (c: Obj): boolean => isAgentNote(typeof c.body === "string" ? c.body : null);
      const latestNote = [...comments].reverse().find(isNote);
      let pointerNow = stored?.agentNoteId ?? null;
      if (pointerNow != null) {
        const pointed = comments.find((c) => Number(c.id) === pointerNow);
        if (!pointed || !isNote(pointed)) {
          // Stale pointer: the comment is deleted, or was edited so
          // it is no longer an agent note. Clear conditionally (a
          // newer webhook-installed pointer is never erased), and
          // record the deletion only when the comment is truly gone.
          await sweepBatch(env, clearAgentNotePointerStmts(repo, number, isPr, pointerNow, !pointed));
          summary.pointers_cleared++;
          pointerNow = null;
        }
      }
      if (latestNote && Number(latestNote.id) !== pointerNow) {
        // buildCommentTouch reads the discriminator + count off the
        // carrying object: a pulls-list object has neither, so PR
        // items are adapted (explicit pull_request marker + the
        // joined count) or the touch would land on the issues table.
        const touchIssue = isPr
          ? {
              ...item,
              comments: commentsByNumber.get(number) ?? 0,
              pull_request: { url: String(item.url ?? "") },
            }
          : item;
        const touch = buildCommentTouch({ issue: touchIssue, comment: latestNote, action: "created" }, repo);
        await sweepBatch(env, commentTouchStmts(touch));
        summary.pointers_set++;
      }
    } catch {
      failed.add(number);
      summary.pointers_failed++;
    }
  }

  // Q5 label START rows for rows observed before this run that have
  // no label history yet (rows mirrored before Q5): set-based, one
  // statement per table, written BEFORE the snapshot flush so the
  // start carries the pre-run set and any diff row below follows it.
  // Fail-open like all history.
  const labelStarts = (): Pending["stmt"][] => [
    labelStartStmt(repo, null, false, "backfill", "backfill"),
    labelStartStmt(repo, null, true, "backfill", "backfill"),
  ];
  try {
    await sweepBatch(env, labelStarts());
  } catch (e) {
    summary.history_failed++;
    console.error(`backfill ${repo}: label start rows failed open: ${String(e)}`);
  }

  const historyFlush: Pending["stmt"][] = [];
  let labelRows = 0;
  for (const p of pending) {
    if (!failed.has(p.number)) {
      flush.push(p.stmt);
      summary.upserted++;
      // History rides only with an upsert that actually flushed: an
      // item withheld for a failed pointer repair keeps its diff —
      // and its history — eligible for the next run.
      historyFlush.push(...p.history, ...p.labelHistory);
      labelRows += p.labelHistory.length;
    }
  }
  for (let i = 0; i < flush.length; i += 200) {
    await sweepBatch(env, flush.slice(i, i + 200));
  }

  // Q7 history flush, FAIL-OPEN (design §10.3): history never gates
  // the sweep, so a failed history batch is counted and logged, not
  // propagated — the repo is not marked incomplete for it.
  if (historyFlush.length > 0) {
    try {
      for (let i = 0; i < historyFlush.length; i += 200) {
        await sweepBatch(env, historyFlush.slice(i, i + 200));
      }
      summary.history_rows += historyFlush.length - labelRows;
      summary.label_history_rows += labelRows;
    } catch (e) {
      summary.history_failed++;
      console.error(`backfill ${repo}: history write failed open: ${String(e)}`);
    }
  }
  // Q5 start rows for this run's first sights (and minimal seeds the
  // snapshot flush just filled in), now that their sets are stored.
  try {
    await sweepBatch(env, labelStarts());
  } catch (e) {
    summary.history_failed++;
    console.error(`backfill ${repo}: label start rows failed open: ${String(e)}`);
  }

  // Count-repair pass: for every row whose count mismatched at diff
  // time and that the version guard still rejected, re-fetch the
  // item's CURRENT count from the API and force-set it — guarded on
  // the stored comments version still being the diff-time stamp, so
  // a webhook update that landed in between is never overwritten by
  // an older list count.
  const afterIssues = countRepairs.some((r) => !r.isPr) ? await storedRows(q, "github_mirror_issues", repo) : null;
  const afterPrs = countRepairs.some((r) => r.isPr) ? await storedRows(q, "github_mirror_pull_requests", repo) : null;
  for (const repair of countRepairs) {
    const now = (repair.isPr ? afterPrs : afterIssues)?.get(repair.number);
    if (!now) continue;
    try {
      const fresh = (await api(`/repos/${repo}/issues/${repair.number}`)) as Obj;
      const freshCount = Number(fresh.comments ?? 0) || 0;
      if (now.comments !== freshCount && now.commentsGhUpdatedAt === repair.guardVersion) {
        await sweepBatch(env, [
          forceCommentsStmt(
            repo,
            repair.number,
            repair.isPr,
            freshCount,
            typeof fresh.updated_at === "string" ? fresh.updated_at : null,
            repair.guardVersion,
          ),
        ]);
        summary.counts_forced++;
        // The forced count is a change the sweep observed: it gets
        // its history row too, fail-open like all history.
        try {
          await sweepBatch(env, [
            historyInsertStmt({
              repo,
              number: repair.number,
              isPr: repair.isPr,
              kind: "comment_count_changed",
              detail: { from: now.comments, to: freshCount },
              actor: "backfill",
              ghUpdatedAt: typeof fresh.updated_at === "string" ? fresh.updated_at : null,
            }),
          ]);
          summary.history_rows++;
        } catch (e) {
          summary.history_failed++;
          console.error(`backfill ${repo}: repair history write failed open: ${String(e)}`);
        }
      }
    } catch {
      summary.counts_repair_failed++;
    }
  }

  // A run with failed repairs is incomplete: partial progress is
  // kept (snapshots flushed, successful repairs applied) but the
  // backfill stamp is withheld and main() reports the repo as failed,
  // so the next run re-diffs the same items and retries.
  summary.incomplete = summary.pointers_failed > 0 || summary.counts_repair_failed > 0;
  if (!summary.incomplete) {
    await sweepBatch(env, [backfillStampStmt(repo, queueNow())]);
  }
  return summary;
};

const MAX_POINTER_REPAIRS = Number(
  process.argv.find((a) => a.startsWith("--max-pointer-repairs="))?.split("=")[1] ?? 400,
);

const main = async (): Promise<void> => {
  const repoArgs = process.argv.slice(2).filter((a) => !a.startsWith("--"));
  const repos = repoArgs.length > 0 ? repoArgs : [...FLEET_REPOS];
  await retry("ensure schema", () => ensureMirrorSchema(env));
  let failures = 0;
  for (const repo of repos) {
    try {
      const s = await sweepRepo(repo);
      console.log(JSON.stringify(s));
      if (s.incomplete) failures++;
    } catch (e) {
      failures++;
      console.log(JSON.stringify({ repo, error: String(e) }));
    }
  }
  console.log(JSON.stringify({ done: true, repos: repos.length, failures }));
  if (failures > 0) process.exitCode = 1;
};

await main();
