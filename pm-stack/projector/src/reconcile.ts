// Reconciliation reads: poll GitHub via the App (or a plain token) so the
// projection self-heals beyond webhook delivery. Idempotent — every write is
// an upsert keyed by a repo-qualified ID, and a monotonic guard prevents stale
// overwrites. Cursors (repo, stream) make each poll incremental.
import type { Client } from "@libsql/client";
import { fromIssueOrPr, isPlainObject, isProjectableItem } from "./projector";
import * as store from "./store";
import { apiTokenFor, listResource, type GitHubAuth } from "./github";

type Env = Record<string, string | undefined>;

const REPO_STREAMS = ["issues", "prs", "issue_comments", "review_comments", "reviews"] as const;
type Stream = (typeof REPO_STREAMS)[number];

/** Overlap subtracted from the reviews cursor before it bounds the PR
 *  sweep (#9850 round 2): covers the skew between a review's
 *  submitted_at (what the cursor stores) and its PR's updated_at (what
 *  the sweep filters on), and keeps a just-failed PR retryable for a
 *  few passes. Five minutes costs a handful of extra PR fetches. */
export const REVIEWS_CURSOR_OVERLAP_MS = 5 * 60 * 1000;

/** The reviews sweep cutoff for a stored cursor, or null for a full
 *  bootstrap sweep (no cursor yet, or an unparseable one). */
export function reviewsCutoff(since: string | null): string | null {
  if (!since) return null;
  const ms = Date.parse(since);
  if (!Number.isFinite(ms)) return null;
  return new Date(ms - REVIEWS_CURSOR_OVERLAP_MS).toISOString();
}

