-- 0010_ainventory_views.sql — AInventory phase 1: port the workbook's
-- Threshold_Metrics + Rankings formula families to SQL views over
-- ainventory.sheet_rows (task 1019, port-ainventory-formulas-2026-10-22).
--
-- Source of truth for the port: the workbook
-- ai_model_benchmark_inventory_advanced_2026-10-21.xlsx. Every derived
-- column in both sheets uses exactly ONE formula pattern, copied down
-- all data rows (verified 2026-10-25 by formula-pattern extraction):
--   Threshold_Metrics: 13 formula columns x 1,144 data rows
--   Rankings:          25 formula columns x 138 data rows
-- Excel semantics mapped to SQL:
--   "" (blank result)            -> NULL
--   INDEX(Sheet!$C:$C, row)      -> join sheet_rows on row_index
--   MAXIFS/MINIFS(col, cond...)  -> window max/min with FILTER / partition
--   1+COUNTIF(range,">"&v)+COUNTIFS(up-to-row, v)-1  (tie-sequenced rank)
--                                -> rank() OVER (...) + row_number() OVER
--                                   (PARTITION BY value ORDER BY row_index) - 1
-- Views read the LATEST workbook import batch only (sources.imported_at),
-- so a Plan Watch re-import flips the views to the new snapshot with no
-- DDL change. Controls (target, weights, filters, benchmark version) and
-- Benchmark_Versions cumulative factors are read from sheet_rows too:
-- editing the workbook controls changes view output on next import,
-- exactly as it changes the workbook today.
-- Re-runnable: CREATE OR REPLACE throughout.

-- ---------------------------------------------------------------- helpers
-- Latest workbook batch scalar (inlined per view; kept as a view so the
-- batch choice is inspectable).
create or replace view ainventory.v_latest_workbook_batch as
select s.import_batch
from ainventory.sources s
where s.kind = 'drive-workbook'
order by s.imported_at desc
limit 1;

-- ------------------------------------------------------- Threshold_Metrics
-- Workbook layout (row 1 = header, data from row 2):
--   A test_run_key  B model_variant_key  C benchmark_version_key
--   D run_metric_row (1-based sheet row in Run_Metrics)
--   E target (literal)
--   F levelled_score =INDEX(Run_Metrics!$F:$F,D)
--   G cost_task      =INDEX(Run_Metrics!$G:$G,D)
--   H time_min       =INDEX(Run_Metrics!$I:$I,D)
--   I usable_surplus =IF(F="","",F-(E-1))
--   J usable         =IF(I>0,1,0)
--   K intel_eff_surplus_per_$ =IF(OR(J=0,G="",G=0),"",I/G)
--   L $/usable_point          =IF(OR(J=0,I=0,G=""),"",G/I)
--   M speed_eff_surplus_per_$minute =IF(OR(J=0,G="",H="",G=0,H=0),"",I/(G*H))
--   N intel_eff_norm =IF(K="","",K/MAXIFS($K:$K,$E:$E,E))
--   O speed_eff_norm =IF(M="","",M/MAXIFS($M:$M,$E:$E,E))
--   P balanced_50_50 =IF(OR(N="",O=""),"",SQRT(N*O))
--   Q smart_70_30    =IF(OR(N="",O=""),"",N^0.7*O^0.3)
--   R speed_30_70    =IF(OR(N="",O=""),"",N^0.3*O^0.7)
create or replace view ainventory.v_threshold_metrics as
with batch as (select import_batch from ainventory.v_latest_workbook_batch),
rm as (
  select sr.row_index,
         nullif(sr.data->'cells'->>5, '')::numeric as levelled_score,
         nullif(sr.data->'cells'->>6, '')::numeric as cost_task,
         nullif(sr.data->'cells'->>8, '')::numeric as time_min
  from ainventory.sheet_rows sr, batch b
  where sr.sheet_name = 'Run_Metrics' and sr.import_batch = b.import_batch
    and sr.row_index >= 2
),
tm_raw as (
  select sr.row_index,
         sr.data->'cells'->>0 as test_run_key,
         sr.data->'cells'->>1 as model_variant_key,
         sr.data->'cells'->>2 as benchmark_version_key,
         nullif(sr.data->'cells'->>3, '')::integer as run_metric_row,
         nullif(sr.data->'cells'->>4, '')::numeric as target
  from ainventory.sheet_rows sr, batch b
  where sr.sheet_name = 'Threshold_Metrics' and sr.import_batch = b.import_batch
    and sr.row_index >= 2
),
base as (
  select t.*, rm.levelled_score, rm.cost_task, rm.time_min
  from tm_raw t
  left join rm on rm.row_index = t.run_metric_row
),
calc as (
  select *,
         case when levelled_score is null then null
              else levelled_score - (target - 1) end as usable_surplus
  from base
),
calc2 as (
  select *,
         case when usable_surplus > 0 then 1 else 0 end as usable,
         case when usable_surplus > 0
               and cost_task is not null and cost_task <> 0
              then usable_surplus / cost_task end as intel_eff_surplus_per_usd,
         case when usable_surplus > 0 and usable_surplus <> 0
               and cost_task is not null
              then cost_task / usable_surplus end as usd_per_usable_point,
         case when usable_surplus > 0
               and cost_task is not null and cost_task <> 0
               and time_min is not null and time_min <> 0
              then usable_surplus / (cost_task * time_min)
              end as speed_eff_surplus_per_usd_minute
  from calc
),
normed as (
  select *,
         case when intel_eff_surplus_per_usd is null then null
              else intel_eff_surplus_per_usd
                   / max(intel_eff_surplus_per_usd) over (partition by target)
              end as intel_eff_norm,
         case when speed_eff_surplus_per_usd_minute is null then null
              else speed_eff_surplus_per_usd_minute
                   / max(speed_eff_surplus_per_usd_minute) over (partition by target)
              end as speed_eff_norm
  from calc2
)
select row_index, test_run_key, model_variant_key, benchmark_version_key,
       run_metric_row, target, levelled_score, cost_task, time_min,
       usable_surplus, usable, intel_eff_surplus_per_usd,
       usd_per_usable_point, speed_eff_surplus_per_usd_minute,
       intel_eff_norm, speed_eff_norm,
       case when intel_eff_norm is null or speed_eff_norm is null then null
            else sqrt(intel_eff_norm * speed_eff_norm) end as balanced_50_50,
       case when intel_eff_norm is null or speed_eff_norm is null then null
            else power(intel_eff_norm, 0.7) * power(speed_eff_norm, 0.3)
            end as smart_70_30,
       case when intel_eff_norm is null or speed_eff_norm is null then null
            else power(intel_eff_norm, 0.3) * power(speed_eff_norm, 0.7)
            end as speed_30_70
