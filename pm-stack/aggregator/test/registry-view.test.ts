// #1209: sessions registry window / search / pagination — the shared
// query pipeline behind /sessions and /api/registry.json.

import { describe, expect, test } from "bun:test";
import { parseRegistryQuery, queryRegistry, registryQueryString, type RegistryQuery } from "../src/registry-view.ts";
import { authorBadge, renderSessions, sessionRecordLine } from "../src/render.ts";
import { fixtureSource } from "../src/sources/fixtures.ts";
import type { SessionRecord } from "../src/model.ts";

const NOW = new Date("2026-10-31T12:00:00Z");
const isoDaysAgo = (d: number): string => new Date(NOW.getTime() - d * 86_400_000).toISOString();

function rec(id: string, over: Partial<SessionRecord> = {}): SessionRecord {
  return {
    id,
    source: "opencode-laptop",
    label: `Session ${id}`,
    agentId: "opencode:main",
    agentLabel: "OpenCode main",
    parentId: null,
    status: "done",
    startedAt: isoDaysAgo(1),
    lastSeenAt: isoDaysAgo(1),
    taskRefs: [],
    ...over,
  };
}

const RECORDS: SessionRecord[] = [
  rec("ses_recent", { lastSeenAt: isoDaysAgo(1), taskRefs: ["developing-today/code#1209"] }),
  rec("ses_week", { lastSeenAt: isoDaysAgo(6), agentId: "jules", agentLabel: "Jules", source: "jules" }),
  rec("ses_old", { lastSeenAt: isoDaysAgo(40), label: "Ancient history session" }),
  rec("ses_undated", { lastSeenAt: undefined, startedAt: undefined }),
  rec("ses_claude", {
    lastSeenAt: isoDaysAgo(2),
    agentId: "claude-code",
    agentLabel: "Claude Code",
    source: "claude-code",
    title: "Fix the roadmap drill-down",
  }),
];

const q = (raw: string): RegistryQuery => parseRegistryQuery(new URLSearchParams(raw));

describe("parseRegistryQuery", () => {
  test("defaults: 7-day window, limit 100, offset 0", () => {
    const query = q("");
    expect(query.windowDays).toBe(7);
    expect(query.limit).toBe(100);
    expect(query.offset).toBe(0);
    expect(query.q).toBeNull();
  });
  test("window=all and explicit day counts; junk falls back to default", () => {
    expect(q("window=all").windowDays).toBeNull();
    expect(q("window=30").windowDays).toBe(30);
    expect(q("window=banana").windowDays).toBe(7);
  });
  test("limit clamps to 1..500", () => {
    expect(q("limit=9999").limit).toBe(500);
    expect(q("limit=0").limit).toBe(100);
    expect(q("limit=25").limit).toBe(25);
  });
});

describe("queryRegistry", () => {
  test("default window excludes old records and reports the hidden count", () => {
    const view = queryRegistry(RECORDS, q(""), NOW);
    expect(view.totalAll).toBe(5);
    expect(view.hiddenByWindow).toBe(1);
    expect(view.records.map((r) => r.id)).not.toContain("ses_old");
    // Undated records are never hidden by the window.
    expect(view.records.map((r) => r.id)).toContain("ses_undated");
  });
  test("window=all shows everything", () => {
    const view = queryRegistry(RECORDS, q("window=all"), NOW);
    expect(view.hiddenByWindow).toBe(0);
    expect(view.matched).toBe(5);
  });
  test("q searches label/title/id/refs", () => {
    expect(queryRegistry(RECORDS, q("q=ancient"), NOW).records.map((r) => r.id)).toEqual([]);
    expect(queryRegistry(RECORDS, q("window=all&q=ancient"), NOW).records.map((r) => r.id)).toEqual(["ses_old"]);
    expect(queryRegistry(RECORDS, q("q=roadmap"), NOW).records.map((r) => r.id)).toEqual(["ses_claude"]);
    expect(queryRegistry(RECORDS, q("q=1209"), NOW).records.map((r) => r.id)).toEqual(["ses_recent"]);
  });
  test("agent and task-ref filters", () => {
    expect(queryRegistry(RECORDS, q("agent=jules"), NOW).records.map((r) => r.id)).toEqual(["ses_week"]);
    expect(queryRegistry(RECORDS, q("agent=claude"), NOW).records.map((r) => r.id)).toEqual(["ses_claude"]);
    expect(queryRegistry(RECORDS, q("task=code#1209"), NOW).records.map((r) => r.id)).toEqual(["ses_recent"]);
    expect(queryRegistry(RECORDS, q("task=9999"), NOW).matched).toBe(0);
  });
  test("pagination slices and reports the remainder", () => {
    const page1 = queryRegistry(RECORDS, q("window=all&limit=2&offset=0"), NOW);
    const page2 = queryRegistry(RECORDS, q("window=all&limit=2&offset=2"), NOW);
    expect(page1.records.length).toBe(2);
    expect(page1.hiddenByPagination).toBe(3);
    const ids = new Set([...page1.records, ...page2.records].map((r) => r.id));
    expect(ids.size).toBe(4);
    // Newest activity first: ses_recent (1d) before ses_week (6d).
    const all = queryRegistry(RECORDS, q("window=all"), NOW);
    expect(all.records[0].id).toBe("ses_recent");
    // Undated records rank last (no activity timestamp to rank by).
    expect(all.records[all.records.length - 1].id).toBe("ses_undated");
  });
  test("deep-link id outside the window still resolves (pinned)", () => {
    const view = queryRegistry(RECORDS, q("id=ses_old"), NOW);
    expect(view.pinnedIncluded).toBe(true);
    expect(view.records.map((r) => r.id)).toContain("ses_old");
  });
  test("deep-link id already in the page is not duplicated", () => {
    const view = queryRegistry(RECORDS, q("id=ses_recent"), NOW);
    expect(view.pinnedIncluded).toBe(false);
    expect(view.records.filter((r) => r.id === "ses_recent").length).toBe(1);
  });
});

