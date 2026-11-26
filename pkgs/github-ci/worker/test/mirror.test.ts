import { afterEach, describe, expect, it, vi } from "vitest";
import {
  bodyHashOf,
  buildCommentTouch,
  buildIssueRow,
  deletionBlocks,
  buildPrRow,
  isAgentNote,
  labelDiff,
  labelStartStmt,
  labelsOf,
  parseLabelSet,
  mirrorCommentEvent,
  mirrorIssueEvent,
  mirrorPrEvent,
  mirrorSubIssueEvent,
  parentOf,
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
    expect(labelsOf({ labels: [{ name: "b" }, { name: "a" }] })).toEqual(["a", "b"]);
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
    expect(sql).toContain("excluded.gh_updated_at >= github_mirror_issues.gh_updated_at");
    expect(sql).toContain("INSERT INTO github_mirror_meta");
    const upsert = stmts.find((s) => s.sql.includes("INSERT INTO github_mirror_issues"));
    const values = (upsert?.args ?? []).map((a) => a.value);
    expect(values).toContain(REPO);
    expect(values).toContain("8580");
    expect(values).toContain('["priority/high","task/small-fix"]');
    // body hash is a real SHA-256 of the payload body
    expect(values.some((v) => typeof v === "string" && /^[0-9a-f]{64}$/.test(v))).toBe(true);
  });

  it("mirrorPrEvent upserts the PR row with merged state", async () => {
    const stmts = capturePipeline();
    await mirrorPrEvent(env, prPayload, REPO);
    const upsert = stmts.find((s) => s.sql.includes("INSERT INTO github_mirror_pull_requests"));
    expect(upsert).toBeDefined();
    expect(upsert?.sql).toContain("excluded.gh_updated_at >= github_mirror_pull_requests.gh_updated_at");
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
    const update = stmts.find((s) => s.sql.startsWith("UPDATE github_mirror_issues"));
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
    expect(sql).toContain("DELETE FROM github_mirror_issues WHERE repo = ? AND number = ?");
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
    expect(deletionBlocks("2026-11-11T03:00:00Z", "2026-11-11T02:00:00Z")).toBe(true);
    expect(deletionBlocks("2026-11-11T03:00:00Z", "2026-11-11T03:00:00Z")).toBe(true);
    expect(deletionBlocks("2026-11-11T03:00:00Z", "2026-11-11T04:00:00Z")).toBe(false);
    // No provable order: a marker absorbs an unversioned item, and a
    // versionless marker still absorbs.
    expect(deletionBlocks("2026-11-11T03:00:00Z", null)).toBe(true);
    expect(deletionBlocks("", "2026-11-11T02:00:00Z")).toBe(true);
    expect(deletionBlocks(null, null)).toBe(false);
  });

  it("mirrorIssueEvent writes a tombstone on an issues deleted delivery", async () => {
    const stmts = capturePipeline();
    await mirrorIssueEvent(env, { ...issuePayload, action: "deleted" }, REPO);
    const tomb = stmts.find((s) => s.sql.includes("INSERT INTO github_mirror_tombstones"));
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
    const tombIdx = stmts.findIndex((s) => s.sql.includes("INSERT INTO github_mirror_tombstones"));
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
    const upsert = stmts.find((s) => s.sql.includes("INSERT INTO github_mirror_issues"));
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
    const seed = stmts.find((s) => s.sql.includes("INSERT INTO github_mirror_issues (repo, number, mirrored_at)"));
    expect(seed?.sql).toContain("FROM github_mirror_tombstones");
    const update = stmts.find((s) => s.sql.startsWith("UPDATE github_mirror_issues"));
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
    const update = stmts.find((s) => s.sql.startsWith("UPDATE github_mirror_issues SET"));
    // Both pointer fields move only when the incoming note id is at
    // least the stored one, so an older redelivery cannot overwrite
    // the pointer to a newer note.
    expect(update?.sql).toContain("latest_agent_note_id = CASE");
    expect(update?.sql).toContain("latest_agent_note_url = CASE");
    expect(update?.sql).toContain("OR ? >= latest_agent_note_id");
    const values = (update?.args ?? []).map((a) => a.value);
    expect(values.filter((v) => v === "777")).toHaveLength(7); // 5 ordering binds + 2 deleted-note marker guard binds
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
    const update = stmts.find((s) => s.sql.startsWith("UPDATE github_mirror_issues SET"));
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
    const upsert = stmts.find((s) => s.sql.includes("INSERT INTO github_mirror_issues"));
    // The snapshot's count is guarded by the separate count version,
    // not the snapshot version: a snapshot older than the newest
    // comment still applies its state but cannot regress the count.
    expect(upsert?.sql).toContain("comments_gh_updated_at");
    expect(upsert?.sql).toContain("comments = CASE");
    expect(upsert?.sql).toContain("OR excluded.gh_updated_at >= github_mirror_issues.comments_gh_updated_at");
    const values = (upsert?.args ?? []).map((a) => a.value);
    // The count version inserted with a fresh row is the object's
    // own updated_at (the count's version at snapshot time).
    expect(values.filter((v) => v === "2026-11-11T02:00:00Z").length).toBeGreaterThan(1);
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
    const clear = stmts.find((s) => s.sql.includes("latest_agent_note_id = NULL"));
    expect(clear).toBeDefined();
    const clearValues = (clear?.args ?? []).map((a) => a.value);
    expect(clearValues).toContain("777");
    expect(clearValues).toContain("8580");
    // The comments update itself assigns no pointer for a deletion:
    // its CASE note args carry no value.
    const update = stmts.find((s) => s.sql.startsWith("UPDATE github_mirror_issues SET"));
    const updateValues = (update?.args ?? []).map((a) => a.value);
    expect(updateValues).not.toContain("https://example.invalid/issues/8580#c777");
  });

  it("a deleted note leaves a marker the pointer CASE consults (Codex P2)", async () => {
    const stmts = capturePipeline();
    const noteComment = {
      id: 777,
      html_url: "https://example.invalid/issues/8580#c777",
      body: "🤖 **Agent note** · checkpoint\n<!-- agent-note:v1 agent=repo-worker-code -->",
    };
    await mirrorCommentEvent(
      env,
      {
        action: "deleted",
        issue: { number: 8580, comments: 2 },
        comment: noteComment,
        repository: { full_name: REPO },
      },
      REPO,
    );
    // The deletion records a per-comment marker naming the deleted id.
    const marker = stmts.find((s) => s.sql.includes("INSERT INTO github_mirror_deleted_notes"));
    expect(marker).toBeDefined();
    const markerValues = (marker?.args ?? []).map((a) => a.value);
    expect(markerValues).toContain("777");
    expect(markerValues).toContain("8580");
    // The schema ships with every batch, like the other mirror tables.
    const sql = stmts.map((s) => s.sql).join("\n");
    expect(sql).toContain("CREATE TABLE IF NOT EXISTS github_mirror_deleted_notes");
    // And the pointer assignment itself refuses a marked id, in the
    // same statement: a delayed created redelivery of comment 777
    // cannot resurrect the pointer through the empty-pointer branch.
    const created = capturePipeline();
    await mirrorCommentEvent(
      env,
      {
        action: "created",
        issue: { number: 8580, comments: 3 },
        comment: noteComment,
        repository: { full_name: REPO },
      },
      REPO,
    );
    const update = created.find((s) => s.sql.startsWith("UPDATE github_mirror_issues SET"));
    expect(update?.sql).toContain("github_mirror_deleted_notes");
    expect(update?.sql).toContain("d.comment_id = ?");
    const updateValues = (update?.args ?? []).map((a) => a.value);
    expect(updateValues).toContain("777");
  });
});

