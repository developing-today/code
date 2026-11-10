import { describe, expect, test } from "bun:test";
import {
  feedSectionStatus,
  coverageFromFeed,
  feedSectionStatus,
  isBurnSegment,
  isChargeRow,
  isFleetFeed,
  isPaygLine,
  isProjectPoolSection,
  isUnmatchedCharge,
  loadFeed,
  poolFromFeed,
  projectAuditFromFeed,
  projectPoolSectionProblem,
  sanitizeCollectionError,
  sanitizeQuotaView,
  sanitizeFeedMetadata,
  sanitizeRegistry,
  sanitizeSubscriptions,
  sanitizeUninstrumentedServices,
} from "../src/feed.ts";
import { buildAttentionFlags } from "../src/derive.ts";
import { renderQuota } from "../src/render.ts";
import { extractPaygLines } from "../src/saas-extract.ts";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { isProjectPoolItem, parseProjectItems } from "../src/projects.ts";
import type { PaygLine, QuotaCredentialRow, QuotaPool, QuotaView, QuotaWindow, SessionRecord } from "../src/model.ts";

describe("feed stale-issue census", () => {
  test("preserves timestamped counts, accepts older census rows, and rejects malformed optional fields", () => {
    const current = {
      repo: "developing-today/code",
      openIssues: 10,
      gateNow: 1,
      gateNext: 2,
      staleOpenIssues: 3,
      staleIssuesCollectedAt: "2026-11-01T11:59:00.000Z",
      staleIssuesError: null,
    };
    expect(coverageFromFeed({ coverage: [current] })).toEqual([current]);
    expect(coverageFromFeed({ coverage: [{ repo: "old/repo", openIssues: 1, gateNow: 0, gateNext: 0 }] })).toEqual([
      { repo: "old/repo", openIssues: 1, gateNow: 0, gateNext: 0 },
    ]);
    expect(coverageFromFeed({ coverage: [{ ...current, staleIssuesCollectedAt: "not-a-timestamp" }] })).toBeNull();
    expect(coverageFromFeed({})).toBeNull();
  });
});

// Minimal valid pool item: edges entirely absent (the missing-edge
// shape must validate cleanly — absence is data, not an error).
const EDGElESS = {
  id: "PVTI_0",
  databaseId: 100,
  url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=100",
  projectUrl: "https://github.com/orgs/developing-today/projects/4",
  ref: "developing-today/code#2399",
  repo: "developing-today/code",
  number: 2399,
  issueUrl: "https://github.com/developing-today/code/issues/2399",
  title: "contract",
  state: "open",
  contentKind: "issue",
  labels: [],
  author: "dezren39",
  updatedAt: "2026-10-29T00:00:00Z",
  status: null,
  area: null,
  horizon: null,
  kind: "Task",
};

// Fully graphed item: every edge field present and well-shaped.
const GRAPHED = {
  ...EDGElESS,
  id: "PVTI_1",
  ref: "developing-today/code#2402",
  number: 2402,
  parentRef: "developing-today/code#2401",
  blockedBy: ["developing-today/code#910"],
  blocks: ["developing-today/code#2403"],
  epicRef: "developing-today/code#2401",
};

