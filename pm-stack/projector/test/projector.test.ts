import { describe, test, expect } from "bun:test";
import { applyEvent, type ProjectorStore, prFromPayload, repoOf } from "../src/projector";
import type { ItemRow, CommentRow } from "../src/store";
import {
  migrate,
  upsertItem as storeUpsertItem,
  upsertComment as storeUpsertComment,
  markCommentDeleted as storeMarkCommentDeleted,
  recordPush as storeRecordPush,
  getItems,
} from "../src/store";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

function fakeStore() {
  const items = new Map<string, Omit<ItemRow, "fetched_at">>();
  const comments = new Map<string, Omit<CommentRow, "fetched_at">>();
  const pushes: any[] = [];
  const deleted: string[] = [];
  const store: ProjectorStore = {
    async upsertItem(i) {
      const prev = items.get(i.ref);
      if (prev && i.merged === null) {
        i = { ...i, draft: prev.draft, merged: prev.merged, merged_at: prev.merged_at };
      }
      items.set(i.ref, i);
    },
    async upsertComment(c) {
      comments.set(c.id, c);
    },
    async markCommentDeleted(c) {
      deleted.push(c.id);
      comments.set(c.id, { ...c, deleted: 1 });
    },
    async recordPush(p) {
      pushes.push(p);
    },
  };
  return { store, items, comments, pushes, deleted };
}

describe("projector: repo allow-list", () => {
  test("ignores repos outside the allow-list", async () => {
    const { store, items } = fakeStore();
    const payload = { repository: { full_name: "someone/else" }, issue: { number: 9, title: "x" } };
    const r = await applyEvent(store, "issues", payload, ["developing-today/code"]);
    expect(r.wrote).toBe(false);
    expect(items.size).toBe(0);
  });
});

describe("projector: issues", () => {
  test("opened issue becomes an item", async () => {
    const { store, items } = fakeStore();
    const payload = {
      action: "opened",
      repository: { full_name: "developing-today/code" },
      issue: {
        number: 100,
        title: "A bug",
        state: "open",
        html_url: "https://github.com/developing-today/code/issues/100",
        url: "https://api.github.com/repos/developing-today/code/issues/100",
        user: { login: "dezren39" },
        labels: [{ name: "bug" }, { name: "status:backlog" }],
        created_at: "2026-10-25T00:00:00Z",
        updated_at: "2026-10-25T00:00:00Z",
      },
    };
    const r = await applyEvent(store, "issues", payload);
    expect(r.refs).toEqual(["developing-today/code#100"]);
    expect(items.get("developing-today/code#100")?.kind).toBe("issue");
    expect(JSON.parse(items.get("developing-today/code#100")!.labels)).toEqual(["bug", "status:backlog"]);
  });

  test("issue with pull_request field is classified as a PR", async () => {
    const { store, items } = fakeStore();
    const payload = {
      action: "opened",
      repository: { full_name: "developing-today/code" },
      issue: {
        number: 101,
        title: "PR via issue event",
        state: "open",
        html_url: "https://github.com/developing-today/code/pull/101",
        pull_request: { url: "x" },
        user: { login: "dezren39" },
        created_at: "2026-10-25T00:00:00Z",
        updated_at: "2026-10-25T00:00:00Z",
      },
    };
    await applyEvent(store, "issues", payload);
    expect(items.get("developing-today/code#101")?.kind).toBe("pr");
  });
});

