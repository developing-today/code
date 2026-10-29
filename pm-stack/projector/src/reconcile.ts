// Reconciliation reads: poll GitHub via the App (or a plain token) so the
// projection self-heals beyond webhook delivery. Idempotent — every write is
// an upsert keyed by a repo-qualified ID, and a monotonic guard prevents stale
// overwrites. Cursors (repo, stream) make each poll incremental.
import type { Client } from "@libsql/client";
import { fromIssueOrPr } from "./projector";
import * as store from "./store";
import { apiTokenFor, listResource, type GitHubAuth } from "./github";

type Env = Record<string, string | undefined>;

const REPO_STREAMS = ["issues", "prs", "issue_comments", "review_comments", "reviews"] as const;
type Stream = (typeof REPO_STREAMS)[number];

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
): Promise<{ repo: string; stream: Stream; written: number; cursor: string | null; error?: string }> {
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

  if (stream === "issues" || stream === "prs") {
    const path = `repos/${repo}/issues?${q}&filter=all`;
    const rows = await listResource<Record<string, any>>(auth, path, fetchFn);
    let cursor: string | null = null;
    for (const obj of rows) {
      const isPr = Boolean(obj.pull_request);
      if (stream === "prs" && !isPr) continue;
      if (stream === "issues" && isPr) continue;
      const row = fromIssueOrPr(repo, obj, isPr ? "pr" : "issue");
      await store.upsertItem(db, row);
      await mark(1);
      const u = obj.updated_at ?? obj.created_at;
      if (typeof u === "string" && (!cursor || u > cursor)) cursor = u;
    }
    return { repo, stream, written: written.n, cursor };
  }

  if (stream === "issue_comments" || stream === "review_comments") {
    const kind = stream === "issue_comments" ? "issue_comment" : "review_comment";
    // The repo-wide collections are /issues/comments and /pulls/comments —
    // there is no /issue_comments or /review_comments path (both 404).
    // These endpoints do not support `since`; pull the full list and upsert.
    const collection = stream === "issue_comments" ? "issues/comments" : "pulls/comments";
    const rows = await listResource<Record<string, any>>(auth, `repos/${repo}/${collection}?per_page=100`, fetchFn);
    let cursor: string | null = null;
    for (const c of rows) {
      // The API comment objects carry the containing issue/PR only via URL;
      // derive the repo-qualified ref from it.
      const ref = commentRef(repo, c);
      const createdAt = c.created_at ?? c.updated_at ?? null;
      if (!ref || !createdAt) continue;
      await store.upsertComment(db, {
        id: String(c.id),
        ref,
        kind,
        author: c.user?.login ?? null,
        body: typeof c.body === "string" ? c.body : null,
        created_at: createdAt,
        updated_at: c.updated_at ?? null,
        url: c.html_url ?? null,
        path: c.path ?? null,
        line: typeof c.line === "number" ? c.line : null,
        diff_hunk: c.diff_hunk ?? null,
        review_state: null,
        submitted_at: null,
        commit_id: c.commit_id ?? null,
        deleted: 0,
        source_updated_at: createdAt,
      });
      await mark(1);
      const u = c.updated_at ?? c.created_at;
      if (typeof u === "string" && (!cursor || u > cursor)) cursor = u;
    }
    return { repo, stream, written: written.n, cursor };
  }

  if (stream === "reviews") {
    // Reviews hang off PRs; pull PRs and their reviews. Guarded so a
    // failure on one PR does not abort the whole stream.
    const prs = await listResource<Record<string, any>>(auth, `repos/${repo}/pulls?per_page=100&state=all`, fetchFn);
    let cursor: string | null = null;
    for (const pr of prs) {
      const reviews = await listResource<Record<string, any>>(
        auth,
        `repos/${repo}/pulls/${pr.number}/reviews?per_page=100`,
        fetchFn,
      );
      for (const r of reviews) {
        const ref = `${repo}#${pr.number}`;
        const createdAt = r.submitted_at ?? r.created_at ?? null;
        if (!createdAt) continue;
        await store.upsertComment(db, {
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
        await mark(1);
        const u = r.submitted_at ?? r.created_at;
        if (typeof u === "string" && (!cursor || u > cursor)) cursor = u;
      }
    }
    return { repo, stream, written: written.n, cursor };
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
      try {
        const r = await reconcileStream(db, auth, repo, stream, since, fetchFn);
        if (r.cursor) await store.setSyncCursor(db, repo, stream, r.cursor);
        results.push(r);
      } catch (e) {
        results.push({ repo, stream, written: 0, error: String((e as Error).message) });
      }
    }
  }
  const total = results.reduce((n, r) => n + (r.written ?? 0), 0);
  return { ok: true, written: total, streams: results };
}
