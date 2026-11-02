// #1057: collector glue for the charges auto-matcher — candidate
// file validation, auto/hand-fed row merge, surfaced-once state,
// and the packet's replay acceptance (one row per charge, rerun
// adds nothing and re-surfaces nothing).
import { describe, expect, test } from "bun:test";
import { filterNewUnmatched, mergeChargeRows, parseChargeCandidates, unmatchedKey } from "../src/charges-collect.ts";
import { matchCharges, type ChargeCandidate } from "../src/charges-match.ts";
import type { ChargeRow } from "../src/model.ts";

const SERVICES = [
  { name: "Anthropic", aliases: ["Claude"] },
  { name: "OpenAI", aliases: ["ChatGPT"] },
  { name: "Cloudflare" },
];

describe("parseChargeCandidates", () => {
  test("accepts the canonical object form and keeps valid entries only", () => {
    const raw = {
      version: 1,
      candidates: [
        { source: "plaid", at: "2026-10-28", merchant: "METAPAY*UC5DPA22Y4 US", amountUsd: 20, method: "Visa •7535" },
        { source: "gmail", at: "2026-10-27T14:03:00Z", merchant: "MetaPay", amountUsd: 20 },
        { source: "carrier-pigeon", at: "2026-10-27", merchant: "Nope", amountUsd: 5 },
        { source: "plaid", at: "not-a-date", merchant: "Nope", amountUsd: 5 },
        { source: "plaid", at: "2026-10-27", merchant: "Nope", amountUsd: "20" },
        { source: "plaid", at: "2026-10-27", merchant: "  ", amountUsd: 5 },
      ],
    };
    expect(parseChargeCandidates(raw)).toEqual([
      { source: "plaid", at: "2026-10-28", merchant: "METAPAY*UC5DPA22Y4 US", amountUsd: 20, method: "Visa •7535" },
      { source: "gmail", at: "2026-10-27", merchant: "MetaPay", amountUsd: 20 },
    ]);
  });

  test("accepts a bare array; rejects anything else", () => {
    expect(
      parseChargeCandidates([{ source: "gmail", at: "2026-10-01", merchant: "Cursor", amountUsd: 20 }]),
    ).toHaveLength(1);
    expect(parseChargeCandidates(null)).toEqual([]);
    expect(parseChargeCandidates("nope")).toEqual([]);
    expect(parseChargeCandidates({ charges: [] })).toEqual([]);
  });

  test("rejects impossible calendar dates but keeps leap days and timestamp slicing", () => {
    const dates = [
      "2026-13-45",
      "2026-02-30",
      "2026-00-00",
      "2026-10-00",
      "2026-02-29",
      "2026-04-31",
      "2024-02-29",
      "2026-02-28T14:03:00Z",
    ];

    expect(
      parseChargeCandidates(dates.map((at) => ({ source: "plaid", at, merchant: "Example", amountUsd: 1 }))).map(
        (candidate) => candidate.at,
      ),
    ).toEqual(["2024-02-29", "2026-02-28"]);
  });
});

describe("mergeChargeRows canonical-label dedupe", () => {
  // Codex finding on PR #3168: the charge catalog canonicalizes a
  // METAPAY auto-match to "Meta Model API", which no longer
  // containment-matches the hand-fed "Meta PAYG spill" row for the
  // same charge, so the charge would appear twice in the feed.
  test("a canonicalized auto row still dedupes the merchant-spelled hand-fed row", () => {
    const auto = [{ at: "2026-10-01", label: "Meta Model API", amountUsd: 20 }];
    const handFed = [{ at: "2026-10-01", label: "Meta PAYG spill", amountUsd: 20 }];
    expect(mergeChargeRows(auto, handFed)).toEqual(auto);
  });

  test("a different amount under the same provider still keeps both rows", () => {
    const auto = [{ at: "2026-10-01", label: "Meta Model API", amountUsd: 20 }];
    const handFed = [{ at: "2026-10-01", label: "Meta PAYG spill", amountUsd: 25 }];
    expect(mergeChargeRows(auto, handFed)).toHaveLength(2);
  });
});