interface StoredFixture {
  bodyHash: string | null;
  comments: number;
  commentsGhUpdatedAt: string | null;
  ghUpdatedAt: string | null;
  parentNumber: number | null;
  /** Stored label set as JSON text (#14170 Q5); omitted = column not returned. */
  labels?: string | null;
}

/**
 * Stub fetch as the Turso pipeline endpoint with a scripted stored
 * row: the Q7 pre-read SELECT is answered from `stored` (null = no
 * row), the pragma probe reports the migrated steady state ("3"),
 * the tombstone SELECT finds no marker, and every statement is
 * captured. With failHistory, INSERTs into github_issue_history
 * return an error result — the killed history write of the oracle.
 */
function capturePipelineWithStored(stored: StoredFixture | null, opts?: { failHistory?: boolean }): CapturedStmt[] {
  const stmts: CapturedStmt[] = [];
  const arg = (v: string | null): { type: string; value?: string } =>
    v === null ? { type: "null" } : { type: "text", value: v };
  const int = (v: number | null): { type: string; value?: string } =>
    v === null ? { type: "null" } : { type: "integer", value: String(v) };
  vi.stubGlobal("fetch", async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body)) as {
      requests: { type: string; stmt?: CapturedStmt }[];
    };
    const results = body.requests.map((r) => {
      if (r.type !== "execute" || !r.stmt) return { type: "success" };
      stmts.push(r.stmt);
      const sql = r.stmt.sql;
      if (opts?.failHistory && sql.includes("INSERT INTO github_issue_history")) {
        return { type: "error", error: { message: "history table is gone" } };
      }
      let rows: { type: string; value?: string }[][] = [];
      if (sql.includes("pragma_table_info")) rows = [[{ type: "text", value: "3" }]];
      else if (sql.includes("SELECT body_hash") && stored) {
        rows = [
          [
            arg(stored.bodyHash),
            int(stored.comments),
            arg(stored.commentsGhUpdatedAt),
            arg(stored.ghUpdatedAt),
            int(stored.parentNumber),
            ...(stored.labels === undefined ? [] : [arg(stored.labels)]),
          ],
        ];
      }
      return { type: "success", response: { type: "execute", result: { rows } } };
    });
    return new Response(JSON.stringify({ results }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  });
  return stmts;
}