describe("projector: PR + review + review comment (PR-interior)", () => {
  test("PR with merged flag", async () => {
    const { store, items } = fakeStore();
    const payload = {
      action: "closed",
      repository: { full_name: "developing-today/code" },
      pull_request: {
        number: 200,
        title: "Feature",
        state: "closed",
        merged: true,
        html_url: "https://github.com/developing-today/code/pull/200",
        url: "https://api.github.com/repos/developing-today/code/pulls/200",
        user: { login: "dezren39" },
        draft: false,
        created_at: "2026-10-25T00:00:00Z",
        updated_at: "2026-10-25T03:00:00Z",
        merged_at: "2026-10-25T03:00:00Z",
        closed_at: "2026-10-25T03:00:00Z",
      },
    };
    await applyEvent(store, "pull_request", payload);
    const pr = items.get("developing-today/code#200");
    expect(pr?.merged).toBe(1);
    expect(pr?.state).toBe("closed");
  });

  test("review and review comment land with PR-interior fields", async () => {
    const { store, comments } = fakeStore();
    const pr = {
      number: 201,
      title: "PR",
      state: "open",
      html_url: "https://github.com/developing-today/code/pull/201",
      url: "https://api.github.com/repos/developing-today/code/pulls/201",
      user: { login: "dezren39" },
      created_at: "2026-10-25T00:00:00Z",
      updated_at: "2026-10-25T00:00:00Z",
    };
    await applyEvent(store, "pull_request_review", {
      action: "submitted",
      repository: { full_name: "developing-today/code" },
      pull_request: pr,
      review: {
        id: 501,
        state: "approved",
        body: "LGTM",
        user: { login: "reviewer" },
        submitted_at: "2026-10-25T01:00:00Z",
        commit_id: "abc123",
      },
    });
    const review = comments.get("501");
    expect(review?.kind).toBe("review");
    expect(review?.review_state).toBe("approved");
    expect(review?.commit_id).toBe("abc123");

    await applyEvent(store, "pull_request_review_comment", {
      action: "created",
      repository: { full_name: "developing-today/code" },
      pull_request: pr,
      comment: {
        id: 601,
        body: "typo here",
        user: { login: "reviewer" },
        path: "src/foo.ts",
        line: 42,
        diff_hunk: "@@ -1 +1 @@",
        created_at: "2026-10-25T01:05:00Z",
        updated_at: "2026-10-25T01:12:00Z",
        html_url: "https://github.com/developing-today/code/pull/201#discussion_r601",
      },
    });
    const rc = comments.get("601");
    expect(rc?.kind).toBe("review_comment");
    expect(rc?.path).toBe("src/foo.ts");
    expect(rc?.line).toBe(42);
    expect(rc?.source_updated_at).toBe("2026-10-25T01:12:00Z");
  });
});