describe("mergeChargeRows", () => {
  const auto: ChargeRow[] = [
    { at: "2026-10-28", label: "METAPAY*UC5DPA22Y4 US", amountUsd: 20, method: "Visa •7535", source: "Plaid" },
    { at: "2026-10-11", label: "Anthropic", amountUsd: 100, method: "card", source: "Plaid" },
  ];

  test("a hand-fed row for the same charge is dropped (auto is source of record)", () => {
    const handFed: ChargeRow[] = [
      {
        at: "2026-10-25",
        label: "Meta PAYG spill",
        amountUsd: 20,
        method: "Visa •7535",
        source: "Gmail card alert (registry billing incidents)",
      },
    ];
    // 2026-10-25 vs 2026-10-28 is inside the 3-day window, same
    // amount, and squashed "METAPAYGSPILL" contains "METAPAY" — so
    // the hand-fed row dedupes against the auto row (date order out).
    expect(mergeChargeRows(auto, handFed)).toEqual([auto[1], auto[0]]);
  });

  test("a hand-fed row outside the window or for another charge is kept", () => {
    const handFed: ChargeRow[] = [
      {
        at: "2026-10-01",
        label: "Meta PAYG spill",
        amountUsd: 20,
        method: "Visa •7535",
        source: "Gmail card alert (registry billing incidents)",
      },
      { at: "2026-10-11", label: "Cline Bot Inc.", amountUsd: 42, method: "card", source: "quota view" },
    ];
    const merged = mergeChargeRows(auto, handFed);
    expect(merged).toHaveLength(4);
    expect(merged[0]?.at).toBe("2026-10-01");
  });

  test("no auto rows = the hand-fed list, sorted (pre-#1057 behaviour)", () => {
    const handFed: ChargeRow[] = [
      { at: "2026-10-11", label: "B", amountUsd: 1, method: "card", source: "x" },
      { at: "2026-10-01", label: "A", amountUsd: 2, method: "card", source: "x" },
    ];
    expect(mergeChargeRows([], handFed).map((r) => r.label)).toEqual(["A", "B"]);
  });
});

describe("filterNewUnmatched / surfaced-once", () => {
  const unmatched = [
    { merchant: "CURSOR.COM", normalized: "CURSOR", amountUsd: 20, at: "2026-10-12", source: "plaid" as const },
    { merchant: "LINEAR", normalized: "LINEAR", amountUsd: 10, at: "2026-10-15", source: "gmail" as const },
  ];

  test("first run surfaces all; persisting the keys makes the rerun surface none", () => {
    const first = filterNewUnmatched(unmatched, new Set());
    expect(first.fresh).toEqual(unmatched);
    const second = filterNewUnmatched(unmatched, new Set(first.keys));
    expect(second.fresh).toEqual([]);
    expect(second.keys).toEqual(first.keys);
  });

  test("a genuinely new unmatched charge still surfaces alongside known ones", () => {
    const known = new Set([unmatchedKey(unmatched[0]!)]);
    const { fresh } = filterNewUnmatched(unmatched, known);
    expect(fresh).toEqual([unmatched[1]]);
  });
});

describe("period replay (packet acceptance)", () => {
  const candidates: ChargeCandidate[] = [
    { source: "plaid", at: "2026-10-28", merchant: "METAPAY*UC5DPA22Y4 US", amountUsd: 20, method: "Visa •7535" },
    { source: "gmail", at: "2026-10-27", merchant: "MetaPay", amountUsd: 20 },
    { source: "gmail", at: "2026-10-09", merchant: "ChatGPT Subscription", amountUsd: 20 },
    { source: "plaid", at: "2026-10-12", merchant: "CURSOR.COM", amountUsd: 20 },
    { source: "plaid", at: "2026-10-13", merchant: "WALMART STORE 1234", amountUsd: 54.12 },
  ];
  const handFed: ChargeRow[] = [
    {
      at: "2026-10-25",
      label: "Meta PAYG spill",
      amountUsd: 7.5,
      method: "Visa •7535",
      source: "Gmail card alert (registry billing incidents)",
    },
  ];

  test("one row per charge with source labels, no cross-source duplicates", () => {
    const { rows, unmatchedSaas } = matchCharges(candidates, SERVICES);
    const feed = mergeChargeRows(rows, handFed);
    // 4 deduped auto charges + the distinct $7.50 hand-fed incident.
    expect(feed).toHaveLength(5);
    expect(feed.filter((r) => r.amountUsd === 20 && r.source === "Plaid")).toHaveLength(2);
    expect(feed.find((r) => r.label === "OpenAI")?.source).toBe("Gmail alert");
    expect(unmatchedSaas.map((u) => u.normalized)).toEqual(["CURSOR", "METAPAY"]);
  });

  test("re-running the collector for the same period adds no rows and re-surfaces nothing", () => {
    const run = (surfaced: ReadonlySet<string>) => {
      const { rows, unmatchedSaas } = matchCharges(candidates, SERVICES);
      return { feed: mergeChargeRows(rows, handFed), ...filterNewUnmatched(unmatchedSaas, surfaced) };
    };
    const first = run(new Set());
    const second = run(new Set(first.keys));
    expect(second.feed).toEqual(first.feed);
    expect(first.fresh).toHaveLength(2);
    expect(second.fresh).toEqual([]);
  });
});
