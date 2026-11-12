// #3302 AA dataset version switching: batch index parsing, content-fingerprint
// dedupe, cache naming, parameterized SQL, and the /aa selector render.
import { describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AA_BATCH_INDEX_SQL, aaPayloadSql, batchExpression, isBatchUuid } from "../src/aa-batch-sql.ts";
import {
  aaBatchFingerprint,
  aaCacheFileName,
  aaPricesFingerprint,
  batchLabel,
  canonicalJson,
  isAaBatchCacheEntry,
  parseBatchIndex,
  summarizeBatches,
} from "../src/aa-batches.ts";
import { renderAa } from "../src/render.ts";
import { loadConfig } from "../src/config.ts";
import { FIXTURE_AA } from "../src/sources/fixtures.ts";
import { liveSource } from "../src/sources/live.ts";
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

  test("configs/cai scope as-of the selected batch, not to the newest event batch (#8817)", () => {
    const sql = aaPayloadSql(B2);
    // The pre-#8817 predicate pinned both sections to the newest
    // workbook-import event batch regardless of the selected batch, so a
    // historical payload silently mixed the newest configs/CAI into data
    // cached under the historical batch's UUID.
    expect(sql).not.toContain("where source_kind = 'workbook-import' order by ingested_at desc limit 1");
    // Both sections now take the as-of mapping: an event batch is eligible
    // only when its workbook import is no newer than the selected batch.
    const asOf = "s.imported_at <= (select max(s2.imported_at) from ainventory.sources s2";
    expect(sql.split(asOf).length - 1).toBe(2);
    expect(sql).toContain("me.ingest_batch_id in (select s.import_batch from ainventory.sources s");
    expect(sql).toContain(`s2.import_batch = '${B2}'::uuid`);
  });

  test("the default (latest) payload keeps the as-of configs/cai scoping (#8817)", () => {
    const sql = aaPayloadSql(null);
    expect(sql).not.toContain("where source_kind = 'workbook-import' order by ingested_at desc limit 1");
    expect(sql.split("s.imported_at <=").length - 1).toBe(2);
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

  test("prices fingerprint is stable across key order and row order", () => {
    // Oracle (#3302 review): the overlay validity key must depend
    // only on the price values — the same overlay fetched twice
    // (raw rows) or read back from a built section (parsed rows)
    // hashes identically.
    const a = [
      {
        modelName: "A",
        inputPer1M: 1,
        outputPer1M: 2,
        cacheReadPer1M: null,
        cacheWritePer1M: null,
        contextMinTokens: 0,
        contextMaxTokens: 100,
        windowKind: "fixed",
      },
      {
        modelName: "B",
        inputPer1M: 3,
        outputPer1M: 4,
        cacheReadPer1M: 1,
        cacheWritePer1M: 2,
        contextMinTokens: null,
        contextMaxTokens: null,
        windowKind: null,
      },
    ];
    const b = [
      {
        windowKind: null,
        contextMaxTokens: null,
        contextMinTokens: null,
        cacheWritePer1M: 2,
        cacheReadPer1M: 1,
        outputPer1M: 4,
        inputPer1M: 3,
        modelName: "B",
      },
      {
        windowKind: "fixed",
        contextMaxTokens: 100,
        contextMinTokens: 0,
        cacheWritePer1M: null,
        cacheReadPer1M: null,
        outputPer1M: 2,
        inputPer1M: 1,
        modelName: "A",
      },
    ];
    expect(aaPricesFingerprint(a)).toBe(aaPricesFingerprint(b));
  });

  test("prices fingerprint changes when any price changes", () => {
    // Oracle (#3302 review): a price-only change — no new workbook,
    // no config change — produces a different validity key, so
    // cached sections rebuild instead of serving stale prices as
    // the current overlay.
    const base = [
      {
        modelName: "A",
        inputPer1M: 1,
        outputPer1M: 2,
        cacheReadPer1M: null,
        cacheWritePer1M: null,
        contextMinTokens: null,
        contextMaxTokens: null,
        windowKind: null,
      },
    ];
    const changed = [{ ...base[0], inputPer1M: 5 }];
    expect(aaPricesFingerprint(base)).not.toBe(aaPricesFingerprint(changed));
    expect(aaPricesFingerprint([])).not.toBe(aaPricesFingerprint(base));
    expect(aaPricesFingerprint(undefined)).toBe(aaPricesFingerprint([]));
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

  test("cache entries may carry a numeric sql revision (#8817)", () => {
    const base = { batch: B1, configFingerprint: "c", fingerprint: "f", builtAt: "t", section: {} };
    expect(isAaBatchCacheEntry(base)).toBe(true);
    expect(isAaBatchCacheEntry({ ...base, sqlRevision: 1 })).toBe(true);
    expect(isAaBatchCacheEntry({ ...base, sqlRevision: "1" })).toBe(false);
  });

  test("cache names are uuid-only and label is human-readable", () => {
    expect(aaCacheFileName(B1)).toBe(`aa-batch-${B1}.json`);
    expect(() => aaCacheFileName("../escape")).toThrow();
    expect(
      isAaBatchCacheEntry({ batch: B1, configFingerprint: "c", fingerprint: "f", builtAt: "t", section: {} }),
    ).toBe(true);
    expect(isAaBatchCacheEntry({ batch: "../x" })).toBe(false);
    expect(
      batchLabel({
        name: "ai_model_benchmark_inventory_advanced_2026-10-21",
        importedAt: "2026-10-31T15:00:00Z",
        driveModifiedAt: null,
      }),
    ).toStartWith("2026-10-31");
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

describe("live AA batch selection", () => {
  test("serves the selected cached payload while preserving latest as the default", async () => {
    const dir = await mkdtemp(join(tmpdir(), "aa-batch-source-"));
    const feedFile = join(dir, "fleet-feed.json");
    const historical = { ...FIXTURE_AA, batch: B3 };
    const latest = {
      ...FIXTURE_AA,
      batch: B1,
      batches: [
        {
          importBatch: B1,
          name: "latest.xlsx",
          importedAt: "2026-10-31T15:00:00Z",
          driveModifiedAt: null,
          ranAt: null,
          insertedCounts: null,
          label: "2026-10-31 · latest.xlsx",
          fingerprint: "latest",
          cached: true,
          latest: true,
        },
        {
          importBatch: B3,
          name: "historical.xlsx",
          importedAt: "2026-10-21T10:00:00Z",
          driveModifiedAt: null,
          ranAt: null,
          insertedCounts: null,
          label: "2026-10-21 · historical.xlsx",
          fingerprint: "historical",
          cached: true,
          latest: false,
        },
      ],
    };
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        aa: latest,
      }),
    );
    const cacheDir = join(dir, "aa-batches");
    await mkdir(cacheDir);
    await writeFile(
      join(cacheDir, aaCacheFileName(B3)),
      JSON.stringify({
        batch: B3,
        configFingerprint: "config",
        fingerprint: "historical",
        builtAt: historical.generatedAt,
        section: historical,
      }),
    );

    const source = liveSource(loadConfig({ PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile }));
    expect((await source.aa())?.batch).toBe(B1);
    const selected = await source.aa(B3);
    expect(selected?.status).toBe("ok");
    expect(selected?.batch).toBe(B3);
    expect(selected?.batches?.map((batch) => batch.importBatch)).toEqual([B1, B3]);
    expect((await source.aa("44444444-4444-4444-8444-444444444444"))?.status).toBe("absent");
  });

  test("a cache invalidated by a config change is served stale, not as ordinary success", async () => {
    const dir = await mkdtemp(join(tmpdir(), "aa-batch-stale-"));
    const feedFile = join(dir, "fleet-feed.json");
    const historical = { ...FIXTURE_AA, batch: B3 };
    const summary = (importBatch: string, latest: boolean, cached: boolean) => ({
      importBatch,
      name: "workbook.xlsx",
      importedAt: latest ? "2026-10-31T15:00:00Z" : "2026-10-21T10:00:00Z",
      driveModifiedAt: null,
      ranAt: null,
      insertedCounts: null,
      label: latest ? "2026-10-31 · workbook.xlsx" : "2026-10-21 · workbook.xlsx",
      fingerprint: latest ? "latest" : "historical",
      cached,
      latest,
    });
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        aa: {
          ...FIXTURE_AA,
          batch: B1,
          // The collector rebuilt the index under a new config fingerprint and
          // has not rebuilt B3's payload yet.
          batches: [summary(B1, true, true), summary(B3, false, false)],
        },
      }),
    );
    const cacheDir = join(dir, "aa-batches");
    await mkdir(cacheDir);
    await writeFile(
      join(cacheDir, aaCacheFileName(B3)),
      JSON.stringify({
        batch: B3,
        configFingerprint: "old-config",
        fingerprint: "historical",
        builtAt: historical.generatedAt,
        section: historical,
      }),
    );

    const source = liveSource(loadConfig({ PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile }));
    const selected = await source.aa(B3);
    expect(selected?.status).toBe("ok");
    expect(selected?.stale).toBe(true);
    expect(selected?.staleReason).toContain("rebuilding");
    const html = renderAa(selected ?? null, new Proxy({}, { get: () => "live" }) as Provenance);
    expect(html).toContain("The latest collection failed");
    expect(html).toContain("rebuilding");
  });

  test("a cached batch built under an old price overlay is served with the current overlay (#8211)", async () => {
    const dir = await mkdtemp(join(tmpdir(), "aa-batch-overlay-"));
    const feedFile = join(dir, "fleet-feed.json");
    const priceRow = (input: number, output: number) => ({
      modelName: "openai/gpt-6-luna",
      inputPer1M: input,
      outputPer1M: output,
      cacheReadPer1M: null,
      cacheWritePer1M: null,
      contextMinTokens: null,
      contextMaxTokens: null,
      windowKind: "flat",
    });
    const oldPrices = [priceRow(1, 4)];
    const newPrices = [priceRow(9.9, 39.6)];
    const summary = (importBatch: string, latest: boolean, cached: boolean) => ({
      importBatch,
      name: "workbook.xlsx",
      importedAt: latest ? "2026-10-31T15:00:00Z" : "2026-10-21T10:00:00Z",
      driveModifiedAt: null,
      ranAt: null,
      insertedCounts: null,
      label: latest ? "2026-10-31 · workbook.xlsx" : "2026-10-21 · workbook.xlsx",
      fingerprint: latest ? "latest" : "historical",
      cached,
      latest,
    });
    const historical = {
      ...FIXTURE_AA,
      batch: B3,
      prices: oldPrices,
      catalog: FIXTURE_AA.catalog?.map((e) =>
        e.id === "openai/gpt-6-luna"
          ? {
              ...e,
              rate: {
                bands: [
                  {
                    minTokens: null,
                    maxTokens: null,
                    input: 1,
                    output: 4,
                    cacheRead: null,
                    cacheWrite: null,
                    window: "flat",
                  },
                ],
                windowed: false,
              },
            }
          : e,
      ),
    };
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        aa: {
          ...FIXTURE_AA,
          batch: B1,
          prices: newPrices,
          batches: [summary(B1, true, true), summary(B3, false, true)],
        },
      }),
    );
    const cacheDir = join(dir, "aa-batches");
    await mkdir(cacheDir);
    await writeFile(
      join(cacheDir, aaCacheFileName(B3)),
      JSON.stringify({
        batch: B3,
        configFingerprint: "config",
        fingerprint: "historical",
        pricesFingerprint: aaPricesFingerprint(oldPrices),
        builtAt: historical.generatedAt,
        section: historical,
      }),
    );

    const source = liveSource(loadConfig({ PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile }));
    const selected = await source.aa(B3);
    if (selected?.status !== "ok") throw new Error("expected an ok section");
    expect(selected.stale).toBeUndefined();
    // The batch-bound payload is the cached one; the prices and the
    // price-derived catalog rates are the current overlay's.
    expect(selected.batch).toBe(B3);
    expect(selected.prices).toEqual(newPrices);
    const luna = selected.catalog?.find((e) => e.id === "openai/gpt-6-luna");
    expect(luna?.rate?.bands[0].input).toBe(9.9);
    expect(luna?.rate?.bands[0].output).toBe(39.6);
  });

  test("a cached batch with prices is served stale when the feed has no current overlay (#8211)", async () => {
    const dir = await mkdtemp(join(tmpdir(), "aa-batch-nooverlay-"));
    const feedFile = join(dir, "fleet-feed.json");
    const historical = {
      ...FIXTURE_AA,
      batch: B3,
      prices: [
        {
          modelName: "openai/gpt-6-luna",
          inputPer1M: 1,
          outputPer1M: 4,
          cacheReadPer1M: null,
          cacheWritePer1M: null,
          contextMinTokens: null,
          contextMaxTokens: null,
          windowKind: "flat",
        },
      ],
    };
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
      }),
    );
    const cacheDir = join(dir, "aa-batches");
    await mkdir(cacheDir);
    await writeFile(
      join(cacheDir, aaCacheFileName(B3)),
      JSON.stringify({
        batch: B3,
        configFingerprint: "config",
        fingerprint: "historical",
        pricesFingerprint: "some-old-overlay",
        builtAt: historical.generatedAt,
        section: historical,
      }),
    );

    const source = liveSource(loadConfig({ PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile }));
    const selected = await source.aa(B3);
    if (selected?.status !== "ok") throw new Error("expected an ok section");
    expect(selected.stale).toBe(true);
    expect(selected.staleReason).toContain("price overlay");
  });
});
