import { describe, expect, test } from "bun:test";
import { isChargeRow, isFleetFeed, isProjectPoolSection, isUnmatchedCharge, loadFeed, poolFromFeed, projectPoolSectionProblem, sanitizeQuotaView, sanitizeSubscriptions } from "../src/feed.ts";
import { renderQuota } from "../src/render.ts";
import { renderQuota } from "../src/render.ts";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { isProjectPoolItem, parseProjectItems } from "../src/projects.ts";
import type { QuotaPool, QuotaView, QuotaWindow } from "../src/model.ts";

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
  const WINDOW: QuotaWindow = { name: "5h", unit: "tokens", limit: 1000, used: 250, resetsAt: "2026-10-29T05:00:00Z", percent: 25 };
  const POOL: QuotaPool = { id: "opencode-go", provider: "opencode", plan: "go", state: "healthy", windows: [WINDOW] };
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

  test("malformed burn segments drop individually", () => {
    const seg = { provider: "opencode", pool: "go", from: "2026-10-01T00:00:00Z", to: "2026-10-01T01:00:00Z", tokens: 42 };
    const out = sanitizeQuotaView(quotaWith([POOL], { burn: [seg, null, { provider: "x" }] }) as never);
    expect(out.droppedBurn).toBe(2);
    expect(out.view.burn).toEqual([seg]);
  });

  test("a fully well-formed quota view is unchanged", () => {
    const q = quotaWith([POOL], { burn: [] }) as unknown as QuotaView;
    const out = sanitizeQuotaView(q);
    expect(out.droppedPools + out.droppedWindows + out.droppedBurn).toBe(0);
    expect(out.view).toEqual(q);
  });

  test("choices are reconciled to surviving pools with usable windows", () => {
    const noWindows = { ...POOL, windows: [null] };
    const out = sanitizeQuotaView(quotaWith([noWindows]) as never);
    expect(out.view.pools.length).toBe(1);
    expect(out.view.choices).toEqual({ hasHeadroom: [], burnBeforeReset: [], doNotStart: [] });
    expect(out.view.dropped).toEqual({ pools: 0, windows: 1, burn: 0 });
  });

  test("a dropped pool leaves the retained choices and surfaces dropped on the view", () => {
    const out = sanitizeQuotaView(quotaWith([null, POOL]) as never);
    expect(out.view.choices.hasHeadroom).toEqual(["opencode-go"]);
    expect(out.view.dropped).toEqual({ pools: 1, windows: 0, burn: 0 });
  });

  test("loadFeed returns the sanitized remainder with drop counts attached", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-feed-quota-"));
    const file = join(dir, "feed.json");
    const pool = { ...POOL, windows: [WINDOW, null] };
    writeFileSync(file, JSON.stringify(feedWith(quotaWith([null, pool]))));
    const loaded = await loadFeed(file);
    expect(loaded).not.toBeNull();
    expect(loaded?.quota.pools.length).toBe(1);
    expect(loaded?.quota.pools[0]?.windows).toEqual([WINDOW]);
    expect(loaded?.quotaDropped).toEqual({ pools: 1, windows: 1, burn: 0 });
    expect(loaded?.quota.dropped).toEqual({ pools: 1, windows: 1, burn: 0 });
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
describe("isChargeRow + loadFeed charges sanitization (#2445)", () => {
  const CHOICES = { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] };
  const ROW = { at: "2026-10-30", label: "Mystery AI Inc", amountUsd: 42.5, method: "Visa \u20227535", source: "plaid" };
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
  const UNMATCHED = { merchant: "Mystery AI Inc", normalized: "mystery ai", amountUsd: 42.5, at: "2026-10-30", source: "plaid" };
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
    const loaded = await loadFrom(feedWithCa([null, UNMATCHED, { ...UNMATCHED, amountUsd: "42.50" }, { merchant: "No Amount Co", at: "2026-10-30" }]));
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
