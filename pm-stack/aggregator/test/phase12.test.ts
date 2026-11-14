// #9922: tests for the Path to Phase 12 panel's pure core — the
// one-issue body feed parser, the #1235 step parser, and the join.
// The fixture body reproduces #1235's real checklist grammar
// (verified against the live body on 2026-11-14): Steps 1–14 checked
// with DONE annotations, plus the unchecked Continuous line.
import { describe, expect, test } from "bun:test";
import {
  PHASE12_REPO,
  fetchIssueBodySnapshot,
  joinPhase12Steps,
  parseIssueBodyResponse,
  parsePhase12Steps,
  phase12IsStale,
  phase12QueueByRef,
  type Phase12Snapshot,
} from "../src/phase12.ts";
import { phase12PanelHtml } from "../src/render.ts";
import type { GhItem } from "../src/model.ts";
import type { QueueSnapshot, QueueTask } from "../src/queue.ts";

const FIXTURE_BODY = `# Path to Phase 12 — the ordered work queue

## The order, and why
CI trust comes first because every validation verdict below is only as good as the test signal it reads.

- [x] Step 1 — #229 Stabilize shared-state failures in Firefox E2E CI — DONE 2026-10-26 (queue worker, PR #987, issue auto-closed on merge)
- [x] Step 7 — #1078 Validate the delivered Phase 11 items (audit + tests) — DONE 2026-10-27 (queue task 1259 audit complete, issue closed with the closeout table)
- [x] Step 14 — #544 F11.16 replay self-check (\`id world verify\`) — DONE 2026-10-27 (issue closed)
- [ ] Continuous — #534 Phase 12 design workflow (designs land ahead of each F12 item's implementation)

## Working rules for whoever picks these up
- **One issue per validation phase is deliberate**: not a step line.
- [ ] Step notes without the em-dash grammar are not steps
`;

describe("parsePhase12Steps (#1235 grammar)", () => {
  const steps = parsePhase12Steps(FIXTURE_BODY);

  test("parses exactly the step lines, in body order", () => {
    expect(steps.map((s) => s.label)).toEqual(["Step 1", "Step 7", "Step 14", "Continuous"]);
    expect(steps.map((s) => s.issueNumber)).toEqual([229, 1078, 544, 534]);
  });

  test("checkbox state and DONE annotation are split from the title", () => {
    const first = steps[0];
    expect(first.checked).toBe(true);
    expect(first.title).toBe("Stabilize shared-state failures in Firefox E2E CI");
    expect(first.doneNote).toBe("DONE 2026-10-26 (queue worker, PR #987, issue auto-closed on merge)");
  });

  test("the unchecked Continuous line has no DONE note and keeps its parenthetical in the title", () => {
    const last = steps[3];
    expect(last.checked).toBe(false);
    expect(last.doneNote).toBeNull();
    expect(last.title).toBe("Phase 12 design workflow (designs land ahead of each F12 item's implementation)");
  });

  test("a body with no step lines parses to an empty list", () => {
    expect(parsePhase12Steps("## nothing here\n")).toEqual([]);
    expect(parsePhase12Steps("")).toEqual([]);
  });
});

describe("joinPhase12Steps", () => {
  const steps = parsePhase12Steps(FIXTURE_BODY);
  const item = (number: number, state: GhItem["state"]): GhItem => ({
    ref: `${PHASE12_REPO}#${number}`,
    kind: "issue",
    url: `https://github.com/${PHASE12_REPO}/issues/${number}`,
    title: `live title ${number}`,
    state,
    labels: [],
    author: "dezren39",
    updatedAt: "2026-10-27T00:00:00Z",
  });
  const items = new Map([
    [`${PHASE12_REPO}#229`, item(229, "closed")],
    [`${PHASE12_REPO}#534`, item(534, "open")],
  ]);
  const joined = joinPhase12Steps(steps, items);

  test("resolved steps carry the live state and title", () => {
    expect(joined[0].liveState).toBe("closed");
    expect(joined[0].liveTitle).toBe("live title 229");
    expect(joined[3].liveState).toBe("open");
  });

  test("unresolved steps say so with null state, never an invented one", () => {
    expect(joined[1].liveState).toBeNull();
    expect(joined[1].liveTitle).toBeNull();
    expect(joined[1].url).toBe(`https://github.com/${PHASE12_REPO}/issues/1078`);
  });
});

describe("parseIssueBodyResponse + fetchIssueBodySnapshot", () => {
  test("a well-formed GraphQL page yields body + updatedAt", () => {
    const parsed = parseIssueBodyResponse({
      data: { repository: { issue: { body: "hello", updatedAt: "2026-11-01T00:00:00Z" } } },
    });
    expect(parsed).toEqual({ body: "hello", issueUpdatedAt: "2026-11-01T00:00:00Z" });
  });

  test("GraphQL errors and a missing issue both throw", () => {
    expect(() => parseIssueBodyResponse({ errors: [{ message: "nope" }] })).toThrow();
    expect(() => parseIssueBodyResponse({ data: { repository: { issue: null } } })).toThrow();
    expect(() => parseIssueBodyResponse({ data: { repository: null } })).toThrow();
  });

  test("fetch posts the issue-body query with the token and stamps fetchedAt", async () => {
    let seen: { url: string; init: RequestInit } | null = null;
    const fetchFn = (async (url: string, init: RequestInit) => {
      seen = { url, init };
      return new Response(
        JSON.stringify({ data: { repository: { issue: { body: FIXTURE_BODY, updatedAt: "2026-11-01T00:00:00Z" } } } }),
        { status: 200 },
      );
    }) as unknown as typeof fetch;
    const snap = await fetchIssueBodySnapshot("test-token", fetchFn, () => new Date("2026-11-14T12:00:00Z"));
    expect(snap.body).toBe(FIXTURE_BODY);
    expect(snap.issueUpdatedAt).toBe("2026-11-01T00:00:00Z");
    expect(snap.fetchedAt).toBe("2026-11-14T12:00:00.000Z");
    expect(seen!.url).toBe("https://api.github.com/graphql");
    const headers = seen!.init.headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer test-token");
    const variables = (JSON.parse(seen!.init.body as string) as { variables: Record<string, unknown> }).variables;
    expect(variables).toEqual({ owner: "developing-today", name: "code", number: 1235 });
  });

  test("fetch throws on an HTTP error instead of fabricating a snapshot", async () => {
    const fetchFn = (async () => new Response("no", { status: 403 })) as unknown as typeof fetch;
    await expect(fetchIssueBodySnapshot("test-token", fetchFn)).rejects.toThrow("HTTP 403");
  });
});

