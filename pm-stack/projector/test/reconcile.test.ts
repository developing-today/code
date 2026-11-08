import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  migrate,
  getItems,
  getComments,
  getSyncCursor,
  upsertItem,
  upsertComment,
} from "../src/store";
import { applyEvent, fromIssueOrPr, type ProjectorStore } from "../src/projector";
import { listResource, type GitHubAuth } from "../src/github";
import { reconcileRepos } from "../src/reconcile";

// Regression coverage for the two defects that made every reconcile stream
// fail github_api_404 against the real GitHub API on the projector's first
// live run (developing-today/code#1199):
//   1. listResource re-prefixed the Link header's ABSOLUTE next-page URL,
//      producing https://api.github.com/https://api.github.com/… — any
//      stream with more than one page of results wrote nothing.
//   2. The comment streams requested repos/<repo>/issue_comments and
//      repos/<repo>/review_comments, which do not exist; the repo-wide
//      collections are issues/comments and pulls/comments.

let db: Client;
let dir: string;

beforeAll(async () => {
  // NB: this libsql client treats ":memory:" as a relative FILE path (it
  // litters a file literally named ":memory:" into the cwd) — use a temp file.
  dir = mkdtempSync(join(tmpdir(), "projector-reconcile-test-"));
  db = createClient({ url: `file:${join(dir, "test.db")}` });
  await migrate(db);
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

const auth: GitHubAuth = { kind: "token", token: "test-token" };

const jsonRes = (body: unknown, link?: string): Response =>
  new Response(JSON.stringify(body), {
    status: 200,
    headers: link ? { link } : {},
  });

describe("listResource pagination", () => {
  test("follows the Link header's absolute next URL without re-prefixing", async () => {
    const seen: string[] = [];
    const fetchFn = (async (url: string) => {
      seen.push(url);
      if (url.endsWith("page=2")) return jsonRes([{ n: 2 }]);
      return jsonRes([{ n: 1 }], '<https://api.github.com/repos/o/r/issues?page=2>; rel="next"');
    }) as unknown as typeof fetch;

    const rows = await listResource<{ n: number }>(auth, "repos/o/r/issues?per_page=100", fetchFn);
    expect(rows.map((r) => r.n)).toEqual([1, 2]);
    expect(seen[0]).toBe("https://api.github.com/repos/o/r/issues?per_page=100");
    expect(seen[1]).toBe("https://api.github.com/repos/o/r/issues?page=2");
  });
});

describe("reconcileRepos against a fake GitHub", () => {
  test("reads issues across pages and both comment collections", async () => {
    const seen: string[] = [];
    const issue = (n: number) => ({
      number: n,
      html_url: `https://github.com/developing-today/code/issues/${n}`,
      url: `https://api.github.com/repos/developing-today/code/issues/${n}`,
      title: `issue ${n}`,
      state: "open",
      user: { login: "drew" },
      labels: [],
      assignees: [],
      created_at: "2026-10-01T00:00:00Z",
      updated_at: "2026-10-02T00:00:00Z",
    });
    const fetchFn = (async (url: string) => {
      seen.push(url);
      if (url.includes("/issues/comments")) {
        return jsonRes([
          {
            id: 9001,
            html_url: "https://github.com/developing-today/code/issues/1#issuecomment-9001",
            body: "an issue comment",
            user: { login: "drew" },
            created_at: "2026-10-03T00:00:00Z",
            updated_at: "2026-10-05T00:00:00Z",
          },
        ]);
      }
      if (url.includes("/pulls/comments")) {
        return jsonRes([
          {
            id: 9002,
            html_url: "https://github.com/developing-today/code/pull/7#discussion_r9002",
            body: "a review comment",
            user: { login: "drew" },
            created_at: "2026-10-04T00:00:00Z",
            updated_at: "2026-10-06T00:00:00Z",
            path: "src/x.ts",
            line: 3,
          },
        ]);
      }
      if (url.includes("/issues?") && url.includes("page=2")) return jsonRes([issue(2)]);
      if (url.includes("/issues?")) {
        return jsonRes([issue(1)], '<https://api.github.com/repos/developing-today/code/issues?page=2>; rel="next"');
      }
      if (url.includes("/pulls?")) return jsonRes([]);
      return jsonRes([]);
    }) as unknown as typeof fetch;

    const result = await reconcileRepos(db, { PM_REPOS: "developing-today/code", GITHUB_TOKEN: "test-token" }, fetchFn);

    expect(result.ok).toBe(true);
    const errors = (result.streams as { error?: string }[]).filter((s) => s.error);
    expect(errors).toEqual([]);
    // Both pages of issues landed.
    const items = await getItems(db, ["developing-today/code#1", "developing-today/code#2"]);
    expect(items.map((i) => i.number).sort()).toEqual([1, 2]);
    // Both comment collections landed.
    const issueComments = await getComments(db, ["developing-today/code#1"]);
    expect(issueComments.map((c) => c.id)).toEqual(["9001"]);
    expect(issueComments[0].source_updated_at).toBe("2026-10-05T00:00:00Z");
    const reviewComments = await getComments(db, ["developing-today/code#7"]);
    expect(reviewComments.map((c) => c.id)).toEqual(["9002"]);
    expect(reviewComments[0].source_updated_at).toBe("2026-10-06T00:00:00Z");
    const commentStreams = result.streams as { stream: string; cursor: string | null }[];
    expect(commentStreams.find((s) => s.stream === "issue_comments")?.cursor).toBe("2026-10-05T00:00:00Z");
    expect(commentStreams.find((s) => s.stream === "review_comments")?.cursor).toBe("2026-10-06T00:00:00Z");
    // The requests used the real collection paths, never the 404 shapes.
    expect(seen.some((u) => u.includes("/issues/comments"))).toBe(true);
    expect(seen.some((u) => u.includes("/pulls/comments"))).toBe(true);
    expect(seen.some((u) => u.includes("/issue_comments"))).toBe(false);
    expect(seen.some((u) => u.includes("api.github.com/https"))).toBe(false);
  });
});

describe("reconcileRepos prs stream sources pulls-shaped data (#4528)", () => {
  // The issues endpoint's PR objects carry no draft/merged/merged_at, so a
  // prs stream reading them materialized draft=0/merged=0 on first sight
  // and the #4158 preservation logic pinned those false defaults forever.
  // The prs stream now reads repos/{repo}/pulls instead.
  const pullsPr = (repo: string, n: number, over: Record<string, any>) => ({
    number: n,
    html_url: `https://github.com/${repo}/pull/${n}`,
    url: `https://api.github.com/repos/${repo}/pulls/${n}`,
    title: `pr ${n}`,
    state: "open",
    user: { login: "drew" },
    labels: [],
    assignees: [],
    draft: false,
    merged_at: null,
    created_at: "2026-10-01T00:00:00Z",
    updated_at: "2026-10-04T00:00:00Z",
    ...over,
  });

  const prsFake = (repo: string, pulls: Record<string, any>[]) => {
    const seen: string[] = [];
    const fetchFn = (async (url: string) => {
      seen.push(url);
      if (url.includes(`/repos/${repo}/pulls?`)) return jsonRes(pulls);
      // The issues stream still reads the issues endpoint; serve no rows.
      if (url.includes("/issues?")) return jsonRes([]);
      if (url.includes("/issues/comments") || url.includes("/pulls/comments")) return jsonRes([]);
      // Per-PR review enumeration for the reviews stream.
      if (url.includes("/reviews")) return jsonRes([]);
      return jsonRes([]);
    }) as unknown as typeof fetch;
    return { seen, fetchFn };
  };

  test("draft-first-sight stores draft=1", async () => {
    const repo = "developing-today/prs-draft-first-sight";
    const { seen, fetchFn } = prsFake(repo, [pullsPr(repo, 7, { draft: true })]);
    const result = await reconcileRepos(db, { PM_REPOS: repo, GITHUB_TOKEN: "test-token" }, fetchFn);
    expect(result.ok).toBe(true);
    const items = await getItems(db, [`${repo}#7`]);
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe("pr");
    expect(items[0].draft).toBe(1);
    expect(items[0].merged).toBe(0);
    expect(items[0].merged_at).toBeNull();
    // The prs pass read the pulls endpoint, not the issues endpoint.
    expect(seen.some((u) => u.includes(`/repos/${repo}/pulls?`))).toBe(true);
  });

  test("merged-first-sight stores merged=1 + merged_at", async () => {
    const repo = "developing-today/prs-merged-first-sight";
    const { fetchFn } = prsFake(repo, [
      pullsPr(repo, 8, { state: "closed", merged_at: "2026-10-03T00:00:00Z" }),
    ]);
    const result = await reconcileRepos(db, { PM_REPOS: repo, GITHUB_TOKEN: "test-token" }, fetchFn);
    expect(result.ok).toBe(true);
    const items = await getItems(db, [`${repo}#8`]);
    expect(items).toHaveLength(1);
    expect(items[0].draft).toBe(0);
    expect(items[0].merged).toBe(1);
    expect(items[0].merged_at).toBe("2026-10-03T00:00:00Z");
  });

  test("heal-after-gap: legacy false-default row is corrected on a later pass", async () => {
    const repo = "developing-today/prs-heal-after-gap";
    // Seed the row exactly as the old issues-sourced prs stream left it: an
    // issue-shaped upsert carries no merge fields, so the INSERT
    // materialized draft=0/merged=0 with no merged_at.
    await upsertItem(
      db,
      fromIssueOrPr(repo, {
        number: 9,
        html_url: `https://github.com/${repo}/pull/9`,
        url: `https://api.github.com/repos/${repo}/issues/9`,
        title: "pr 9",
        state: "open",
        user: { login: "drew" },
        labels: [],
        assignees: [],
        pull_request: { url: `https://api.github.com/repos/${repo}/pulls/9` },
        created_at: "2026-10-01T00:00:00Z",
        updated_at: "2026-10-02T00:00:00Z",
      }, "pr"),
    );
    expect((await getItems(db, [`${repo}#9`]))[0].draft).toBe(0);
    // A later polls pass with pulls truth heals the row.
    const { fetchFn } = prsFake(repo, [pullsPr(repo, 9, { draft: true })]);
    const result = await reconcileRepos(db, { PM_REPOS: repo, GITHUB_TOKEN: "test-token" }, fetchFn);
    expect(result.ok).toBe(true);
    const healed = (await getItems(db, [`${repo}#9`]))[0];
    expect(healed.draft).toBe(1);
    expect(healed.merged).toBe(0);
  });
});

describe("reconcileRepos reviews stream per-PR guard (#2745)", () => {
  test("[ok, 500, ok]: both ok PRs' reviews are stored, exactly one per-PR error is reported, written counts actual upserts, cursor advances over upserted reviews", async () => {
    const repo = "developing-today/reviews-guard";
    const review = (id: number, at: string) => ({
      id,
      body: `review ${id}`,
      user: { login: "drew" },
      state: "APPROVED",
      submitted_at: at,
      html_url: `https://github.com/${repo}/pull/1#pullrequestreview-${id}`,
    });
    const fetchFn = (async (url: string) => {
      if (url.includes("/pulls/2/reviews")) return new Response("boom", { status: 500 });
      if (url.includes("/pulls/1/reviews")) return jsonRes([review(101, "2026-10-05T00:00:00Z")]);
      if (url.includes("/pulls/3/reviews")) return jsonRes([review(103, "2026-10-06T00:00:00Z")]);
      if (url.includes("/pulls?")) return jsonRes([{ number: 1 }, { number: 2 }, { number: 3 }]);
      return jsonRes([]);
    }) as unknown as typeof fetch;

    const result = await reconcileRepos(db, { PM_REPOS: repo, GITHUB_TOKEN: "test-token" }, fetchFn);

    expect(result.ok).toBe(true);
    const reviews = (result.streams as Record<string, any>[]).find((s) => s.stream === "reviews");
    expect(reviews).toBeDefined();
    expect(reviews!.error).toBeUndefined();
    expect(reviews!.written).toBe(2);
    expect(reviews!.errors).toEqual([{ pr: 2, error: "github_api_500" }]);
    expect(reviews!.cursor).toBe("2026-10-06T00:00:00Z");
    expect((await getComments(db, [`${repo}#1`])).map((c) => c.id)).toEqual(["101"]);
    expect((await getComments(db, [`${repo}#3`])).map((c) => c.id)).toEqual(["103"]);
    expect(await getSyncCursor(db, repo, "reviews")).toBe("2026-10-06T00:00:00Z");
  });
});

describe("webhook and reconcile review rows (#3087)", () => {
  const repo = "developing-today/review-timestamp-roundtrip";
  const prNumber = 41;
  const submittedAt = "2026-10-02T03:04:05Z";

  const review = (state: string, body: string) => ({
    id: 3087001,
    body,
    user: { login: "reviewer" },
    state,
    submitted_at: submittedAt,
    html_url: `https://github.com/${repo}/pull/${prNumber}#pullrequestreview-3087001`,
    commit_id: "abc123",
  });

  const reconcileReview = async (row: ReturnType<typeof review>) => {
    const fetchFn = (async (url: string) => {
      if (url.includes(`/repos/${repo}/pulls/${prNumber}/reviews`)) return jsonRes([row]);
      if (url.includes(`/repos/${repo}/pulls?`)) return jsonRes([{ number: prNumber }]);
      return jsonRes([]);
    }) as unknown as typeof fetch;
    await reconcileRepos(db, { PM_REPOS: repo, GITHUB_TOKEN: "test-token" }, fetchFn);
  };

  const webhookStore: ProjectorStore = {
    async upsertItem(row) { await upsertItem(db, row); },
    async upsertComment(row) { await upsertComment(db, row); },
    async markCommentDeleted() {},
    async deleteItem() {},
    async recordPush() {},
  };

  const webhook = async (state: string, body: string) => {
    await applyEvent(webhookStore, "pull_request_review", {
      action: "submitted",
      repository: { full_name: repo },
      pull_request: {
        number: prNumber,
        title: "review timestamp test",
        state: "open",
        html_url: `https://github.com/${repo}/pull/${prNumber}`,
        created_at: "2026-10-01T00:00:00Z",
        updated_at: "2026-10-01T00:00:00Z",
      },
      review: review(state, body),
    });
  };

  const stored = async () => {
    const [row] = await getComments(db, [`${repo}#${prNumber}`]);
    return row;
  };

  test("uses the same submitted timestamp in either writer order and keeps redelivery stable", async () => {
    // Webhook first, then reconciliation: the real store's monotonic guard
    // must accept the matching source timestamp rather than reject it as old.
    await webhook("approved", "LGTM");
    const webhookFirst = await stored();
    expect(webhookFirst.created_at).toBe(submittedAt);
    expect(webhookFirst.source_updated_at).toBe(submittedAt);
    expect(webhookFirst.submitted_at).toBe(submittedAt);
    await reconcileReview(review("APPROVED", "LGTM"));
    const afterReconcile = await stored();
    expect(afterReconcile.created_at).toBe(submittedAt);
    expect(afterReconcile.source_updated_at).toBe(submittedAt);
    expect(afterReconcile.review_state).toBe("approved");

    // Reconciliation first, then an edited/dismissed webhook delivery: the
    // source timestamp stays fixed while the review state still advances.
    await reconcileReview(review("APPROVED", "LGTM"));
    const beforeRedelivery = await stored();
    await webhook("dismissed", "withdrawn");
    const afterRedelivery = await stored();
    expect(beforeRedelivery.created_at).toBe(submittedAt);
    expect(afterRedelivery.created_at).toBe(beforeRedelivery.created_at);
    expect(afterRedelivery.source_updated_at).toBe(submittedAt);
    expect(afterRedelivery.review_state).toBe("dismissed");
    expect(afterRedelivery.body).toBe("withdrawn");
  });
});
