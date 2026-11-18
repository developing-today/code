// SQLite/Turso port of aa-batch-sql.ts. Window and metric formula semantics are preserved.
// Views are inlined so the migration depends only on restored source tables.
// Parameterized AA payload SQL for a selected ainventory import batch (#3302).
//
// The live ainventory views v_rankings and v_threshold_metrics each pin
// v_latest_workbook_batch in their opening `batch` CTE, so filtering around
// them still returns the latest batch. The two bodies below are verbatim
// copies of those view definitions (read read-only from pg_views on
// 2026-11-03; see the parity test in test/aa-batches.test.ts) with only that
// CTE replaced by the selected batch. The maths is untouched.
//
// A null batch keeps the default latest behaviour by substituting the same
// `v_latest_import_batch` subselect the collector used before this module.
// A uuid is emitted as a validated SQL literal, never as user text.

const BATCH_UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function isBatchUuid(value: unknown): value is string {
  return typeof value === "string" && BATCH_UUID_RE.test(value);
}

/** The batch expression the payload is scoped to: latest when null. */
export function batchExpression(batch: string | null): string {
  if (batch === null) return "(select import_batch from (select import_batch from ainventory_sources where kind = 'drive-workbook' order by imported_at desc limit 1))";
  if (!isBatchUuid(batch)) throw new Error("aa payload: invalid batch uuid");
  return `'${batch}'`;
}

// --- verbatim ainventory view bodies, batch CTE parameterized ---------------