from normed;

-- ------------------------------------------------------------- Rankings
-- Workbook layout (row 1 = title, row 2 = blank, row 3 = header,
-- data rows 4..141). Literal inputs: A test_run_key, B model_variant_key,
-- C family, D creator, E effort, F benchmark_version_key, G raw_score,
-- J cost_task, K time_task_sec, L speed_tps, M qual_status, N availability.
-- Derived:
--   H version_level_factor =IFERROR(INDEX(Benchmark_Versions!$I:$I,
--                             MATCH(F,Benchmark_Versions!$A:$A,0)),1)
--   I adjusted_score =IF(G="","",G*H)
--   O eligible =IF(F<>Controls!$C$12,0,
--                 IF(AND(Controls!$C$10="Contender",M<>"Contender"),0,
--                 IF(AND(Controls!$C$10="Contender+Close",
--                        NOT(OR(M="Contender",M="Close"))),0,
--                 IF(AND(Controls!$C$11="Current",
--                        NOT(OR(N="yes",N="limited"))),0,1))))
--   P target =Controls!$C$7
--   Q usable_units =IF(I="","",I-(P-1))
--   R intel_eff_per_$ =IF(OR(O=0,Q<=0,J="",J=0),"",Q/J)
--   S $/usable_unit   =IF(OR(O=0,Q<=0,J="",Q=0),"",J/Q)
--   T speed_eff_per_$minute =IF(OR(O=0,Q<=0,J="",K="",J=0,K=0),"",Q/(J*(K/60)))
--   U abs_intel_norm =IF(OR(O=0,I=""),"",I/MAXIFS($I:$I,$O:$O,1))
--   V abs_speed_norm =IF(OR(O=0,L=""),"",L/MAXIFS($L:$L,$O:$O,1))
--   W cost_norm      =IF(OR(O=0,J=""),"",MINIFS($J:$J,$O:$O,1,$J:$J,">0")/J)
--   X intel_eff_norm =IF(R="","",R/MAXIFS($R:$R,$O:$O,1))
--   Y speed_eff_norm =IF(T="","",T/MAXIFS($T:$T,$O:$O,1))
--   Z balanced_value =IF(OR(X="",Y=""),"",SQRT(X*Y))
--   AA smart_value   =IF(OR(X="",Y=""),"",X^Controls!$C$8*Y^Controls!$C$9)
--   AB speed_value   =IF(OR(X="",Y=""),"",X^Controls!$C$9*Y^Controls!$C$8)
--   AC speed_x_intel =IF(OR(U="",V=""),"",SQRT(U*V))
--   AD points_per_dollar =IF(OR(I="",J="",J=0),"",I/J)
--   AE points_per_second =IF(OR(I="",K="",K=0),"",I/K)
--   AF..AH ranks over Z/AA/AB across ALL rows (no eligibility filter):
--        =IF(v="","",1+COUNTIF($v$4:$v$141,">"&v)+COUNTIFS($v$4:v,v)-1)
--   AI rank_intelligence =IF(OR(O=0,I=""),"",1+COUNTIFS($O:$O,1,$I:$I,">"&I)
--                            +COUNTIFS($O$4:O,I,I)-1)   [eligible only]
--   AJ rank_abs_speed    — same shape over L
--   AK rank_low_cost     =IF(OR(O=0,J="",J=0),"",1+COUNTIFS($O:$O,1,
--                            $J:$J,"<"&J,$J:$J,">0")+COUNTIFS($O$4:O,1,$J$4:J,J)-1)
create or replace view ainventory.v_rankings as
with batch as (select import_batch from ainventory.v_latest_workbook_batch),
ctrl as (
  select
    max(case when sr.row_index = 7  then nullif(sr.data->'cells'->>2, '')::numeric end) as target,
    max(case when sr.row_index = 8  then nullif(sr.data->'cells'->>2, '')::numeric end) as intel_weight,
    max(case when sr.row_index = 9  then nullif(sr.data->'cells'->>2, '')::numeric end) as speed_weight,
    max(case when sr.row_index = 10 then sr.data->'cells'->>2 end) as status_filter,
    max(case when sr.row_index = 11 then sr.data->'cells'->>2 end) as availability_filter,
    max(case when sr.row_index = 12 then sr.data->'cells'->>2 end) as benchmark_version
  from ainventory.sheet_rows sr, batch b
  where sr.sheet_name = 'Controls' and sr.import_batch = b.import_batch
),
bv as (
  select sr.data->'cells'->>0 as benchmark_version_key,
         nullif(sr.data->'cells'->>8, '')::numeric as cumulative_factor
  from ainventory.sheet_rows sr, batch b
  where sr.sheet_name = 'Benchmark_Versions' and sr.import_batch = b.import_batch
    and sr.row_index >= 2 and coalesce(sr.data->'cells'->>0, '') <> ''
),
rk_raw as (
  select sr.row_index,
         sr.data->'cells'->>0 as test_run_key,
         sr.data->'cells'->>1 as model_variant_key,
         sr.data->'cells'->>2 as family,
         sr.data->'cells'->>3 as creator,
         sr.data->'cells'->>4 as effort,
         sr.data->'cells'->>5 as benchmark_version_key,
         nullif(sr.data->'cells'->>6, '')::numeric as raw_score,
         nullif(sr.data->'cells'->>9, '')::numeric as cost_task,
         nullif(sr.data->'cells'->>10, '')::numeric as time_task_sec,
         nullif(sr.data->'cells'->>11, '')::numeric as speed_tps,
         sr.data->'cells'->>12 as qual_status,
         sr.data->'cells'->>13 as availability,
         c.target, c.intel_weight, c.speed_weight,
         c.status_filter, c.availability_filter, c.benchmark_version
  from ainventory.sheet_rows sr, batch b
  cross join ctrl c
  where sr.sheet_name = 'Rankings' and sr.import_batch = b.import_batch
    and sr.row_index >= 4 and coalesce(sr.data->'cells'->>0, '') <> ''
),
base as (
  select r.*,
         coalesce(bv.cumulative_factor, 1) as version_level_factor
  from rk_raw r
  left join bv on bv.benchmark_version_key = r.benchmark_version_key
),
calc as (
  select *,
         case when raw_score is null then null
              else raw_score * version_level_factor end as adjusted_score,
         case
           when coalesce(benchmark_version_key, '') <> coalesce(benchmark_version, '') then 0
           when status_filter = 'Contender'
                and coalesce(qual_status, '') <> 'Contender' then 0
           when status_filter = 'Contender+Close'
                and coalesce(qual_status, '') not in ('Contender', 'Close') then 0
           when availability_filter = 'Current'
                and coalesce(availability, '') not in ('yes', 'limited') then 0
           else 1 end as eligible
  from base
),
calc2 as (
  select *,
         case when adjusted_score is null then null
              else adjusted_score - (target - 1) end as usable_units,
         target as target_value
  from calc
),
calc3 as (
  select *,
         case when eligible = 0 or usable_units is null or usable_units <= 0
                   or cost_task is null or cost_task = 0 then null
              else usable_units / cost_task end as intel_eff_per_usd,
         case when eligible = 0 or usable_units is null or usable_units <= 0
                   or cost_task is null then null
              else cost_task / usable_units end as usd_per_usable_unit,
         case when eligible = 0 or usable_units is null or usable_units <= 0
                   or cost_task is null or cost_task = 0
                   or time_task_sec is null or time_task_sec = 0 then null
              else usable_units / (cost_task * (time_task_sec / 60))
              end as speed_eff_per_usd_minute
  from calc2
),
normed as (
  select *,
         case when eligible = 0 or adjusted_score is null then null
              else adjusted_score
                   / max(adjusted_score) filter (where eligible = 1) over ()
              end as abs_intel_norm,
         case when eligible = 0 or speed_tps is null then null
              else speed_tps
                   / max(speed_tps) filter (where eligible = 1) over ()
              end as abs_speed_norm,
         case when eligible = 0 or cost_task is null then null
              else (min(cost_task) filter (where eligible = 1 and cost_task > 0) over ())
                   / cost_task end as cost_norm,
         case when intel_eff_per_usd is null then null
              else intel_eff_per_usd
                   / max(intel_eff_per_usd) filter (where eligible = 1) over ()
              end as intel_eff_norm,
         case when speed_eff_per_usd_minute is null then null
              else speed_eff_per_usd_minute
                   / max(speed_eff_per_usd_minute) filter (where eligible = 1) over ()
              end as speed_eff_norm
  from calc3
),
valued as (
  select *,
         case when intel_eff_norm is null or speed_eff_norm is null then null
              else sqrt(intel_eff_norm * speed_eff_norm) end as balanced_value,
         case when intel_eff_norm is null or speed_eff_norm is null then null
              else power(intel_eff_norm, intel_weight)
                   * power(speed_eff_norm, speed_weight) end as smart_value,
         case when intel_eff_norm is null or speed_eff_norm is null then null
              else power(intel_eff_norm, speed_weight)
                   * power(speed_eff_norm, intel_weight) end as speed_value,
         case when abs_intel_norm is null or abs_speed_norm is null then null
              else sqrt(abs_intel_norm * abs_speed_norm) end as speed_x_intel,
         case when adjusted_score is null or cost_task is null or cost_task = 0
              then null else adjusted_score / cost_task end as points_per_dollar,
         case when adjusted_score is null or time_task_sec is null
                   or time_task_sec = 0 then null
              else adjusted_score / time_task_sec end as points_per_second
  from normed
)
select row_index, test_run_key, model_variant_key, family, creator, effort,
       benchmark_version_key, raw_score, version_level_factor,
       adjusted_score, cost_task, time_task_sec, speed_tps, qual_status,
       availability, eligible, target_value as target, usable_units,
       intel_eff_per_usd, usd_per_usable_unit, speed_eff_per_usd_minute,
       abs_intel_norm, abs_speed_norm, cost_norm, intel_eff_norm,
       speed_eff_norm, balanced_value, smart_value, speed_value,
       speed_x_intel, points_per_dollar, points_per_second,
       case when balanced_value is null then null
            else rank() over (order by balanced_value desc nulls last)
                 + row_number() over (partition by balanced_value
                                      order by row_index) - 1
            end as rank_balanced,
       case when smart_value is null then null
            else rank() over (order by smart_value desc nulls last)
                 + row_number() over (partition by smart_value
                                      order by row_index) - 1
            end as rank_smart,
       case when speed_value is null then null
            else rank() over (order by speed_value desc nulls last)
                 + row_number() over (partition by speed_value
                                      order by row_index) - 1
            end as rank_speed,
       case when eligible = 0 or adjusted_score is null then null
            else rank() over (partition by eligible order by adjusted_score desc nulls last)
                 + row_number() over (partition by eligible, adjusted_score
                                      order by row_index) - 1
            end as rank_intelligence,
       case when eligible = 0 or speed_tps is null then null
            else rank() over (partition by eligible order by speed_tps desc nulls last)
                 + row_number() over (partition by eligible, speed_tps
                                      order by row_index) - 1
            end as rank_abs_speed,
       case when eligible = 0 or cost_task is null or cost_task = 0 then null
            else rank() over (partition by (eligible = 1 and cost_task > 0)
                              order by cost_task asc)
                 + row_number() over (partition by (eligible = 1 and cost_task > 0),
                                      cost_task order by row_index) - 1
            end as rank_low_cost
from valued;

-- Grants: views inherit the schema exposure from 0005; make them readable
-- by the same roles as the base tables.
grant select on ainventory.v_latest_workbook_batch,
                ainventory.v_threshold_metrics,
                ainventory.v_rankings
  to anon, authenticated, service_role;
