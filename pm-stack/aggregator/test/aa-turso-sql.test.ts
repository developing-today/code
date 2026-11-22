import { test, expect } from "bun:test";
import { Database } from "bun:sqlite";
import {
  aaPayloadSql,
  AA_BATCH_INDEX_SQL,
  AA_MEASUREMENTS_SQL,
  AA_PRICES_SQL,
  assertAaNumericInputs,
  aaRankingValuesSql,
  rankAaValues,
  aaSectionQueries,
} from "../src/aa-turso-sql.ts";
import { storeStatement } from "../src/turso-stores.ts";
// Oracle: explicit workbook arithmetic and as-of selection, executed by SQLite.
test("Turso AA preserves workbook metrics, boolean JSON and historical batches", () => {
  const db = new Database(":memory:");
  db.exec(`create table ainventory_sources(id integer,name text,kind text,import_batch text,imported_at text,drive_modified_at text);
 create table ainventory_ingest_batches(id text,ran_at text,inserted_counts text);
 create table ainventory_sheet_rows(import_batch text,sheet_name text,row_index integer,data text);
 create table ainventory_measurement_events(id integer,ingest_batch_id text,source_kind text,ingested_at text,source_date text,model_variant_key text,model_name text,benchmark text,effort text,value_num real,cost_per_task_usd real,output_tps real,metric text,benchmark_version text,raw text);
 create table ainventory_price_events(id integer,model_id text,model_name text,model_variant_key text,provider text,input_usd_per_1m real,output_usd_per_1m real,cached_input_usd_per_1m real,cache_write_usd_per_1m real,context_min_tokens integer,context_max_tokens integer,window_kind text,effective_date text,source_date text,ingested_at text);`);
  const old = "11111111-1111-1111-1111-111111111111",
    latest = "22222222-2222-2222-2222-222222222222";
  for (const [i, b] of [old, latest].entries()) {
    db.query("insert into ainventory_sources values(?,?,?,?,?,?)").run(
      i,
      "book",
      "drive-workbook",
      b,
      `2026-11-${10 + i}T00:00:00Z`,
      null,
    );
    const row = (sheet: string, n: number, cells: unknown[]) =>
      db.query("insert into ainventory_sheet_rows values(?,?,?,?)").run(b, sheet, n, JSON.stringify({ cells }));
    for (const [n, v] of [
      [7, "40"],
      [8, ".7"],
      [9, ".3"],
      [10, "Contender"],
      [11, "Current"],
      [12, "v1"],
    ] as const)
      row("Controls", n, ["", "", v]);
    row("Benchmark_Versions", 2, ["v1", "", "", "", "", "", "", "", "1"]);
    row("Rankings", 4, [
      "test",
      "model",
      "family",
      "creator",
      "high",
      "v1",
      String(50 + i),
      "",
      "",
      "2",
      "60",
      "100",
      "Contender",
      "yes",
    ]);
    row("Run_Metrics", 2, ["", "", "", "", "", "50", "2", "", "1"]);
    row("Threshold_Metrics", 2, ["test", "model", "v1", "2", "40"]);
  }
  db.query("insert into ainventory_ingest_batches values(?,?,?)").run(latest, "2026-11-11T00:00:00Z", '{"rows":3}');
  db.query("insert into ainventory_sheet_rows values(?,?,?,?)").run(old, "Test_Runs", 2, '{"cells":["test","model"]}');
  for (const [benchmark, metric, raw] of [
    ["AA Intelligence Index v1", "intelligence_index", "{}"],
    ["Artificial Analysis Coding Agent Index v1", "cai_index_score", '{"benchmark_version":"v1"}'],
  ]) {
    db.query("insert into ainventory_measurement_events values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)").run(
      benchmark.startsWith("AA Intelligence") ? 1 : 2,
      old,
      "workbook-import",
      "2026-11-10T01:00:00Z",
      "2026-11-09",
      "model",
      "model",
      benchmark,
      "high",
      50,
      2,
      100,
      metric,
      null,
      raw,
    );
  }
  db.query("insert into ainventory_measurement_events values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)").run(
    3, latest, "aa", "2026-11-12T01:00:00Z", "2026-11-12", "model", "model", "AA Intelligence Index",
    "high", 75, 1, 150, "intelligence_index", null, "{}",
  );
  db.query("insert into ainventory_measurement_events values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)").run(
    4, latest, "aa", "2026-11-12T01:00:00Z", "2026-11-12", "model", "model", "Artificial Analysis Coding Agent Index",
    "high", 75, 1, 150, "cai_index_score", "v2", '{"benchmark_version":"v2"}',
  );
  db.exec(
    "insert into ainventory_price_events(id,model_name,provider,input_usd_per_1m,effective_date) values(1,'model','p',1,'2026-01-01'),(2,'model','p',2,'2026-11-01');",
  );
  const run = (sql: string) => {
    const stmt = storeStatement(sql);
    const r = db.query(stmt.sql).get(...stmt.args.map((a) => a.value)) as any;
    return JSON.parse(Object.values(r)[0] as string);
  };
  const a = run(aaPayloadSql(old)),
    b = run(aaPayloadSql(null)),
    current = run(aaPayloadSql(latest, true));
  for (const key of ["sources", "rankings", "thresholds", "configs", "testRunRows", "cai", "prices"])
    expect(Array.isArray(a[key])).toBe(true);
  expect(a.configs).toHaveLength(1);
  expect(a.testRunRows).toHaveLength(1);
  expect(a.cai).toHaveLength(1);
  expect(a.prices).toHaveLength(1);
  expect(current.configs[0].intelligence).toBe(75);
  expect(current.cai[0].score).toBe(75);
  expect(b.configs[0].intelligence).toBe(50);
  expect(b.cai[0].score).toBe(50);
  const measurementStmt = storeStatement(AA_MEASUREMENTS_SQL);
  const measurementRow = db.query(measurementStmt.sql).get(...measurementStmt.args.map((a) => a.value)) as {
    configs: string;
    cai: string;
  };
  expect(JSON.parse(measurementRow.configs)[0].intelligence).toBe(75);
  expect(JSON.parse(measurementRow.cai)[0].score).toBe(75);
  expect(rankAaValues(db.query(aaRankingValuesSql(old)).all() as any[])).toEqual(a.rankings);
  expect(Object.keys(aaSectionQueries(old))).toHaveLength(8);
  expect(Object.keys(aaSectionQueries(latest, true))).toHaveLength(8);
  expect(a.batch).toBe(old);
  expect(b.batch).toBe(latest);
  expect(a.rankings[0].rawScore).toBe(50);
  expect(b.rankings[0].rawScore).toBe(51);
  expect(a.rankings[0].eligible).toBe(true);
  expect(a.rankings[0].intelEffPerUsd).toBe(5.5);
  expect(a.rankings[0].pointsPerDollar).toBe(25);
  expect(a.thresholds[0].usable).toBe(true);
  expect(a.thresholds[0].usdPerUsablePoint).toBeCloseTo(2 / 11);
  expect(run(AA_BATCH_INDEX_SQL)[0].insertedCounts).toEqual({ rows: 3 });
  expect(run(AA_PRICES_SQL)[0].inputPer1M).toBe(2);
  db.close();
});

test("malformed workbook numbers fail closed instead of SQLite casting to zero", () => {
  expect(() => assertAaNumericInputs([{ cell: "oops" }])).toThrow("malformed");
  expect(() => assertAaNumericInputs([{ cell: "Infinity" }])).toThrow("malformed");
  expect(() => assertAaNumericInputs([{ cell: "2cats" }])).toThrow("malformed");
  expect(() => assertAaNumericInputs([{ cell: "" }, { cell: null }, { cell: "2.5e-3" }, { cell: 42 }])).not.toThrow();
});

test("ranking rejects ambiguous row ids and nonfinite sorting values", () => {
  const row = {
    row_index: 4,
    eligible: 1,
    balanced_value: 1,
    smart_value: 1,
    speed_value: 1,
    adjusted_score: 50,
    cost_task: 2,
  };
  expect(() => rankAaValues([row, { ...row }])).toThrow("duplicate");
  expect(() => rankAaValues([{ ...row, row_index: 4.5 }])).toThrow("row_index");
  expect(() => rankAaValues([{ ...row, cost_task: Infinity }])).toThrow("numeric");
});
