/**
 * Backfill / repair sweep for the GitHub mirror (#8580, task 16834).
 *
 * The mirror (src/mirror.ts) is a read model: webhooks keep it warm,
 * this sweep reconciles it from the GitHub API. One run per repo:
 *
 *   1. List every issue and PR (all states) from the REST API. The
 *      issues list also carries each PR's comment count, which the
 *      pulls list omits — the two are joined on number.
 *   2. Diff against the stored rows on (gh_updated_at, body_hash).
 *      Changed or missing items are re-upserted through the SAME
 *      statement builders the webhook paths use, so every guard
 *      (tombstone, version, comment-count versioning) applies
 *      identically. Unchanged items are not rewritten.
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
  clearAgentNotePointerStmt,
  commentTouchStmts,
  ensureMirrorSchema,
  isAgentNote,
  issueUpsertStmt,
  mirrorSweepBatch,
  prUpsertStmt,
  tombstoneDeleteStmts,
} from "../src/mirror";
import { FLEET_REPOS } from "../src/policy";
import { queueNow, type SinkEnv } from "../src/sink";
import { Turso } from "../src/turso";

type Obj = Record<string, unknown>;

const TURSO_QUEUE_URL =
  process.env.TURSO_QUEUE_URL ??
  "https://fleet-queue-dezren39.aws-us-east-1.turso.io";

const env: SinkEnv = {
  TURSO_QUEUE_URL,
  TURSO_QUEUE_TOKEN: process.env.TURSO_QUEUE_TOKEN ?? "",
};

const GH_TOKEN = process.env.GH_TOKEN ?? "";
if (!GH_TOKEN)
  throw new Error("GH_TOKEN is unset (see usage in the file header)");
if (!env.TURSO_QUEUE_TOKEN)
  throw new Error("TURSO_QUEUE_TOKEN is unset (see usage in the file header)");

/**
 * Retry transient transport failures (socket termination, 5xx) with
 * backoff. The fleet's laptop network drops long request sequences
 * regularly; a sweep is exactly that, so every call it makes goes
 * through here or through sweepBatch. HTTP 4xx is not retried.
 */
const retry = async <T>(label: string, fn: () => Promise<T>): Promise<T> => {
  let lastErr: unknown;
  for (let attempt = 1; attempt <= 5; attempt++) {
    try {
      return await fn();
    } catch (e) {
      lastErr = e;
      if (attempt < 5)
        await new Promise((r) => setTimeout(r, 500 * 2 ** attempt));
    }
  }
  throw new Error(`${label}: failed after 5 attempts: ${String(lastErr)}`);
};

const sweepBatch: typeof mirrorSweepBatch = (e, stmts) =>
  retry(`turso batch (${stmts.length} stmts)`, () =>
    mirrorSweepBatch(e, stmts),
  );