describe("feed project-pool section gate (#2399)", () => {
  test("absent and empty sections are valid", () => {
    expect(projectPoolSectionProblem(undefined)).toBeNull();
    expect(isProjectPoolSection(undefined)).toBe(true);
    expect(projectPoolSectionProblem([])).toBeNull();
  });

  test("edgeless and graphed items both validate (lockstep with the collector)", () => {
    expect(isProjectPoolItem(EDGElESS)).toBe(true);
    expect(isProjectPoolItem(GRAPHED)).toBe(true);
    expect(projectPoolSectionProblem([EDGElESS, GRAPHED])).toBeNull();
    expect(isProjectPoolSection([EDGElESS, GRAPHED])).toBe(true);
  });

  test("malformed edges fail the item, diagnosed by index", () => {
    expect(projectPoolSectionProblem("nope")).toBe("not an array");
    const badEdges = { ...GRAPHED, blockedBy: ["ok#1", 42] };
    expect(isProjectPoolItem(badEdges)).toBe(false);
    expect(projectPoolSectionProblem([EDGElESS, badEdges])).toBe("item 1");
    expect(isProjectPoolSection([EDGElESS, badEdges])).toBe(false);
    expect(projectPoolSectionProblem([{ ...EDGElESS, parentRef: 7 }])).toBe("item 0");
    expect(projectPoolSectionProblem([{ ...EDGElESS, epicRef: null }])).toBe("item 0");
    expect(projectPoolSectionProblem([{ ...EDGElESS, blocks: "developing-today/code#1" }])).toBe("item 0");
  });

  test("every edge ref uses the shared parseRef validity rule and drops only its bad item", () => {
    const unsafe = "developing-today/code#99999999999999999999";
    const malformed = [
      { ...GRAPHED, blockedBy: [unsafe] },
      { ...GRAPHED, blockedBy: ["812"] },
      { ...GRAPHED, blocks: [unsafe] },
      { ...GRAPHED, parentRef: "812" },
      { ...GRAPHED, epicRef: unsafe },
    ];

    expect(isProjectPoolItem(GRAPHED)).toBe(true);
    for (const bad of malformed) expect(isProjectPoolItem(bad)).toBe(false);
    expect(projectPoolSectionProblem([GRAPHED, malformed[0]])).toBe("item 1");
    expect(
      poolFromFeed({ projects: [EDGElESS, ...malformed, GRAPHED] as unknown as never }).map((p) => p.ref),
    ).toEqual([EDGElESS.ref, GRAPHED.ref]);
  });

  test("structural fields + optional-field types are gated (#2418)", () => {
    // labels: a bare string or non-string members crash renders; both fail.
    expect(isProjectPoolItem({ ...EDGElESS, labels: "blocked" })).toBe(false);
    expect(isProjectPoolItem({ ...EDGElESS, labels: [42] })).toBe(false);
    // required url/projectUrl/contentKind.
    const noUrl = { ...EDGElESS } as Record<string, unknown>;
    delete noUrl.url;
    expect(isProjectPoolItem(noUrl)).toBe(false);
    const noProjectUrl = { ...EDGElESS } as Record<string, unknown>;
    delete noProjectUrl.projectUrl;
    expect(isProjectPoolItem(noProjectUrl)).toBe(false);
    const noKind = { ...EDGElESS } as Record<string, unknown>;
    delete noKind.contentKind;
    expect(isProjectPoolItem(noKind)).toBe(false);
    expect(isProjectPoolItem({ ...EDGElESS, contentKind: "draft" })).toBe(false);
    // required nullable fields: string or null only, never a wrong type or absent.
    expect(isProjectPoolItem({ ...EDGElESS, status: 7 })).toBe(false);
    expect(isProjectPoolItem({ ...EDGElESS, area: undefined })).toBe(false);
    expect(isProjectPoolItem({ ...EDGElESS, horizon: false })).toBe(false);
    expect(isProjectPoolItem({ ...EDGElESS, kind: ["Task"] })).toBe(false);
    // present optional fields must match their declared types.
    expect(isProjectPoolItem({ ...EDGElESS, databaseId: "42" })).toBe(false);
    expect(isProjectPoolItem({ ...EDGElESS, draft: "yes" })).toBe(false);
    expect(isProjectPoolItem({ ...EDGElESS, author: 42 })).toBe(false);
    expect(isProjectPoolItem({ ...EDGElESS, updatedAt: null })).toBe(false);
    // absent optional fields remain valid; a mixed pool drops only the bad entry.
    const bare = { ...EDGElESS } as Record<string, unknown>;
    delete bare.labels;
    delete bare.databaseId;
    delete bare.draft;
    delete bare.author;
    delete bare.updatedAt;
    expect(isProjectPoolItem(bare)).toBe(true);
    expect(
      poolFromFeed({ projects: [EDGElESS, { ...EDGElESS, labels: "blocked" }, GRAPHED] as unknown as never }),
    ).toHaveLength(2);
  });

  test("number must be a positive integer, bad values drop individually (#3514)", () => {
    // Real GraphQL numbers are positive integers; NaN/Infinity/1.5/-3/0
    // used to pass the bare typeof gate and render as fabricated refs.
    expect(isProjectPoolItem(EDGElESS)).toBe(true);
    for (const bad of [1.5, -3, 0, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(isProjectPoolItem({ ...EDGElESS, number: bad })).toBe(false);
    }
    expect(
      poolFromFeed({
        projects: [
          EDGElESS,
          { ...EDGElESS, number: 1.5 },
          { ...EDGElESS, number: -3 },
          { ...EDGElESS, number: 0 },
          { ...EDGElESS, number: Number.NaN },
          { ...EDGElESS, number: Number.POSITIVE_INFINITY },
          GRAPHED,
        ] as unknown as never,
      }),
    ).toHaveLength(2);
  });

  test("poolFromFeed is fail-soft: garbage sections yield no items, bad items drop", () => {
    expect(poolFromFeed(null)).toEqual([]);
    expect(poolFromFeed(undefined)).toEqual([]);
    expect(poolFromFeed({})).toEqual([]);
    expect(poolFromFeed({ projects: "nope" as unknown as never })).toEqual([]);
    expect(poolFromFeed({ projects: { items: [] } as unknown as never })).toEqual([]);
    const mixed = poolFromFeed({ projects: [EDGElESS, { nope: true }, GRAPHED] as unknown as never });
    expect(mixed.map((p) => p.ref)).toEqual(["developing-today/code#2399", "developing-today/code#2402"]);
  });

  test("collector-parsed pools (graphed and edgeless) pass the feed gate", () => {
    const pool = parseProjectItems({
      data: {
        node: {
          items: {
            nodes: [
              {
                id: "PVTI_1",
                type: "ISSUE",
                content: {
                  number: 2402,
                  title: "child",
                  state: "OPEN",
                  url: "https://github.com/developing-today/code/issues/2402",
                  repository: { nameWithOwner: "developing-today/code" },
                  updatedAt: "2026-10-29T00:00:00Z",
                  parent: {
                    number: 2401,
                    repository: { nameWithOwner: "developing-today/code" },
                  },
                  blockedBy: { nodes: [] },
                  blocking: { nodes: [] },
                },
                fieldValues: { nodes: [] },
              },
              {
                id: "PVTI_2",
                type: "ISSUE",
                content: {
                  number: 2407,
                  title: "plain",
                  state: "OPEN",
                  url: "https://github.com/developing-today/code/issues/2407",
                  repository: { nameWithOwner: "developing-today/code" },
                  updatedAt: "2026-10-29T00:00:00Z",
                  parent: null,
                  blockedBy: { nodes: [] },
                  blocking: { nodes: [] },
                },
                fieldValues: { nodes: [] },
              },
            ],
          },
        },
      },
    });
    expect(pool.length).toBe(2);
    expect(isProjectPoolSection(pool)).toBe(true);
    expect(poolFromFeed({ projects: pool }).length).toBe(2);
  });
});

describe("Project #4 audit feed section (#1218)", () => {
  const audit = {
    title: "Developing Today",
    number: 4,
    url: "https://github.com/orgs/developing-today/projects/4",
    generatedAt: "2026-11-01T20:00:00.000Z",
    totalItems: 1,
    fields: [{ id: "status", name: "Status", dataType: "SINGLE_SELECT", options: [{ id: "todo", name: "Todo" }] }],
    views: [{ id: "board", name: "Board", layout: "BOARD_LAYOUT", filter: null }],
    counts: { status: { Todo: 1 }, area: { "(unset)": 1 }, repo: { "developing-today/code": 1 } },
  };

  test("returns only a well-shaped audit snapshot", () => {
    expect(projectAuditFromFeed({ projectAudit: audit })).toEqual(audit);
    expect(projectAuditFromFeed({ projectAudit: { ...audit, totalItems: "1" } as never })).toBeNull();
    expect(projectAuditFromFeed({})).toBeNull();
  });
});

describe("isFleetFeed generatedAt stamps (#2578 Phase 2, #2242)", () => {
  const feed = (generatedAt: string) => ({
    version: 1,
    generatedAt,
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
  });

  test("a past real stamp validates; an impossible date does not", () => {
    expect(isFleetFeed(feed("2026-10-01T00:00:00Z"))).toBe(true);
    expect(isFleetFeed(feed("2026-02-30T00:00:00Z"))).toBe(false);
  });

  test("a stamp far in the future is invalid, not maximally fresh", () => {
    const future = new Date(Date.now() + 3_600_000).toISOString();
    expect(isFleetFeed(feed(future))).toBe(false);
  });
});

describe("feed quota section gate + sanitization (#2429)", () => {
  const WINDOW: QuotaWindow = {
    name: "5h",
    unit: "tokens",
    limit: 1000,
    used: 250,
    resetsAt: "2026-10-29T05:00:00Z",
    percent: 25,
  };
  const POOL: QuotaPool = { id: "opencode-go", provider: "opencode", plan: "go", state: "healthy", windows: [WINDOW] };
  const CREDENTIAL: QuotaCredentialRow = {
    label: "OpenAI 2",
    providerId: "openai",
    windows: [WINDOW],
    freshness: { measuredAt: "2026-10-01T00:00:00Z", source: "quota-current", state: "ok" },
  };
  const FRESHNESS = { measuredAt: "2026-10-01T00:00:00Z", source: "quota-current", state: "ok" } as const;
  const ACTIVE_SESSION = { session: "session-1", model: "gpt-5", tokens: 42 };
  const CHOICES = { hasHeadroom: ["opencode-go"], burnBeforeReset: [], doNotStart: [] };
  const feedWith = (quota: unknown) => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota,
    attention: [],
  });
  const quotaWith = (pools: unknown, extra: Record<string, unknown> = {}) => ({
    pools,
    burn: [],
    choices: CHOICES,
    ...extra,
  });

  test("a feed missing burn or choices is rejected whole (old-collector shape)", () => {
    expect(isFleetFeed(feedWith({ pools: [] }))).toBe(false);
    expect(isFleetFeed(feedWith({ pools: [], burn: [] }))).toBe(false);
    expect(isFleetFeed(feedWith(quotaWith([])))).toBe(true);
  });

  test("pools: [null] drops the pool, keeps the feed", () => {
    const out = sanitizeQuotaView(quotaWith([null, POOL]) as never);
    expect(out.droppedPools).toBe(1);
    expect(out.view.pools).toEqual([POOL]);
  });

  test("a pool missing windows drops; a non-array windows is never iterated", () => {
    const noWindows = { id: "a", provider: "p", plan: "x", state: "near" };
    const stringWindows = { ...POOL, id: "b", windows: "soon" };
    const out = sanitizeQuotaView(quotaWith([noWindows, stringWindows, POOL]) as never);
    expect(out.droppedPools).toBe(2);
    expect(out.view.pools).toEqual([POOL]);
  });

  test("windows: [null] drops the bad window, keeps pool and good windows", () => {
    const pool = { ...POOL, windows: [null, WINDOW, { bogus: true }] };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);
    expect(out.droppedPools).toBe(0);
    expect(out.droppedWindows).toBe(2);
    expect(out.view.pools[0]?.windows).toEqual([WINDOW]);
  });

  // #3427: value-malformed windows (impossible resetsAt, negative
  // limit/used, out-of-range percent) used to pass the type-only guard
  // and mislead guidance/health. They now drop individually and count,
  // exactly like shape failures; valid windows are untouched.
  test("Feb-30 resetsAt window drops individually", () => {
    const bad = { ...WINDOW, resetsAt: "2026-02-30T00:00:00Z" };
    const pool = { ...POOL, windows: [WINDOW, bad] };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);
    expect(out.droppedWindows).toBe(1);
    expect(out.view.pools[0]?.windows).toEqual([WINDOW]);
  });

  test("negative used window drops individually", () => {
    const bad = { ...WINDOW, used: -5 };
    const pool = { ...POOL, windows: [WINDOW, bad] };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);
    expect(out.droppedWindows).toBe(1);
    expect(out.view.pools[0]?.windows).toEqual([WINDOW]);
  });

  test("negative limit window drops individually", () => {
    const bad = { ...WINDOW, limit: -100 };
    const pool = { ...POOL, windows: [WINDOW, bad] };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);
    expect(out.droppedWindows).toBe(1);
    expect(out.view.pools[0]?.windows).toEqual([WINDOW]);
  });

  test("percent -50 window drops individually", () => {
    const bad = { ...WINDOW, percent: -50 };
    const pool = { ...POOL, windows: [WINDOW, bad] };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);
    expect(out.droppedWindows).toBe(1);
    expect(out.view.pools[0]?.windows).toEqual([WINDOW]);
  });

  test("percent 15000 window drops individually", () => {
    const bad = { ...WINDOW, percent: 15000 };
    const pool = { ...POOL, windows: [WINDOW, bad] };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);
    expect(out.droppedWindows).toBe(1);
    expect(out.view.pools[0]?.windows).toEqual([WINDOW]);
  });

  test("valid windows are untouched and droppedWindows counts every drop", () => {
    const feb30 = { ...WINDOW, name: "feb30", resetsAt: "2026-02-30T00:00:00Z" };
    const negUsed = { ...WINDOW, name: "neg-used", used: -1 };
    const negLimit = { ...WINDOW, name: "neg-limit", limit: -1 };
    const negPct = { ...WINDOW, name: "neg-pct", percent: -50 };
    const hugePct = { ...WINDOW, name: "huge-pct", percent: 15000 };
    const pool = { ...POOL, windows: [WINDOW, feb30, negUsed, negLimit, negPct, hugePct] };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);
    expect(out.droppedWindows).toBe(5);
    expect(out.view.pools[0]?.windows).toEqual([WINDOW]);
    expect(out.view.dropped).toEqual({
      pools: 0,
      credentials: 0,
      windows: 5,
      activeSessions: 0,
      burn: 0,
      poolFreshness: 0,
      burnTokensPerMin: 0,
    });
  });

  test("malformed credential rows and active sessions drop individually and render safely", () => {
    const malformedCredentialRows: unknown[] = [
      { label: "missing freshness", providerId: "openai", windows: [WINDOW] },
      { ...CREDENTIAL, freshness: null },
      { ...CREDENTIAL, label: 42 },
      CREDENTIAL,
    ];
    const malformedSessions = [
      null,
      { session: "missing-tokens", model: "gpt-5" },
      ACTIVE_SESSION,
      { session: "non-finite", model: "gpt-5", tokens: Number.POSITIVE_INFINITY },
      { session: "negative", model: "gpt-5", tokens: -1 },
    ];
    const pool = { ...POOL, credentials: malformedCredentialRows, activeSessions: malformedSessions };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);

    expect(out.droppedPools).toBe(0);
    expect(out.droppedCredentials).toBe(3);
    expect(out.droppedActiveSessions).toBe(4);
    expect(out.view.pools[0]).toEqual({ ...POOL, credentials: [CREDENTIAL], activeSessions: [ACTIVE_SESSION] });
    expect(out.view.dropped).toEqual({
      pools: 0,
      credentials: 3,
      windows: 0,
      activeSessions: 4,
      burn: 0,
      poolFreshness: 0,
      burnTokensPerMin: 0,
    });
    const html = renderQuota(out.view, {} as never);
    expect(html).toContain("credential OpenAI 2");
    expect(html).toContain("burning: session-1 (gpt-5) 42 tok");
  });

  test("a present non-array activeSessions section becomes empty and counts once", () => {
    const out = sanitizeQuotaView(quotaWith([{ ...POOL, activeSessions: "not-an-array" }]) as never);

    expect(out.droppedPools).toBe(0);
    expect(out.droppedActiveSessions).toBe(1);
    expect(out.view.pools[0]).toEqual({ ...POOL, activeSessions: [] });
    expect(out.view.dropped).toEqual({
      pools: 0,
      credentials: 0,
      windows: 0,
      activeSessions: 1,
      burn: 0,
      poolFreshness: 0,
      burnTokensPerMin: 0,
    });
    expect(() => renderQuota(out.view, {} as never)).not.toThrow();
  });

  test("well-formed credential and active-session rows remain unchanged", () => {
    const quota = quotaWith([{ ...POOL, credentials: [CREDENTIAL], activeSessions: [ACTIVE_SESSION] }]) as QuotaView;
    const out = sanitizeQuotaView(quota);

    expect(out.view).toEqual(quota);
    expect(out.droppedCredentials).toBe(0);
    expect(out.droppedActiveSessions).toBe(0);
    expect(renderQuota(out.view, {} as never)).toContain("credential OpenAI 2");
  });

  test("malformed pool freshness drops and counts only freshness, keeping windows and siblings", () => {
    const malformed = { ...POOL, freshness: { ...FRESHNESS, measuredAt: "2026-02-30T00:00:00Z" } };
    const sibling = { ...POOL, id: "sibling", freshness: FRESHNESS };
    const out = sanitizeQuotaView(quotaWith([malformed, sibling]) as never);

    expect(out.droppedPools).toBe(0);
    expect(out.droppedPoolFreshness).toBe(1);
    expect(out.view.pools).toEqual([POOL, sibling]);
    expect(out.view.dropped).toEqual({
      pools: 0,
      credentials: 0,
      windows: 0,
      activeSessions: 0,
      burn: 0,
      poolFreshness: 1,
      burnTokensPerMin: 0,
    });
    expect(() => renderQuota(out.view, {} as never)).not.toThrow();
  });

  test("credential freshness measuredAt must be a real ISO date", () => {
    const invalidDates = ["banana", "2026-02-30T00:00:00Z"];
    const credentials = [
      ...invalidDates.map((measuredAt) => ({ ...CREDENTIAL, freshness: { ...FRESHNESS, measuredAt } })),
      CREDENTIAL,
    ];
    const out = sanitizeQuotaView(quotaWith([{ ...POOL, credentials }]) as never);

    expect(out.droppedCredentials).toBe(2);
    expect(out.view.pools[0]?.credentials).toEqual([CREDENTIAL]);
  });

  test("negative burnTokensPerMin drops and counts only the invalid rate", () => {
    const pool = { ...POOL, burnTokensPerMin: -0.1 };
    const out = sanitizeQuotaView(quotaWith([pool]) as never);

    expect(out.droppedPools).toBe(0);
    expect(out.droppedBurnTokensPerMin).toBe(1);
    expect(out.view.pools).toEqual([POOL]);
    expect(out.view.dropped).toEqual({
      pools: 0,
      credentials: 0,
      windows: 0,
      activeSessions: 0,
      burn: 0,
      poolFreshness: 0,
      burnTokensPerMin: 1,
    });
  });

  test("malformed burn segments drop individually", () => {
    const seg = {
      provider: "opencode",
      pool: "go",
      from: "2026-10-01T00:00:00Z",
      to: "2026-10-01T01:00:00Z",
      tokens: 42,
    };
    const out = sanitizeQuotaView(quotaWith([POOL], { burn: [seg, null, { provider: "x" }] }) as never);
    expect(out.droppedBurn).toBe(2);
    expect(out.view.burn).toEqual([seg]);
  });

  // #3428: value-malformed segments (negative tokens, non-date or
  // impossible from/to, inverted range) used to pass the type-only
  // guard and render as measured history. They now drop individually
  // and count, exactly like shape failures.
  test("value-malformed burn segments drop individually", () => {
    const seg = {
      provider: "opencode",
      pool: "go",
      from: "2026-10-01T00:00:00Z",
      to: "2026-10-01T01:00:00Z",
      tokens: 42,
    };
    const negative = { ...seg, tokens: -12345 };
    const garbageFrom = { ...seg, from: "banana" };
    const impossibleTo = { ...seg, to: "2026-02-30T01:00:00Z" };
    const inverted = { ...seg, from: "2026-10-02T00:00:00Z", to: "2026-10-01T00:00:00Z" };
    expect(isBurnSegment(seg)).toBe(true);
    expect(isBurnSegment(negative)).toBe(false);
    expect(isBurnSegment(garbageFrom)).toBe(false);
    expect(isBurnSegment(impossibleTo)).toBe(false);
    expect(isBurnSegment(inverted)).toBe(false);
    const out = sanitizeQuotaView(
      quotaWith([POOL], { burn: [seg, negative, garbageFrom, impossibleTo, inverted] }) as never,
    );
    expect(out.droppedBurn).toBe(4);
    expect(out.view.burn).toEqual([seg]);
  });

  test("a fully well-formed quota view is unchanged", () => {
    const q = quotaWith(
      [
        {
          ...POOL,
          freshness: FRESHNESS,
          burnTokensPerMin: 12,
          credentials: [CREDENTIAL],
          activeSessions: [ACTIVE_SESSION],
        },
      ],
      { burn: [] },
    ) as unknown as QuotaView;
    const out = sanitizeQuotaView(q);
    expect(
      out.droppedPools +
        out.droppedCredentials +
        out.droppedWindows +
        out.droppedActiveSessions +
        out.droppedBurn +
        out.droppedPoolFreshness +
        out.droppedBurnTokensPerMin,
    ).toBe(0);
    expect(out.view).toEqual(q);
    expect(renderQuota(out.view, {} as never)).toContain("credential OpenAI 2");
  });

  test("choices are reconciled to surviving pools with usable windows", () => {
    const noWindows = { ...POOL, windows: [null] };
    const out = sanitizeQuotaView(quotaWith([noWindows]) as never);
    expect(out.view.pools.length).toBe(1);
    expect(out.view.choices).toEqual({ hasHeadroom: [], burnBeforeReset: [], doNotStart: [] });
    expect(out.view.dropped).toEqual({
      pools: 0,
      credentials: 0,
      windows: 1,
      activeSessions: 0,
      burn: 0,
      poolFreshness: 0,
      burnTokensPerMin: 0,
    });
  });

  test("a dropped pool leaves the retained choices and surfaces dropped on the view", () => {
    const out = sanitizeQuotaView(quotaWith([null, POOL]) as never);
    expect(out.view.choices.hasHeadroom).toEqual(["opencode-go"]);
    expect(out.view.dropped).toEqual({
      pools: 1,
      credentials: 0,
      windows: 0,
      activeSessions: 0,
      burn: 0,
      poolFreshness: 0,
      burnTokensPerMin: 0,
    });
  });

  test("loadFeed returns the sanitized remainder with drop counts attached", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-quota-"));
    const file = join(dir, "feed.json");
    const pool = {
      ...POOL,
      windows: [WINDOW, null],
      credentials: [{ label: "missing freshness", providerId: "openai", windows: [WINDOW] }, CREDENTIAL],
      activeSessions: [null, ACTIVE_SESSION, { session: "negative", model: "gpt-5", tokens: -1 }],
      freshness: { ...FRESHNESS, measuredAt: "banana" },
      burnTokensPerMin: -1,
    };
    writeFileSync(file, JSON.stringify(feedWith(quotaWith([null, pool]))));
    const loaded = await loadFeed(file);
    expect(loaded).not.toBeNull();
    expect(loaded?.quota.pools.length).toBe(1);
    expect(loaded?.quota.pools[0]?.windows).toEqual([WINDOW]);
    expect(loaded?.quota.pools[0]?.credentials).toEqual([CREDENTIAL]);
    expect(loaded?.quota.pools[0]?.activeSessions).toEqual([ACTIVE_SESSION]);
    expect(loaded?.quota.pools[0]?.freshness).toBeUndefined();
    expect(loaded?.quota.pools[0]?.burnTokensPerMin).toBeUndefined();
    expect(loaded?.quotaDropped).toEqual({
      pools: 1,
      credentials: 1,
      windows: 1,
      activeSessions: 2,
      burn: 0,
      poolFreshness: 1,
      burnTokensPerMin: 1,
    });
    expect(loaded?.quota.dropped).toEqual({
      pools: 1,
      credentials: 1,
      windows: 1,
      activeSessions: 2,
      burn: 0,
      poolFreshness: 1,
      burnTokensPerMin: 1,
    });
  });

  test("loadFeed rejects a feed missing burn/choices (fixture-fallback path)", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-quota-"));
    const file = join(dir, "feed.json");
    writeFileSync(file, JSON.stringify(feedWith({ pools: [POOL] })));
    expect(await loadFeed(file)).toBeNull();
  });
});