const RANKINGS_BODY = `
WITH batch AS (SELECT {{BATCH}} AS import_batch), ctrl AS (
         SELECT max(
                CASE
                    WHEN (sr.row_index = 7) THEN CAST((NULLIF(((sr.data -> 'cells') ->> 2), '')) AS REAL)
                    ELSE CAST(NULL AS REAL)
                END) AS target,
            max(
                CASE
                    WHEN (sr.row_index = 8) THEN CAST((NULLIF(((sr.data -> 'cells') ->> 2), '')) AS REAL)
                    ELSE CAST(NULL AS REAL)
                END) AS intel_weight,
            max(
                CASE
                    WHEN (sr.row_index = 9) THEN CAST((NULLIF(((sr.data -> 'cells') ->> 2), '')) AS REAL)
                    ELSE CAST(NULL AS REAL)
                END) AS speed_weight,
            max(
                CASE
                    WHEN (sr.row_index = 10) THEN ((sr.data -> 'cells') ->> 2)
                    ELSE NULL
                END) AS status_filter,
            max(
                CASE
                    WHEN (sr.row_index = 11) THEN ((sr.data -> 'cells') ->> 2)
                    ELSE NULL
                END) AS availability_filter,
            max(
                CASE
                    WHEN (sr.row_index = 12) THEN ((sr.data -> 'cells') ->> 2)
                    ELSE NULL
                END) AS benchmark_version
           FROM ainventory_sheet_rows sr,
            batch b
          WHERE ((sr.sheet_name = 'Controls') AND (sr.import_batch = b.import_batch))
        ), bv AS (
         SELECT ((sr.data -> 'cells') ->> 0) AS benchmark_version_key,
            CAST((NULLIF(((sr.data -> 'cells') ->> 8), '')) AS REAL) AS cumulative_factor
           FROM ainventory_sheet_rows sr,
            batch b
          WHERE ((sr.sheet_name = 'Benchmark_Versions') AND (sr.import_batch = b.import_batch) AND (sr.row_index >= 2) AND (COALESCE(((sr.data -> 'cells') ->> 0), '') <> ''))
        ), rk_raw AS (
         SELECT sr.row_index,
            ((sr.data -> 'cells') ->> 0) AS test_run_key,
            ((sr.data -> 'cells') ->> 1) AS model_variant_key,
            ((sr.data -> 'cells') ->> 2) AS family,
            ((sr.data -> 'cells') ->> 3) AS creator,
            ((sr.data -> 'cells') ->> 4) AS effort,
            ((sr.data -> 'cells') ->> 5) AS benchmark_version_key,
            CAST((NULLIF(((sr.data -> 'cells') ->> 6), '')) AS REAL) AS raw_score,
            CAST((NULLIF(((sr.data -> 'cells') ->> 9), '')) AS REAL) AS cost_task,
            CAST((NULLIF(((sr.data -> 'cells') ->> 10), '')) AS REAL) AS time_task_sec,
            CAST((NULLIF(((sr.data -> 'cells') ->> 11), '')) AS REAL) AS speed_tps,
            ((sr.data -> 'cells') ->> 12) AS qual_status,
            ((sr.data -> 'cells') ->> 13) AS availability,
            c.target,
            c.intel_weight,
            c.speed_weight,
            c.status_filter,
            c.availability_filter,
            c.benchmark_version
           FROM ainventory_sheet_rows sr,
            (batch b
             CROSS JOIN ctrl c)
          WHERE ((sr.sheet_name = 'Rankings') AND (sr.import_batch = b.import_batch) AND (sr.row_index >= 4) AND (COALESCE(((sr.data -> 'cells') ->> 0), '') <> ''))
        ), base AS (
         SELECT r.row_index,
            r.test_run_key,
            r.model_variant_key,
            r.family,
            r.creator,
            r.effort,
            r.benchmark_version_key,
            r.raw_score,
            r.cost_task,
            r.time_task_sec,
            r.speed_tps,
            r.qual_status,
            r.availability,
            r.target,
            r.intel_weight,
            r.speed_weight,
            r.status_filter,
            r.availability_filter,
            r.benchmark_version,
            COALESCE(bv.cumulative_factor, CAST((1) AS REAL)) AS version_level_factor
           FROM (rk_raw r
             LEFT JOIN bv ON ((bv.benchmark_version_key = r.benchmark_version_key)))
        ), calc AS (
         SELECT base.row_index,
            base.test_run_key,
            base.model_variant_key,
            base.family,
            base.creator,
            base.effort,
            base.benchmark_version_key,
            base.raw_score,
            base.cost_task,
            base.time_task_sec,
            base.speed_tps,
            base.qual_status,
            base.availability,
            base.target,
            base.intel_weight,
            base.speed_weight,
            base.status_filter,
            base.availability_filter,
            base.benchmark_version,
            base.version_level_factor,
                CASE
                    WHEN (base.raw_score IS NULL) THEN CAST(NULL AS REAL)
                    ELSE (base.raw_score * base.version_level_factor)
                END AS adjusted_score,
                CASE
                    WHEN (COALESCE(base.benchmark_version_key, '') <> COALESCE(base.benchmark_version, '')) THEN 0
                    WHEN ((base.status_filter = 'Contender') AND (COALESCE(base.qual_status, '') <> 'Contender')) THEN 0
                    WHEN ((base.status_filter = 'Contender+Close') AND (COALESCE(base.qual_status, '') NOT IN ('Contender', 'Close'))) THEN 0
                    WHEN ((base.availability_filter = 'Current') AND (COALESCE(base.availability, '') NOT IN ('yes', 'limited'))) THEN 0
                    ELSE 1
                END AS eligible
           FROM base
        ), calc2 AS (
         SELECT calc.row_index,
            calc.test_run_key,
            calc.model_variant_key,
            calc.family,
            calc.creator,
            calc.effort,
            calc.benchmark_version_key,
            calc.raw_score,
            calc.cost_task,
            calc.time_task_sec,
            calc.speed_tps,
            calc.qual_status,
            calc.availability,
            calc.target,
            calc.intel_weight,
            calc.speed_weight,
            calc.status_filter,
            calc.availability_filter,
            calc.benchmark_version,
            calc.version_level_factor,
            calc.adjusted_score,
            calc.eligible,
                CASE
                    WHEN (calc.adjusted_score IS NULL) THEN CAST(NULL AS REAL)
                    ELSE (calc.adjusted_score - (calc.target - CAST((1) AS REAL)))
                END AS usable_units,
            calc.target AS target_value
           FROM calc
        ), calc3 AS (
         SELECT calc2.row_index,
            calc2.test_run_key,
            calc2.model_variant_key,
            calc2.family,
            calc2.creator,
            calc2.effort,
            calc2.benchmark_version_key,
            calc2.raw_score,
            calc2.cost_task,
            calc2.time_task_sec,
            calc2.speed_tps,
            calc2.qual_status,
            calc2.availability,
            calc2.target,
            calc2.intel_weight,
            calc2.speed_weight,
            calc2.status_filter,
            calc2.availability_filter,
            calc2.benchmark_version,
            calc2.version_level_factor,
            calc2.adjusted_score,
            calc2.eligible,
            calc2.usable_units,
            calc2.target_value,
                CASE
                    WHEN ((calc2.eligible = 0) OR (calc2.usable_units IS NULL) OR (calc2.usable_units <= CAST((0) AS REAL)) OR (calc2.cost_task IS NULL) OR (calc2.cost_task = CAST((0) AS REAL))) THEN CAST(NULL AS REAL)
                    ELSE (calc2.usable_units / calc2.cost_task)
                END AS intel_eff_per_usd,
                CASE
                    WHEN ((calc2.eligible = 0) OR (calc2.usable_units IS NULL) OR (calc2.usable_units <= CAST((0) AS REAL)) OR (calc2.cost_task IS NULL)) THEN CAST(NULL AS REAL)
                    ELSE (calc2.cost_task / calc2.usable_units)
                END AS usd_per_usable_unit,
                CASE
                    WHEN ((calc2.eligible = 0) OR (calc2.usable_units IS NULL) OR (calc2.usable_units <= CAST((0) AS REAL)) OR (calc2.cost_task IS NULL) OR (calc2.cost_task = CAST((0) AS REAL)) OR (calc2.time_task_sec IS NULL) OR (calc2.time_task_sec = CAST((0) AS REAL))) THEN CAST(NULL AS REAL)
                    ELSE (calc2.usable_units / (calc2.cost_task * (calc2.time_task_sec / CAST((60) AS REAL))))
                END AS speed_eff_per_usd_minute
           FROM calc2
        ), normed AS (
         SELECT calc3.row_index,
            calc3.test_run_key,
            calc3.model_variant_key,
            calc3.family,
            calc3.creator,
            calc3.effort,
            calc3.benchmark_version_key,
            calc3.raw_score,
            calc3.cost_task,
            calc3.time_task_sec,
            calc3.speed_tps,
            calc3.qual_status,
            calc3.availability,
            calc3.target,
            calc3.intel_weight,
            calc3.speed_weight,
            calc3.status_filter,
            calc3.availability_filter,
            calc3.benchmark_version,
            calc3.version_level_factor,
            calc3.adjusted_score,
            calc3.eligible,
            calc3.usable_units,
            calc3.target_value,
            calc3.intel_eff_per_usd,
            calc3.usd_per_usable_unit,
            calc3.speed_eff_per_usd_minute,
                CASE
                    WHEN ((calc3.eligible = 0) OR (calc3.adjusted_score IS NULL)) THEN CAST(NULL AS REAL)
                    ELSE (calc3.adjusted_score / max(calc3.adjusted_score) FILTER (WHERE (calc3.eligible = 1)) OVER ())
                END AS abs_intel_norm,
                CASE
                    WHEN ((calc3.eligible = 0) OR (calc3.speed_tps IS NULL)) THEN CAST(NULL AS REAL)
                    ELSE (calc3.speed_tps / max(calc3.speed_tps) FILTER (WHERE (calc3.eligible = 1)) OVER ())
                END AS abs_speed_norm,
                CASE
                    WHEN ((calc3.eligible = 0) OR (calc3.cost_task IS NULL)) THEN CAST(NULL AS REAL)
                    ELSE (min(calc3.cost_task) FILTER (WHERE ((calc3.eligible = 1) AND (calc3.cost_task > CAST((0) AS REAL)))) OVER () / calc3.cost_task)
                END AS cost_norm,
                CASE
                    WHEN (calc3.intel_eff_per_usd IS NULL) THEN CAST(NULL AS REAL)
                    ELSE (calc3.intel_eff_per_usd / max(calc3.intel_eff_per_usd) FILTER (WHERE (calc3.eligible = 1)) OVER ())
                END AS intel_eff_norm,
                CASE
                    WHEN (calc3.speed_eff_per_usd_minute IS NULL) THEN CAST(NULL AS REAL)
                    ELSE (calc3.speed_eff_per_usd_minute / max(calc3.speed_eff_per_usd_minute) FILTER (WHERE (calc3.eligible = 1)) OVER ())
                END AS speed_eff_norm
           FROM calc3
        ), valued AS (
         SELECT normed.row_index,
            normed.test_run_key,
            normed.model_variant_key,
            normed.family,
            normed.creator,
            normed.effort,
            normed.benchmark_version_key,
            normed.raw_score,
            normed.cost_task,
            normed.time_task_sec,
            normed.speed_tps,
            normed.qual_status,
            normed.availability,
            normed.target,
            normed.intel_weight,
            normed.speed_weight,
            normed.status_filter,
            normed.availability_filter,
            normed.benchmark_version,
            normed.version_level_factor,
            normed.adjusted_score,
            normed.eligible,
            normed.usable_units,
            normed.target_value,
            normed.intel_eff_per_usd,
            normed.usd_per_usable_unit,
            normed.speed_eff_per_usd_minute,
            normed.abs_intel_norm,
            normed.abs_speed_norm,
            normed.cost_norm,
            normed.intel_eff_norm,
            normed.speed_eff_norm,
                CASE
                    WHEN ((normed.intel_eff_norm IS NULL) OR (normed.speed_eff_norm IS NULL)) THEN CAST(NULL AS REAL)
                    ELSE sqrt((normed.intel_eff_norm * normed.speed_eff_norm))
                END AS balanced_value,
                CASE
                    WHEN ((normed.intel_eff_norm IS NULL) OR (normed.speed_eff_norm IS NULL)) THEN CAST(NULL AS REAL)
                    ELSE (power(normed.intel_eff_norm, normed.intel_weight) * power(normed.speed_eff_norm, normed.speed_weight))
                END AS smart_value,
                CASE
                    WHEN ((normed.intel_eff_norm IS NULL) OR (normed.speed_eff_norm IS NULL)) THEN CAST(NULL AS REAL)
                    ELSE (power(normed.intel_eff_norm, normed.speed_weight) * power(normed.speed_eff_norm, normed.intel_weight))
                END AS speed_value,
                CASE
                    WHEN ((normed.abs_intel_norm IS NULL) OR (normed.abs_speed_norm IS NULL)) THEN CAST(NULL AS REAL)
                    ELSE sqrt((normed.abs_intel_norm * normed.abs_speed_norm))
                END AS speed_x_intel,
                CASE
                    WHEN ((normed.adjusted_score IS NULL) OR (normed.cost_task IS NULL) OR (normed.cost_task = CAST((0) AS REAL))) THEN CAST(NULL AS REAL)
                    ELSE (normed.adjusted_score / normed.cost_task)
                END AS points_per_dollar,
                CASE
                    WHEN ((normed.adjusted_score IS NULL) OR (normed.time_task_sec IS NULL) OR (normed.time_task_sec = CAST((0) AS REAL))) THEN CAST(NULL AS REAL)
                    ELSE (normed.adjusted_score / normed.time_task_sec)
                END AS points_per_second
           FROM normed
        )
 SELECT row_index,
    test_run_key,
    model_variant_key,
    family,
    creator,
    effort,
    benchmark_version_key,
    raw_score,
    version_level_factor,
    adjusted_score,
    cost_task,
    time_task_sec,
    speed_tps,
    qual_status,
    availability,
    eligible,
    target_value AS target,
    usable_units,
    intel_eff_per_usd,
    usd_per_usable_unit,
    speed_eff_per_usd_minute,
    abs_intel_norm,
    abs_speed_norm,
    cost_norm,
    intel_eff_norm,
    speed_eff_norm,
    balanced_value,
    smart_value,
    speed_value,
    speed_x_intel,
    points_per_dollar,
    points_per_second,
        CASE
            WHEN (balanced_value IS NULL) THEN NULL
            ELSE ((rank() OVER (ORDER BY balanced_value DESC NULLS LAST) + row_number() OVER (PARTITION BY balanced_value ORDER BY row_index)) - 1)
        END AS rank_balanced,
        CASE
            WHEN (smart_value IS NULL) THEN NULL
            ELSE ((rank() OVER (ORDER BY smart_value DESC NULLS LAST) + row_number() OVER (PARTITION BY smart_value ORDER BY row_index)) - 1)
        END AS rank_smart,
        CASE
            WHEN (speed_value IS NULL) THEN NULL
            ELSE ((rank() OVER (ORDER BY speed_value DESC NULLS LAST) + row_number() OVER (PARTITION BY speed_value ORDER BY row_index)) - 1)
        END AS rank_speed,
        CASE
            WHEN ((eligible = 0) OR (adjusted_score IS NULL)) THEN NULL
            ELSE ((rank() OVER (PARTITION BY eligible ORDER BY adjusted_score DESC NULLS LAST) + row_number() OVER (PARTITION BY eligible, adjusted_score ORDER BY row_index)) - 1)
        END AS rank_intelligence,
        CASE
            WHEN ((eligible = 0) OR (speed_tps IS NULL)) THEN NULL
            ELSE ((rank() OVER (PARTITION BY eligible ORDER BY speed_tps DESC NULLS LAST) + row_number() OVER (PARTITION BY eligible, speed_tps ORDER BY row_index)) - 1)
        END AS rank_abs_speed,
        CASE
            WHEN ((eligible = 0) OR (cost_task IS NULL) OR (cost_task = CAST((0) AS REAL))) THEN NULL
            ELSE ((rank() OVER (PARTITION BY ((eligible = 1) AND (cost_task > CAST((0) AS REAL))) ORDER BY cost_task) + row_number() OVER (PARTITION BY ((eligible = 1) AND (cost_task > CAST((0) AS REAL))), cost_task ORDER BY row_index)) - 1)
        END AS rank_low_cost
   FROM valued`;