const api = async (path: string): Promise<unknown> =>
  retry(`github api ${path}`, async () => {
    const res = await fetch(`https://api.github.com${path}`, {
      // A wedged connection must fail into the retry loop in
      // seconds, not stall the sweep for minutes.
      signal: AbortSignal.timeout(20_000),
      headers: {
        Authorization: `Bearer ${GH_TOKEN}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "github-ci-mirror-backfill",
      },
    });
    if (res.status >= 500)
      throw new Error(`github api ${path}: HTTP ${res.status}`);
    if (!res.ok) throw new Error(`github api ${path}: HTTP ${res.status}`);
    return res.json();
  });

/** One list page plus the last page number from the Link header. */
const apiPage = async (
  path: string,
  page: number,
): Promise<{ items: Obj[]; lastPage: number }> => {
  const sep = path.includes("?") ? "&" : "?";
  const full = `${path}${sep}per_page=100&page=${page}`;
  return retry(`github api ${full}`, async () => {
    const res = await fetch(`https://api.github.com${full}`, {
      signal: AbortSignal.timeout(20_000),
      headers: {
        Authorization: `Bearer ${GH_TOKEN}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "github-ci-mirror-backfill",
      },
    });
    if (!res.ok) throw new Error(`github api ${full}: HTTP ${res.status}`);
    const link = res.headers.get("link") ?? "";
    const last = /[?&]page=(\d+)>; rel="last"/.exec(link);
    return {
      items: (await res.json()) as Obj[],
      lastPage: last ? Number(last[1]) : page,
    };
  });
};

/** Fetch every page of a list endpoint, pages 2..N four at a time. */
const apiAll = async (path: string): Promise<Obj[]> => {
  const first = await apiPage(path, 1);
  if (first.lastPage <= 1) return first.items;
  const pages: Obj[][] = [first.items];
  let next = 2;
  const worker = async (): Promise<void> => {
    while (next <= first.lastPage) {
      const page = next++;
      pages[page - 1] = (await apiPage(path, page)).items;
    }
  };
  await Promise.all([worker(), worker(), worker(), worker()]);
  return pages.flat();
};

interface StoredRow {
  number: number;
  ghUpdatedAt: string | null;
  bodyHash: string | null;
  comments: number;
  agentNoteId: number | null;
}

const storedRows = async (
  q: Turso,
  table: string,
  repo: string,
): Promise<Map<number, StoredRow>> => {
  const r = await retry(`read ${table}`, () =>
    q.execute(
      `SELECT number, gh_updated_at, body_hash, comments, latest_agent_note_id
         FROM ${table} WHERE repo = ?`,
      [{ type: "text", value: repo }],
    ),
  );
  const map = new Map<number, StoredRow>();
  for (const row of r.response?.result?.rows ?? []) {
    const cell = (i: number): string | null => {
      const c = row[i];
      return !c || c.type === "null"
        ? null
        : "value" in c
          ? String(c.value)
          : null;
    };
    const number = Number(cell(0));
    map.set(number, {
      number,
      ghUpdatedAt: cell(1),
      bodyHash: cell(2),
      comments: Number(cell(3) ?? 0) || 0,
      agentNoteId: cell(4) === null ? null : Number(cell(4)),
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
    pointers_set: 0,
    pointers_cleared: 0,
    pointers_deferred: 0,
    unchanged: 0,
  };
  const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
  const issueList = await apiAll(
    `/repos/${repo}/issues?state=all&sort=created&direction=asc`,
  );
  const pullList = await apiAll(
    `/repos/${repo}/pulls?state=all&sort=created&direction=asc`,
  );
  const commentsByNumber = new Map<number, number>();
  for (const item of issueList) {
    commentsByNumber.set(Number(item.number), Number(item.comments ?? 0) || 0);
  }
  const pureIssues = issueList.filter((i) => !i.pull_request);
  const storedIssues = await storedRows(q, "github_mirror_issues", repo);
  const storedPrs = await storedRows(q, "github_mirror_pull_requests", repo);
  summary.issues_seen = pureIssues.length;
  summary.prs_seen = pullList.length;

  const stmts: Parameters<typeof mirrorSweepBatch>[1] = [];
  const changed: { item: Obj; isPr: boolean }[] = [];
  const consider = async (
    item: Obj,
    isPr: boolean,
    stored: Map<number, StoredRow>,
  ): Promise<void> => {
    const number = Number(item.number);
    const prev = stored.get(number);
    const hash = await bodyHashOf(item);
    if (
      prev &&
      prev.ghUpdatedAt === String(item.updated_at ?? "") &&
      prev.bodyHash === hash
    ) {
      summary.unchanged++;
      return;
    }
    if (isPr) {
      const withComments = {
        ...item,
        comments: commentsByNumber.get(number) ?? Number(item.comments ?? 0),
      };
      stmts.push(
        prUpsertStmt(buildPrRow({ pull_request: withComments }, repo, hash)),
      );
    } else {
      stmts.push(issueUpsertStmt(buildIssueRow({ issue: item }, repo, hash)));
    }
    summary.upserted++;
    changed.push({ item, isPr });
  };
  for (const issue of pureIssues) await consider(issue, false, storedIssues);
  for (const pr of pullList) await consider(pr, true, storedPrs);

  // Vanished: stored numbers (one namespace per repo) the API did
  // not return at all — deleted on GitHub while no webhook watched.
  const seen = new Set<number>([
    ...issueList.map((i) => Number(i.number)),
    ...pullList.map((p) => Number(p.number)),
  ]);
  for (const number of new Set([...storedIssues.keys(), ...storedPrs.keys()])) {
    if (!seen.has(number)) {
      stmts.push(...tombstoneDeleteStmts(repo, number));
      summary.vanished_deleted++;
    }
  }
  // Flush snapshot writes in bounded batches before pointer work.
  for (let i = 0; i < stmts.length; i += 200) {
    await sweepBatch(env, stmts.slice(i, i + 200));
  }

  // Pointer re-derivation for changed items that carry comments,
  // most recently updated first, capped per run: a first sweep of a
  // large repo changes everything, and one comments listing per item
  // would exhaust the API budget repairing pointers for items nobody
  // has touched in years. Deferred items are counted, and any later
  // change to an item (or a rerun with a raised cap) repairs it.
  changed.sort((a, b) =>
    String(b.item.updated_at ?? "").localeCompare(
      String(a.item.updated_at ?? ""),
    ),
  );
  let pointerBudget = MAX_POINTER_REPAIRS;
  for (const { item, isPr } of changed) {
    const number = Number(item.number);
    if (
      (commentsByNumber.get(number) ?? 0) === 0 &&
      Number(item.comments ?? 0) === 0
    )
      continue;
    if (pointerBudget <= 0) {
      summary.pointers_deferred++;
      continue;
    }
    pointerBudget--;
    const comments = await apiAll(
      `/repos/${repo}/issues/${number}/comments?sort=created&direction=asc`,
    );
    const stored = (isPr ? storedPrs : storedIssues).get(number);
    const latestNote = [...comments]
      .reverse()
      .find((c) => isAgentNote(typeof c.body === "string" ? c.body : null));
    if (latestNote) {
      if (!stored || stored.agentNoteId !== Number(latestNote.id)) {
        const touch = buildCommentTouch(
          { issue: item, comment: latestNote, action: "created" },
          repo,
        );
        await sweepBatch(env, commentTouchStmts(touch));
        summary.pointers_set++;
      }
    } else if (
      stored?.agentNoteId != null &&
      !comments.some((c) => Number(c.id) === stored.agentNoteId)
    ) {
      await sweepBatch(env, [clearAgentNotePointerStmt(repo, number, isPr)]);
      summary.pointers_cleared++;
    }
  }

  await sweepBatch(env, [backfillStampStmt(repo, queueNow())]);
  return summary;
};

const MAX_POINTER_REPAIRS = Number(
  process.argv
    .find((a) => a.startsWith("--max-pointer-repairs="))
    ?.split("=")[1] ?? 400,
);

const main = async (): Promise<void> => {
  const repoArgs = process.argv.slice(2).filter((a) => !a.startsWith("--"));
  const repos = repoArgs.length > 0 ? repoArgs : [...FLEET_REPOS];
  await retry("ensure schema", () => ensureMirrorSchema(env));
  let failures = 0;
  for (const repo of repos) {
    try {
      console.log(JSON.stringify(await sweepRepo(repo)));
    } catch (e) {
      failures++;
      console.log(JSON.stringify({ repo, error: String(e) }));
    }
  }
  console.log(JSON.stringify({ done: true, repos: repos.length, failures }));
  if (failures > 0) process.exitCode = 1;
};

await main();
