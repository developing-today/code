import { afterEach, describe, expect, it, vi } from "vitest";
import {
  buildCommentTouch,
  buildIssueRow,
  deletionBlocks,
  buildPrRow,
  isAgentNote,
  labelsOf,
  mirrorCommentEvent,
  mirrorIssueEvent,
  mirrorPrEvent,
} from "../src/mirror";
import type { SinkEnv } from "../src/sink";

const env: SinkEnv = {
  TURSO_QUEUE_URL: "https://fleet-queue.example.invalid",
  TURSO_QUEUE_TOKEN: "queue-token",
};

const REPO = "developing-today/code";

const issuePayload = {
  action: "labeled",
  label: { name: "status/in-progress" },
  issue: {
    number: 8580,
    title: "Fleet unification slice 2",
    body: "mirror everything",
    state: "open",
    labels: [{ name: "task/small-fix" }, { name: "priority/high" }],
    assignees: [{ login: "dezren39" }],
    comments: 3,
    user: { login: "dezren39" },
    html_url: "https://example.invalid/issues/8580",
    created_at: "2026-11-11T01:00:00Z",
    updated_at: "2026-11-11T02:00:00Z",
  },
  repository: { full_name: REPO },
};

const prPayload = {
  action: "closed",
  pull_request: {
    number: 8601,
    title: "mirror writes",
    body: "implements the mirror",
    state: "closed",
    merged: true,
    merged_at: "2026-11-11T03:00:00Z",
    draft: false,
    head: { sha: "abc123", ref: "queue/14908-8580-github-mirror" },
    base: { ref: "main" },
    labels: [{ name: "queue/claimed" }],
    assignees: [],
    assignee: { login: "dezren39" },
    comments: 1,
    user: { login: "dezren39" },
    html_url: "https://example.invalid/pull/8601",
    created_at: "2026-11-11T01:30:00Z",
    updated_at: "2026-11-11T03:00:00Z",
  },
  repository: { full_name: REPO },
};

interface CapturedStmt {
  sql: string;
  args?: { type: string; value?: string }[];
}

