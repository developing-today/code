import { describe, expect, test } from "bun:test";
import { isAaSection } from "../src/aa.ts";

// #2415: isAaSection's "ok" branch never checked thresholds or sources
// and did not check testRuns entries — a feed section missing those
// arrays passed the gate and crashed summarizeAa / the /aa renderer
// instead of degrading to the not-collected banner. The gate now checks
// each field at the depth its consumer dereferences it; empty
// thresholds/sources arrays stay valid.

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

describe("isAaSection gate depth (#2415)", () => {
  test("accepts a complete minimal section", () => {
    expect(isAaSection(completeSection())).toBe(true);
  });

  test("accepts empty thresholds and sources arrays (render-fine control)", () => {
    const s = completeSection();
    s.thresholds = [];
    s.sources = [];
    expect(isAaSection(s)).toBe(true);
  });

  test("accepts a testRuns entry with a null snapshotDate", () => {
    const s = completeSection();
    s.testRuns = [{ testRunKey: "run-1", modelVariantKey: "openai|gpt-6-luna|max", snapshotDate: null }];
    expect(isAaSection(s)).toBe(true);
  });

  test("rejects missing or non-array thresholds", () => {
    const missing = completeSection();
    delete missing.thresholds;
    expect(isAaSection(missing)).toBe(false);
    const wrongType = completeSection();
    wrongType.thresholds = "none";
    expect(isAaSection(wrongType)).toBe(false);
  });

  test("rejects missing or non-array sources", () => {
    const missing = completeSection();
    delete missing.sources;
    expect(isAaSection(missing)).toBe(false);
    const wrongType = completeSection();
    wrongType.sources = 42;
    expect(isAaSection(wrongType)).toBe(false);
  });

  test("rejects a thresholds entry missing usable or target", () => {
    const noUsable = completeSection();
    noUsable.thresholds = [{ testRunKey: "run-1", target: 42 }];
    expect(isAaSection(noUsable)).toBe(false);
    const noTarget = completeSection();
    noTarget.thresholds = [{ testRunKey: "run-1", usable: true }];
    expect(isAaSection(noTarget)).toBe(false);
    const nanTarget = completeSection();
    nanTarget.thresholds = [{ testRunKey: "run-1", target: Number.NaN, usable: true }];
    expect(isAaSection(nanTarget)).toBe(false);
  });

  test("rejects a sources entry missing name or kind", () => {
    const noName = completeSection();
    noName.sources = [{ kind: "sheet" }];
    expect(isAaSection(noName)).toBe(false);
    const noKind = completeSection();
    noKind.sources = [{ name: "ainventory.xlsx" }];
    expect(isAaSection(noKind)).toBe(false);
  });

  test("rejects a null testRuns entry and an undefined snapshotDate", () => {
    const nullEntry = completeSection();
    nullEntry.testRuns = [null];
    expect(isAaSection(nullEntry)).toBe(false);
    const undefDate = completeSection();
    undefDate.testRuns = [{ testRunKey: "run-1", modelVariantKey: "openai|gpt-6-luna|max" }];
    expect(isAaSection(undefDate)).toBe(false);
  });

  test("rejects impossible generatedAt dates Date.parse rolls over (#3441)", () => {
    for (const generatedAt of ["2026-02-30T00:00:00Z", "2026-02-29T00:00:00Z", "2026-04-31T00:00:00Z"]) {
      const s = completeSection();
      s.generatedAt = generatedAt;
      expect(isAaSection(s)).toBe(false);
    }
  });
});
