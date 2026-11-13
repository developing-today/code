import { describe, expect, test } from "bun:test";
import { isAaSection, testRunsFromSheetRows } from "../src/aa.ts";

// #5289: isAaSection gated a test run's snapshotDate as string-or-null
// only, while the same gate validates generatedAt with isIsoDate.
// summarizeAa publishes the snapshotDate strings verbatim as the
// batch's snapshot vintage, so an impossible or non-ISO date
// ("2026-02-30", "not-a-date") falsified the /aa provenance range
// instead of degrading the section at the gate. The gate now requires
// snapshotDate to be null or a real ISO calendar date (the date-only
// form real rows carry passes).

function completeSection(): Record<string, unknown> {
  return {
    status: "ok",
    generatedAt: "2026-11-01T00:00:00Z",
    batch: "batch-1",
    sources: [{ name: "ainventory.xlsx", kind: "sheet", importedAt: null, driveModifiedAt: null }],
    rankings: [{ testRunKey: "run-1", family: "GPT" }],
    thresholds: [{ testRunKey: "run-1", target: 42, usable: true }],
    configs: [],
    testRuns: [{ testRunKey: "run-1", modelVariantKey: "openai|gpt-6-luna|max", snapshotDate: "2026-10-01" }],
  };
}

function sectionWithSnapshotDate(snapshotDate: unknown): Record<string, unknown> {
  const s = completeSection();
  s.testRuns = [{ testRunKey: "run-1", modelVariantKey: "openai|gpt-6-luna|max", snapshotDate }];
  return s;
}

describe("isAaSection snapshotDate validity (#5289)", () => {
  test("rejects impossible and non-ISO snapshotDates", () => {
    for (const snapshotDate of ["2026-02-30", "2026-04-31", "not-a-date"]) {
      expect(isAaSection(sectionWithSnapshotDate(snapshotDate))).toBe(false);
    }
  });

  test("rejects a mixed section when any test run carries a bad snapshotDate", () => {
    const s = completeSection();
    s.testRuns = [
      { testRunKey: "run-1", modelVariantKey: "openai|gpt-6-luna|max", snapshotDate: "2026-09-28" },
      { testRunKey: "run-2", modelVariantKey: "openai|gpt-6-luna|max", snapshotDate: "2026-02-30" },
    ];
    expect(isAaSection(s)).toBe(false);
  });

  test("accepts a date-only snapshotDate and a null snapshotDate as controls", () => {
    expect(isAaSection(sectionWithSnapshotDate("2026-09-28"))).toBe(true);
    expect(isAaSection(sectionWithSnapshotDate(null))).toBe(true);
  });

  test("accepts collector-built test runs (testRunsFromSheetRows output)", () => {
    // aaSectionFromSupabase builds its testRuns with
    // testRunsFromSheetRows: a real sheet date stays a date-only
    // string and an empty snapshot_date cell becomes null.
    const rows = [
      { rowIndex: 1, cells: ["test_run_key", "snapshot_date", "model_variant_key", "ttft_seconds"] },
      { rowIndex: 2, cells: ["openai|luna|k", "2026-09-28", "openai|gpt-6-luna|max", "93.76"] },
      { rowIndex: 3, cells: ["deepseek|flash|k", "", "deepseek|v4.1-flash|max", ""] },
    ];
    const built = testRunsFromSheetRows(rows);
    expect(built.map((r) => r.snapshotDate)).toEqual([null, "2026-09-28"]); // sorted by testRunKey
    const s = completeSection();
    s.testRuns = built;
    expect(isAaSection(s)).toBe(true);
  });
});
