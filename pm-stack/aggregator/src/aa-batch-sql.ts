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
  if (batch === null) return "(select import_batch from ainventory.v_latest_import_batch)";
  if (!isBatchUuid(batch)) throw new Error("aa payload: invalid batch uuid");
  return `'${batch}'::uuid`;
}

// --- verbatim ainventory view bodies, batch CTE parameterized ---------------

const RANKINGS_BODY = `
WITH batch AS (SELECT {{BATCH}}::uuid AS import_batch), ctrl AS (
         SELECT max(
                CASE
                    WHEN (sr.row_index = 7) THEN (NULLIF(((sr.data -> 'cells'::text) ->> 2), ''::text))::numeric
                    ELSE NULL::numeric
                END) AS target,
            max(
                CASE
                    WHEN (sr.row_index = 8) THEN (NULLIF(((sr.data -> 'cells'::text) ->> 2), ''::text))::numeric
                    ELSE NULL::numeric
                END) AS intel_weight,
            max(
                CASE
                    WHEN (sr.row_index = 9) THEN (NULLIF(((sr.data -> 'cells'::text) ->> 2), ''::text))::numeric
                    ELSE NULL::numeric
                END) AS speed_weight,
            max(
                CASE
                    WHEN (sr.row_index = 10) THEN ((sr.data -> 'cells'::text) ->> 2)
                    ELSE NULL::text
                END) AS status_filter,
            max(
                CASE
                    WHEN (sr.row_index = 11) THEN ((sr.data -> 'cells'::text) ->> 2)
                    ELSE NULL::text
                END) AS availability_filter,
            max(
                CASE
                    WHEN (sr.row_index = 12) THEN ((sr.data -> 'cells'::text) ->> 2)
                    ELSE NULL::text
                END) AS benchmark_version
           FROM ainventory.sheet_rows sr,
            batch b
          WHERE ((sr.sheet_name = 'Controls'::text) AND (sr.import_batch = b.import_batch))
        ), bv AS (
         SELECT ((sr.data -> 'cells'::text) ->> 0) AS benchmark_version_key,
            (NULLIF(((sr.data -> 'cells'::text) ->> 8), ''::text))::numeric AS cumulative_factor
           FROM ainventory.sheet_rows sr,
            batch b
          WHERE ((sr.sheet_name = 'Benchmark_Versions'::text) AND (sr.import_batch = b.import_batch) AND (sr.row_index >= 2) AND (COALESCE(((sr.data -> 'cells'::text) ->> 0), ''::text) <> ''::text))
        ), rk_raw AS (
         SELECT sr.row_index,
            ((sr.data -> 'cells'::text) ->> 0) AS test_run_key,
            ((sr.data -> 'cells'::text) ->> 1) AS model_variant_key,
            ((sr.data -> 'cells'::text) ->> 2) AS family,
            ((sr.data -> 'cells'::text) ->> 3) AS creator,
            ((sr.data -> 'cells'::text) ->> 4) AS effort,
            ((sr.data -> 'cells'::text) ->> 5) AS benchmark_version_key,
            (NULLIF(((sr.data -> 'cells'::text) ->> 6), ''::text))::numeric AS raw_score,
            (NULLIF(((sr.data -> 'cells'::text) ->> 9), ''::text))::numeric AS cost_task,
            (NULLIF(((sr.data -> 'cells'::text) ->> 10), ''::text))::numeric AS time_task_sec,
            (NULLIF(((sr.data -> 'cells'::text) ->> 11), ''::text))::numeric AS speed_tps,
            ((sr.data -> 'cells'::text) ->> 12) AS qual_status,
            ((sr.data -> 'cells'::text) ->> 13) AS availability,
            c.target,
            c.intel_weight,
            c.speed_weight,
            c.status_filter,
            c.availability_filter,
            c.benchmark_version
           FROM ainventory.sheet_rows sr,
            (batch b
             CROSS JOIN ctrl c)
          WHERE ((sr.sheet_name = 'Rankings'::text) AND (sr.import_batch = b.import_batch) AND (sr.row_index >= 4) AND (COALESCE(((sr.data -> 'cells'::text) ->> 0), ''::text) <> ''::text))
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
            COALESCE(bv.cumulative_factor, (1)::numeric) AS version_level_factor
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
                    WHEN (base.raw_score IS NULL) THEN NULL::numeric
                    ELSE (base.raw_score * base.version_level_factor)
                END AS adjusted_score,
                CASE
                    WHEN (COALESCE(base.benchmark_version_key, ''::text) <> COALESCE(base.benchmark_version, ''::text)) THEN 0
                    WHEN ((base.status_filter = 'Contender'::text) AND (COALESCE(base.qual_status, ''::text) <> 'Contender'::text)) THEN 0
                    WHEN ((base.status_filter = 'Contender+Close'::text) AND (COALESCE(base.qual_status, ''::text) <> ALL (ARRAY['Contender'::text, 'Close'::text]))) THEN 0
                    WHEN ((base.availability_filter = 'Current'::text) AND (COALESCE(base.availability, ''::text) <> ALL (ARRAY['yes'::text, 'limited'::text]))) THEN 0
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
                    WHEN (calc.adjusted_score IS NULL) THEN NULL::numeric
                    ELSE (calc.adjusted_score - (calc.target - (1)::numeric))
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
                    WHEN ((calc2.eligible = 0) OR (calc2.usable_units IS NULL) OR (calc2.usable_units <= (0)::numeric) OR (calc2.cost_task IS NULL) OR (calc2.cost_task = (0)::numeric)) THEN NULL::numeric
                    ELSE (calc2.usable_units / calc2.cost_task)
                END AS intel_eff_per_usd,
                CASE
                    WHEN ((calc2.eligible = 0) OR (calc2.usable_units IS NULL) OR (calc2.usable_units <= (0)::numeric) OR (calc2.cost_task IS NULL)) THEN NULL::numeric
                    ELSE (calc2.cost_task / calc2.usable_units)
                END AS usd_per_usable_unit,
                CASE
                    WHEN ((calc2.eligible = 0) OR (calc2.usable_units IS NULL) OR (calc2.usable_units <= (0)::numeric) OR (calc2.cost_task IS NULL) OR (calc2.cost_task = (0)::numeric) OR (calc2.time_task_sec IS NULL) OR (calc2.time_task_sec = (0)::numeric)) THEN NULL::numeric
                    ELSE (calc2.usable_units / (calc2.cost_task * (calc2.time_task_sec / (60)::numeric)))
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
                    WHEN ((calc3.eligible = 0) OR (calc3.adjusted_score IS NULL)) THEN NULL::numeric
                    ELSE (calc3.adjusted_score / max(calc3.adjusted_score) FILTER (WHERE (calc3.eligible = 1)) OVER ())
                END AS abs_intel_norm,
                CASE
                    WHEN ((calc3.eligible = 0) OR (calc3.speed_tps IS NULL)) THEN NULL::numeric
                    ELSE (calc3.speed_tps / max(calc3.speed_tps) FILTER (WHERE (calc3.eligible = 1)) OVER ())
                END AS abs_speed_norm,
                CASE
                    WHEN ((calc3.eligible = 0) OR (calc3.cost_task IS NULL)) THEN NULL::numeric
                    ELSE (min(calc3.cost_task) FILTER (WHERE ((calc3.eligible = 1) AND (calc3.cost_task > (0)::numeric))) OVER () / calc3.cost_task)
                END AS cost_norm,
                CASE
                    WHEN (calc3.intel_eff_per_usd IS NULL) THEN NULL::numeric
                    ELSE (calc3.intel_eff_per_usd / max(calc3.intel_eff_per_usd) FILTER (WHERE (calc3.eligible = 1)) OVER ())
                END AS intel_eff_norm,
                CASE
                    WHEN (calc3.speed_eff_per_usd_minute IS NULL) THEN NULL::numeric
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
                    WHEN ((normed.intel_eff_norm IS NULL) OR (normed.speed_eff_norm IS NULL)) THEN NULL::numeric
                    ELSE sqrt((normed.intel_eff_norm * normed.speed_eff_norm))
                END AS balanced_value,
                CASE
                    WHEN ((normed.intel_eff_norm IS NULL) OR (normed.speed_eff_norm IS NULL)) THEN NULL::numeric
                    ELSE (power(normed.intel_eff_norm, normed.intel_weight) * power(normed.speed_eff_norm, normed.speed_weight))
                END AS smart_value,
                CASE
                    WHEN ((normed.intel_eff_norm IS NULL) OR (normed.speed_eff_norm IS NULL)) THEN NULL::numeric
                    ELSE (power(normed.intel_eff_norm, normed.speed_weight) * power(normed.speed_eff_norm, normed.intel_weight))
                END AS speed_value,
                CASE
                    WHEN ((normed.abs_intel_norm IS NULL) OR (normed.abs_speed_norm IS NULL)) THEN NULL::numeric
                    ELSE sqrt((normed.abs_intel_norm * normed.abs_speed_norm))
                END AS speed_x_intel,
                CASE
                    WHEN ((normed.adjusted_score IS NULL) OR (normed.cost_task IS NULL) OR (normed.cost_task = (0)::numeric)) THEN NULL::numeric
                    ELSE (normed.adjusted_score / normed.cost_task)
                END AS points_per_dollar,
                CASE
                    WHEN ((normed.adjusted_score IS NULL) OR (normed.time_task_sec IS NULL) OR (normed.time_task_sec = (0)::numeric)) THEN NULL::numeric
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
            WHEN (balanced_value IS NULL) THEN NULL::bigint
            ELSE ((rank() OVER (ORDER BY balanced_value DESC NULLS LAST) + row_number() OVER (PARTITION BY balanced_value ORDER BY row_index)) - 1)
        END AS rank_balanced,
        CASE
            WHEN (smart_value IS NULL) THEN NULL::bigint
            ELSE ((rank() OVER (ORDER BY smart_value DESC NULLS LAST) + row_number() OVER (PARTITION BY smart_value ORDER BY row_index)) - 1)
        END AS rank_smart,
        CASE
            WHEN (speed_value IS NULL) THEN NULL::bigint
            ELSE ((rank() OVER (ORDER BY speed_value DESC NULLS LAST) + row_number() OVER (PARTITION BY speed_value ORDER BY row_index)) - 1)
        END AS rank_speed,
        CASE
            WHEN ((eligible = 0) OR (adjusted_score IS NULL)) THEN NULL::bigint
            ELSE ((rank() OVER (PARTITION BY eligible ORDER BY adjusted_score DESC NULLS LAST) + row_number() OVER (PARTITION BY eligible, adjusted_score ORDER BY row_index)) - 1)
        END AS rank_intelligence,
        CASE
            WHEN ((eligible = 0) OR (speed_tps IS NULL)) THEN NULL::bigint
            ELSE ((rank() OVER (PARTITION BY eligible ORDER BY speed_tps DESC NULLS LAST) + row_number() OVER (PARTITION BY eligible, speed_tps ORDER BY row_index)) - 1)
        END AS rank_abs_speed,
        CASE
            WHEN ((eligible = 0) OR (cost_task IS NULL) OR (cost_task = (0)::numeric)) THEN NULL::bigint
            ELSE ((rank() OVER (PARTITION BY ((eligible = 1) AND (cost_task > (0)::numeric)) ORDER BY cost_task) + row_number() OVER (PARTITION BY ((eligible = 1) AND (cost_task > (0)::numeric)), cost_task ORDER BY row_index)) - 1)
        END AS rank_low_cost
   FROM valued`;

