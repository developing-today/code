import { describe, test, expect } from "bun:test";
import { applyEvent, type ProjectorStore, prFromPayload, repoOf } from "../src/projector";
import type { ItemRow, CommentRow } from "../src/store";

function fakeStore() {
  const items = new Map<string, Omit<ItemRow, "fetched_at">>();
  const comments = new Map<string, Omit<CommentRow, "fetched_at">>();
  const pushes: any[] = [];
  const deleted: string[] = [];
  const store: ProjectorStore = {
    async upsertItem(i) {
      items.set(i.ref, i);
    },
    async upsertComment(c) {
      comments.set(c.id, c);
    },
    async markCommentDeleted(id) {
      deleted.push(id);
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
        html_url: "https://github.com/developing-today/code/pull/201#discussion_r601",
      },
    });
    const rc = comments.get("601");
    expect(rc?.kind).toBe("review_comment");
    expect(rc?.path).toBe("src/foo.ts");
    expect(rc?.line).toBe(42);
  });
});

describe("projector: comment deletion + push", () => {
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

describe("projector: helpers", () => {
  test("repoOf and prFromPayload", () => {
    expect(repoOf({ repository: { full_name: "a/b" } })).toBe("a/b");
    expect(repoOf({})).toBeNull();
    expect(prFromPayload({ pull_request: { number: 1 } })?.number).toBe(1);
  });
});