const THRESHOLD_BODY = `
WITH batch AS (SELECT {{BATCH}} AS import_batch), rm AS (
         SELECT sr.row_index,
            CAST((NULLIF(((sr.data -> 'cells') ->> 5), '')) AS REAL) AS levelled_score,
            CAST((NULLIF(((sr.data -> 'cells') ->> 6), '')) AS REAL) AS cost_task,
            CAST((NULLIF(((sr.data -> 'cells') ->> 8), '')) AS REAL) AS time_min
           FROM ainventory_sheet_rows sr,
            batch b
          WHERE ((sr.sheet_name = 'Run_Metrics') AND (sr.import_batch = b.import_batch) AND (sr.row_index >= 2))
        ), tm_raw AS (
         SELECT sr.row_index,
            ((sr.data -> 'cells') ->> 0) AS test_run_key,
            ((sr.data -> 'cells') ->> 1) AS model_variant_key,
            ((sr.data -> 'cells') ->> 2) AS benchmark_version_key,
            CAST((NULLIF(((sr.data -> 'cells') ->> 3), '')) AS INTEGER) AS run_metric_row,
            CAST((NULLIF(((sr.data -> 'cells') ->> 4), '')) AS REAL) AS target
           FROM ainventory_sheet_rows sr,
            batch b
          WHERE ((sr.sheet_name = 'Threshold_Metrics') AND (sr.import_batch = b.import_batch) AND (sr.row_index >= 2))
        ), base AS (
         SELECT t.row_index,
            t.test_run_key,
            t.model_variant_key,
            t.benchmark_version_key,
            t.run_metric_row,
            t.target,
            rm.levelled_score,
            rm.cost_task,
            rm.time_min
           FROM (tm_raw t
             LEFT JOIN rm ON ((rm.row_index = t.run_metric_row)))
        ), calc AS (
         SELECT base.row_index,
            base.test_run_key,
            base.model_variant_key,
            base.benchmark_version_key,
            base.run_metric_row,
            base.target,
            base.levelled_score,
            base.cost_task,
            base.time_min,
                CASE
                    WHEN (base.levelled_score IS NULL) THEN CAST(NULL AS REAL)
                    ELSE (base.levelled_score - (base.target - CAST((1) AS REAL)))
                END AS usable_surplus
           FROM base
        ), calc2 AS (
         SELECT calc.row_index,
            calc.test_run_key,
            calc.model_variant_key,
            calc.benchmark_version_key,
            calc.run_metric_row,
            calc.target,
            calc.levelled_score,
            calc.cost_task,
            calc.time_min,
            calc.usable_surplus,
                CASE
                    WHEN (calc.usable_surplus > CAST((0) AS REAL)) THEN 1
                    ELSE 0
                END AS usable,
                CASE
                    WHEN ((calc.usable_surplus > CAST((0) AS REAL)) AND (calc.cost_task IS NOT NULL) AND (calc.cost_task <> CAST((0) AS REAL))) THEN (calc.usable_surplus / calc.cost_task)
                    ELSE CAST(NULL AS REAL)
                END AS intel_eff_surplus_per_usd,
                CASE
                    WHEN ((calc.usable_surplus > CAST((0) AS REAL)) AND (calc.usable_surplus <> CAST((0) AS REAL)) AND (calc.cost_task IS NOT NULL)) THEN (calc.cost_task / calc.usable_surplus)
                    ELSE CAST(NULL AS REAL)
                END AS usd_per_usable_point,
                CASE
                    WHEN ((calc.usable_surplus > CAST((0) AS REAL)) AND (calc.cost_task IS NOT NULL) AND (calc.cost_task <> CAST((0) AS REAL)) AND (calc.time_min IS NOT NULL) AND (calc.time_min <> CAST((0) AS REAL))) THEN (calc.usable_surplus / (calc.cost_task * calc.time_min))
                    ELSE CAST(NULL AS REAL)
                END AS speed_eff_surplus_per_usd_minute
           FROM calc
        ), normed AS (
         SELECT calc2.row_index,
            calc2.test_run_key,
            calc2.model_variant_key,
            calc2.benchmark_version_key,
            calc2.run_metric_row,
            calc2.target,
            calc2.levelled_score,
            calc2.cost_task,
            calc2.time_min,
            calc2.usable_surplus,
            calc2.usable,
            calc2.intel_eff_surplus_per_usd,
            calc2.usd_per_usable_point,
            calc2.speed_eff_surplus_per_usd_minute,
                CASE
                    WHEN (calc2.intel_eff_surplus_per_usd IS NULL) THEN CAST(NULL AS REAL)
                    ELSE (calc2.intel_eff_surplus_per_usd / max(calc2.intel_eff_surplus_per_usd) OVER (PARTITION BY calc2.target))
                END AS intel_eff_norm,
                CASE
                    WHEN (calc2.speed_eff_surplus_per_usd_minute IS NULL) THEN CAST(NULL AS REAL)
                    ELSE (calc2.speed_eff_surplus_per_usd_minute / max(calc2.speed_eff_surplus_per_usd_minute) OVER (PARTITION BY calc2.target))
                END AS speed_eff_norm
           FROM calc2
        )
 SELECT row_index,
    test_run_key,
    model_variant_key,
    benchmark_version_key,
    run_metric_row,
    target,
    levelled_score,
    cost_task,
    time_min,
    usable_surplus,
    usable,
    intel_eff_surplus_per_usd,
    usd_per_usable_point,
    speed_eff_surplus_per_usd_minute,
    intel_eff_norm,
    speed_eff_norm,
        CASE
            WHEN ((intel_eff_norm IS NULL) OR (speed_eff_norm IS NULL)) THEN CAST(NULL AS REAL)
            ELSE sqrt((intel_eff_norm * speed_eff_norm))
        END AS balanced_50_50,
        CASE
            WHEN ((intel_eff_norm IS NULL) OR (speed_eff_norm IS NULL)) THEN CAST(NULL AS REAL)
            ELSE (power(intel_eff_norm, 0.7) * power(speed_eff_norm, 0.3))
        END AS smart_70_30,
        CASE
            WHEN ((intel_eff_norm IS NULL) OR (speed_eff_norm IS NULL)) THEN CAST(NULL AS REAL)
            ELSE (power(intel_eff_norm, 0.3) * power(speed_eff_norm, 0.7))
        END AS speed_30_70
   FROM normed`;

