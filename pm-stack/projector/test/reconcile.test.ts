import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { migrate, getItems, getComments } from "../src/store";
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
            updated_at: "2026-10-03T00:00:00Z",
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
            updated_at: "2026-10-04T00:00:00Z",
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
    expect((await getComments(db, ["developing-today/code#1"])).map((c) => c.id)).toEqual(["9001"]);
    // The requests used the real collection paths, never the 404 shapes.
    expect(seen.some((u) => u.includes("/issues/comments"))).toBe(true);
    expect(seen.some((u) => u.includes("/pulls/comments"))).toBe(true);
    expect(seen.some((u) => u.includes("/issue_comments"))).toBe(false);
    expect(seen.some((u) => u.includes("api.github.com/https"))).toBe(false);
  });
});
