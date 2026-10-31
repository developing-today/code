import { describe, expect, test } from "bun:test";
import { isFleetFeed } from "../src/feed.ts";
import { FIXTURE_ITEMS, FIXTURE_QUOTA } from "../src/sources/fixtures.ts";
import {
  MAX_FUTURE_SKEW_MS,
  arrayOf,
  isFiniteNumber,
  isIsoDate,
  isNonEmptyString,
  isNonNegative,
  isPercentOf,
  isRecord,
  isStringArray,
  isTimestampNotFuture,
  parseIsoDate,
} from "../src/validate.ts";

// Malformed-input probe matrix for the #2576 ingest primitives
// (#2577): one table per primitive — impossible dates, non-finite
// numbers, negative counts, over-limit percents, and null/string/
// array element confusion — plus the real fixture shapes as
// must-pass cases.

describe("validate: records and strings", () => {
  const records: [unknown, boolean][] = [
    [{}, true],
    [{ a: 1 }, true],
    [null, false],
    [[], false],
    [[1], false],
    ["x", false],
    [42, false],
    [undefined, false],
  ];
  for (const [v, want] of records) {
    test(`isRecord(${JSON.stringify(v)}) === ${want}`, () => {
      expect(isRecord(v)).toBe(want);
    });
  }

  const strings: [unknown, boolean][] = [
    ["doc", true],
    [" x ", true],
    ["", false],
    ["   ", false],
    [null, false],
    [42, false],
    [["doc"], false],
    [{}, false],
  ];
  for (const [v, want] of strings) {
    test(`isNonEmptyString(${JSON.stringify(v)}) === ${want}`, () => {
      expect(isNonEmptyString(v)).toBe(want);
    });
  }

  const stringArrays: [unknown, boolean][] = [
    [[], true],
    [["a", "b"], true],
    [["a", 1], false],
    [["a", null], false],
    ["a", false],
    [null, false],
    [{}, false],
  ];
  for (const [v, want] of stringArrays) {
    test(`isStringArray(${JSON.stringify(v)}) === ${want}`, () => {
      expect(isStringArray(v)).toBe(want);
    });
  }
});

describe("validate: numbers", () => {
  const finite: [unknown, boolean][] = [
    [0, true],
    [-0, true],
    [3.14, true],
    [-7, true],
    [NaN, false],
    [Infinity, false],
    [-Infinity, false],
    ["42", false],
    [null, false],
    [undefined, false],
  ];
  for (const [v, want] of finite) {
    test(`isFiniteNumber(${String(v)}) === ${want}`, () => {
      expect(isFiniteNumber(v)).toBe(want);
    });
  }

  const nonNegative: [unknown, boolean][] = [
    [0, true], // a genuine 0 passes as 0
    [0.5, true],
    [250, true],
    [-1, false],
    [-0.001, false],
    [NaN, false],
    [Infinity, false],
    ["0", false],
    [null, false],
  ];
  for (const [v, want] of nonNegative) {
    test(`isNonNegative(${String(v)}) === ${want}`, () => {
      expect(isNonNegative(v)).toBe(want);
    });
  }

  const percentOf100 = isPercentOf(100);
  const percents: [unknown, boolean][] = [
    [0, true],
    [98, true],
    [100, true],
    [100.01, false], // over the limit the document carries
    [101, false],
    [-1, false],
    [NaN, false],
    [Infinity, false],
    ["98", false],
    [null, false],
  ];
  for (const [v, want] of percents) {
    test(`isPercentOf(100)(${String(v)}) === ${want}`, () => {
      expect(percentOf100(v)).toBe(want);
    });
  }
  test("isPercentOf with a non-finite limit validates nothing", () => {
    expect(isPercentOf(Infinity)(50)).toBe(false);
    expect(isPercentOf("100")(50)).toBe(false);
    expect(isPercentOf(undefined)(50)).toBe(false);
  });
});

describe("validate: real calendar dates", () => {
  const valid = [
    "2026-10-25",
    "2026-10-25T17:00:00Z",
    "2026-10-25T17:00:00.123Z",
    "2024-02-29", // leap day, real in 2024
    "2026-01-31T23:59:59Z",
    "2026-10-25T17:00:00+02:00",
    "2026-10-25T17:00Z",
  ];
  for (const v of valid) {
    test(`isIsoDate(${v}) === true`, () => {
      expect(isIsoDate(v)).toBe(true);
      expect(parseIsoDate(v)).not.toBeNull();
    });
  }
  const invalid: unknown[] = [
    "2026-02-30", // February has no 30th — Date.parse rolls this over
    "2026-02-29", // 2026 is not a leap year
    "2026-13-01", // no month 13
    "2026-00-10",
    "2026-10-00",
    "2026-10-32",
    "2026-04-31", // April has 30 days
    "2026-10-25T24:00:00Z",
    "2026-10-25T17:60:00Z",
    "2026-10-25T17:00:60Z",
    "2026/10/25",
    "25-10-2026",
    "2026-10-25 17:00:00",
    "not-a-date",
    "",
    null,
    undefined,
    1_700_000_000,
    ["2026-10-25"],
    {},
  ];
  for (const v of invalid) {
    test(`isIsoDate(${JSON.stringify(v)}) === false`, () => {
      expect(isIsoDate(v)).toBe(false);
      expect(parseIsoDate(v)).toBeNull();
    });
  }
  test("parseIsoDate returns the exact instant", () => {
    expect(parseIsoDate("2026-10-25T17:00:00Z")?.getTime()).toBe(Date.UTC(2026, 9, 25, 17));
    expect(parseIsoDate("2026-10-25")?.getTime()).toBe(Date.UTC(2026, 9, 25));
    expect(parseIsoDate("2026-10-25T17:00:00+02:00")?.getTime()).toBe(Date.UTC(2026, 9, 25, 15));
  });
});