function csv(env: Env): string[] {
  return (env.PM_REPOS ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

async function reconcileStream(
  db: Client,
  auth: GitHubAuth,
  repo: string,
  stream: Stream,
  since: string | null,
  fetchFn: typeof fetch,
): Promise<{
  repo: string;
  stream: Stream;
  written: number;
  skipped?: number;
  cursor: string | null;
  error?: string;
  errors?: { pr: number; error: string }[];
}> {
  const q = since ? `since=${encodeURIComponent(since)}&per_page=100&state=all` : "per_page=100&state=all";
  const written = { n: 0 };
  const mark = async (n: number) => {
    written.n += n;
  };

  // Derive "owner/repo#N" from an API comment's html_url / pull_request_url.
  const commentRef = (repoName: string, c: Record<string, any>): string | null => {
    const target = c.html_url ?? c.pull_request_url ?? c.issue_url ?? "";
    const m = /github\.com\/([\w.-]+\/[\w.-]+)\/(?:issues|pull)\/(\d+)/.exec(String(target));
    if (m) return `${m[1]}#${m[2]}`;
    return null;
  };

  // The prs stream sources pulls-shaped data (#4528): the issues endpoint's
  // PR objects carry no draft/merged/merged_at, so first-sight PRs
  // materialized draft=0/merged=0 and the #4158 preservation logic pinned
  // those false defaults forever. The pulls list carries the merge fields,
  // so upserts from it store the truth on first sight and heal any legacy
  // false-default rows on a later pass. The issues stream is unchanged.
  if (stream === "prs") {
    // Incremental (#9850 round 2): the list is walked newest-first and
    // the walk stops at the stored cursor; rows at or below the cursor
    // are not re-upserted. That is safe because the cursor only ever
    // covers PRs an earlier unfiltered pass already upserted from
    // pulls-shaped data — a PR is re-upserted on the first pass that
    // sees it change, which is also what heals rows first written by
    // another stream. The cursor still advances over the full walk.
    const pulls = await listResource<Record<string, any>>(
      auth,
      `repos/${repo}/pulls?per_page=100&state=all&sort=updated&direction=desc`,
      fetchFn,
      since
        ? {
            stopAfterPage: (page) => page.every((p) => typeof p?.updated_at === "string" && p.updated_at <= since),
          }
        : {},
    );
    let cursor: string | null = null;
    let skipped = 0;
    const rows: ReturnType<typeof fromIssueOrPr>[] = [];
    for (const pr of pulls) {
      if (!isProjectableItem(pr)) {
        skipped++;
        continue;
      }
      const u = pr.updated_at ?? pr.created_at;
      if (typeof u === "string" && (!cursor || u > cursor)) cursor = u;
      if (since && typeof pr.updated_at === "string" && pr.updated_at <= since) continue;
      rows.push(fromIssueOrPr(repo, pr, "pr"));
    }
    // One batched write for the whole stream, not a round trip per row (#9850).
    await store.upsertItems(db, rows);
    return { repo, stream, written: rows.length, ...(skipped > 0 ? { skipped } : {}), cursor };
  }

  if (stream === "issues") {
    const path = `repos/${repo}/issues?${q}&filter=all`;
    const objs = await listResource<Record<string, any>>(auth, path, fetchFn);
    let cursor: string | null = null;
    let skipped = 0;
    const itemRows: ReturnType<typeof fromIssueOrPr>[] = [];
    for (const obj of objs) {
      if (!isProjectableItem(obj)) {
        skipped++;
        continue;
      }
      if (Boolean(obj.pull_request)) continue;
      itemRows.push(fromIssueOrPr(repo, obj, "issue"));
      const u = obj.updated_at ?? obj.created_at;
      if (typeof u === "string" && (!cursor || u > cursor)) cursor = u;
    }
    await store.upsertItems(db, itemRows);
    return { repo, stream, written: itemRows.length, ...(skipped > 0 ? { skipped } : {}), cursor };
  }

  if (stream === "issue_comments" || stream === "review_comments") {
    const kind = stream === "issue_comments" ? "issue_comment" : "review_comment";
    // The repo-wide collections are /issues/comments and /pulls/comments —
    // there is no /issue_comments or /review_comments path (both 404).
    // Both collections DO support `since`, and using the stored cursor is
    // load-bearing (#9850): without it every pass re-pulled and re-wrote
    // the repo's entire comment history — the write flood that stalled
    // /health. Only the first pass (no cursor yet) pulls the full list.
    const collection = stream === "issue_comments" ? "issues/comments" : "pulls/comments";
    const path = `repos/${repo}/${collection}?per_page=100${since ? `&since=${encodeURIComponent(since)}` : ""}`;
    const rows = await listResource<Record<string, any>>(auth, path, fetchFn);
    let cursor: string | null = null;
    let skipped = 0;
    const commentRows: Parameters<typeof store.upsertComment>[1][] = [];
    for (const c of rows) {
      if (!isPlainObject(c)) {
        skipped++;
        continue;
      }
      // The API comment objects carry the containing issue/PR only via URL;
      // derive the repo-qualified ref from it.
      const ref = commentRef(repo, c);
      const createdAt = c.created_at ?? c.updated_at ?? null;
      if (!ref || !createdAt) continue;
      const updatedAt = c.updated_at ?? null;
      commentRows.push({
        id: String(c.id),
        ref,
        kind,
        author: c.user?.login ?? null,
        body: typeof c.body === "string" ? c.body : null,
        created_at: createdAt,
        updated_at: updatedAt,
        url: c.html_url ?? null,
        path: c.path ?? null,
        line: typeof c.line === "number" ? c.line : null,
        diff_hunk: c.diff_hunk ?? null,
        review_state: null,
        submitted_at: null,
        commit_id: c.commit_id ?? null,
        deleted: 0,
        source_updated_at: updatedAt ?? createdAt,
      });
      const u = c.updated_at ?? c.created_at;
      if (typeof u === "string" && (!cursor || u > cursor)) cursor = u;
    }
    await store.upsertComments(db, commentRows);
    return { repo, stream, written: commentRows.length, ...(skipped > 0 ? { skipped } : {}), cursor };
  }

  if (stream === "reviews") {
    // Reviews hang off PRs; pull PRs and their reviews. Guarded per-PR:
    // a failure fetching or upserting one PR's reviews is recorded in
    // `errors` and contained to that PR — later PRs still reconcile in
    // the same run, `written` counts the upserts that actually landed,
    // and the cursor advances over those upserted reviews.
    //
    // Incremental (#9850 round 2): the first version of this stream
    // ignored its cursor and re-fetched the reviews of EVERY PR on
    // every pass — for developing-today/code that is ~4,700 GitHub
    // calls and ~5,400 review upserts per pass, and a full pass took
    // ~30 minutes of sustained DB writes, which is what stalled
    // /health. A new review updates its PR, so only PRs updated at or
    // after the cursor (minus a small overlap for clock skew between
    // a review's submitted_at and its PR's updated_at) can hold
    // reviews the cursor has not covered. A failing PR stays a
    // candidate while it is inside that window and re-enters the
    // moment the PR changes again; when a PR is fetched, its full
    // review list comes back, so nothing is skipped permanently.
    const cutoff = reviewsCutoff(since);
    const prs = await listResource<Record<string, any>>(
      auth,
      `repos/${repo}/pulls?per_page=100&state=all&sort=updated&direction=desc`,
      fetchFn,
      cutoff
        ? {
            stopAfterPage: (page) => page.every((p) => typeof p?.updated_at === "string" && p.updated_at < cutoff),
          }
        : {},
    );
    let cursor: string | null = null;
    let skipped = 0;
    const errors: { pr: number; error: string }[] = [];
    for (const pr of prs) {
      if (!isProjectableItem(pr)) {
        skipped++;
        continue;
      }
      // Positively stale: the PR has not changed since before the
      // cursor window, so its reviews are already projected. A missing
      // updated_at is never treated as stale.
      if (cutoff && typeof pr.updated_at === "string" && pr.updated_at < cutoff) continue;
      try {
        const reviews = await listResource<Record<string, any>>(
          auth,
          `repos/${repo}/pulls/${pr.number}/reviews?per_page=100`,
          fetchFn,
        );
        const reviewRows: Parameters<typeof store.upsertComment>[1][] = [];
        // This PR's cursor contribution merges into the stream cursor
        // only after its batch lands, so a failed PR never advances the
        // cursor past reviews that were not written.
        let prCursor: string | null = null;
        for (const r of reviews) {
          if (!isPlainObject(r)) {
            skipped++;
            continue;
          }
          const ref = `${repo}#${pr.number}`;
          const createdAt = r.submitted_at ?? r.created_at ?? null;
          if (!createdAt) continue;
          reviewRows.push({
            id: String(r.id),
            ref,
            kind: "review",
            author: r.user?.login ?? null,
            body: typeof r.body === "string" ? r.body : null,
            created_at: createdAt,
            updated_at: null,
            url: r.html_url ?? null,
            path: null,
            line: null,
            diff_hunk: null,
            review_state: r.state ?? null,
            submitted_at: r.submitted_at ?? null,
            commit_id: r.commit_id ?? null,
            deleted: 0,
            source_updated_at: createdAt,
          });
          const u = r.submitted_at ?? r.created_at;
          if (typeof u === "string" && (!prCursor || u > prCursor)) prCursor = u;
        }
        await store.upsertComments(db, reviewRows);
        await mark(reviewRows.length);
        if (prCursor && (!cursor || prCursor > cursor)) cursor = prCursor;
      } catch (e) {
        errors.push({ pr: pr.number, error: String((e as Error).message) });
      }
    }
    return {
      repo,
      stream,
      written: written.n,
      ...(skipped > 0 ? { skipped } : {}),
      cursor,
      ...(errors.length > 0 ? { errors } : {}),
    };
  }

  return { repo, stream, written: 0, cursor: null };
}

export async function reconcileRepos(
  db: Client,
  env: Env,
  fetchFn: typeof fetch = fetch,
): Promise<Record<string, any>> {
  const repos = csv(env);
  if (repos.length === 0) {
    return { ok: false, error: "PM_REPOS is empty — nothing to reconcile", streams: [] };
  }
  const results: Record<string, any>[] = [];
  const passStarted = Date.now();
  console.log(`[projector] reconcile pass started repos=${repos.join(",")}`);
  for (const repo of repos) {
    let auth: GitHubAuth;
    try {
      auth = await apiTokenFor(env, repo, fetchFn);
    } catch (e) {
      for (const stream of REPO_STREAMS) {
        results.push({ repo, stream, written: 0, error: String((e as Error).message) });
      }
      continue;
    }
    for (const stream of REPO_STREAMS) {
      const since = await store.getSyncCursor(db, repo, stream);
      // Per-stream timing in the journal (#9850): a pass that starts
      // stalling again is visible as a stream line that stops arriving,
      // instead of only as sentinel noise downstream.
      const streamStarted = Date.now();
      try {
        const r = await reconcileStream(db, auth, repo, stream, since, fetchFn);
        if (r.cursor) await store.setSyncCursor(db, repo, stream, r.cursor);
        console.log(`[projector] reconcile ${repo} ${stream}: written=${r.written} ms=${Date.now() - streamStarted}`);
        results.push(r);
      } catch (e) {
        console.log(
          `[projector] reconcile ${repo} ${stream}: error=${String((e as Error).message)} ms=${Date.now() - streamStarted}`,
        );
        results.push({ repo, stream, written: 0, error: String((e as Error).message) });
      }
    }
  }
  const total = results.reduce((n, r) => n + (r.written ?? 0), 0);
  console.log(`[projector] reconcile pass finished written=${total} ms=${Date.now() - passStarted}`);
  return { ok: true, written: total, streams: results };
}
