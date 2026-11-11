// #7741: the three aggregator diagnostic-text caps raw-sliced at 240 UTF-16
// code units — safeProviderReason (credential-lifecycle.ts),
// sanitizeCollectionError (feed.ts), and extractNeedsDrewReason
// (needs-drew.ts) — so an astral character straddling the cap was split,
// leaving a lone high surrogate in the lifecycle row reason, the persisted
// feed section error, and the needs-Drew reason (the #3178 class;
// #7694/#7711/#7725/#7731/#7736 fixed the same defect in the neighbouring
// stores via capUnits, but these three caps were not audited).
import { describe, expect, test } from "bun:test";
import { safeProviderReason } from "../src/credential-lifecycle.ts";
import { sanitizeCollectionError } from "../src/feed.ts";
import { extractNeedsDrewReason } from "../src/needs-drew.ts";

const EMOJI = "\u{1F600}"; // 😀 — one astral character, two UTF-16 units
const loneHighAtEnd = (s: string) => {
  const c = s.charCodeAt(s.length - 1);
  return c >= 0xd800 && c <= 0xdbff;
};
const roundTrips = (s: string) => Buffer.from(s, "utf8").toString("utf8") === s;
// 239 units of short words (no 40+ char token, no credential-shaped text, so
// no redaction replacement fires), then a pair straddling unit 240.
const STRADDLE = "ab ".repeat(79) + "ab" + EMOJI + "tail";
// 238 units + the pair ending exactly at unit 240: must pass through intact.
const EXACT_FIT = "ab ".repeat(79) + "a" + EMOJI;

describe("#7741 safeProviderReason cap is surrogate-pair-safe", () => {
  test("pair straddling the 240 cap is dropped whole", () => {
    const r = safeProviderReason(STRADDLE);
    expect(r.length).toBe(239);
    expect(loneHighAtEnd(r)).toBe(false);
    expect(roundTrips(r)).toBe(true);
  });

  test("pair ending exactly at the 240 cap stays intact (control)", () => {
    expect(safeProviderReason(EXACT_FIT)).toBe(EXACT_FIT);
  });

  test("redaction still applies before the cap", () => {
    const r = safeProviderReason("Bearer abcdefgh " + STRADDLE);
    expect(r).not.toContain("abcdefgh");
    expect(loneHighAtEnd(r)).toBe(false);
  });
});

describe("#7741 sanitizeCollectionError cap is surrogate-pair-safe", () => {
  test("pair straddling the 240 cap is dropped whole", () => {
    const r = sanitizeCollectionError(STRADDLE);
    expect(r.length).toBe(239);
    expect(loneHighAtEnd(r)).toBe(false);
    expect(roundTrips(r)).toBe(true);
  });

  test("pair ending exactly at the 240 cap stays intact (control)", () => {
    expect(sanitizeCollectionError(EXACT_FIT)).toBe(EXACT_FIT);
  });

  test("Error input takes the same capped path", () => {
    const r = sanitizeCollectionError(new Error(STRADDLE));
    expect(r.length).toBe(239);
    expect(loneHighAtEnd(r)).toBe(false);
  });
});

describe("#7741 extractNeedsDrewReason cap is surrogate-pair-safe", () => {
  test("pair straddling the 240 cap is dropped whole", () => {
    const r = extractNeedsDrewReason("Reason: " + STRADDLE);
    expect(r).not.toBeNull();
    expect(r!.length).toBe(239);
    expect(loneHighAtEnd(r!)).toBe(false);
    expect(roundTrips(r!)).toBe(true);
  });

  test("pair ending exactly at the 240 cap stays intact (control)", () => {
    expect(extractNeedsDrewReason("Reason: " + EXACT_FIT)).toBe(EXACT_FIT);
  });
});