describe("feed subscriptions sanitization (#2446)", () => {
  const SUB = { name: "Wispr Flow", plan: "Pro $151.91/yr", renewsAt: "2027-10-09" };
  const SUB_NULL_RENEW = { name: "Amazon RxPass", plan: "$5/mo flat pharmacy", renewsAt: null };
  const feedWithSubs = (subscriptions: unknown) => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    subscriptions,
  });
  const loadWithSubs = async (subscriptions: unknown) => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-subs-"));
    const file = join(dir, "feed.json");
    writeFileSync(file, JSON.stringify(feedWithSubs(subscriptions)));
    return loadFeed(file);
  };

  test("subscriptions: [null] drops the row, keeps the feed, counts the drop", async () => {
    const loaded = await loadWithSubs([null]);
    expect(loaded).not.toBeNull();
    expect(loaded?.subscriptions).toEqual([]);
    expect(loaded?.subscriptionsDropped).toBe(1);
  });

  test("a non-array subscriptions section is dropped whole, treated as absent", async () => {
    expect(sanitizeSubscriptions("nope")).toEqual({ dropped: 1 });
    const loaded = await loadWithSubs("nope");
    expect(loaded?.subscriptions).toBeUndefined();
    expect(loaded?.subscriptionsDropped).toBe(1);
  });

  test("a row with a numeric name drops; valid rows survive verbatim", async () => {
    const out = sanitizeSubscriptions([SUB, { name: 42, plan: "x", renewsAt: null }, SUB_NULL_RENEW]);
    expect(out.rows).toEqual([SUB, SUB_NULL_RENEW]);
    expect(out.dropped).toBe(1);
  });

  test("invalid renewsAt strings drop; ISO dates and null survive (#3513)", async () => {
    const bad = (renewsAt: string) => ({ name: "Bad", plan: "p", renewsAt });
    const out = sanitizeSubscriptions([
      SUB,
      SUB_NULL_RENEW,
      bad("banana"),
      bad("2026-02-30"),
      bad("2026-13-01"),
      bad("12/01/2026"),
      bad("2026-12-01banana"),
    ]);
    expect(out.rows).toEqual([SUB, SUB_NULL_RENEW]);
    expect(out.dropped).toBe(5);
  });

  test("a mixed valid/invalid array through loadFeed keeps only renderer-safe rows", async () => {
    const loaded = await loadWithSubs([SUB, null, "junk", { name: "Ok", plan: 7, renewsAt: null }, SUB_NULL_RENEW]);
    expect(loaded?.subscriptions).toEqual([SUB, SUB_NULL_RENEW]);
    expect(loaded?.subscriptionsDropped).toBe(3);
  });

  test("renderQuota over the sanitized usage view does not throw", async () => {
    const loaded = await loadWithSubs([null, SUB]);
    expect(loaded).not.toBeNull();
    const usage = {
      services: [],
      payg: [],
      charges: [],
      attention: [],
      domains: null,
      subscriptions: loaded?.subscriptions ?? [],
      history: {},
    } as never;
    expect(() => renderQuota(loaded!.quota, {} as never, usage)).not.toThrow();
  });

  test("absent and well-formed subscriptions are unchanged, no drop count attached", async () => {
    expect(sanitizeSubscriptions(undefined)).toEqual({ dropped: 0 });
    const absent = await loadWithSubs(undefined);
    expect(absent?.subscriptions).toBeUndefined();
    expect(absent?.subscriptionsDropped).toBeUndefined();
    const clean = await loadWithSubs([SUB, SUB_NULL_RENEW]);
    expect(clean?.subscriptions).toEqual([SUB, SUB_NULL_RENEW]);
    expect(clean?.subscriptionsDropped).toBeUndefined();
  });
});