describe("projector: comment deletion + push", () => {
  test("issue and review comments use their update timestamps", async () => {
    const { store, comments } = fakeStore();
    const issue = {
      number: 301,
      title: "x",
      html_url: "https://github.com/developing-today/code/issues/301",
      created_at: "2026-10-25T00:00:00Z",
      updated_at: "2026-10-25T00:00:00Z",
    };
    await applyEvent(store, "issue_comment", {
      action: "edited",
      repository: { full_name: "developing-today/code" },
      issue,
      comment: {
        id: 701,
        body: "edited issue comment",
        created_at: "2026-10-25T01:00:00Z",
        updated_at: "2026-10-25T01:15:00Z",
      },
    });
    await applyEvent(store, "pull_request_review_comment", {
      action: "edited",
      repository: { full_name: "developing-today/code" },
      pull_request: { ...issue, html_url: "https://github.com/developing-today/code/pull/301" },
      comment: {
        id: 702,
        body: "edited review comment",
        created_at: "2026-10-25T01:00:00Z",
        updated_at: "2026-10-25T01:20:00Z",
      },
    });
    await applyEvent(store, "issue_comment", {
      action: "created",
      repository: { full_name: "developing-today/code" },
      issue,
      comment: {
        id: 704,
        body: "comment without updated_at",
        created_at: "2026-10-25T01:30:00Z",
      },
    });

    expect(comments.get("701")?.source_updated_at).toBe("2026-10-25T01:15:00Z");
    expect(comments.get("702")?.source_updated_at).toBe("2026-10-25T01:20:00Z");
    expect(comments.get("704")?.source_updated_at).toBe("2026-10-25T01:30:00Z");
  });

  test("deleted issue comment marks deleted", async () => {
    const { store, deleted } = fakeStore();
    await applyEvent(store, "issue_comment", {
      action: "deleted",
      repository: { full_name: "developing-today/code" },
      issue: {
        number: 300,
        title: "x",
        html_url: "https://github.com/developing-today/code/issues/300",
        user: { login: "a" },
        created_at: "2026-10-25T00:00:00Z",
        updated_at: "2026-10-25T00:00:00Z",
      },
      comment: { id: 700 },
    });
    expect(deleted).toContain("700");
  });

  test("deleted review comment marks deleted", async () => {
    const { store, deleted } = fakeStore();
    await applyEvent(store, "pull_request_review_comment", {
      action: "deleted",
      repository: { full_name: "developing-today/code" },
      pull_request: {
        number: 302,
        title: "x",
        html_url: "https://github.com/developing-today/code/pull/302",
        created_at: "2026-10-25T00:00:00Z",
        updated_at: "2026-10-25T00:00:00Z",
      },
      comment: { id: 703 },
    });
    expect(deleted).toContain("703");
  });

  test("push event records head/before/pusher", async () => {
    const { store, pushes } = fakeStore();
    await applyEvent(
      store,
      "push",
      {
        repository: { full_name: "developing-today/code" },
        ref: "refs/heads/main",
        after: "deadbeef",
        before: "cafebabe",
        pusher: { name: "dezren39" },
      },
      [],
      "delivery-1",
    );
    expect(pushes[0]).toMatchObject({
      deliveryId: "delivery-1",
      headSha: "deadbeef",
      beforeSha: "cafebabe",
      pusher: "dezren39",
    });
  });
});

