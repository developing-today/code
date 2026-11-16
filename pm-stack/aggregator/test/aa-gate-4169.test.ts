import { describe, expect, test } from "bun:test";
import { isAaSection } from "../src/aa.ts";

// #4169: isAaSection's "ok" branch checked configs only as an array,
// testRuns entries only for snapshotDate, and never mentioned the
// optional coverage/catalog/cai fields — while renderAa dereferences
// configs entries (modelName), testRuns entries (modelVariantKey via
// variantLabel), the coverage name lists, catalog entries, and cai rows.
// Gate-passing malformed sections threw on /aa instead of degrading to
// the not-collected banner. The gate now checks each field at the depth
// its consumer dereferences it; absent optional fields stay valid.

function completeSection(): Record<string, unknown> {
  return {
    status: "ok",
    generatedAt: "2026-11-01T00:00:00Z",
    batch: "batch-1",
    sources: [{ name: "ainventory.xlsx", kind: "sheet", importedAt: null, driveModifiedAt: null }],
    rankings: [{ testRunKey: "run-1", family: "GPT" }],
    thresholds: [{ testRunKey: "run-1", target: 42, usable: true }],
    configs: [{ modelName: "GPT-6 Sol", benchmark: "AA" }],
    testRuns: [{ testRunKey: "run-1", modelVariantKey: "openai|gpt-6-sol|max", snapshotDate: "2026-10-01" }],
    coverage: {
      measuredNotPooled: ["GPT-6 Sol"],
      catalogNotPooledMeasured: ["GPT-6 Sol"],
      measuredNotInCatalog: [],
      catalogNotPooledUnmeasured: [],
    },
    catalog: [{ id: "openai/gpt-6-sol", provider: "openai", routes: ["pool-a"] }],
    cai: [{ modelName: "GPT-6.1 Sol (xhigh)", family: "GPT-6.1 Sol", creator: "OpenAI" }],
  };
}

describe("isAaSection gate depth (#4169)", () => {
  test("accepts a complete section carrying every optional field", () => {
    expect(isAaSection(completeSection())).toBe(true);
  });

  test("accepts a section with all optional fields absent", () => {
    const s = completeSection();
    delete s.coverage;
    delete s.catalog;
    delete s.cai;
    expect(isAaSection(s)).toBe(true);
  });

  test("rejects configs=[null] and a configs entry without a string modelName", () => {
    const nullEntry = completeSection();
    nullEntry.configs = [null];
    expect(isAaSection(nullEntry)).toBe(false);
    const noName = completeSection();
    noName.configs = [{ benchmark: "AA" }];
    expect(isAaSection(noName)).toBe(false);
  });

  test("rejects a testRuns entry without a string modelVariantKey", () => {
    const missing = completeSection();
    missing.testRuns = [{ testRunKey: "run-1", snapshotDate: "2026-10-01" }];
    expect(isAaSection(missing)).toBe(false);
    const wrongType = completeSection();
    wrongType.testRuns = [{ testRunKey: "run-1", modelVariantKey: 42, snapshotDate: "2026-10-01" }];
    expect(isAaSection(wrongType)).toBe(false);
  });

  test("rejects coverage={} and a coverage name list of the wrong type", () => {
    const empty = completeSection();
    empty.coverage = {};
    expect(isAaSection(empty)).toBe(false);
    const stringField = completeSection();
    stringField.coverage = {
      measuredNotPooled: [],
      catalogNotPooledMeasured: "GPT-6 Sol",
      measuredNotInCatalog: [],
      catalogNotPooledUnmeasured: [],
    };
    expect(isAaSection(stringField)).toBe(false);
    const nonStringMember = completeSection();
    nonStringMember.coverage = {
      measuredNotPooled: [null],
      catalogNotPooledMeasured: [],
      measuredNotInCatalog: [],
      catalogNotPooledUnmeasured: [],
    };
    expect(isAaSection(nonStringMember)).toBe(false);
  });

  test("rejects a non-array catalog and a catalog entry without provider/routes depth", () => {
    const wrongType = completeSection();
    wrongType.catalog = "openai/gpt-6-sol";
    expect(isAaSection(wrongType)).toBe(false);
    const nullEntry = completeSection();
    nullEntry.catalog = [null];
    expect(isAaSection(nullEntry)).toBe(false);
    const noProvider = completeSection();
    noProvider.catalog = [{ id: "openai/gpt-6-sol", routes: [] }];
    expect(isAaSection(noProvider)).toBe(false);
    const noRoutes = completeSection();
    noRoutes.catalog = [{ id: "openai/gpt-6-sol", provider: "openai" }];
    expect(isAaSection(noRoutes)).toBe(false);
  });

  test("rejects a non-array cai and a null cai row", () => {
    const wrongType = completeSection();
    wrongType.cai = "x";
    expect(isAaSection(wrongType)).toBe(false);
    const nullRow = completeSection();
    nullRow.cai = [null];
    expect(isAaSection(nullRow)).toBe(false);
  });
});