describe("phase12IsStale", () => {
  const snap = (fetchedAt: string): Phase12Snapshot => ({ body: "", issueUpdatedAt: "", fetchedAt });
  test("fresh within the window, stale past it, unparseable is stale", () => {
    const now = new Date("2026-11-14T12:10:00Z");
    expect(phase12IsStale(snap("2026-11-14T12:00:00Z"), now)).toBe(false);
    expect(phase12IsStale(snap("2026-11-14T11:00:00Z"), now)).toBe(true);
    expect(phase12IsStale(snap("not a date"), now)).toBe(true);
  });
});

describe("phase12 queue join (#9920 collector)", () => {
  const steps = parsePhase12Steps(FIXTURE_BODY);
  const item = (number: number, state: GhItem["state"]): GhItem => ({
    ref: `${PHASE12_REPO}#${number}`,
    kind: "issue",
    url: `https://github.com/${PHASE12_REPO}/issues/${number}`,
    title: `live title ${number}`,
    state,
    labels: [],
    author: "dezren39",
    updatedAt: "2026-10-27T00:00:00Z",
  });
  const items = new Map([
    [`${PHASE12_REPO}#229`, item(229, "closed")],
    [`${PHASE12_REPO}#534`, item(534, "open")],
  ]);
  const task = (partial: Partial<QueueTask> & { id: number }): QueueTask => ({
    queue: "repo-health-code-pilot",
    title: `task ${partial.id}`,
    priority: 20,
    status: "done",
    dedupeKey: null,
    repo: PHASE12_REPO,
    issue: null,
    join: "dedupe",
    worker: "repo-worker-code",
    lane: "code-c",
    attempts: 1,
    maxAttempts: 1000,
    createdAt: "2026-10-26T00:00:00Z",
    leaseUntil: null,
    progressStage: null,
    progressNext: null,
    progressAt: null,
    ...partial,
  });
  const queue: QueueSnapshot = {
    fetchedAt: Date.parse("2026-11-14T16:00:00Z"),
    available: true,
    stale: false,
    errors: [],
    queues: [],
    tasks: [
      task({ id: 1259, issue: 1078, status: "done" }),
      task({ id: 900, issue: 229, status: "done" }),
      task({ id: 901, issue: 229, status: "completed" }),
      task({ id: 902, issue: null, repo: null, join: "none", status: "queued" }),
      task({ id: 903, issue: 9999, status: "queued" }),
    ],
  };

  test("phase12QueueByRef groups by ref, newest first, skipping unjoined tasks", () => {
    const byRef = phase12QueueByRef(queue);
    expect(byRef.get(`${PHASE12_REPO}#229`)?.map((t) => t.id)).toEqual([901, 900]);
    expect(byRef.get(`${PHASE12_REPO}#1078`)?.map((t) => t.id)).toEqual([1259]);
    expect(byRef.get(`${PHASE12_REPO}#9999`)?.map((t) => t.id)).toEqual([903]);
    expect(byRef.size).toBe(3);
    expect(phase12QueueByRef(null).size).toBe(0);
  });

  test("joinPhase12Steps attaches queue tasks to their steps only", () => {
    const joined = joinPhase12Steps(steps, items, queue);
    expect(joined[0].queueTasks.map((t) => t.id)).toEqual([901, 900]);
    expect(joined[1].queueTasks.map((t) => t.id)).toEqual([1259]);
    expect(joined[2].queueTasks).toEqual([]);
    // Without a snapshot the join stays empty (GitHub-only rendering).
    expect(joinPhase12Steps(steps, items)[0].queueTasks).toEqual([]);
  });

  test("the panel renders queue badges, the no-task complement, and the stale warning", () => {
    const snap: Phase12Snapshot = {
      body: FIXTURE_BODY,
      issueUpdatedAt: "2026-11-14T00:00:00Z",
      fetchedAt: new Date().toISOString(),
    };
    const html = phase12PanelHtml(snap, items, queue);
    expect(html).toContain("queue: completed");
    expect(html).toContain("queue: done");
    expect(html).toContain("no queue task");
    expect(html).toContain("queue state from the fleet-queue collector");
    const staleHtml = phase12PanelHtml(snap, items, { ...queue, stale: true });
    expect(staleHtml).toContain("Stale queue source");
    const noQueue = phase12PanelHtml(snap, items, null);
    expect(noQueue).toContain("queue state unavailable in the current source");
    expect(noQueue).not.toContain("no queue task");
    const failed = phase12PanelHtml(snap, items, { ...queue, available: false });
    expect(failed).toContain("queue read failed");
    expect(failed).not.toContain("no queue task");
  });
});