describe("projector: issue-shaped payloads never reset merge fields (#4158)", () => {
  const repo = { full_name: "developing-today/code" };
  // The `issue` object GitHub sends for a PR in issues / issue_comment
  // events: it has a pull_request link but NO merged / merged_at / draft.
  const issueShapeForPr = (over: Record<string, any>) => ({
    number: 400,
    title: "Merged feature",
    state: "closed",
    html_url: "https://github.com/developing-today/code/pull/400",
    url: "https://api.github.com/repos/developing-today/code/issues/400",
    user: { login: "dezren39" },
    labels: [{ name: "bug" }],
    pull_request: {
      url: "https://api.github.com/repos/developing-today/code/pulls/400",
      merged_at: "2026-10-25T03:00:00Z",
    },
    created_at: "2026-10-25T00:00:00Z",
    updated_at: "2026-10-25T03:00:00Z",
    closed_at: "2026-10-25T03:00:00Z",
    ...over,
  });
  const mergedPrEvent = {
    action: "closed",
    repository: repo,
    pull_request: {
      number: 400,
      title: "Merged feature",
      state: "closed",
      merged: true,
      draft: false,
      html_url: "https://github.com/developing-today/code/pull/400",
      url: "https://api.github.com/repos/developing-today/code/pulls/400",
      user: { login: "dezren39" },
      labels: [],
      created_at: "2026-10-25T00:00:00Z",
      updated_at: "2026-10-25T03:00:00Z",
      merged_at: "2026-10-25T03:00:00Z",
      closed_at: "2026-10-25T03:00:00Z",
    },
  };

  test("merged PR survives issues and issue_comment events (fake store)", async () => {
    const { store, items } = fakeStore();
    await applyEvent(store, "pull_request", mergedPrEvent);
    await applyEvent(store, "issues", {
      action: "labeled",
      repository: repo,
      issue: issueShapeForPr({ updated_at: "2026-10-25T04:00:00Z" }),
    });
    await applyEvent(store, "issue_comment", {
      action: "created",
      repository: repo,
      issue: issueShapeForPr({ updated_at: "2026-10-25T05:00:00Z" }),
      comment: {
        id: 900,
        body: "post-merge comment",
        user: { login: "dezren39" },
        created_at: "2026-10-25T05:00:00Z",
        updated_at: "2026-10-25T05:00:00Z",
        html_url: "https://github.com/developing-today/code/pull/400#issuecomment-900",
      },
    });
    const row = items.get("developing-today/code#400");
    expect(row?.merged).toBe(1);
    expect(row?.merged_at).toBe("2026-10-25T03:00:00Z");
    expect(row?.state).toBe("closed");
    // The later issue-shaped upserts really did apply (labels moved).
    expect(JSON.parse(row!.labels)).toEqual(["bug"]);
  });

  test("PR-shaped payloads still update merge fields (fake store)", async () => {
    const { store, items } = fakeStore();
    const pr = {
      number: 401,
      title: "Draft work",
      state: "open",
      merged: false,
      merged_at: null,
      draft: true,
      html_url: "https://github.com/developing-today/code/pull/401",
      url: "https://api.github.com/repos/developing-today/code/pulls/401",
      user: { login: "dezren39" },
      created_at: "2026-10-25T00:00:00Z",
      updated_at: "2026-10-25T00:00:00Z",
    };
    await applyEvent(store, "pull_request", { action: "opened", repository: repo, pull_request: pr });
    expect(items.get("developing-today/code#401")?.draft).toBe(1);
    await applyEvent(store, "pull_request", {
      action: "ready_for_review",
      repository: repo,
      pull_request: { ...pr, draft: false, updated_at: "2026-10-25T01:00:00Z" },
    });
    const row = items.get("developing-today/code#401");
    expect(row?.draft).toBe(0);
    expect(row?.merged).toBe(0);
    expect(row?.merged_at).toBeNull();
  });

  test("merged PR survives issues and issue_comment events (real store)", async () => {
    const dir = mkdtempSync(join(tmpdir(), "projector-4158-"));
    const db: Client = createClient({ url: `file:${join(dir, "test.db")}` });
    try {
      await migrate(db);
      const store: ProjectorStore = {
        upsertItem: (i) => storeUpsertItem(db, i),
        upsertComment: (c) => storeUpsertComment(db, c),
        markCommentDeleted: (c) => storeMarkCommentDeleted(db, c),
        recordPush: (p) => storeRecordPush(db, p),
      };
      await applyEvent(store, "pull_request", mergedPrEvent);
      await applyEvent(store, "issues", {
        action: "labeled",
        repository: repo,
        issue: issueShapeForPr({ updated_at: "2026-10-25T04:00:00Z" }),
      });
      await applyEvent(store, "issue_comment", {
        action: "created",
        repository: repo,
        issue: issueShapeForPr({ updated_at: "2026-10-25T05:00:00Z" }),
        comment: {
          id: 901,
          body: "post-merge comment",
          user: { login: "dezren39" },
          created_at: "2026-10-25T05:00:00Z",
          updated_at: "2026-10-25T05:00:00Z",
          html_url: "https://github.com/developing-today/code/pull/400#issuecomment-901",
        },
      });
      const rows = await getItems(db, ["developing-today/code#400"]);
      expect(rows.length).toBe(1);
      expect(rows[0].merged).toBe(1);
      expect(rows[0].merged_at).toBe("2026-10-25T03:00:00Z");
      expect(rows[0].draft).toBe(0);
      expect(rows[0].state).toBe("closed");
      expect(JSON.parse(rows[0].labels)).toEqual(["bug"]);
    } finally {
      db.close();
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("projector: helpers", () => {
  test("repoOf and prFromPayload", () => {
    expect(repoOf({ repository: { full_name: "a/b" } })).toBe("a/b");
    expect(repoOf({})).toBeNull();
    expect(prFromPayload({ pull_request: { number: 1 } })?.number).toBe(1);
  });
});
