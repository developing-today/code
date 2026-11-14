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
  type Phase12Snapshot,
} from "../src/phase12.ts";
import type { GhItem } from "../src/model.ts";

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