describe("uninstrumented tracker feed sanitization (#1172)", () => {
  const SERVICE = {
    name: "Render",
    whatFor: "PaaS",
    status: "checked",
    checkedAt: "2026-10-29",
    overageBehavior: "wall",
    guardrailNotes: "Keep no payment method on free workspaces",
  };
  const UNDATED_SERVICE = { ...SERVICE, status: "unchecked", checkedAt: null };
  const feedWithServices = (uninstrumentedServices: unknown) => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    uninstrumentedServices,
  });
  const loadWithServices = async (uninstrumentedServices: unknown) => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-tracker-"));
    const file = join(dir, "feed.json");
    writeFileSync(file, JSON.stringify(feedWithServices(uninstrumentedServices)));
    return loadFeed(file);
  };

  test("valid tracker rows survive; malformed rows drop and are counted", async () => {
    const out = sanitizeUninstrumentedServices([SERVICE, UNDATED_SERVICE, null, { ...SERVICE, checkedAt: "yesterday" }]);
    expect(out.rows).toEqual([SERVICE, UNDATED_SERVICE]);
    expect(out.dropped).toBe(2);
    const loaded = await loadWithServices([SERVICE, UNDATED_SERVICE, null]);
    expect(loaded?.uninstrumentedServices).toEqual([SERVICE, UNDATED_SERVICE]);
    expect(loaded?.uninstrumentedServicesDropped).toBe(1);
  });

  test("an absent tracker section remains absent for older feeds", async () => {
    expect(sanitizeUninstrumentedServices(undefined)).toEqual({ dropped: 0 });
    const loaded = await loadWithServices(undefined);
    expect(loaded?.uninstrumentedServices).toBeUndefined();
    expect(loaded?.uninstrumentedServicesDropped).toBeUndefined();
  });
});