/**
 * The workbook batch index. `sources` is the complete register: every
 * workbook import row carries its batch id and timestamps. `ingest_batches`
 * is left-joined only for optional metadata — recent workbook rows have no
 * matching row and must not disappear from the list.
 */
export const AA_BATCH_INDEX_SQL = `
select coalesce(json_group_array(json_object(
  'importBatch', s.import_batch,
  'name', s.name,
  'importedAt', s.imported_at,
  'driveModifiedAt', s.drive_modified_at,
  'ranAt', ib.ran_at,
  'insertedCounts', json(ib.inserted_counts)
) order by s.imported_at desc nulls last), json('[]')) as batches
from ainventory_sources s
left join ainventory_ingest_batches ib on ib.id = s.import_batch
where s.kind = 'drive-workbook'
`;

/** The current-price overlay alone, in exactly the shape and order
 * aaPayloadSql embeds it as `prices` (#3302): the collector hashes
 * this every tick to validity-key the per-batch caches, because the
 * overlay is current data, not batch-bound data. */
export const AA_PRICES_SQL = `
select coalesce((
  select json_group_array(json_object(
    'modelName', model_name, 'inputPer1M', input_usd_per_1m, 'outputPer1M', output_usd_per_1m,
    'cacheReadPer1M', cached_input_usd_per_1m, 'cacheWritePer1M', cache_write_usd_per_1m,
    'contextMinTokens', context_min_tokens, 'contextMaxTokens', context_max_tokens,
    'windowKind', window_kind
  ) order by model_name, context_min_tokens nulls first)
  from (select * from (select *, row_number() over (partition by model_name, coalesce(provider, ''), context_min_tokens, context_max_tokens, window_kind order by effective_date desc nulls last, source_date desc nulls last, ingested_at desc, id desc) as price_rank from ainventory_price_events) where price_rank = 1)
), json('[]')) as prices
`;

