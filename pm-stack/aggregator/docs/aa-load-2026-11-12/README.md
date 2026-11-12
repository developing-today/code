# AA web pull 2026-11-12 — ainventory load record

Drew's order (2026-11-12): upload all new Artificial Analysis results and
incorporate them into the model review page (PM stack `/aa`, data
`/api/aa.json`). The dataset is the compiled pull in
`~/workspace/model-review-2026-11-12/` (`aa-results.md` / `aa-results.json`,
30 model×effort rows retrieved from artificialanalysis.ai 2026-11-12
~20:09–20:11 UTC; Intelligence Index v5.0.1).

## Mechanism (the store's own paths — no rendered output touched)

The AA section is collected from the Supabase `ainventory` schema by
`scripts/collect-fleet-feed.ts` (payload SQL in `src/aa-batch-sql.ts`). The
load wrote to the CURRENT workbook batch
`fe02783b-046c-4401-bb35-8c529b216b8d` (the page's selected batch), following
the 2026-11-02 `aa-web` precedent (Supabase-only, Drive workbook untouched):

- `sources`: one new row, kind `aa-web` — "Artificial Analysis Intelligence
  Index v5 — web pull 2026-11-12" — with the full load note.
- `sheet_rows` (source_id = the new source row):
  - Rankings: 15 rows refreshed IN PLACE (Sonnet 5.5 ×5, Opus 5.5 ×5,
    Gemini 3.8 Flash ×3, Argon high, Carbon high — one-decimal AA values;
    Argon high cost $7.68→$9.95, time 1362s→1812s) and 15 rows APPENDED
    (287–301): Gemini 4 Hydrogen ×4 (new model), Carbon low/medium,
    5 estimated rows, Gemini 2.5 family ×4 on previous versions.
  - Test_Runs: 25 rows appended (185–209) — measured + previous-version
    rows only; v5 rows gain TTFT/timing the 2026-10-31 load lacked.
  - Benchmark_Versions: `AA|v4.1` row appended (factor 1, own scale, no
    bridge — scores never compared across index versions).
  - Maintenance_Log: row 49 appended with the full load account.
- `measurement_events` is APPEND-ONLY (#1248 trigger): the refresh is a
  superseding emission under this batch, never an update.
  - An `ingest_batches` registry row for this batch was created first —
    the events'/prices' FK home (the 2026-10-31 import predates the
    registry; the aggregator's batch index treats it as optional).
  - 188 of the 199 workbook-import events of batch `8152e746` re-emitted
    verbatim under this batch; the 11 superseded current configs excluded
    there and re-emitted corrected; 10 new current configs added
    (Hydrogen ×4, Carbon low/medium/high, Argon high, Sonnet low,
    3.8 Flash low); 25 versioned events added (21 × `5.0.1`, 2 × `4.1`,
    2 × `4.3.2`, Rankings-derived shape). Total under this batch: 234.
  - Estimates carry NO workbook-import events (not measurements).
  - sheet-backfill stratum (code#4401 mapping): 26 events appended under a
    follow-on `ingest_batches` record — 15 superseding tranche-1 events
    for the refreshed run_keys, 11 for the new AA|v5 Rankings rows.
- `price_events`: 9 appended (AA list prices for the dataset models);
  `v_current_prices` now resolves 13 models.
- `models`: 5 slug rows created (gemini-4-hydrogen, gemini-2.5-pro,
  gemini-2.5-flash, gemini-2.5-flash-thinking, gemini-2.5-flash-lite).

## How ii_source survives

- `AA-v5` → Rankings `benchmark_version_key = 'AA|v5'` (the page's
  benchmark filter/column); events carry `benchmark_version '5.0.1'`
  (versioned stratum).
- `AA-prev` → keys `AA|v4.1` / `AA|v4.3.2`, ineligible under the current
  Controls (benchmark version AA|v5), rendered under their own version.
- `GUESS` → the workbook's own estimate flag: `|estimated` suffix on the
  `test_run_key` (the same flag the 42 pre-existing estimated rows carry),
  documented in the sources note + Maintenance_Log; no measurement events.

## Verification (2026-11-12)

- Collector payload SQL (`aaPayloadSql(null)`) against the store:
  rankings 298, configs 45 (no duplicates), cai 27, thresholds 1144,
  prices 13 — Hydrogen rows II 27/41/51/54 at $0.29/$0.80/$1.87/$3.16.
- Live `GET /api/aa.json` after the next collector run (log line: "aa 298
  rankings / 45 configs / 208 test runs"): Hydrogen present in rankings,
  Test_Runs (TTFT 5.81/12.33/14.96/41.11s), and the Model Explorer
  catalog — `_antigravity/gemini-4-hydrogen` aaScore 54, rate $1.50/$7.50.
- Unit tests: `bun test test/aa.test.ts test/aa-batches.test.ts` — 23
  pass; the one failure is the HTTP suite's server-boot `beforeAll`
  timing out at bun's 5s hook limit under load average ~35–46
  (fixtures-only, pre-existing environmental flake; no code changed).

## Rollback

Pre-load snapshot of every touched Rankings row + store counts:
`~/workspace/model-review-2026-11-12/store-backup-before.json`. Events and
prices are append-only: rollback = delete this load's emissions by
`ingest_batch_id` (this batch + the follow-on backfill batch) /
`source_id` (the aa-web source row) and restore the 15 Rankings rows from
the snapshot; the batch then resolves to the 8152e746 event set again.

## Files

- `aa-load-2026-11-12.sql` — the exact transaction that ran (via the
  Supabase Management API SQL endpoint, token by Bitwarden name).
- `gen_load.py` — the generator that produced it from `aa-results.json`
  (as-run; paths point at the sandbox dataset).