const THRESHOLD_BODY = `
WITH batch AS (SELECT {{BATCH}}::uuid AS import_batch), rm AS (
         SELECT sr.row_index,
            (NULLIF(((sr.data -> 'cells'::text) ->> 5), ''::text))::numeric AS levelled_score,
            (NULLIF(((sr.data -> 'cells'::text) ->> 6), ''::text))::numeric AS cost_task,
            (NULLIF(((sr.data -> 'cells'::text) ->> 8), ''::text))::numeric AS time_min
           FROM ainventory.sheet_rows sr,
            batch b
          WHERE ((sr.sheet_name = 'Run_Metrics'::text) AND (sr.import_batch = b.import_batch) AND (sr.row_index >= 2))
        ), tm_raw AS (
         SELECT sr.row_index,
            ((sr.data -> 'cells'::text) ->> 0) AS test_run_key,
            ((sr.data -> 'cells'::text) ->> 1) AS model_variant_key,
            ((sr.data -> 'cells'::text) ->> 2) AS benchmark_version_key,
            (NULLIF(((sr.data -> 'cells'::text) ->> 3), ''::text))::integer AS run_metric_row,
            (NULLIF(((sr.data -> 'cells'::text) ->> 4), ''::text))::numeric AS target
           FROM ainventory.sheet_rows sr,
            batch b
          WHERE ((sr.sheet_name = 'Threshold_Metrics'::text) AND (sr.import_batch = b.import_batch) AND (sr.row_index >= 2))
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
                    WHEN (base.levelled_score IS NULL) THEN NULL::numeric
                    ELSE (base.levelled_score - (base.target - (1)::numeric))
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
                    WHEN (calc.usable_surplus > (0)::numeric) THEN 1
                    ELSE 0
                END AS usable,
                CASE
                    WHEN ((calc.usable_surplus > (0)::numeric) AND (calc.cost_task IS NOT NULL) AND (calc.cost_task <> (0)::numeric)) THEN (calc.usable_surplus / calc.cost_task)
                    ELSE NULL::numeric
                END AS intel_eff_surplus_per_usd,
                CASE
                    WHEN ((calc.usable_surplus > (0)::numeric) AND (calc.usable_surplus <> (0)::numeric) AND (calc.cost_task IS NOT NULL)) THEN (calc.cost_task / calc.usable_surplus)
                    ELSE NULL::numeric
                END AS usd_per_usable_point,
                CASE
                    WHEN ((calc.usable_surplus > (0)::numeric) AND (calc.cost_task IS NOT NULL) AND (calc.cost_task <> (0)::numeric) AND (calc.time_min IS NOT NULL) AND (calc.time_min <> (0)::numeric)) THEN (calc.usable_surplus / (calc.cost_task * calc.time_min))
                    ELSE NULL::numeric
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
                    WHEN (calc2.intel_eff_surplus_per_usd IS NULL) THEN NULL::numeric
                    ELSE (calc2.intel_eff_surplus_per_usd / max(calc2.intel_eff_surplus_per_usd) OVER (PARTITION BY calc2.target))
                END AS intel_eff_norm,
                CASE
                    WHEN (calc2.speed_eff_surplus_per_usd_minute IS NULL) THEN NULL::numeric
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
            WHEN ((intel_eff_norm IS NULL) OR (speed_eff_norm IS NULL)) THEN NULL::numeric
            ELSE sqrt((intel_eff_norm * speed_eff_norm))
        END AS balanced_50_50,
        CASE
            WHEN ((intel_eff_norm IS NULL) OR (speed_eff_norm IS NULL)) THEN NULL::numeric
            ELSE (power(intel_eff_norm, 0.7) * power(speed_eff_norm, 0.3))
        END AS smart_70_30,
        CASE
            WHEN ((intel_eff_norm IS NULL) OR (speed_eff_norm IS NULL)) THEN NULL::numeric
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
select coalesce(jsonb_agg(jsonb_build_object(
  'importBatch', s.import_batch::text,
  'name', s.name,
  'importedAt', s.imported_at,
  'driveModifiedAt', s.drive_modified_at,
  'ranAt', ib.ran_at,
  'insertedCounts', ib.inserted_counts
) order by s.imported_at desc nulls last), '[]'::jsonb) as batches
from ainventory.sources s
left join ainventory.ingest_batches ib on ib.id = s.import_batch
where s.kind = 'drive-workbook'
`;