/**
 * The full AA section payload for one batch (null = latest). This mirrors the
 * collector's previous AA_SQL exactly except that every batch predicate and
 * both latest-pinned views now take the same batch expression.
 *
 * Tranche 2 of code#4401 (2026-11-11): the 'configs' and 'cai' sections no
 * longer read the frozen ainventory_benchmark_results table; they read the
 * canonical ainventory_measurement_events store, with the filters that
 * reproduce the retired table's rows exactly (verified by full-row EXCEPT
 * comparison before the table was dropped: configs 35/35, cai 27/27, zero
 * differences either way). benchmark_results is dropped;
 * measurement_events is the one canonical read path for benchmark facts.
 *
 * code#8817: measurement events ARE keyed by workbook import —
 * ingest_batch_id is the importing batch's ainventory_sources.import_batch
 * (verified live 2026-11-12) — but not every workbook import produces
 * benchmark events (the imports after 2026-10-26 have none yet). The two
 * sections therefore scope AS-OF the selected batch: the newest
 * workbook-import event batch imported at or before it. For the latest
 * batch that is exactly the newest event batch (the previous behaviour);
 * for a historical batch it reconstructs that batch's own sections
 * instead of silently mixing the newest sections into a payload cached
 * under the historical UUID; a batch older than every event batch gets
 * empty sections rather than another batch's data.
 */
