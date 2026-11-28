// #3302 AA dataset version switching: batch index parsing, content-fingerprint
// dedupe, cache naming, parameterized SQL, and the /aa selector render.
import { describe, expect, test } from "bun:test";
import { AA_BATCH_INDEX_SQL, aaPayloadSql, batchExpression, isBatchUuid } from "../src/aa-batch-sql.ts";
import {
  aaBatchFingerprint,
  aaCacheFileName,
  batchLabel,
  canonicalJson,
  isAaBatchCacheEntry,
  parseBatchIndex,
  summarizeBatches,
} from "../src/aa-batches.ts";
import { renderAa } from "../src/render.ts";
import type { AaBatchSummary, AaSection, Provenance } from "../src/model.ts";

const B1 = "11111111-1111-4111-8111-111111111111";
const B2 = "22222222-2222-4222-8222-222222222222";
const B3 = "33333333-3333-4333-8333-333333333333";

function rawRow(batch: string, importedAt: string, extra: Record<string, unknown> = {}) {
  return {
    importBatch: batch,
    name: "ai_model_benchmark_inventory_advanced_2026-10-21",
    importedAt,
    driveModifiedAt: importedAt,
    ranAt: null,
    insertedCounts: null,
    ...extra,
  };
}

describe("batch index", () => {
  test("keeps workbook rows with no ingest_batches metadata", () => {
    const rows = parseBatchIndex([
      rawRow(B2, "2026-10-31T14:00:00Z"),
      rawRow(B1, "2026-10-31T15:00:00Z", { ranAt: "2026-10-31T14:58:00Z", insertedCounts: { sheet_rows: 4493 } }),
      { importBatch: "not-a-uuid", name: "x", importedAt: "2026-11-01T00:00:00Z" },
      null,
    ]);
    expect(rows.map((r) => r.importBatch)).toEqual([B1, B2]);
    expect(rows[0].insertedCounts).toEqual({ sheet_rows: 4493 });
    expect(rows[1].ranAt).toBeNull();
    expect(rows[1].insertedCounts).toBeNull();
  });

  test("index SQL reads sources and only left-joins ingest_batches", () => {
    expect(AA_BATCH_INDEX_SQL).toContain("from ainventory.sources s");
    expect(AA_BATCH_INDEX_SQL).toContain("left join ainventory.ingest_batches");
    expect(AA_BATCH_INDEX_SQL).toContain("kind = 'drive-workbook'");
  });
});

describe("parameterized payload SQL", () => {
  test("null keeps the latest subselect; a uuid becomes a validated literal", () => {
    expect(batchExpression(null)).toContain("v_latest_import_batch");
    expect(batchExpression(B1)).toBe(`'${B1}'::uuid`);
    expect(() => batchExpression("'; drop table x; --")).toThrow();
    expect(isBatchUuid(B1)).toBe(true);
    expect(isBatchUuid("nope")).toBe(false);
  });

  test("a historical query never touches the latest-only views", () => {
    const sql = aaPayloadSql(B2);
    expect(sql).toContain(`'${B2}'::uuid`);
    expect(sql).not.toContain("v_latest_workbook_batch");
    expect(sql).not.toContain("v_latest_import_batch");
    // The ranking/threshold maths is copied verbatim from the views.
    expect(sql).toContain("rank_balanced");
    expect(sql).toContain("balanced_50_50");
    expect(sql).toContain("power(intel_eff_norm, 0.7)");
  });

  test("the default query still pins v_latest_import_batch", () => {
    expect(aaPayloadSql(null)).toContain("(select import_batch from ainventory.v_latest_import_batch)");
  });
});

describe("semantic fingerprint", () => {
  const base = {
    rankings: [{ rowIndex: 1, family: "A", rawScore: "10" }],
    thresholds: [{ testRunKey: "k", target: "20" }],
    configs: [{ modelName: "A", intelligence: 10 }],
    testRunRows: [{ rowIndex: 1, cells: ["test_run_key"] }],
    cai: [],
    sources: [{ name: "workbook.xlsx", kind: "drive-workbook", importedAt: "2026-10-01T00:00:00Z" }],
  };

  test("identical content with different source timestamps collapses", () => {
    const other = {
      ...base,
      sources: [{ name: "workbook.xlsx", kind: "drive-workbook", importedAt: "2026-10-31T00:00:00Z" }],
    };
    expect(aaBatchFingerprint(base)).toBe(aaBatchFingerprint(other));
  });

  test("changed batch-bound content changes the fingerprint", () => {
    const changed = { ...base, rankings: [{ rowIndex: 1, family: "A", rawScore: "11" }] };
    expect(aaBatchFingerprint(base)).not.toBe(aaBatchFingerprint(changed));
  });

  test("the current-price overlay is not part of the fingerprint", () => {
    const withPrices = { ...base, prices: [{ modelName: "A", inputPer1M: 1 }] } as typeof base;
    expect(aaBatchFingerprint(withPrices)).toBe(aaBatchFingerprint(base));
  });

  test("canonical JSON sorts keys at every level", () => {
    expect(canonicalJson({ b: 1, a: { d: 2, c: 3 } })).toBe('{"a":{"c":3,"d":2},"b":1}');
  });
});

