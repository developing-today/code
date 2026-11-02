import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { commentKindCounts, CommentsStore, queuedComments } from "../src/comments.ts";
import type { DashComment } from "../src/model.ts";
import { PAGES } from "../src/pages.ts";
import { commentBadge, dashCommentHtml, renderCommentsQueue } from "../src/render.ts";
import { fixtureSource } from "../src/sources/fixtures.ts";

// #1210: the comments queue view, kind/state-aware counts, and honest
// failed-state rendering (age + last error, no phantom retry promise).

const AUTHOR = { kind: "drew", id: "drew", label: "Drew" } as const;

function comment(partial: Partial<DashComment>): DashComment {
  return {
    id: "dc-test",
    objectRef: "gh:developing-today/code#1210",
    kind: "note",
    author: { ...AUTHOR },
    body: "body",
    createdAt: "2026-11-01T00:00:00.000Z",
    ...partial,
  };
}

const NEW_MESSAGE = comment({ id: "m-new", kind: "message", triageState: "new", body: "please triage me" });
const TRIAGED_MESSAGE = comment({ id: "m-done", kind: "message", triageState: "triaged", body: "already triaged" });
const PENDING_GH = comment({
  id: "g-pending",
  kind: "github",
  githubState: "pending",
  githubRef: "developing-today/code#1210",
  body: "waiting to post",
  createdAt: "2026-10-31T00:00:00.000Z",
});
const FAILED_GH = comment({
  id: "g-failed",
  kind: "github",
  githubState: "failed",
  githubRef: "developing-today/code#1210",
  githubError: "gh: HTTP 403 forbidden",
  body: "failed to post",
  createdAt: "2026-10-30T00:00:00.000Z",
});
const POSTED_GH = comment({
  id: "g-posted",
  kind: "github",
  githubState: "posted",
  githubUrl: "https://github.com/developing-today/code/issues/1210#issuecomment-1",
  body: "already posted",
});
const NOTE = comment({ id: "n-1", kind: "note", body: "just a note" });
const ALL = [NEW_MESSAGE, TRIAGED_MESSAGE, PENDING_GH, FAILED_GH, POSTED_GH, NOTE];

describe("queuedComments (#1210)", () => {
  test("lists new messages and pending/failed github comments, oldest first, and omits terminal items and notes", () => {
    const queued = queuedComments(ALL);
    expect(queued.map((c) => c.id)).toEqual(["g-failed", "g-pending", "m-new"]);
  });
});

describe("commentKindCounts (#1210)", () => {
  test("distinguishes kind and state; actionable counts only work waiting on a loop", () => {
    const k = commentKindCounts(ALL);
    expect(k.total).toBe(6);
    expect(k.notes).toBe(1);
    expect(k.messages).toBe(2);
    expect(k.messagesNew).toBe(1);
    expect(k.github).toBe(3);
    expect(k.githubPending).toBe(1);
    expect(k.githubFailed).toBe(1);
    expect(k.actionable).toBe(3);
  });

  test("badge renders the total plus the queued count and a kind/state title", () => {
    const detailed = new Map([["gh:developing-today/code#1210", commentKindCounts(ALL)]]);
    const badge = commentBadge("gh:developing-today/code#1210", detailed);
    expect(badge).toContain("💬 6 · 3 queued");
    expect(badge).toContain("1 notes");
    expect(badge).toContain("1 messages awaiting triage");
    expect(badge).toContain("1 failed to post");
    // A plain number count keeps the original badge shape.
    const plain = commentBadge("gh:developing-today/code#1210", new Map([["gh:developing-today/code#1210", 2]]));
    expect(plain).toContain("💬 2");
    expect(plain).not.toContain("queued");
    expect(plain).toContain('title="Dashboard comments on this item"');
  });
});

describe("dashCommentHtml failed state (#1210)", () => {
  test("shows age and last error and never promises a retry", () => {
    const html = dashCommentHtml(FAILED_GH);
    expect(html).toContain("posting failed");
    expect(html).toContain("last error: gh: HTTP 403 forbidden");
    expect(html).toContain("no automatic retry exists");
    expect(html).not.toContain("will retry");
    // Age: created 2026-10-30, rendered now (test run is later) — a
    // relative stamp is present in the failed copy.
    expect(html).toMatch(/posting failed (just now|\d+[mhd] ago)/);
  });

  test("a failed comment without a stored error still drops the retry promise", () => {
    const html = dashCommentHtml(comment({ kind: "github", githubState: "failed", githubRef: "developing-today/code#1210" }));
    expect(html).toContain("no automatic retry exists");
    expect(html).not.toContain("last error");
    expect(html).not.toContain("will retry");
  });
});

describe("renderCommentsQueue (#1210)", () => {
  test("lists the queued items with their object refs and omits terminal items and notes", () => {
    const html = renderCommentsQueue(ALL, fixtureSource().provenance());
    expect(html).toContain("Comments queue");
    expect(html).toContain("please triage me");
    expect(html).toContain("waiting to post");
    expect(html).toContain("failed to post");
    expect(html).toContain("gh: HTTP 403 forbidden");
    expect(html).not.toContain("already triaged");
    expect(html).not.toContain("already posted");
    expect(html).not.toContain("just a note");
    expect(html).toContain("3 queued");
    expect(html).toContain("/discuss/gh%3Adeveloping-today%2Fcode%231210");
  });

  test("empty queue renders the nothing-queued state", () => {
    const html = renderCommentsQueue([TRIAGED_MESSAGE, POSTED_GH, NOTE], fixtureSource().provenance());
    expect(html).toContain("Nothing is queued");
    expect(html).toContain("0 queued");
  });

  test("the page is registered in the manifest with its api twin", () => {
    const page = PAGES.find((p) => p.id === "comments-queue");
    expect(page?.route).toBe("/comments");
    expect(page?.api).toBe("/api/comments/queue.json");
  });
});

describe("CommentsStore githubError persistence (#1210)", () => {
  test("an error report stores the last error; a later post clears it; a repeat failure refreshes it", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-comments-1210-"));
    try {
      const file = join(dir, "comments.json");
      const store = new CommentsStore(file);
      const c = await store.add({
        objectRef: "gh:developing-today/code#1210",
        kind: "github",
        author: { ...AUTHOR },
        body: "post me",
        githubRef: "developing-today/code#1210",
      });
      const failed = await store.markGithubResult(c.id, { error: "first failure" });
      expect(failed?.githubState).toBe("failed");
      expect(failed?.githubError).toBe("first failure");
      const failedAgain = await store.markGithubResult(c.id, { error: "second failure" });
      expect(failedAgain?.githubError).toBe("second failure");
      // Persisted, not just in memory.
      const reloaded = new CommentsStore(file);
      const stored = (await reloaded.all()).find((x) => x.id === c.id);
      expect(stored?.githubError).toBe("second failure");
      const posted = await store.markGithubResult(c.id, { url: "https://github.com/developing-today/code/issues/1210#issuecomment-9" });
      expect(posted?.githubState).toBe("posted");
      expect(posted?.githubError).toBeUndefined();
      // countsDetailed groups the posted comment as non-actionable.
      const detailed = await store.countsDetailed();
      expect(detailed.get("gh:developing-today/code#1210")?.actionable).toBe(0);
      expect(detailed.get("gh:developing-today/code#1210")?.github).toBe(1);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