/** Revision of the aaPayloadSql semantics. The collector validity-keys
 * every cached batch payload with this number: entries built under older
 * SQL carry no revision (read as 0) and are rebuilt, never served as
 * current, once the payload semantics move. Bump when a change alters
 * what a cached payload means. (#8817: configs/cai scoping.) */
export const AA_PAYLOAD_SQL_REVISION = 2;

export function aaPayloadSql(batch: string | null): string {
  const b = batchExpression(batch);
  const rankings = RANKINGS_BODY.replace("{{BATCH}}", b);
  const thresholds = THRESHOLD_BODY.replace("{{BATCH}}", b);
  return `
select json_object(
  'batch', ${b},
  'sources', coalesce((
    select json_group_array(json_object(
      'name', name, 'kind', kind, 'importedAt', imported_at, 'driveModifiedAt', drive_modified_at
    ) order by id)
    from ainventory_sources
    where import_batch = ${b}
  ), json('[]')),
  'rankings', coalesce((
    select json_group_array(json_object(
      'rowIndex', row_index, 'testRunKey', test_run_key, 'modelVariantKey', model_variant_key,
      'family', family, 'creator', creator, 'effort', effort,
      'benchmarkVersionKey', benchmark_version_key, 'rawScore', raw_score,
      'adjustedScore', adjusted_score, 'costTask', cost_task, 'timeTaskSec', time_task_sec,
      'speedTps', speed_tps, 'qualStatus', qual_status, 'availability', availability,
      'eligible', json(case when eligible = 1 then 'true' else 'false' end), 'intelEffPerUsd', intel_eff_per_usd,
      'usdPerUsableUnit', usd_per_usable_unit, 'pointsPerDollar', points_per_dollar,
      'pointsPerSecond', points_per_second, 'balancedValue', balanced_value,
      'smartValue', smart_value, 'speedValue', speed_value, 'rankBalanced', rank_balanced,
      'rankSmart', rank_smart, 'rankSpeed', rank_speed, 'rankIntelligence', rank_intelligence,
      'rankLowCost', rank_low_cost
    ) order by rank_balanced nulls last, row_index)
    from (${rankings}) vr
  ), json('[]')),
  'thresholds', coalesce((
    select json_group_array(json_object(
      'testRunKey', test_run_key, 'target', target, 'usable', json(case when usable = 1 then 'true' else 'false' end),
      'levelledScore', levelled_score, 'costTask', cost_task, 'timeMin', time_min,
      'usdPerUsablePoint', usd_per_usable_point
    ) order by test_run_key, target)
    from (${thresholds}) vt
  ), json('[]')),
  'configs', coalesce((
    select json_group_array(json_object(
      'modelName', model_name, 'benchmark', benchmark, 'effort', effort,
      'intelligence', value_num, 'costPerTaskUsd', cost_per_task_usd, 'speedTps', output_tps
    ) order by model_name, effort)
    from ainventory_measurement_events br
    where br.ingest_batch_id = (select me.ingest_batch_id from ainventory_measurement_events me
      where me.source_kind = 'workbook-import'
        and me.ingest_batch_id in (select s.import_batch from ainventory_sources s
          where s.kind = 'drive-workbook'
            and s.imported_at <= (select max(s2.imported_at) from ainventory_sources s2
              where s2.import_batch = ${b} and s2.kind = 'drive-workbook'))
      order by me.ingested_at desc limit 1)
      and br.benchmark like 'AA Intelligence Index%'
      and br.metric = 'intelligence_index'
      and br.benchmark_version is null
  ), json('[]')),
  'testRunRows', coalesce((
    select json_group_array(json_object('rowIndex', row_index, 'cells', data->'cells') order by row_index)
    from ainventory_sheet_rows sr
    where sr.import_batch = ${b}
      and sr.sheet_name = 'Test_Runs'
  ), json('[]')),
  'cai', coalesce((
    select json_group_array(json_object(
      'modelName', model_name, 'effort', effort, 'version', raw->>'benchmark_version',
      'score', value_num, 'cost', cost_per_task_usd,
      'wallSec', CAST(nullif(raw->>'time_per_task_seconds', '') AS REAL),
      'tokensPerTask', CAST(nullif(raw->>'token_usage_per_task', '') AS REAL),
      'harness', raw->>'agent_harness',
      'creatorSlug', case when instr(raw->>'observation_key', '|') = 0 or instr(substr(raw->>'observation_key', instr(raw->>'observation_key', '|') + 1), '|') = 0 then '' when instr(substr(substr(raw->>'observation_key', instr(raw->>'observation_key', '|') + 1), instr(substr(raw->>'observation_key', instr(raw->>'observation_key', '|') + 1), '|') + 1), '|') = 0 then substr(substr(raw->>'observation_key', instr(raw->>'observation_key', '|') + 1), instr(substr(raw->>'observation_key', instr(raw->>'observation_key', '|') + 1), '|') + 1) else substr(substr(substr(raw->>'observation_key', instr(raw->>'observation_key', '|') + 1), instr(substr(raw->>'observation_key', instr(raw->>'observation_key', '|') + 1), '|') + 1), 1, instr(substr(substr(raw->>'observation_key', instr(raw->>'observation_key', '|') + 1), instr(substr(raw->>'observation_key', instr(raw->>'observation_key', '|') + 1), '|') + 1), '|') - 1) end,
      'deepSwe', CAST(nullif(raw->>'deep_swe_v1_1_pct', '') AS REAL),
      'terminalBench', CAST(nullif(raw->>'terminal_bench_4_0_pct', '') AS REAL),
      'sweAtlas', CAST(nullif(raw->>'swe_atlas_qna_pct', '') AS REAL),
      'snapshotDate', nullif(raw->>'snapshot_date', ''), 'sourceUrl', nullif(raw->>'source_url', '')
    ) order by value_num desc nulls last, model_name)
    from ainventory_measurement_events br
    where br.ingest_batch_id = (select me.ingest_batch_id from ainventory_measurement_events me
      where me.source_kind = 'workbook-import'
        and me.ingest_batch_id in (select s.import_batch from ainventory_sources s
          where s.kind = 'drive-workbook'
            and s.imported_at <= (select max(s2.imported_at) from ainventory_sources s2
              where s2.import_batch = ${b} and s2.kind = 'drive-workbook'))
      order by me.ingested_at desc limit 1)
      and br.benchmark like 'Artificial Analysis Coding Agent Index%'
      and br.metric = 'cai_index_score'
      and br.raw->>'benchmark_version' is not null
  ), json('[]')),
  'prices', coalesce((
    select json_group_array(json_object(
      'modelName', model_name, 'inputPer1M', input_usd_per_1m, 'outputPer1M', output_usd_per_1m,
      'cacheReadPer1M', cached_input_usd_per_1m, 'cacheWritePer1M', cache_write_usd_per_1m,
      'contextMinTokens', context_min_tokens, 'contextMaxTokens', context_max_tokens,
      'windowKind', window_kind
    ) order by model_name, context_min_tokens nulls first)
    from (select * from (select *, row_number() over (partition by model_name, coalesce(provider, ''), context_min_tokens, context_max_tokens, window_kind order by effective_date desc nulls last, source_date desc nulls last, ingested_at desc, id desc) as price_rank from ainventory_price_events) where price_rank = 1)
  ), json('[]'))
) as aa`;
}
