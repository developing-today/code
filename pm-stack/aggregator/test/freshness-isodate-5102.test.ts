import { describe, expect, test } from "bun:test";
import { freshness } from "../src/derive.ts";

// #5102: freshness() aged measuredAt with raw Date.parse, which rolls
// impossible dates (2026-02-30 -> Mar 2) and parses non-ISO strings
// ('March 2, 2026'), so malformed stamps rendered as fresh (ageDays 0).
// A stamp that is not a real ISO date now has honestly unknown age:
// { stale: true, ageDays: null }, in both the date-only and the
// datetime branch. Valid stamps age exactly as before.

const now = new Date("2026-03-01T12:00:00Z");

describe("freshness() measuredAt validity (#5102)", () => {
  test("impossible date-only stamps are unknown age, not rolled", () => {
    expect(freshness("2026-02-30", 7, now)).toEqual({ stale: true, ageDays: null });
    expect(freshness("2026-04-31", 7, now)).toEqual({ stale: true, ageDays: null });
  });

  test("impossible datetime stamps are unknown age, not rolled", () => {
    expect(freshness("2026-02-30T12:00:00Z", 7, now)).toEqual({ stale: true, ageDays: null });
    expect(freshness("2026-04-31T12:00:00Z", 7, now)).toEqual({ stale: true, ageDays: null });
  });

  test("non-ISO stamps are unknown age, not leniently parsed", () => {
    expect(freshness("March 2, 2026", 7, now)).toEqual({ stale: true, ageDays: null });
    expect(freshness("03/02/2026", 7, now)).toEqual({ stale: true, ageDays: null });
  });

  test("valid controls unchanged", () => {
    // Chicago calendar-day ageing for date-only stamps (#1629).
    expect(freshness("2026-02-27", 7, now)).toEqual({ stale: false, ageDays: 2 });
    expect(freshness("2026-03-01", 7, now)).toEqual({ stale: false, ageDays: 0 });
    // Datetimes age by elapsed whole days; offsets still parse.
    expect(freshness("2026-02-27T12:00:00Z", 7, now)).toEqual({ stale: false, ageDays: 2 });
    expect(freshness("2026-02-27T12:00:00+00:00", 7, now)).toEqual({ stale: false, ageDays: 2 });
    // Real future dates beyond tolerance stay stale/null per #1568.
    expect(freshness("2026-03-05", 7, now)).toEqual({ stale: true, ageDays: null });
    expect(freshness("2026-03-05T12:00:00Z", 7, now)).toEqual({ stale: true, ageDays: null });
  });
});