describe("feed registry sanitization (#2539)", () => {
  const RECORD: SessionRecord = {
    id: "ses-1",
    source: "claude-code",
    label: "Muse run",
    agentId: "claude-code",
    agentLabel: "Muse",
    parentId: null,
    status: "in-progress",
    taskRefs: [],
  };
  const feedWithRegistry = (registry: unknown) => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    registry,
  });
  const loadWithRegistry = async (registry: unknown) => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-registry-"));
    const file = join(dir, "feed.json");
    writeFileSync(file, JSON.stringify(feedWithRegistry(registry)));
    return loadFeed(file);
  };

  test("preserves an absent section and a well-formed registry", async () => {
    expect(sanitizeRegistry(undefined)).toEqual({ dropped: 0 });
    const absent = await loadWithRegistry(undefined);
    expect(absent?.registry).toBeUndefined();
    expect(absent?.registryDropped).toBeUndefined();

    expect(sanitizeRegistry([RECORD])).toEqual({ rows: [RECORD], dropped: 0 });
    const clean = await loadWithRegistry([RECORD]);
    expect(clean?.registry).toEqual([RECORD]);
    expect(clean?.registryDropped).toBeUndefined();
    expect(clean?.quota.pools).toEqual([]);
  });

  test("drops malformed containers and rows without rejecting the feed", async () => {
    for (const malformed of ["bad", { registry: "bad" }, 42, [null, RECORD]]) {
      const loaded = await loadWithRegistry(malformed);
      expect(loaded).not.toBeNull();
      expect(loaded?.registry).toEqual(Array.isArray(malformed) ? [RECORD] : []);
      expect(loaded?.registryDropped).toBe(1);
      expect(loaded?.quota.pools).toEqual([]);
    }
    expect(sanitizeRegistry("bad")).toEqual({ rows: [], dropped: 1 });
    expect(sanitizeRegistry({ registry: "bad" })).toEqual({ rows: [], dropped: 1 });
    expect(sanitizeRegistry(42)).toEqual({ rows: [], dropped: 1 });
    expect(sanitizeRegistry([null])).toEqual({ rows: [], dropped: 1 });
  });
});

