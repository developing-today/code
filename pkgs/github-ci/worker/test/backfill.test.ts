import { describe, expect, it } from "vitest";
import {
  backfillStampStmt,
  buildCommentTouch,
  buildIssueRow,
  buildPrRow,
  clearAgentNotePointerStmts,
  commentTouchStmts,
  forceCommentsStmt,
  issueUpsertStmt,
  prUpsertStmt,
  tombstoneDeleteStmts,
} from "../src/mirror";

const REPO = "developing-today/code";

const issueObj = {
  number: 8580,
  title: "Fleet unification slice 2",
  body: "mirror everything",
  state: "open",
  labels: [{ name: "task/small-fix" }],
  assignees: [{ login: "dezren39" }],
  comments: 3,
  user: { login: "dezren39" },
  html_url: "https://example.invalid/issues/8580",
  created_at: "2026-11-11T01:00:00Z",
  updated_at: "2026-11-11T02:00:00Z",
};

const prObj = {
  ...issueObj,
  number: 8601,
  state: "closed",
  merged: true,
  merged_at: "2026-11-11T03:00:00Z",
  draft: false,
  head: { sha: "abc123", ref: "queue/14908-8580-github-mirror" },
  base: { ref: "main" },
  comments: 1,
};

describe("sweep statement builders (#8580 backfill)", () => {
  it("issue upsert carries the tombstone guard and every row field", () => {
    const stmt = issueUpsertStmt(buildIssueRow({ issue: issueObj }, REPO, "deadbeef"));
    expect(stmt.sql).toContain("INSERT INTO github_mirror_issues");
    expect(stmt.sql).toContain("NOT EXISTS");
    expect(stmt.sql).toContain("github_mirror_tombstones");
    // 14 row fields + mirrored_at is among them; 4 guard binds follow.
    expect(stmt.args).toHaveLength(18);
    expect(stmt.args?.[0]).toEqual({ type: "text", value: REPO });
    expect(stmt.args?.[1]).toEqual({ type: "integer", value: "8580" });
  });

  it("PR upsert binds merged/head/base and the guard", () => {
    const stmt = prUpsertStmt(buildPrRow({ pull_request: prObj }, REPO, "cafe"));
    expect(stmt.sql).toContain("INSERT INTO github_mirror_pull_requests");
    expect(stmt.sql).toContain("NOT EXISTS");
    expect(stmt.args).toHaveLength(23);
    expect(stmt.args?.[5]).toEqual({ type: "integer", value: "1" }); // merged
    expect(stmt.args?.[7]).toEqual({ type: "text", value: "abc123" }); // head_sha
  });

  it("backfill stamp touches last_backfill_at and never the webhook fields", () => {
    const stmt = backfillStampStmt(REPO, "2026-11-14T15:00:00Z");
    expect(stmt.sql).toContain("last_backfill_at");
    expect(stmt.sql).not.toContain("last_webhook_at");
    expect(stmt.sql).not.toContain("last_webhook_event");
    expect(stmt.args).toEqual([
      { type: "text", value: REPO },
      { type: "text", value: "2026-11-14T15:00:00Z" },
    ]);
  });

  it("tombstone delete writes the marker before deleting from both tables", () => {
    const stmts = tombstoneDeleteStmts(REPO, 4242);
    expect(stmts).toHaveLength(3);
    expect(stmts[0].sql).toContain("INSERT INTO github_mirror_tombstones");
    // A version-less marker: gh_updated_at is a literal NULL in the
    // statement (binds are repo, number, mirrored_at), so the guard
    // absorbs every later insertion for this number.
    expect(stmts[0].sql).toContain("NULL");
    expect(stmts[0].args).toHaveLength(3);
    expect(stmts[1].sql).toContain("DELETE FROM github_mirror_issues");
    expect(stmts[2].sql).toContain("DELETE FROM github_mirror_pull_requests");
  });

  it("clear-pointer is conditional on the inspected note and targets the right table", () => {
    const issueStmts = clearAgentNotePointerStmts(REPO, 1, false, 777, false);
    expect(issueStmts).toHaveLength(1);
    expect(issueStmts[0].sql).toContain("UPDATE github_mirror_issues SET latest_agent_note_id = NULL");
    // Conditional: a newer pointer installed after the sweep's read survives.
    expect(issueStmts[0].sql).toContain("latest_agent_note_id = ?");
    expect(issueStmts[0].args?.[3]).toEqual({ type: "integer", value: "777" });
    const prStmts = clearAgentNotePointerStmts(REPO, 1, true, 777, false);
    expect(prStmts[0].sql).toContain("UPDATE github_mirror_pull_requests SET latest_agent_note_id = NULL");
  });

  it("clear-pointer records the deletion marker first when the comment is gone", () => {
    const stmts = clearAgentNotePointerStmts(REPO, 1, false, 777, true);
    expect(stmts).toHaveLength(2);
    expect(stmts[0].sql).toContain("INSERT INTO github_mirror_deleted_notes");
    expect(stmts[0].args?.[2]).toEqual({ type: "integer", value: "777" });
    expect(stmts[1].sql).toContain("UPDATE github_mirror_issues");
  });

  it("PR comments version comes from comments_updated_at when the sweep joins it", () => {
    const joined = { ...prObj, comments_updated_at: "2026-11-11T02:30:00Z" };
    const stmt = prUpsertStmt(buildPrRow({ pull_request: joined }, REPO, "cafe"));
    // Bind 13 is the comments_gh_updated_at slot: the count's own version.
    expect(stmt.args?.[13]).toEqual({
      type: "text",
      value: "2026-11-11T02:30:00Z",
    });
    expect(stmt.sql).toContain("excluded.comments_gh_updated_at >= github_mirror_pull_requests.comments_gh_updated_at");
    // Without the join field (webhook path) it falls back to updated_at.
    const plain = prUpsertStmt(buildPrRow({ pull_request: prObj }, REPO, "cafe"));
    expect(plain.args?.[13]).toEqual({ type: "text", value: prObj.updated_at });
  });

  it("force-comments targets the right table and is version-guarded", () => {
    const stmt = forceCommentsStmt(REPO, 10056, true, 4, "2026-11-14T14:35:00Z", "2026-11-14T15:40:00Z");
    expect(stmt.sql).toContain("UPDATE github_mirror_pull_requests SET comments = ?");
    // The write only lands if the stored comments version is still
    // the stamp the sweep diffed against — a newer webhook count is
    // never overwritten by an older API count.
    expect(stmt.sql).toContain("comments_gh_updated_at IS ?");
    expect(stmt.args?.[0]).toEqual({ type: "integer", value: "4" });
    expect(stmt.args?.[1]).toEqual({
      type: "text",
      value: "2026-11-14T14:35:00Z",
    });
    expect(stmt.args?.[5]).toEqual({
      type: "text",
      value: "2026-11-14T15:40:00Z",
    });
    expect(forceCommentsStmt(REPO, 1, false, 0, null, null).sql).toContain("UPDATE github_mirror_issues");
  });

  it("comment touch: plain comment is seed+update; deleted note adds marker+clear", () => {
    const plain = buildCommentTouch(
      {
        issue: { number: 5, comments: 2, updated_at: "2026-11-11T02:00:00Z" },
        comment: { id: 9, body: "hi" },
        action: "created",
      },
      REPO,
    );
    expect(commentTouchStmts(plain)).toHaveLength(2);
    const deletedNote = buildCommentTouch(
      {
        issue: { number: 5, comments: 1, updated_at: "2026-11-11T02:00:00Z" },
        comment: {
          id: 9,
          body: "<!-- agent-note:v1 --> note",
          html_url: "https://example.invalid/c9",
        },
        action: "deleted",
      },
      REPO,
    );
    expect(commentTouchStmts(deletedNote)).toHaveLength(4);
  });
});