const historyInserts = (stmts: CapturedStmt[]): CapturedStmt[] =>
  stmts.filter((s) => s.sql.includes("INSERT INTO github_issue_history"));
const kindOf = (s: CapturedStmt): string | undefined => s.args?.[3]?.value;
const detailOf = (s: CapturedStmt): Record<string, unknown> =>
  JSON.parse(String(s.args?.[4]?.value ?? "{}")) as Record<string, unknown>;

const T0 = "2026-11-11T02:00:00Z";
const T1 = "2026-11-11T03:00:00Z";

describe("mirror history (#14170 Q7)", () => {
  it("parentOf: absence is unknown, null is no-parent, a URL names the parent", () => {
    expect(parentOf({})).toEqual({ known: false, parent: null, parentRepo: null });
    expect(parentOf({ parent_issue_url: null })).toEqual({ known: true, parent: null, parentRepo: null });
    expect(parentOf({ parent_issue_url: "https://api.github.com/repos/developing-today/code/issues/20" })).toEqual({
      known: true,
      parent: 20,
      parentRepo: "developing-today/code",
    });
    // An unparseable value must not be guessed into a projection.
    expect(parentOf({ parent_issue_url: "not a url" }).known).toBe(false);
  });

  it("oracle: a body edit writes exactly one body_edited row, hashes only", async () => {
    const oldHash = await bodyHashOf({ body: "the original spec" });
    const newHash = await bodyHashOf(issuePayload.issue);
    const stored: StoredFixture = {
      bodyHash: oldHash,
      comments: 3,
      commentsGhUpdatedAt: T0,
      ghUpdatedAt: T0,
      parentNumber: null,
    };
    const payload = { ...issuePayload, sender: { login: "dezren39" } };
    let stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, payload, REPO);
    const rows = historyInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(kindOf(rows[0])).toBe("body_edited");
    expect(detailOf(rows[0])).toEqual({ old_hash: oldHash, new_hash: newHash });
    // Hashes only: neither body text appears anywhere in the row.
    expect(String(rows[0].args?.[4]?.value)).not.toContain("original spec");
    expect(String(rows[0].args?.[4]?.value)).not.toContain("mirror everything");
    expect(rows[0].args?.[5]).toEqual({ type: "text", value: "dezren39" }); // actor

    // Redelivery of the same edit (stored hash now the new one)
    // writes no second row — exactly one per change.
    stored.bodyHash = newHash;
    stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, payload, REPO);
    expect(historyInserts(stmts)).toHaveLength(0);
  });

  it("a first sight writes no history rows", async () => {
    const stmts = capturePipelineWithStored(null);
    await mirrorIssueEvent(env, { ...issuePayload, action: "opened" }, REPO);
    expect(stmts.some((s) => s.sql.includes("INSERT INTO github_mirror_issues"))).toBe(true);
    expect(historyInserts(stmts)).toHaveLength(0);
  });

  it("a stale snapshot writes no history (its upsert is refused too)", async () => {
    const stored: StoredFixture = {
      bodyHash: "some-other-hash",
      comments: 9,
      commentsGhUpdatedAt: "2026-11-11T05:00:00Z",
      ghUpdatedAt: "2026-11-11T05:00:00Z",
      parentNumber: null,
    };
    const stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, issuePayload, REPO); // payload is older (T0)
    expect(historyInserts(stmts)).toHaveLength(0);
  });

  it("oracle: a snapshot count move writes one comment_count_changed row", async () => {
    const stored: StoredFixture = {
      bodyHash: await bodyHashOf(issuePayload.issue),
      comments: 3,
      commentsGhUpdatedAt: T0,
      ghUpdatedAt: T0,
      parentNumber: null,
    };
    const payload = {
      ...issuePayload,
      issue: { ...issuePayload.issue, comments: 5, updated_at: T1 },
    };
    const stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, payload, REPO);
    const rows = historyInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(kindOf(rows[0])).toBe("comment_count_changed");
    expect(detailOf(rows[0])).toEqual({ from: 3, to: 5 });
  });

  it("oracle: commenting writes comment_added / comment_removed with the delta", async () => {
    const stored: StoredFixture = {
      bodyHash: "hash",
      comments: 3,
      commentsGhUpdatedAt: T0,
      ghUpdatedAt: T0,
      parentNumber: null,
    };
    let stmts = capturePipelineWithStored(stored);
    await mirrorCommentEvent(
      env,
      {
        action: "created",
        issue: { number: 8580, comments: 4, updated_at: T1 },
        comment: { id: 99, html_url: "u", body: "thanks!" },
        repository: { full_name: REPO },
      },
      REPO,
    );
    let rows = historyInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(kindOf(rows[0])).toBe("comment_added");
    expect(detailOf(rows[0])).toEqual({ from: 3, to: 4 });

    stmts = capturePipelineWithStored(stored);
    await mirrorCommentEvent(
      env,
      {
        action: "deleted",
        issue: { number: 8580, comments: 2, updated_at: T1 },
        comment: { id: 98, html_url: "u", body: "gone" },
        repository: { full_name: REPO },
      },
      REPO,
    );
    rows = historyInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(kindOf(rows[0])).toBe("comment_removed");
    expect(detailOf(rows[0])).toEqual({ from: 3, to: 2 });
  });

  it("an agent note's pointer change rides its count row's detail", async () => {
    const stored: StoredFixture = {
      bodyHash: "hash",
      comments: 3,
      commentsGhUpdatedAt: T0,
      ghUpdatedAt: T0,
      parentNumber: null,
    };
    const stmts = capturePipelineWithStored(stored);
    await mirrorCommentEvent(
      env,
      {
        action: "created",
        issue: { number: 8580, comments: 4, updated_at: T1 },
        comment: {
          id: 777,
          html_url: "https://example.invalid/issues/8580#c777",
          body: "🤖 **Agent note** · checkpoint\n<!-- agent-note:v1 agent=repo-worker-code -->",
        },
        repository: { full_name: REPO },
      },
      REPO,
    );
    const rows = historyInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(kindOf(rows[0])).toBe("comment_added");
    expect(detailOf(rows[0])).toEqual({ from: 3, to: 4, agent_note_id: 777 });
  });

  it("a comment on an unseen row seeds it and writes no history", async () => {
    const stmts = capturePipelineWithStored(null);
    await mirrorCommentEvent(
      env,
      {
        action: "created",
        issue: { number: 8580, comments: 1, updated_at: T1 },
        comment: { id: 99, html_url: "u", body: "first!" },
        repository: { full_name: REPO },
      },
      REPO,
    );
    expect(historyInserts(stmts)).toHaveLength(0);
  });

  it("oracle: re-parenting via snapshot writes parent_changed and moves the projection", async () => {
    const stored: StoredFixture = {
      bodyHash: await bodyHashOf(issuePayload.issue),
      comments: 3,
      commentsGhUpdatedAt: T0,
      ghUpdatedAt: T0,
      parentNumber: 10,
    };
    const payload = {
      ...issuePayload,
      issue: {
        ...issuePayload.issue,
        updated_at: T1,
        parent_issue_url: "https://api.github.com/repos/developing-today/code/issues/20",
      },
    };
    const stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, payload, REPO);
    const rows = historyInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(kindOf(rows[0])).toBe("parent_changed");
    expect(detailOf(rows[0])).toEqual({ from: 10, to: 20, parent_repo: "developing-today/code" });
    // The projection itself moves in the upsert, guarded by the
    // parent_known CASE (binds 19/20: known flag, parent number).
    const upsert = stmts.find((s) => s.sql.includes("INSERT INTO github_mirror_issues"));
    expect(upsert?.sql).toContain("parent_number = CASE WHEN ? = 1 THEN ?");
    expect(upsert?.args?.[13]).toEqual({ type: "integer", value: "20" });
    expect(upsert?.args?.[19]).toEqual({ type: "integer", value: "1" });
    expect(upsert?.args?.[20]).toEqual({ type: "integer", value: "20" });
  });

  it("a snapshot that does not state a parent leaves the projection standing", async () => {
    const stored: StoredFixture = {
      bodyHash: await bodyHashOf(issuePayload.issue),
      comments: 3,
      commentsGhUpdatedAt: T0,
      ghUpdatedAt: T0,
      parentNumber: 10,
    };
    const stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, issuePayload, REPO); // no parent_issue_url key
    expect(historyInserts(stmts)).toHaveLength(0);
    const upsert = stmts.find((s) => s.sql.includes("INSERT INTO github_mirror_issues"));
    expect(upsert?.args?.[19]).toEqual({ type: "integer", value: "0" }); // parent_known = 0
  });

  it("oracle: a sub_issues delivery moves the projection and writes parent_changed", async () => {
    const stored: StoredFixture = {
      bodyHash: "hash",
      comments: 0,
      commentsGhUpdatedAt: T0,
      ghUpdatedAt: T0,
      parentNumber: null,
    };
    let stmts = capturePipelineWithStored(stored);
    await mirrorSubIssueEvent(
      env,
      {
        action: "parent_issue_added",
        sub_issue: { number: 8580, updated_at: T1 },
        parent_issue: { number: 20 },
        sender: { login: "dezren39" },
        repository: { full_name: REPO },
      },
      REPO,
    );
    const update = stmts.find((s) => s.sql.startsWith("UPDATE github_mirror_issues SET parent_number"));
    expect(update).toBeDefined();
    expect(update?.args?.[0]).toEqual({ type: "integer", value: "20" });
    let rows = historyInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(kindOf(rows[0])).toBe("parent_changed");
    expect(detailOf(rows[0])).toEqual({ from: null, to: 20 });

    stored.parentNumber = 20;
    stmts = capturePipelineWithStored(stored);
    await mirrorSubIssueEvent(
      env,
      {
        action: "parent_issue_removed",
        sub_issue: { number: 8580, updated_at: T1 },
        parent_issue: { number: 20 },
        repository: { full_name: REPO },
      },
      REPO,
    );
    rows = historyInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(detailOf(rows[0])).toEqual({ from: 20, to: null });
  });

  it("a sub_issues delivery for an unseen child seeds the row, no history", async () => {
    const stmts = capturePipelineWithStored(null);
    await mirrorSubIssueEvent(
      env,
      {
        action: "sub_issue_added",
        sub_issue: { number: 8580, updated_at: T1 },
        parent_issue: { number: 20 },
        repository: { full_name: REPO },
      },
      REPO,
    );
    expect(stmts.some((s) => s.sql.includes("INSERT INTO github_mirror_issues (repo, number, parent_number"))).toBe(
      true,
    );
    expect(historyInserts(stmts)).toHaveLength(0);
  });

  it("a PR body edit writes a body_edited row with is_pr set", async () => {
    const stored: StoredFixture = {
      bodyHash: await bodyHashOf({ body: "the old PR body" }),
      comments: 1,
      commentsGhUpdatedAt: T1,
      ghUpdatedAt: T1,
      parentNumber: null,
    };
    const stmts = capturePipelineWithStored(stored);
    await mirrorPrEvent(env, prPayload, REPO);
    const rows = historyInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(kindOf(rows[0])).toBe("body_edited");
    expect(rows[0].args?.[2]).toEqual({ type: "integer", value: "1" }); // is_pr
  });

  it("oracle: a killed history write still lands the mirror upsert (fail-open)", async () => {
    const stored: StoredFixture = {
      bodyHash: await bodyHashOf({ body: "the original spec" }),
      comments: 3,
      commentsGhUpdatedAt: T0,
      ghUpdatedAt: T0,
      parentNumber: null,
    };
    const stmts = capturePipelineWithStored(stored, { failHistory: true });
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    // The event resolves: the history failure is swallowed, and the
    // mirror upsert + meta stamp were captured before it.
    await expect(mirrorIssueEvent(env, issuePayload, REPO)).resolves.toBeUndefined();
    expect(stmts.some((s) => s.sql.includes("INSERT INTO github_mirror_issues"))).toBe(true);
    expect(stmts.some((s) => s.sql.includes("INSERT INTO github_mirror_meta"))).toBe(true);
    expect(historyInserts(stmts)).toHaveLength(1); // attempted, then dropped
    expect(log).toHaveBeenCalledWith(expect.stringContaining("history write failed open"));
    log.mockRestore();
  });

  it("the history schema ships in the DDL batch", async () => {
    const stmts = capturePipelineWithStored(null);
    await mirrorIssueEvent(env, issuePayload, REPO);
    const sql = stmts.map((s) => s.sql).join("\n");
    expect(sql).toContain("CREATE TABLE IF NOT EXISTS github_issue_history");
    expect(sql).toContain("CREATE INDEX IF NOT EXISTS github_issue_history_issue");
    expect(sql).toContain("parent_number INTEGER");
  });
});