describe("feed PAYG sanitization (#2450)", () => {
  const PAYG: PaygLine = {
    id: "meta-payg",
    label: "Meta Model API PAYG",
    accruedUsd: 23.92,
    collectedUsd: 23.91,
    estimate: { low: 20, high: 25, period: "Oct 19–25", note: "console estimate" },
    unbilledUsd: 6.01,
    note: "Oct 19–25 console read",
    measuredAt: "2026-10-25",
    sourceClass: "console",
  };
  const feedWithPayg = (payg: unknown) => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    ...(payg === undefined ? {} : { payg }),
  });
  const loadWithPayg = async (payg: unknown) => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-payg-"));
    const file = join(dir, "feed.json");
    writeFileSync(file, JSON.stringify(feedWithPayg(payg)));
    return loadFeed(file);
  };

  test("PaygLine validates the fields its consumers dereference", () => {
    expect(isPaygLine(PAYG)).toBe(true);
    expect(
      isPaygLine({
        ...PAYG,
        accruedUsd: null,
        collectedUsd: null,
        estimate: null,
        unbilledUsd: null,
        measuredAt: null,
        sourceClass: "invoice",
      }),
    ).toBe(true);
    for (const bad of [
      { ...PAYG, id: 42 },
      { ...PAYG, label: null },
      { ...PAYG, accruedUsd: "23.92" },
      { ...PAYG, accruedUsd: Number.POSITIVE_INFINITY },
      { ...PAYG, accruedUsd: -0.01 },
      { ...PAYG, collectedUsd: "23.91" },
      { ...PAYG, collectedUsd: -0.01 },
      { ...PAYG, unbilledUsd: "6.01" },
      { ...PAYG, unbilledUsd: -0.01 },
      { ...PAYG, note: 42 },
      { ...PAYG, measuredAt: 42 },
      { ...PAYG, measuredAt: "banana" },
      { ...PAYG, sourceClass: "unknown" },
    ]) {
      expect(isPaygLine(bad)).toBe(false);
    }
  });

  test("[null] drops the row, keeps the feed, and surfaces the count", async () => {
    const loaded = await loadWithPayg([null]);
    expect(loaded).not.toBeNull();
    expect(loaded?.payg).toEqual([]);
    expect(loaded?.paygDropped).toBe(1);
  });

  test("a non-array section is dropped whole and counted without rejecting the feed", async () => {
    for (const bad of ["nope", { id: "not-an-array" }]) {
      const feed = feedWithPayg(bad);
      expect(isFleetFeed(feed)).toBe(true);
      const loaded = await loadWithPayg(bad);
      expect(loaded).not.toBeNull();
      expect(loaded?.payg).toBeUndefined();
      expect(loaded?.paygDropped).toBe(1);
    }
  });

  test("malformed money rows drop individually while valid siblings survive", async () => {
    const loaded = await loadWithPayg([{ ...PAYG, accruedUsd: "23.92" }, PAYG, null]);
    expect(loaded?.payg).toEqual([PAYG]);
    expect(loaded?.paygDropped).toBe(2);
  });

  test("negative and inverted measurements drop individually; valid and null controls survive", async () => {
    const allNull = {
      ...PAYG,
      accruedUsd: null,
      collectedUsd: null,
      estimate: null,
      unbilledUsd: null,
      measuredAt: null,
    };
    const invalid = [
      { ...PAYG, accruedUsd: -1 },
      { ...PAYG, collectedUsd: -1 },
      { ...PAYG, unbilledUsd: -1 },
      { ...PAYG, estimate: { low: -1, high: 5, period: "month" } },
      { ...PAYG, estimate: { low: 1, high: -1, period: "month" } },
      { ...PAYG, estimate: { low: 6, high: 5, period: "month" } },
      { ...PAYG, measuredAt: "banana" },
    ];
    const loaded = await loadWithPayg([PAYG, ...invalid, allNull]);
    expect(loaded?.payg).toEqual([PAYG, allNull]);
    expect(loaded?.paygDropped).toBe(invalid.length);
  });

  test("estimate variants are checked structurally", async () => {
    const noEstimate = { ...PAYG };
    delete noEstimate.estimate;
    const nullEstimate = { ...PAYG, estimate: null };
    const estimateWithoutNote = { ...PAYG, estimate: { low: 20, high: 25, period: "month" } };
    const malformed = [
      { ...PAYG, estimate: "25" },
      { ...PAYG, estimate: { high: 25, period: "month" } },
      { ...PAYG, estimate: { low: "20", high: 25, period: "month" } },
      { ...PAYG, estimate: { low: Number.POSITIVE_INFINITY, high: 25, period: "month" } },
      { ...PAYG, estimate: { low: -1, high: 25, period: "month" } },
      { ...PAYG, estimate: { low: 20, high: -1, period: "month" } },
      { ...PAYG, estimate: { low: 26, high: 25, period: "month" } },
      { ...PAYG, estimate: { low: 20, high: 25, period: 3 } },
      { ...PAYG, estimate: { low: 20, high: 25, period: "month", note: 3 } },
    ];
    const loaded = await loadWithPayg([noEstimate, nullEstimate, PAYG, estimateWithoutNote, ...malformed]);
    expect(loaded?.payg).toEqual([noEstimate, nullEstimate, PAYG, estimateWithoutNote]);
    expect(loaded?.paygDropped).toBe(malformed.length);
  });

  test("absent and clean PAYG stay unchanged; collector-produced lines are accepted", async () => {
    const absent = await loadWithPayg(undefined);
    expect(absent?.payg).toBeUndefined();
    expect(absent?.paygDropped).toBeUndefined();
    const clean = await loadWithPayg([PAYG]);
    expect(clean?.payg).toEqual([PAYG]);
    expect(clean?.paygDropped).toBeUndefined();

    const produced = extractPaygLines(
      "### Meta Model API\nPAYG Oct 19–25: $23.92 accrued / $23.91 collected",
      "$29.60 per budget alert vs $45.10 per gateway cost attribution",
    );
    expect(produced).toHaveLength(2);
    const loaded = await loadWithPayg(produced);
    expect(loaded?.payg).toEqual(produced);
    expect(loaded?.paygDropped).toBeUndefined();
  });

  test("sanitized survivors still render and contribute attention flags", async () => {
    const loaded = await loadWithPayg([PAYG, null]);
    const usage = {
      services: [],
      payg: loaded?.payg ?? [],
      charges: [],
      attention: [],
      domains: null,
      subscriptions: [],
      history: {},
    } as never;
    expect(() => renderQuota(loaded!.quota, {} as never, usage)).not.toThrow();
    expect(buildAttentionFlags(loaded!.quota, { services: [], payg: loaded!.payg ?? [] })).toContainEqual(
      expect.objectContaining({ kind: "payg-unbilled", label: PAYG.label }),
    );
    expect(loaded?.paygDropped).toBe(1);
  });
});

