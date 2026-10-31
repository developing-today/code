// #2576 Phase 1: the malformed-input probe matrix for src/validate.ts —
// one table per primitive. The must-pass rows are the real shapes the
// fleet's producers emit (fixture shapes from src/sources/fixtures.ts);
// the must-fail rows are the defect shapes the validation family of
// issues kept re-filing: impossible dates, non-finite numbers,
// negative counts, over-limit percents, and null/string/array element
// confusion.
import { describe, expect, test } from "bun:test";
import { isFleetFeed } from "../src/feed.ts";
import { FIXTURE_DNS } from "../src/sources/fixtures.ts";
import {
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

describe("isRecord", () => {
  const cases: Array<[unknown, boolean]> = [
    [{}, true],
    [{ a: 1 }, true],
    [null, false],
    [[], false],
    [["a"], false],
    ["{}", false],
    [42, false],
    [undefined, false],
  ];
  for (const [v, want] of cases) {
    test(`${JSON.stringify(v)} -> ${want}`, () => {
      expect(isRecord(v)).toBe(want);
    });
  }
});

describe("isNonEmptyString", () => {
  const cases: Array<[unknown, boolean]> = [
    ["demo", true],
    ["0", true],
    ["", false],
    [null, false],
    [0, false],
    [["x"], false],
    [{}, false],
  ];
  for (const [v, want] of cases) {
    test(`${JSON.stringify(v)} -> ${want}`, () => {
      expect(isNonEmptyString(v)).toBe(want);
    });
  }
});

describe("isStringArray", () => {
  const cases: Array<[unknown, boolean]> = [
    [[], true],
    [["a", "b"], true],
    [["a", 1], false],
    [["a", null], false],
    ["a,b", false],
    [null, false],
    [{ 0: "a" }, false],
  ];
  for (const [v, want] of cases) {
    test(`${JSON.stringify(v)} -> ${want}`, () => {
      expect(isStringArray(v)).toBe(want);
    });
  }
});

describe("isFiniteNumber / isNonNegative", () => {
  const finite: Array<[unknown, boolean]> = [
    [0, true], // a genuine 0 is a measurement (#1153)
    [-0, true],
    [3.14, true],
    [-7, true],
    [Number.MAX_SAFE_INTEGER, true],
    [NaN, false],
    [Infinity, false],
    [-Infinity, false],
    ["42", false],
    [null, false],
    [undefined, false],
  ];
  for (const [v, want] of finite) {
    test(`isFiniteNumber(${String(v)}) -> ${want}`, () => {
      expect(isFiniteNumber(v)).toBe(want);
    });
  }
  const nonNeg: Array<[unknown, boolean]> = [
    [0, true], // zero passes as zero
    [0.5, true],
    [1e9, true],
    [-1, false], // negative counts are malformed, not live data
    [-0.01, false],
    [NaN, false],
    [Infinity, false],
    ["3", false],
  ];
  for (const [v, want] of nonNeg) {
    test(`isNonNegative(${String(v)}) -> ${want}`, () => {
      expect(isNonNegative(v)).toBe(want);
    });
  }
});

describe("isPercentOf(limit)", () => {
  const pct100 = isPercentOf(100);
  const cases: Array<[unknown, boolean]> = [
    [0, true],
    [42.5, true],
    [100, true],
    [100.01, false],
    [1e9, false], // #2571: an over-limit window is malformed
    [-20, false], // #2571: so is a negative one
    [NaN, false],
    [Infinity, false],
    ["50", false],
    [null, false],
  ];
  for (const [v, want] of cases) {
    test(`isPercentOf(100)(${String(v)}) -> ${want}`, () => {
      expect(pct100(v)).toBe(want);
    });
  }
  test("the limit is the document's own, not a constant", () => {
    expect(isPercentOf(1000)(999)).toBe(true);
    expect(isPercentOf(1000)(1001)).toBe(false);
  });
});

describe("parseIsoDate / isIsoDate — real calendar dates only", () => {
  const valid = [
    "2026-02-28",
    "2024-02-29", // leap day, real in 2024
    "2026-10-25T17:00:00Z", // FIXTURE_DNS generatedAt shape
    "2026-10-26T18:05:00Z",
    "2026-01-01T00:00:00.500Z",
    "2026-06-15T12:30:00+02:00",
    "2026-06-15",
  ];
  for (const s of valid) {
    test(`accepts ${s}`, () => {
      expect(isIsoDate(s)).toBe(true);
      expect(parseIsoDate(s)).toBeInstanceOf(Date);
    });
  }
  const invalid: Array<unknown> = [
    "2026-02-30", // Date.parse rolls this into March — we must not
    "2026-02-29", // 2026 is not a leap year
    "2026-13-01", // no thirteenth month
    "2026-00-10",
    "2026-04-31", // April has 30 days
    "2026-01-01T25:00:00Z", // no 25th hour
    "2026-01-01T00:61:00Z",
    "2026-1-1", // shape alone is not ISO
    "10/25/2026",
    "2026-10-25T17:00:00Z trailing",
    "",
    "not a date",
    null,
    1761435600000, // epoch numbers are not date strings
    undefined,
    {},
    ["2026-01-01"],
  ];
  for (const s of invalid) {
    test(`rejects ${JSON.stringify(s)}`, () => {
      expect(isIsoDate(s)).toBe(false);
      expect(parseIsoDate(s)).toBeNull();
    });
  }
  test("parses the components it was given, in UTC", () => {
    const d = parseIsoDate("2026-10-25T17:00:00Z");
    expect(d?.toISOString()).toBe("2026-10-25T17:00:00.000Z");
  });
  test("an explicit offset shifts to the matching UTC instant", () => {
    const d = parseIsoDate("2026-06-15T12:30:00+02:00");
    expect(d?.toISOString()).toBe("2026-06-15T10:30:00.000Z");
  });
});

describe("isTimestampNotFuture — the #2242 skew bound", () => {
  const now = new Date("2026-10-31T10:00:00Z");
  const guard = isTimestampNotFuture(); // default 5-minute skew
  const cases: Array<[unknown, boolean]> = [
    ["2026-10-31T09:00:00Z", true],
    ["2026-10-31T10:04:59Z", true], // inside the skew
    ["2026-10-31T10:05:01Z", false], // just past it
    ["2027-01-01T00:00:00Z", false], // maximally "fresh" is invalid
    ["2026-02-30T10:00:00Z", false], // impossible dates fail first
    [null, false],
  ];
  for (const [v, want] of cases) {
    test(`${JSON.stringify(v)} -> ${want}`, () => {
      expect(guard(v, now)).toBe(want);
    });
  }
  test("a boundary can pin its own bound", () => {
    const strict = isTimestampNotFuture(0);
    expect(strict("2026-10-31T10:00:01Z", now)).toBe(false);
    expect(strict("2026-10-31T10:00:00Z", now)).toBe(true);
  });
});

describe("arrayOf — drop the element, count it", () => {
  const strings = arrayOf(isNonEmptyString);
  test("keeps good elements and counts the dropped ones", () => {
    expect(strings(["a", "", null, "b", 42])).toEqual({
      values: ["a", "b"],
      dropped: 3,
    });
  });
  test("an all-good container drops zero", () => {
    expect(strings(["a"])).toEqual({ values: ["a"], dropped: 0 });
  });
  test("an empty container is an empty result, not a failure", () => {
    expect(strings([])).toEqual({ values: [], dropped: 0 });
  });
  test("a non-array container is null — never a fabricated empty list", () => {
    expect(strings("a,b")).toBeNull();
    expect(strings(null)).toBeNull();
    expect(strings({ 0: "a", length: 1 })).toBeNull();
  });
  test("composes with the numeric guards", () => {
    const counts = arrayOf(isNonNegative);
    expect(counts([3, -1, 0, NaN, 7])).toEqual({
      values: [3, 0, 7],
      dropped: 2,
    });
  });
});

describe("fixture shapes must pass (the over-strict-validation guard)", () => {
  test("FIXTURE_DNS carries a real generatedAt and a finite total", () => {
    expect(isIsoDate(FIXTURE_DNS.generatedAt)).toBe(true);
    expect(isTimestampNotFuture()(FIXTURE_DNS.generatedAt, new Date("2026-10-25T17:04:00Z"))).toBe(true);
    expect(isNonNegative(FIXTURE_DNS.totalAnnualListUsd)).toBe(true);
    expect(isPercentOf(100)(42.5)).toBe(true);
  });
});

describe("isFleetFeed proof consumer — strict generatedAt (#2577)", () => {
  const feed = (generatedAt: unknown) => ({
    version: 1,
    generatedAt,
    quota: { pools: [] },
    attention: [],
  });
  test("accepts a real timestamp", () => {
    expect(isFleetFeed(feed("2026-10-25T17:00:00Z"))).toBe(true);
  });
  test("rejects the impossible dates Date.parse used to roll over", () => {
    expect(isFleetFeed(feed("2026-02-30T00:00:00Z"))).toBe(false);
    expect(isFleetFeed(feed("2026-13-01T00:00:00Z"))).toBe(false);
  });
});