describe("validate: future timestamps (#2242 shape)", () => {
  const now = Date.UTC(2026, 9, 25, 17, 0, 0);
  test("past and present stamps pass", () => {
    expect(isTimestampNotFuture("2026-10-25T16:00:00Z", MAX_FUTURE_SKEW_MS, now)).toBe(true);
    expect(isTimestampNotFuture("2026-10-25T17:00:00Z", MAX_FUTURE_SKEW_MS, now)).toBe(true);
  });
  test("within the 5-minute skew passes, beyond it fails", () => {
    expect(isTimestampNotFuture("2026-10-25T17:04:59Z", MAX_FUTURE_SKEW_MS, now)).toBe(true);
    expect(isTimestampNotFuture("2026-10-25T17:05:01Z", MAX_FUTURE_SKEW_MS, now)).toBe(false);
    expect(isTimestampNotFuture("2027-10-25T17:00:00Z", MAX_FUTURE_SKEW_MS, now)).toBe(false);
  });
  test("impossible dates are not timestamps", () => {
    expect(isTimestampNotFuture("2026-02-30T00:00:00Z", MAX_FUTURE_SKEW_MS, now)).toBe(false);
    expect(isTimestampNotFuture(null, MAX_FUTURE_SKEW_MS, now)).toBe(false);
  });
});

describe("validate: arrayOf drops and counts", () => {
  const stringsOf = arrayOf(isNonEmptyString);
  test("keeps good elements, counts the dropped", () => {
    expect(stringsOf(["a", "", null, "b", 42, "  "])).toEqual({
      values: ["a", "b"],
      dropped: 4,
    });
  });
  test("an all-good array drops nothing; an empty array is a valid container", () => {
    expect(stringsOf(["a"])).toEqual({ values: ["a"], dropped: 0 });
    expect(stringsOf([])).toEqual({ values: [], dropped: 0 });
  });
  test("a non-array container is null (shape failure), not an empty success", () => {
    expect(stringsOf("a")).toBeNull();
    expect(stringsOf(null)).toBeNull();
    expect(stringsOf({ 0: "a" })).toBeNull();
  });
});

describe("validate: fixture shapes must pass", () => {
  test("quota fixture windows: resetsAt/wallLiftsAt are real dates, percents within limit", () => {
    for (const pool of FIXTURE_QUOTA.pools) {
      if (typeof pool.wallLiftsAt === "string") expect(isIsoDate(pool.wallLiftsAt)).toBe(true);
      for (const w of pool.windows) {
        // resetsAt is nullable in the fixture shape (no known reset);
        // when a string is carried it must be a real date.
        if (typeof w.resetsAt === "string") expect(isIsoDate(w.resetsAt)).toBe(true);
        expect(isPercentOf(100)(w.percent)).toBe(true);
        expect(isNonNegative(w.used)).toBe(true);
      }
    }
  });
  test("item fixture updatedAt stamps are real dates", () => {
    for (const item of FIXTURE_ITEMS) {
      expect(isIsoDate(item.updatedAt)).toBe(true);
    }
  });
});

describe("feed gate: isFleetFeed uses the strict date (#2577 proof consumer)", () => {
  const feed = (generatedAt: unknown) => ({
    version: 1,
    generatedAt,
    quota: { pools: [] },
    attention: [],
  });
  test("a genuine feed passes", () => {
    expect(isFleetFeed(feed("2026-10-25T17:00:00Z"))).toBe(true);
  });
  test("impossible generatedAt values Date.parse used to accept now fail", () => {
    expect(isFleetFeed(feed("2026-02-30T00:00:00Z"))).toBe(false);
    expect(isFleetFeed(feed("2026-13-01T00:00:00Z"))).toBe(false);
    expect(isFleetFeed(feed("2026-02-29T00:00:00Z"))).toBe(false);
  });
  test("non-string generatedAt still fails", () => {
    expect(isFleetFeed(feed(1_700_000_000))).toBe(false);
    expect(isFleetFeed(feed(null))).toBe(false);
  });
});