describe("isChargeRow + loadFeed charges sanitization (#2445)", () => {
  const CHOICES = { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] };
  const ROW = {
    at: "2026-10-30",
    label: "Mystery AI Inc",
    amountUsd: 42.5,
    method: "Visa \u20227535",
    source: "plaid",
  };
  const feedWithCharges = (charges: unknown) => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota: { pools: [], burn: [], choices: CHOICES },
    attention: [],
    ...(charges === undefined ? {} : { charges }),
  });
  const load = async (feed: unknown) => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-charges-"));
    const file = join(dir, "feed.json");
    writeFileSync(file, JSON.stringify(feed));
    return loadFeed(file);
  };

  test("isChargeRow accepts a well-formed row and rejects malformed ones", () => {
    expect(isChargeRow(ROW)).toBe(true);
    expect(isChargeRow(null)).toBe(false);
    expect(isChargeRow({ ...ROW, amountUsd: undefined })).toBe(false);
    expect(isChargeRow({ ...ROW, amountUsd: "42.50" })).toBe(false);
    expect(isChargeRow({ at: ROW.at, label: ROW.label, amountUsd: 1, source: "plaid" })).toBe(false);
  });

  test("charges [null] drops the row, keeps the feed, surfaces the count", async () => {
    const loaded = await load(feedWithCharges([null]));
    expect(loaded).not.toBeNull();
    expect(loaded?.charges).toEqual([]);
    expect(loaded?.chargesDropped).toBe(1);
  });

  test("[valid, null] keeps the valid row", async () => {
    const loaded = await load(feedWithCharges([ROW, null]));
    expect(loaded?.charges).toEqual([ROW]);
    expect(loaded?.chargesDropped).toBe(1);
  });

  test("entries missing amountUsd or with a string amountUsd drop individually", async () => {
    const noAmount = { at: ROW.at, label: ROW.label, method: ROW.method, source: ROW.source };
    const loaded = await load(feedWithCharges([noAmount, { ...ROW, amountUsd: "42.50" }, ROW]));
    expect(loaded?.charges).toEqual([ROW]);
    expect(loaded?.chargesDropped).toBe(2);
  });

  test("a non-array charges section yields [] with the drop surfaced", async () => {
    for (const bad of ["oops", { at: ROW.at }]) {
      const loaded = await load(feedWithCharges(bad));
      expect(loaded).not.toBeNull();
      expect(loaded?.charges).toEqual([]);
      expect(loaded?.chargesDropped).toBe(1);
    }
  });

  test("an absent charges section stays absent", async () => {
    const loaded = await load(feedWithCharges(undefined));
    expect(loaded).not.toBeNull();
    expect(loaded?.charges).toBeUndefined();
    expect(loaded?.chargesDropped).toBeUndefined();
  });

  test("a well-formed feed is unchanged through the gate + loadFeed (collector self-acceptance)", async () => {
    const feed = feedWithCharges([ROW]);
    expect(isFleetFeed(feed)).toBe(true);
    const loaded = await load(feed);
    expect(loaded?.charges).toEqual([ROW]);
    expect(loaded?.chargesDropped).toBeUndefined();
  });
});

describe("feed chargesAttention gate + sanitization (#2439)", () => {
  const UNMATCHED = {
    merchant: "Mystery AI Inc",
    normalized: "mystery ai",
    amountUsd: 42.5,
    at: "2026-10-30",
    source: "plaid" as const,
  };
  const feedWithCa = (chargesAttention: unknown) => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    chargesAttention,
  });
  const loadFrom = async (feed: unknown) => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-ca-"));
    const file = join(dir, "feed.json");
    writeFileSync(file, JSON.stringify(feed));
    return loadFeed(file);
  };

  test("the gate accepts absent and array chargesAttention, rejects a present non-array", () => {
    const absent = feedWithCa(undefined) as Record<string, unknown>;
    delete absent.chargesAttention;
    expect(isFleetFeed(absent)).toBe(true);
    expect(isFleetFeed(feedWithCa([]))).toBe(true);
    expect(isFleetFeed(feedWithCa([UNMATCHED]))).toBe(true);
    expect(isFleetFeed(feedWithCa("nope"))).toBe(false);
    expect(isFleetFeed(feedWithCa(42))).toBe(false);
    expect(isFleetFeed(feedWithCa(null))).toBe(false);
  });

  test("isUnmatchedCharge pins exactly the fields the live source dereferences", () => {
    expect(isUnmatchedCharge(UNMATCHED)).toBe(true);
    expect(isUnmatchedCharge(null)).toBe(false);
    expect(isUnmatchedCharge({ ...UNMATCHED, merchant: 42 })).toBe(false);
    expect(isUnmatchedCharge({ ...UNMATCHED, amountUsd: "42.50" })).toBe(false);
    expect(isUnmatchedCharge({ ...UNMATCHED, amountUsd: Number.NaN })).toBe(false);
    const noAmount = { ...UNMATCHED } as Record<string, unknown>;
    delete noAmount.amountUsd;
    expect(isUnmatchedCharge(noAmount)).toBe(false);
    expect(isUnmatchedCharge({ ...UNMATCHED, at: 20261030 })).toBe(false);
    // source/normalized are tolerated in any form (consumer never dereferences them unsafely)
    expect(isUnmatchedCharge({ merchant: "X", amountUsd: 1, at: "2026-10-30" })).toBe(true);
  });

  test("loadFeed drops malformed entries individually and surfaces the count", async () => {
    const loaded = await loadFrom(
      feedWithCa([
        null,
        UNMATCHED,
        { ...UNMATCHED, amountUsd: "42.50" },
        { merchant: "No Amount Co", at: "2026-10-30" },
      ]),
    );
    expect(loaded).not.toBeNull();
    expect(loaded?.chargesAttention).toEqual([UNMATCHED]);
    expect(loaded?.chargesAttentionDropped).toBe(3);
  });

  test("loadFeed returns null for a non-array chargesAttention (fixture-fallback path)", async () => {
    expect(await loadFrom(feedWithCa("nope"))).toBeNull();
  });

  test("a well-formed chargesAttention passes loadFeed unchanged, with no dropped field", async () => {
    const loaded = await loadFrom(feedWithCa([UNMATCHED]));
    expect(loaded?.chargesAttention).toEqual([UNMATCHED]);
    expect(loaded?.chargesAttentionDropped).toBeUndefined();
  });
});