/** Stub fetch as the Turso pipeline endpoint; return captured statements. */
function capturePipeline(): CapturedStmt[] {
  const stmts: CapturedStmt[] = [];
  vi.stubGlobal("fetch", async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body)) as {
      requests: { type: string; stmt?: CapturedStmt }[];
    };
    for (const r of body.requests) if (r.stmt) stmts.push(r.stmt);
    return new Response(
      JSON.stringify({
        results: body.requests.map((r) =>
          r.type === "execute"
            ? {
                type: "success",
                response: { type: "execute", result: { rows: [] } },
              }
            : { type: "success" },
        ),
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  return stmts;
}

afterEach(() => vi.unstubAllGlobals());

describe("mirror builders (#8580)", () => {
  it("labelsOf returns sorted label names", () => {
    expect(labelsOf({ labels: [{ name: "b" }, { name: "a" }] })).toEqual([
      "a",
      "b",
    ]);
    expect(labelsOf({})).toEqual([]);
  });

  it("buildIssueRow maps the issue fields with stable JSON", () => {
    const row = buildIssueRow(issuePayload, REPO, "hash123");
    expect(row.number).toBe(8580);
    expect(row.title).toBe("Fleet unification slice 2");
    expect(row.body_hash).toBe("hash123");
    expect(row.state).toBe("open");
    expect(row.labels).toBe('["priority/high","task/small-fix"]');
    expect(row.assignees).toBe('["dezren39"]');
    expect(row.comments).toBe(3);
    expect(row.author).toBe("dezren39");
  });

  it("buildPrRow detects merged and head/base", () => {
    const row = buildPrRow(prPayload, REPO, "hash456");
    expect(row.merged).toBe(1);
    expect(row.draft).toBe(0);
    expect(row.head_sha).toBe("abc123");
    expect(row.head_ref).toBe("queue/14908-8580-github-mirror");
    expect(row.base_ref).toBe("main");
    expect(row.assignees).toBe('["dezren39"]');
  });

  it("isAgentNote keys on the marker, not the author", () => {
    expect(isAgentNote("text\n<!-- agent-note:v1 agent=x -->")).toBe(true);
    expect(isAgentNote("🤖 **Agent note** · hi")).toBe(true);
    expect(isAgentNote("ordinary comment")).toBe(false);
    expect(isAgentNote(null)).toBe(false);
  });

  it("buildCommentTouch targets PR rows and only notes agent comments", () => {
    const note = buildCommentTouch(
      {
        action: "created",
        issue: { number: 8601, comments: 5, pull_request: { url: "x" } },
        comment: {
          id: 4242,
          html_url: "https://example.invalid/pull/8601#c4242",
          body: "🤖 **Agent note** · done\n<!-- agent-note:v1 agent=repo-worker-code -->",
        },
      },
      REPO,
    );
    expect(note).toMatchObject({
      isPr: true,
      number: 8601,
      comments: 5,
      agentNoteId: 4242,
    });
    const plain = buildCommentTouch(
      {
        action: "created",
        issue: { number: 8580, comments: 4 },
        comment: { id: 99, html_url: "u", body: "thanks!" },
      },
      REPO,
    );
    expect(plain).toMatchObject({
      isPr: false,
      agentNoteId: null,
      agentNoteUrl: null,
    });
  });

  it("buildCommentTouch never assigns the note pointer from a deletion", () => {
    const deleted = buildCommentTouch(
      {
        action: "deleted",
        issue: { number: 8580, comments: 2 },
        comment: {
          id: 555,
          html_url: "https://example.invalid/issues/8580#c555",
          body: "🤖 **Agent note** · old\n<!-- agent-note:v1 agent=repo-worker-code -->",
        },
      },
      REPO,
    );
    expect(deleted).toMatchObject({
      agentNoteId: null,
      agentNoteUrl: null,
      deletedNoteId: 555,
      comments: 2,
    });
    const created = buildCommentTouch(
      {
        action: "created",
        issue: { number: 8580, comments: 3 },
        comment: {
          id: 556,
          html_url: "https://example.invalid/issues/8580#c556",
          body: "🤖 **Agent note** · new\n<!-- agent-note:v1 agent=repo-worker-code -->",
        },
      },
      REPO,
    );
    expect(created).toMatchObject({ agentNoteId: 556, deletedNoteId: null });
  });
});

describe("mirror writers (#8580)", () => {
  it("mirrorIssueEvent ensures schema, upserts the row, stamps meta", async () => {
    const stmts = capturePipeline();
    await mirrorIssueEvent(env, issuePayload, REPO);
    const sql = stmts.map((s) => s.sql).join("\n");
    expect(sql).toContain("CREATE TABLE IF NOT EXISTS github_mirror_issues");
    expect(sql).toContain("INSERT INTO github_mirror_issues");
    expect(sql).toContain("ON CONFLICT(repo, number) DO UPDATE");
    // Out-of-order deliveries must not overwrite a newer stored snapshot.
    expect(sql).toContain(
      "excluded.gh_updated_at >= github_mirror_issues.gh_updated_at",
    );
    expect(sql).toContain("INSERT INTO github_mirror_meta");
    const upsert = stmts.find((s) =>
      s.sql.includes("INSERT INTO github_mirror_issues"),
    );
    const values = (upsert?.args ?? []).map((a) => a.value);
    expect(values).toContain(REPO);
    expect(values).toContain("8580");
    expect(values).toContain('["priority/high","task/small-fix"]');
    // body hash is a real SHA-256 of the payload body
    expect(
      values.some((v) => typeof v === "string" && /^[0-9a-f]{64}$/.test(v)),
    ).toBe(true);
  });

  it("mirrorPrEvent upserts the PR row with merged state", async () => {
    const stmts = capturePipeline();
    await mirrorPrEvent(env, prPayload, REPO);
    const upsert = stmts.find((s) =>
      s.sql.includes("INSERT INTO github_mirror_pull_requests"),
    );
    expect(upsert).toBeDefined();
    expect(upsert?.sql).toContain(
      "excluded.gh_updated_at >= github_mirror_pull_requests.gh_updated_at",
    );
    const values = (upsert?.args ?? []).map((a) => a.value);
    expect(values).toContain("8601");
    expect(values).toContain("abc123");
    expect(values).toContain("main");
  });

  it("mirrorCommentEvent seeds-then-updates comment fields only", async () => {
    const stmts = capturePipeline();
    await mirrorCommentEvent(
      env,
      {
        action: "created",
        issue: { number: 8580, comments: 4 },
        comment: {
          id: 777,
          html_url: "https://example.invalid/issues/8580#c777",
          body: "🤖 **Agent note** · checkpoint\n<!-- agent-note:v1 agent=repo-worker-code -->",
        },
        repository: { full_name: REPO },
      },
      REPO,
    );
    const sql = stmts.map((s) => s.sql).join("\n");
    expect(sql).toContain("ON CONFLICT(repo, number) DO NOTHING");
    expect(sql).toContain("UPDATE github_mirror_issues SET");
    expect(sql).toContain("latest_agent_note_id = CASE");
    expect(sql).toContain("OR ? >= latest_agent_note_id");
    const update = stmts.find((s) =>
      s.sql.startsWith("UPDATE github_mirror_issues"),
    );
    const values = (update?.args ?? []).map((a) => a.value);
    expect(values).toContain("777");
    expect(values).toContain("4");
  });

  it("mirrorIssueEvent deletes the row on an issues deleted delivery", async () => {
    const stmts = capturePipeline();
    await mirrorIssueEvent(env, { ...issuePayload, action: "deleted" }, REPO);
    const sql = stmts.map((s) => s.sql).join("\n");
    // A deleted issue leaves the read model (projector precedent, #4210):
    // no upsert of its final snapshot, and the meta stamp still lands.
    expect(sql).toContain(
      "DELETE FROM github_mirror_issues WHERE repo = ? AND number = ?",
    );
    expect(sql).not.toContain("INSERT INTO github_mirror_issues");
    expect(sql).toContain("INSERT INTO github_mirror_meta");
    const del = stmts.find((s) => s.sql.startsWith("DELETE FROM"));
    const values = (del?.args ?? []).map((a) => a.value);
    expect(values).toContain(REPO);
    expect(values).toContain("8580");
  });

  it("deletionBlocks absorbs items at or before the marker version", () => {
    // Oracle: the projector's deletionBlocks rule (#6782) — a
    // deletion marker at or after the incoming item's source
    // timestamp absorbs the item; only a strictly newer item passes.
    expect(deletionBlocks(null, "2026-11-11T02:00:00Z")).toBe(false);
    expect(deletionBlocks("2026-11-11T03:00:00Z", "2026-11-11T02:00:00Z")).toBe(
      true,
    );
    expect(deletionBlocks("2026-11-11T03:00:00Z", "2026-11-11T03:00:00Z")).toBe(
      true,
    );
    expect(deletionBlocks("2026-11-11T03:00:00Z", "2026-11-11T04:00:00Z")).toBe(
      false,
    );
    // No provable order: a marker absorbs an unversioned item, and a
    // versionless marker still absorbs.
    expect(deletionBlocks("2026-11-11T03:00:00Z", null)).toBe(true);
    expect(deletionBlocks("", "2026-11-11T02:00:00Z")).toBe(true);
    expect(deletionBlocks(null, null)).toBe(false);
  });

  it("mirrorIssueEvent writes a tombstone on an issues deleted delivery", async () => {
    const stmts = capturePipeline();
    await mirrorIssueEvent(env, { ...issuePayload, action: "deleted" }, REPO);
    const tomb = stmts.find((s) =>
      s.sql.includes("INSERT INTO github_mirror_tombstones"),
    );
    expect(tomb).toBeDefined();
    const values = (tomb?.args ?? []).map((a) => a.value);
    expect(values).toContain(REPO);
    expect(values).toContain("8580");
    // The marker carries the deleted issue's own updated_at: that is
    // the version delayed pre-deletion deliveries are compared to.
    expect(values).toContain("2026-11-11T02:00:00Z");
  });

  it("the tombstone is written before the row deletion", async () => {
    // Oracle (Codex P2, PR #8604): the batch is a sequence of
    // separate executes, not a transaction, so the marker must be
    // established first — a delayed delivery inserting after the
    // marker is guard-blocked, and one inserting before it is
    // removed by the DELETE that follows. DELETE-then-marker would
    // leave the window between them unguarded.
    const stmts = capturePipeline();
    await mirrorIssueEvent(env, { ...issuePayload, action: "deleted" }, REPO);
    const tombIdx = stmts.findIndex((s) =>
      s.sql.includes("INSERT INTO github_mirror_tombstones"),
    );
    const delIdx = stmts.findIndex((s) => s.sql.startsWith("DELETE FROM"));
    expect(tombIdx).toBeGreaterThanOrEqual(0);
    expect(delIdx).toBeGreaterThanOrEqual(0);
    expect(tombIdx).toBeLessThan(delIdx);
  });

  it("both insertion paths carry the tombstone guard in SQL", async () => {
    // The guard is encoded in the statements themselves (atomic
    // with the write), in every path that can create a mirror row:
    // the snapshot upsert and the comment seed + update.
    let stmts = capturePipeline();
    await mirrorIssueEvent(env, issuePayload, REPO);
    const upsert = stmts.find((s) =>
      s.sql.includes("INSERT INTO github_mirror_issues"),
    );
    expect(upsert?.sql).toContain("FROM github_mirror_tombstones");
    expect(upsert?.sql).toContain("m.gh_updated_at >= ?");
    stmts = capturePipeline();
    await mirrorCommentEvent(
      env,
      {
        action: "created",
        issue: {
          number: 8580,
          comments: 4,
          updated_at: "2026-11-11T02:00:00Z",
        },
        comment: { id: 777, body: "a plain comment" },
        repository: { full_name: REPO },
      },
      REPO,
    );
    const seed = stmts.find((s) =>
      s.sql.includes(
        "INSERT INTO github_mirror_issues (repo, number, mirrored_at)",
      ),
    );
    expect(seed?.sql).toContain("FROM github_mirror_tombstones");
    const update = stmts.find((s) =>
      s.sql.startsWith("UPDATE github_mirror_issues"),
    );
    expect(update?.sql).toContain("FROM github_mirror_tombstones");
  });

  it("mirrorCommentEvent guards the note pointer by creation order", async () => {
    const stmts = capturePipeline();
    await mirrorCommentEvent(
      env,
      {
        action: "created",
        issue: { number: 8580, comments: 4 },
        comment: {
          id: 777,
          html_url: "https://example.invalid/issues/8580#c777",
          body: "🤖 **Agent note** · checkpoint\n<!-- agent-note:v1 agent=repo-worker-code -->",
        },
        repository: { full_name: REPO },
      },
      REPO,
    );
    const update = stmts.find((s) =>
      s.sql.startsWith("UPDATE github_mirror_issues SET"),
    );
    // Both pointer fields move only when the incoming note id is at
    // least the stored one, so an older redelivery cannot overwrite
    // the pointer to a newer note.
    expect(update?.sql).toContain("latest_agent_note_id = CASE");
    expect(update?.sql).toContain("latest_agent_note_url = CASE");
    expect(update?.sql).toContain("OR ? >= latest_agent_note_id");
    const values = (update?.args ?? []).map((a) => a.value);
    expect(values.filter((v) => v === "777")).toHaveLength(5);
  });

  it("mirrorCommentEvent versions the count by the object updated_at", async () => {
    const stmts = capturePipeline();
    await mirrorCommentEvent(
      env,
      {
        action: "created",
        issue: {
          number: 8580,
          comments: 5,
          updated_at: "2026-11-11T04:00:00Z",
        },
        comment: {
          id: 888,
          html_url: "https://example.invalid/issues/8580#c888",
          body: "plain comment",
        },
        repository: { full_name: REPO },
      },
      REPO,
    );
    const update = stmts.find((s) =>
      s.sql.startsWith("UPDATE github_mirror_issues SET"),
    );
    // The count moves only when the carrying object's updated_at is
    // at least the stored COUNT version (comments_gh_updated_at),
    // which advances with it — separately from the snapshot version:
    // a comment event never reads or moves gh_updated_at, so a newer
    // comment cannot make a delayed older snapshot look stale.
    expect(update?.sql).toContain("comments = CASE");
    expect(update?.sql).toContain("OR ? >= comments_gh_updated_at");
    expect(update?.sql).toContain("THEN COALESCE(?, comments_gh_updated_at)");
    expect(update?.sql).not.toMatch(/\n\s+gh_updated_at = CASE/);
    const values = (update?.args ?? []).map((a) => a.value);
    // 5 count-version binds (CASE guards + COALESCE) plus 2 in the tombstone guard.
    expect(values.filter((v) => v === "2026-11-11T04:00:00Z")).toHaveLength(7);
    expect(values).toContain("5");
    // buildCommentTouch carries the version off the issue object.
    const touch = buildCommentTouch(
      {
        action: "created",
        issue: {
          number: 8580,
          comments: 5,
          updated_at: "2026-11-11T04:00:00Z",
        },
        comment: { id: 888, html_url: "u", body: "hi" },
      },
      REPO,
    );
    expect(touch.ghUpdatedAt).toBe("2026-11-11T04:00:00Z");
  });

  it("snapshot upserts move the count only under the count-version guard", async () => {
    const stmts = capturePipeline();
    await mirrorIssueEvent(env, issuePayload, REPO);
    const upsert = stmts.find((s) =>
      s.sql.includes("INSERT INTO github_mirror_issues"),
    );
    // The snapshot's count is guarded by the separate count version,
    // not the snapshot version: a snapshot older than the newest
    // comment still applies its state but cannot regress the count.
    expect(upsert?.sql).toContain("comments_gh_updated_at");
    expect(upsert?.sql).toContain("comments = CASE");
    expect(upsert?.sql).toContain(
      "OR excluded.gh_updated_at >= github_mirror_issues.comments_gh_updated_at",
    );
    const values = (upsert?.args ?? []).map((a) => a.value);
    // The count version inserted with a fresh row is the object's
    // own updated_at (the count's version at snapshot time).
    expect(
      values.filter((v) => v === "2026-11-11T02:00:00Z").length,
    ).toBeGreaterThan(1);
  });

  it("mirrorCommentEvent clears a pointer naming the deleted note", async () => {
    const stmts = capturePipeline();
    await mirrorCommentEvent(
      env,
      {
        action: "deleted",
        issue: { number: 8580, comments: 2 },
        comment: {
          id: 777,
          html_url: "https://example.invalid/issues/8580#c777",
          body: "🤖 **Agent note** · checkpoint\n<!-- agent-note:v1 agent=repo-worker-code -->",
        },
        repository: { full_name: REPO },
      },
      REPO,
    );
    const clear = stmts.find((s) =>
      s.sql.includes("latest_agent_note_id = NULL"),
    );
    expect(clear).toBeDefined();
    const clearValues = (clear?.args ?? []).map((a) => a.value);
    expect(clearValues).toContain("777");
    expect(clearValues).toContain("8580");
    // The comments update itself assigns no pointer for a deletion:
    // its CASE note args carry no value.
    const update = stmts.find((s) =>
      s.sql.startsWith("UPDATE github_mirror_issues SET"),
    );
    const updateValues = (update?.args ?? []).map((a) => a.value);
    expect(updateValues).not.toContain(
      "https://example.invalid/issues/8580#c777",
    );
  });
});