describe("/sessions render with a view", () => {
  test("banner shows counts, hidden remainder, toggle, and form", () => {
    const view = queryRegistry(RECORDS, q(""), NOW);
    const html = renderSessions(view.records, fixtureSource().provenance(), undefined, undefined, view);
    expect(html).toContain("4 of 5 sessions");
    expect(html).toContain("1 hidden by the last 7 days window");
    expect(html).toContain("all history");
    expect(html).toContain('name="q"');
    expect(html).toContain('name="agent"');
    expect(html).toContain('name="task"');
    expect(html).not.toContain("ses_old");
  });
  test("all-history render mentions the window toggle back", () => {
    const view = queryRegistry(RECORDS, q("window=all"), NOW);
    const html = renderSessions(view.records, fixtureSource().provenance(), undefined, undefined, view);
    expect(html).toContain("5 of 5 sessions");
    expect(html).toContain("last 7 days");
  });
  test("registryQueryString round-trips the active filters", () => {
    const query = q("window=30&q=foo&agent=jules&offset=100");
    const s = registryQueryString(query, { offset: 200 });
    const back = parseRegistryQuery(new URLSearchParams(s.slice(1)));
    expect(back.windowDays).toBe(30);
    expect(back.q).toBe("foo");
    expect(back.agent).toBe("jules");
    expect(back.offset).toBe(200);
  });
});

describe("Codex findings on PR 3303 (regressions)", () => {
  test("remainingAfterPage is 0 on the final page even though earlier pages are hidden", () => {
    // Codex example shape: 5 matched, limit 2 — final page offset 4 shows 1.
    const final = queryRegistry(RECORDS, q("window=all&limit=2&offset=4"), NOW);
    expect(final.records.length).toBe(1);
    expect(final.hiddenByPagination).toBe(4);
    expect(final.remainingAfterPage).toBe(0);
    const mid = queryRegistry(RECORDS, q("window=all&limit=2&offset=2"), NOW);
    expect(mid.remainingAfterPage).toBe(1);
    const first = queryRegistry(RECORDS, q("window=all&limit=2&offset=0"), NOW);
    expect(first.remainingAfterPage).toBe(3);
  });
  test("rendered Next link follows remainingAfterPage, not hiddenByPagination", () => {
    const final = queryRegistry(RECORDS, q("window=all&limit=2&offset=4"), NOW);
    const finalHtml = renderSessions(final.records, fixtureSource().provenance(), undefined, undefined, final);
    expect(finalHtml).not.toContain("next →");
    const first = queryRegistry(RECORDS, q("window=all&limit=2&offset=0"), NOW);
    const firstHtml = renderSessions(first.records, fixtureSource().provenance(), undefined, undefined, first);
    expect(firstHtml).toContain("next →");
  });
  test("session deep links carry the ?id= pin as well as the fragment", () => {
    const line = sessionRecordLine(rec("ses_child", { parentId: "ses_old" }));
    expect(line).toContain('href="/sessions?id=ses_old#ses_old"');
    const badge = authorBadge({ kind: "agent", id: "agent-x", label: "Agent X", sessionId: "ses_old" });
    expect(badge).toContain('href="/sessions?id=ses_old#ses_old"');
  });
});