describe("batch summaries", () => {
  test("collapses equal fingerprints, keeps distinct ones, marks latest", () => {
    const rows = parseBatchIndex([
      rawRow(B1, "2026-10-31T15:00:00Z"),
      rawRow(B2, "2026-10-31T14:00:00Z"),
      rawRow(B3, "2026-10-31T13:00:00Z"),
    ]);
    const cache = new Map([
      [B1, { fingerprint: "same", cached: true }],
      [B2, { fingerprint: "same", cached: true }],
      [B3, { fingerprint: "different", cached: false }],
    ]);
    const out = summarizeBatches(rows, cache);
    expect(out.map((b) => b.importBatch)).toEqual([B1, B3]);
    expect(out[0].latest).toBe(true);
    expect(out[1].latest).toBe(false);
    expect(out[1].cached).toBe(false);
  });

  test("uncached batches are never collapsed on unproven identity", () => {
    const rows = parseBatchIndex([rawRow(B1, "2026-10-31T15:00:00Z"), rawRow(B2, "2026-10-31T14:00:00Z")]);
    const out = summarizeBatches(rows, new Map());
    expect(out).toHaveLength(2);
    expect(out.every((b) => b.fingerprint === null)).toBe(true);
  });

  test("cache names are uuid-only and label is human-readable", () => {
    expect(aaCacheFileName(B1)).toBe(`aa-batch-${B1}.json`);
    expect(() => aaCacheFileName("../escape")).toThrow();
    expect(isAaBatchCacheEntry({ batch: B1, configFingerprint: "c", fingerprint: "f", builtAt: "t", section: {} })).toBe(true);
    expect(isAaBatchCacheEntry({ batch: "../x" })).toBe(false);
    expect(batchLabel({ name: "ai_model_benchmark_inventory_advanced_2026-10-21", importedAt: "2026-10-31T15:00:00Z", driveModifiedAt: null })).toStartWith("2026-10-31");
  });
});

describe("/aa selector", () => {
  const PROV = new Proxy({}, { get: () => "fixture" }) as Provenance;
  const batches: AaBatchSummary[] = [
    {
      importBatch: B1,
      name: "workbook",
      importedAt: "2026-10-31T15:00:00Z",
      driveModifiedAt: null,
      ranAt: null,
      insertedCounts: null,
      label: "2026-10-31 · workbook (latest)",
      fingerprint: "a",
      cached: true,
      latest: true,
    },
    {
      importBatch: B3,
      name: "workbook",
      importedAt: "2026-10-21T10:00:00Z",
      driveModifiedAt: null,
      ranAt: null,
      insertedCounts: null,
      label: "2026-10-21 · workbook",
      fingerprint: "b",
      cached: true,
      latest: false,
    },
  ];
  const section: AaSection = {
    status: "ok",
    generatedAt: "2026-10-31T15:01:00Z",
    batch: B3,
    batches,
    sources: [],
    rankings: [],
    thresholds: [],
    configs: [],
    testRuns: [],
  };

  test("selected historical batch is marked and API/download links follow it", () => {
    const html = renderAa(section, PROV);
    expect(html).toContain('id="xf-batch"');
    expect(html).toContain(`value="${B3}" selected`);
    expect(html).toContain("(latest)");
    expect(html).toContain("historical version");
    expect(html).toContain(`/api/aa.json?batch=${B3}`);
    expect(html).toContain("current-price overlay");
  });

  test("latest default keeps the plain API link", () => {
    const html = renderAa({ ...section, batch: B1 }, PROV);
    expect(html).toContain(`value="${B1}" selected`);
    expect(html).toContain('href="/api/aa.json"');
    expect(html).not.toContain(`/api/aa.json?batch=${B1}`);
  });
});