describe("charge gates reject impossible rows (#3458)", () => {
  const CHOICES = { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] };
  const ROW = {
    at: "2026-10-30",
    label: "Mystery AI Inc",
    amountUsd: 42.5,
    method: "Visa …7535",
    source: "plaid",
  };
  const UNMATCHED = {
    merchant: "Mystery AI Inc",
    normalized: "mystery ai",
    amountUsd: 42.5,
    at: "2026-10-30",
    source: "plaid" as const,
  };
  const baseFeed = (extra: unknown) => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota: { pools: [], burn: [], choices: CHOICES },
    attention: [],
    ...(extra as Record<string, unknown>),
  });
  const load = async (feed: unknown) => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-3458-"));
    const file = join(dir, "feed.json");
    writeFileSync(file, JSON.stringify(feed));
    return loadFeed(file);
  };

  test("isChargeRow rejects non-positive amounts and non-calendar dates, accepts a valid row", () => {
    expect(isChargeRow(ROW)).toBe(true);
    expect(isChargeRow({ ...ROW, amountUsd: -1 })).toBe(false);
    expect(isChargeRow({ ...ROW, amountUsd: 0 })).toBe(false);
    expect(isChargeRow({ ...ROW, at: "banana" })).toBe(false);
    expect(isChargeRow({ ...ROW, at: "2026-02-30" })).toBe(false);
  });

  test("isUnmatchedCharge rejects non-positive amounts and non-calendar dates, accepts a valid row", () => {
    expect(isUnmatchedCharge(UNMATCHED)).toBe(true);
    expect(isUnmatchedCharge({ ...UNMATCHED, amountUsd: -5 })).toBe(false);
    expect(isUnmatchedCharge({ ...UNMATCHED, amountUsd: 0 })).toBe(false);
    expect(isUnmatchedCharge({ ...UNMATCHED, at: "banana" })).toBe(false);
    expect(isUnmatchedCharge({ ...UNMATCHED, at: "2026-02-30" })).toBe(false);
  });

  test("loadFeed drops impossible charges individually and surfaces chargesDropped", async () => {
    const loaded = await load(
      baseFeed({
        charges: [
          ROW,
          { ...ROW, amountUsd: -1 },
          { ...ROW, amountUsd: 0 },
          { ...ROW, at: "banana" },
          { ...ROW, at: "2026-02-30" },
        ],
      }),
    );
    expect(loaded).not.toBeNull();
    expect(loaded?.charges).toEqual([ROW]);
    expect(loaded?.chargesDropped).toBe(4);
  });

  test("loadFeed drops impossible chargesAttention individually and surfaces chargesAttentionDropped", async () => {
    const loaded = await load(
      baseFeed({
        chargesAttention: [
          UNMATCHED,
          { ...UNMATCHED, amountUsd: -1 },
          { ...UNMATCHED, amountUsd: 0 },
          { ...UNMATCHED, at: "banana" },
          { ...UNMATCHED, at: "2026-02-30" },
        ],
      }),
    );
    expect(loaded).not.toBeNull();
    expect(loaded?.chargesAttention).toEqual([UNMATCHED]);
    expect(loaded?.chargesAttentionDropped).toBe(4);
  });
});

describe("collector section metadata (#1178)", () => {
  const baseFeed = () => ({
    version: 1,
    generatedAt: "2026-10-01T00:00:00Z",
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
  });

  test("status entries use explicit timestamps and sanitize errors", () => {
    const status = feedSectionStatus("error", "2026-10-01T00:00:00Z", "Bearer abc.def?token=private");
    expect(status.status).toBe("error");
    expect(status.at).toBe("2026-10-01T00:00:00Z");
    expect(status.error).toContain("[redacted]");
    expect(status.error).not.toContain("abc.def");
    expect(status.error).not.toContain("private");
    expect(sanitizeCollectionError("Authorization: Basic dXNlcjpwYXNz")).not.toContain("dXNlcjpwYXNz");
    expect(feedSectionStatus("ok", "2026-10-01T00:00:00Z").status).toBe("ok");
    expect(feedSectionStatus("skipped", "2026-10-01T00:00:00Z").error).toBeUndefined();
  });

  test("metadata is optional and malformed entries are dropped, not the legacy feed", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-meta-"));
    const file = join(dir, "feed.json");
    const feed = {
      ...baseFeed(),
      meta: {
        sections: {
          dns: { status: "error", at: "2026-10-01T00:00:00Z", error: "request failed token=secret-value" },
          orchestration: { status: "unknown", at: "bad timestamp" },
        },
      },
    };
    writeFileSync(file, JSON.stringify(feed));
    expect(isFleetFeed(baseFeed())).toBe(true);
    expect(isFleetFeed(feed)).toBe(true);
    expect(sanitizeFeedMetadata({ sections: { legacy: { status: "ok", at: "2026-10-01T00:00:00Z" } } })).toEqual({
      sections: { legacy: { status: "ok", at: "2026-10-01T00:00:00Z" } },
    });
    const loaded = await loadFeed(file);
    expect(loaded?.meta?.sections.dns?.status).toBe("error");
    expect(loaded?.meta?.sections.dns?.error).not.toContain("secret-value");
    expect(loaded?.meta?.sections.orchestration).toBeUndefined();

    const legacyFile = join(dir, "legacy.json");
    writeFileSync(legacyFile, JSON.stringify(baseFeed()));
    expect((await loadFeed(legacyFile))?.meta).toBeUndefined();
  });
});

describe("sanitizeCollectionError quoted-key + URL redaction (#3966)", () => {
  test("JSON double-quoted keys redact the value, keep the key and text", () => {
    expect(sanitizeCollectionError('{"api_key": "abc123short"}')).toBe('{"api_key": [redacted]}');
    expect(sanitizeCollectionError('{"password": "hunter2pw"}')).toBe('{"password": [redacted]}');
    expect(sanitizeCollectionError('upstream 500 {"token":"tok-abc12345"} retrying')).toBe(
      'upstream 500 {"token":[redacted]} retrying',
    );
  });

  test("single-quoted and nested keys redact the value", () => {
    expect(sanitizeCollectionError("{'token': 'tok-abc12345'}")).toBe("{'token': [redacted]}");
    expect(sanitizeCollectionError('{"data": {"secret": "s3cr3t-value-9"}}')).toBe(
      '{"data": {"secret": [redacted]}}',
    );
    expect(sanitizeCollectionError('{"secret": "contains \\" quote and remains private"}')).toBe(
      '{"secret": [redacted]}',
    );
  });

  test("bare URL credential params redact the value, sibling params survive", () => {
    expect(sanitizeCollectionError("fetch failed: https://api.example.com/data?key=supersecretvalue")).toBe(
      "fetch failed: https://api.example.com/data?key=[redacted]",
    );
    expect(sanitizeCollectionError("get https://h.example/?api_key=abc123&foo=bar failed")).toBe(
      "get https://h.example/?api_key=[redacted]&foo=bar failed",
    );
  });

  test("words merely containing key do not redact, unquoted forms still redact", () => {
    expect(sanitizeCollectionError("monkey=x")).toBe("monkey=x");
    expect(sanitizeCollectionError("keyboard=y")).toBe("keyboard=y");
    expect(sanitizeCollectionError("turnkey=abc")).toBe("turnkey=abc");
    expect(sanitizeCollectionError("request failed token=secret-value")).toBe("request failed token=[redacted]");
    expect(sanitizeCollectionError("plain error with no secrets at all")).toBe("plain error with no secrets at all");
  });

  test("quoted forms are redacted through sanitizeFeedMetadata", () => {
    const meta = sanitizeFeedMetadata({
      sections: { dns: { status: "error", at: "2026-10-01T00:00:00Z", error: '{"api_key": "abc123short"}' } },
    });
    expect(meta?.sections.dns?.error).toBe('{"api_key": [redacted]}');
  });
});