const labelInserts = (stmts: CapturedStmt[]): CapturedStmt[] =>
  stmts.filter((s) => s.sql.includes("INSERT INTO github_label_history") && s.sql.includes("VALUES"));
const labelStarts = (stmts: CapturedStmt[]): CapturedStmt[] =>
  stmts.filter((s) => s.sql.includes("INSERT INTO github_label_history") && s.sql.includes("NOT EXISTS"));
const jsonArg = (s: CapturedStmt, i: number): unknown => JSON.parse(String(s.args?.[i]?.value ?? "null"));

describe("mirror label history (#14170 Q5)", () => {
  const observed = (labels: string[], ghUpdatedAt = T0): StoredFixture => ({
    bodyHash: "hash",
    comments: 3,
    commentsGhUpdatedAt: T0,
    ghUpdatedAt,
    parentNumber: null,
    labels: JSON.stringify(labels),
  });

  it("labelDiff / parseLabelSet: exact, sorted, and honest about unknown sets", () => {
    expect(labelDiff(["a", "b"], ["b", "c", "d"])).toEqual({ added: ["c", "d"], removed: ["a"] });
    expect(labelDiff(["x"], ["x"])).toEqual({ added: [], removed: [] });
    expect(parseLabelSet('["user/hold","priority/high"]')).toEqual(["priority/high", "user/hold"]);
    expect(parseLabelSet(null)).toBeNull();
    expect(parseLabelSet("not json")).toBeNull();
    expect(parseLabelSet("[1,2]")).toBeNull();
  });

  it("oracle: labeling an issue writes one row with the exact added name and the full set", async () => {
    const stored = observed(["task/small-fix"]);
    const payload = {
      ...issuePayload,
      action: "labeled",
      label: { name: "priority/high" },
      issue: { ...issuePayload.issue, updated_at: T1 },
      sender: { login: "dezren39" },
    };
    let stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, payload, REPO);
    const rows = labelInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(labelStarts(stmts)).toHaveLength(0);
    expect(rows[0].args?.[2]).toEqual({ type: "integer", value: "0" }); // is_pr
    expect(jsonArg(rows[0], 3)).toEqual(["priority/high"]); // added
    expect(jsonArg(rows[0], 4)).toEqual([]); // removed
    expect(jsonArg(rows[0], 5)).toEqual(["priority/high", "task/small-fix"]); // label_set
    expect(rows[0].args?.[6]).toEqual({ type: "text", value: "labeled" });
    expect(rows[0].args?.[7]).toEqual({ type: "text", value: "dezren39" });
    expect(rows[0].args?.[8]).toEqual({ type: "text", value: T1 });
    // The mirror upsert itself still lands, before the history batch.
    const upsertAt = stmts.findIndex((s) => s.sql.includes("INSERT INTO github_mirror_issues"));
    const historyAt = stmts.indexOf(rows[0]);
    expect(upsertAt).toBeGreaterThanOrEqual(0);
    expect(historyAt).toBeGreaterThan(upsertAt);

    // Redelivery after the change landed: same set, no second row.
    stmts = capturePipelineWithStored(observed(["priority/high", "task/small-fix"], T1));
    await mirrorIssueEvent(env, payload, REPO);
    expect(labelInserts(stmts)).toHaveLength(0);
  });

  it("oracle: unlabeling writes one row with the exact removed name", async () => {
    const stored = observed(["priority/high", "task/small-fix", "user/hold"]);
    const payload = {
      ...issuePayload,
      action: "unlabeled",
      label: { name: "user/hold" },
      issue: { ...issuePayload.issue, updated_at: T1 },
      sender: { login: "drew" },
    };
    const stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, payload, REPO);
    const rows = labelInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(jsonArg(rows[0], 3)).toEqual([]);
    expect(jsonArg(rows[0], 4)).toEqual(["user/hold"]);
    expect(jsonArg(rows[0], 5)).toEqual(["priority/high", "task/small-fix"]);
    expect(rows[0].args?.[6]).toEqual({ type: "text", value: "unlabeled" });
  });

  it("a stale delivery writes no label row (its upsert is refused too)", async () => {
    const stored = observed(["user/hold"], "2026-11-11T05:00:00Z");
    const stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, issuePayload, REPO); // payload is older (T0)
    expect(labelInserts(stmts)).toHaveLength(0);
    expect(labelStarts(stmts)).toHaveLength(0);
  });

  it("a first sight writes the item's start row, read from the landed mirror row", async () => {
    const stmts = capturePipelineWithStored(null);
    await mirrorIssueEvent(env, { ...issuePayload, action: "opened", sender: { login: "dezren39" } }, REPO);
    expect(labelInserts(stmts)).toHaveLength(0);
    const starts = labelStarts(stmts);
    expect(starts).toHaveLength(1);
    expect(starts[0].sql).toContain("FROM github_mirror_issues m");
    expect(starts[0].sql).toContain("m.gh_updated_at IS NOT NULL");
    const values = (starts[0].args ?? []).map((a) => a.value);
    expect(values).toContain("opened");
    expect(values).toContain("dezren39");
    expect(values).toContain("8580");
  });

  it("a minimal seed (labels never observed) gets a start row, not a fictional diff", async () => {
    const stored: StoredFixture = { ...observed([]), ghUpdatedAt: null };
    const stmts = capturePipelineWithStored(stored);
    await mirrorIssueEvent(env, issuePayload, REPO);
    expect(labelInserts(stmts)).toHaveLength(0);
    expect(labelStarts(stmts)).toHaveLength(1);
  });

  it("a PR label change is recorded with is_pr = 1", async () => {
    const stored = observed([], "2026-11-11T01:30:00Z");
    const stmts = capturePipelineWithStored(stored);
    await mirrorPrEvent(env, { ...prPayload, action: "labeled", label: { name: "queue/claimed" } }, REPO);
    const rows = labelInserts(stmts);
    expect(rows).toHaveLength(1);
    expect(rows[0].args?.[2]).toEqual({ type: "integer", value: "1" });
    expect(jsonArg(rows[0], 3)).toEqual(["queue/claimed"]);
  });

  it("a comment delivery never writes label history", async () => {
    const stmts = capturePipelineWithStored(observed(["a"]));
    await mirrorCommentEvent(
      env,
      {
        action: "created",
        issue: { number: 8580, comments: 4, updated_at: T1, labels: [{ name: "b" }] },
        comment: { id: 99, html_url: "u", body: "thanks!" },
        repository: { full_name: REPO },
      },
      REPO,
    );
    expect(stmts.some((s) => s.sql.includes("INSERT INTO github_label_history"))).toBe(false);
  });

  it("the sweep's repo-wide start statement binds no item number", () => {
    const one = labelStartStmt(REPO, 7, false, "opened", "x");
    const all = labelStartStmt(REPO, null, true, "backfill", "backfill");
    expect(one.sql).toContain("AND m.number = ?");
    expect(all.sql).not.toContain("AND m.number = ?");
    expect(all.sql).toContain("FROM github_mirror_pull_requests m");
    expect((one.args ?? []).length).toBe((all.args ?? []).length + 1);
  });

  it("the label history schema ships in the DDL batch", async () => {
    const stmts = capturePipelineWithStored(null);
    await mirrorIssueEvent(env, issuePayload, REPO);
    const sql = stmts.map((s) => s.sql).join("\n");
    expect(sql).toContain("CREATE TABLE IF NOT EXISTS github_label_history");
    expect(sql).toContain("CREATE INDEX IF NOT EXISTS github_label_history_issue");
  });
});