/** The current-price overlay alone, in exactly the shape and order
 * aaPayloadSql embeds it as `prices` (#3302): the collector hashes
 * this every tick to validity-key the per-batch caches, because the
 * overlay is current data, not batch-bound data. */
export const AA_PRICES_SQL = `
select coalesce((
  select jsonb_agg(jsonb_build_object(
    'modelName', model_name, 'inputPer1M', input_usd_per_1m, 'outputPer1M', output_usd_per_1m,
    'cacheReadPer1M', cached_input_usd_per_1m, 'cacheWritePer1M', cache_write_usd_per_1m,
    'contextMinTokens', context_min_tokens, 'contextMaxTokens', context_max_tokens,
    'windowKind', window_kind
  ) order by model_name, context_min_tokens nulls first)
  from ainventory.v_current_prices
), '[]'::jsonb) as prices
`;

/**
 * The full AA section payload for one batch (null = latest). This mirrors the
 * collector's previous AA_SQL exactly except that every batch predicate and
 * both latest-pinned views now take the same batch expression.
 *
 * Tranche 2 of code#4401 (2026-11-11): the 'configs' and 'cai' sections no
 * longer read the frozen ainventory.benchmark_results table; they read the
 * canonical ainventory.measurement_events store, with the filters that
 * reproduce the retired table's rows exactly (verified by full-row EXCEPT
 * comparison before the table was dropped: configs 35/35, cai 27/27, zero
 * differences either way). benchmark_results is dropped;
 * measurement_events is the one canonical read path for benchmark facts.
 *
 * code#8817: measurement events ARE keyed by workbook import —
 * ingest_batch_id is the importing batch's ainventory.sources.import_batch
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
export const AA_PAYLOAD_SQL_REVISION = 1;

export function aaPayloadSql(batch: string | null): string {
  const b = batchExpression(batch);
  const rankings = RANKINGS_BODY.replace("{{BATCH}}", b);
  const thresholds = THRESHOLD_BODY.replace("{{BATCH}}", b);
  return `
select jsonb_build_object(
  'batch', ${b}::text,
  'sources', coalesce((
    select jsonb_agg(jsonb_build_object(
      'name', name, 'kind', kind, 'importedAt', imported_at, 'driveModifiedAt', drive_modified_at
    ) order by id)
    from ainventory.sources
    where import_batch = ${b}
  ), '[]'::jsonb),
  'rankings', coalesce((
    select jsonb_agg(jsonb_build_object(
      'rowIndex', row_index, 'testRunKey', test_run_key, 'modelVariantKey', model_variant_key,
      'family', family, 'creator', creator, 'effort', effort,
      'benchmarkVersionKey', benchmark_version_key, 'rawScore', raw_score,
      'adjustedScore', adjusted_score, 'costTask', cost_task, 'timeTaskSec', time_task_sec,
      'speedTps', speed_tps, 'qualStatus', qual_status, 'availability', availability,
      'eligible', (eligible = 1), 'intelEffPerUsd', intel_eff_per_usd,
      'usdPerUsableUnit', usd_per_usable_unit, 'pointsPerDollar', points_per_dollar,
      'pointsPerSecond', points_per_second, 'balancedValue', balanced_value,
      'smartValue', smart_value, 'speedValue', speed_value, 'rankBalanced', rank_balanced,
      'rankSmart', rank_smart, 'rankSpeed', rank_speed, 'rankIntelligence', rank_intelligence,
      'rankLowCost', rank_low_cost
    ) order by rank_balanced nulls last, row_index)
    from (${rankings}) vr
  ), '[]'::jsonb),
  'thresholds', coalesce((
    select jsonb_agg(jsonb_build_object(
      'testRunKey', test_run_key, 'target', target, 'usable', (usable = 1),
      'levelledScore', levelled_score, 'costTask', cost_task, 'timeMin', time_min,
      'usdPerUsablePoint', usd_per_usable_point
    ) order by test_run_key, target)
    from (${thresholds}) vt
  ), '[]'::jsonb),
  'configs', coalesce((
    select jsonb_agg(jsonb_build_object(
      'modelName', model_name, 'benchmark', benchmark, 'effort', effort,
      'intelligence', value_num, 'costPerTaskUsd', cost_per_task_usd, 'speedTps', output_tps
    ) order by model_name, effort)
    from ainventory.measurement_events br
    where br.ingest_batch_id = (select me.ingest_batch_id from ainventory.measurement_events me
      where me.source_kind = 'workbook-import'
        and me.ingest_batch_id in (select s.import_batch from ainventory.sources s
          where s.kind = 'drive-workbook'
            and s.imported_at <= (select max(s2.imported_at) from ainventory.sources s2
              where s2.import_batch = ${b} and s2.kind = 'drive-workbook'))
      order by me.ingested_at desc limit 1)
      and br.benchmark like 'AA Intelligence Index%'
      and br.metric = 'intelligence_index'
      and br.benchmark_version is null
  ), '[]'::jsonb),
  'testRunRows', coalesce((
    select jsonb_agg(jsonb_build_object('rowIndex', row_index, 'cells', data->'cells') order by row_index)
    from ainventory.sheet_rows sr
    where sr.import_batch = ${b}
      and sr.sheet_name = 'Test_Runs'
  ), '[]'::jsonb),
  'cai', coalesce((
    select jsonb_agg(jsonb_build_object(
      'modelName', model_name, 'effort', effort, 'version', raw->>'benchmark_version',
      'score', value_num, 'cost', cost_per_task_usd,
      'wallSec', nullif(raw->>'time_per_task_seconds', '')::numeric,
      'tokensPerTask', nullif(raw->>'token_usage_per_task', '')::numeric,
      'harness', raw->>'agent_harness',
      'creatorSlug', split_part(raw->>'observation_key', '|', 3),
      'deepSwe', nullif(raw->>'deep_swe_v1_1_pct', '')::numeric,
      'terminalBench', nullif(raw->>'terminal_bench_4_0_pct', '')::numeric,
      'sweAtlas', nullif(raw->>'swe_atlas_qna_pct', '')::numeric,
      'snapshotDate', nullif(raw->>'snapshot_date', ''), 'sourceUrl', nullif(raw->>'source_url', '')
    ) order by value_num desc nulls last, model_name)
    from ainventory.measurement_events br
    where br.ingest_batch_id = (select me.ingest_batch_id from ainventory.measurement_events me
      where me.source_kind = 'workbook-import'
        and me.ingest_batch_id in (select s.import_batch from ainventory.sources s
          where s.kind = 'drive-workbook'
            and s.imported_at <= (select max(s2.imported_at) from ainventory.sources s2
              where s2.import_batch = ${b} and s2.kind = 'drive-workbook'))
      order by me.ingested_at desc limit 1)
      and br.benchmark like 'Artificial Analysis Coding Agent Index%'
      and br.metric = 'cai_index_score'
      and br.raw->>'benchmark_version' is not null
  ), '[]'::jsonb),
  'prices', coalesce((
    select jsonb_agg(jsonb_build_object(
      'modelName', model_name, 'inputPer1M', input_usd_per_1m, 'outputPer1M', output_usd_per_1m,
      'cacheReadPer1M', cached_input_usd_per_1m, 'cacheWritePer1M', cache_write_usd_per_1m,
      'contextMinTokens', context_min_tokens, 'contextMaxTokens', context_max_tokens,
      'windowKind', window_kind
    ) order by model_name, context_min_tokens nulls first)
    from ainventory.v_current_prices
  ), '[]'::jsonb)
) as aa`;
}
