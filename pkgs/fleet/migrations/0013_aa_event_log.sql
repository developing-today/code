-- 0013_aa_event_log.sql — Workbook-free, append-only event log for model
-- data (issue developing-today/code#1248, Phase 1). Design:
-- workspace/goals/supabase-muse-dev-platform-backups/files/
--   aa-event-log-design-2026-10-26.md
--
-- New tables land BESIDE the workbook-era ainventory tables (sources,
-- models, sheet_rows, benchmark_results stay untouched and readable).
-- Measurement and price facts become INSERT-only event rows with full
-- provenance (source kind/ref, the source's own date, ingest timestamp).
-- "Current" is a view: newest event per natural key. Nothing here edits
-- or deletes an existing row, ever:
--   * BEFORE UPDATE/DELETE triggers raise on the event tables, and
--   * UPDATE/DELETE grants are revoked from every API role.
--
-- Re-runnable: CREATE IF NOT EXISTS / CREATE OR REPLACE throughout.

create schema if not exists ainventory;

-- ------------------------------------------------------------ batches
-- One row per gather run (or, for the backfill, per original workbook
-- import batch — the batch uuid is preserved as the row id so provenance
-- chains stay intact).
create table if not exists ainventory.ingest_batches (
  id              uuid primary key default gen_random_uuid(),
  source_kind     text not null,  -- aa | modelsdev | firstparty | thirdparty
                                  -- | probe | provider-doc | workbook-import
  source_ref      text,
  ran_at          timestamptz not null default now(),
  inserted_counts jsonb not null default '{}'::jsonb,
  note            text,
  created_at      timestamptz not null default now()
);

-- -------------------------------------------------- measurement events
-- INSERT-only. benchmark_version is split out of the legacy combined
-- benchmark name ('AA Intelligence Index v4.3.2' -> benchmark
-- 'AA Intelligence Index', version '4.3.2'). source_date is the SOURCE's
-- own date (AA snapshot date, workbook run date) — never the ingest date;
-- ingested_at records when the row actually landed here.
create table if not exists ainventory.measurement_events (
  id                     bigint generated always as identity primary key,
  model_id               bigint references ainventory.models(id),
  model_name             text not null,
  model_variant_key      text,
  run_key                text,
  effort                 text,
  benchmark              text not null,
  benchmark_version      text,
  metric                 text not null,
  value_num              numeric not null,
  intelligence_index     numeric,
  ttft_seconds           numeric,
  output_tps             numeric,
  output_tokens_per_task numeric,
  cost_per_task_usd      numeric,
  task_time_seconds      numeric,
  source_kind            text not null,
  source_ref             text,
  source_date            date,
  ingested_at            timestamptz not null default now(),
  ingest_batch_id        uuid references ainventory.ingest_batches(id),
  raw                    jsonb
);
create index if not exists measurement_events_key_idx
  on ainventory.measurement_events
     (model_variant_key, benchmark, benchmark_version, effort);
create index if not exists measurement_events_model_idx
  on ainventory.measurement_events (model_name, benchmark);
create index if not exists measurement_events_batch_idx
  on ainventory.measurement_events (ingest_batch_id);

-- -------------------------------------------------------- price events
-- INSERT-only, dimensional: context band (min/max tokens) x time window
-- (flat | peak | offpeak, with local hours when windowed). provider is
-- null for vendor list prices; a provider-scoped row records what that
-- door actually charges.
create table if not exists ainventory.price_events (
  id                        bigint generated always as identity primary key,
  model_id                  bigint references ainventory.models(id),
  model_name                text not null,
  model_variant_key         text,
  provider                  text,
  input_usd_per_1m          numeric,
  output_usd_per_1m         numeric,
  cached_input_usd_per_1m   numeric,
  cache_write_usd_per_1m    numeric,
  context_min_tokens        bigint not null default 0,
  context_max_tokens        bigint,
  window_kind               text not null default 'flat'
                            check (window_kind in ('flat','peak','offpeak')),
  window_hours              text,
  window_tz                 text,
  effective_date            date,
  source_kind               text not null,
  source_ref                text,
  source_date               date,
  ingested_at               timestamptz not null default now(),
  ingest_batch_id           uuid references ainventory.ingest_batches(id),
  raw                       jsonb
);
create index if not exists price_events_key_idx
  on ainventory.price_events
     (model_name, provider, context_min_tokens, context_max_tokens, window_kind);
create index if not exists price_events_batch_idx
  on ainventory.price_events (ingest_batch_id);

-- --------------------------------------------------- provider policies
-- Versioned by new rows (also append-only): whether a provider passes
-- windowed pricing through to its quota or charges one flat rate.
create table if not exists ainventory.provider_policies (
  id              bigint generated always as identity primary key,
  provider        text not null,
  scope_kind      text not null default 'all'
                  check (scope_kind in ('all','family','model')),
  scope_value     text,
  window_behavior text not null
                  check (window_behavior in ('pass_through','flat','unknown')),
  note            text,
  effective_from  date,
  source_kind     text,
  source_ref      text,
  created_at      timestamptz not null default now()
);

-- ---------------------------------------------------------------- notes
-- Dated, attributed notes on models and ingest batches. Multiple per
-- model; never collapsed into a summary field.
create table if not exists ainventory.notes (
  id              bigint generated always as identity primary key,
  model_id        bigint references ainventory.models(id),
  model_name      text,
  ingest_batch_id uuid references ainventory.ingest_batches(id),
  kind            text not null
                  check (kind in ('first_party_claim','third_party_report',
                                  'agent_note','anecdote')),
  body            text not null,
  source_ref      text,
  author          text,
  created_at      timestamptz not null default now()
);
create index if not exists notes_model_idx
  on ainventory.notes (model_id, model_name);
create index if not exists notes_batch_idx
  on ainventory.notes (ingest_batch_id);

-- ------------------------------------------- append-only enforcement
create or replace function ainventory.reject_event_mutation()
returns trigger language plpgsql as $$
begin
  raise exception 'ainventory.% is append-only (issue #1248): % refused',
    tg_table_name, tg_op;
end $$;

drop trigger if exists measurement_events_no_mutate
  on ainventory.measurement_events;
create trigger measurement_events_no_mutate
  before update or delete on ainventory.measurement_events
  for each row execute function ainventory.reject_event_mutation();

drop trigger if exists price_events_no_mutate on ainventory.price_events;
create trigger price_events_no_mutate
  before update or delete on ainventory.price_events
  for each row execute function ainventory.reject_event_mutation();

drop trigger if exists provider_policies_no_mutate
  on ainventory.provider_policies;
create trigger provider_policies_no_mutate
  before update or delete on ainventory.provider_policies
  for each row execute function ainventory.reject_event_mutation();

-- ------------------------------------------------------------- RLS/grants
alter table ainventory.ingest_batches     enable row level security;
alter table ainventory.measurement_events enable row level security;
alter table ainventory.price_events       enable row level security;
alter table ainventory.provider_policies  enable row level security;
alter table ainventory.notes              enable row level security;
-- No policies: API roles see nothing without one, same posture as the
-- workbook-era tables. service_role (server-side only) reads/writes.

grant select, insert on ainventory.ingest_batches,
                        ainventory.measurement_events,
                        ainventory.price_events,
                        ainventory.provider_policies,
                        ainventory.notes
  to service_role;
grant usage, select on all sequences in schema ainventory to service_role;
-- Defense in depth on top of the triggers: no API role may mutate events.
revoke update, delete on ainventory.measurement_events,
                         ainventory.price_events,
                         ainventory.provider_policies
  from service_role, authenticated, anon;

-- ------------------------------------------------------------ current views
-- Newest event per (model x effort x benchmark x benchmark version):
-- newest SOURCE date first (a fact's own date outranks when we heard
-- it), then newest ingest, then highest id as a stable tiebreak.
create or replace view ainventory.v_current_measurements as
select distinct on (
         coalesce(model_variant_key, model_name),
         coalesce(effort, ''), benchmark, coalesce(benchmark_version, '')
       ) *
from ainventory.measurement_events
order by coalesce(model_variant_key, model_name),
         coalesce(effort, ''), benchmark, coalesce(benchmark_version, ''),
         source_date desc nulls last, ingested_at desc, id desc;

-- Newest price per (model x provider x context band x window).
create or replace view ainventory.v_current_prices as
select distinct on (
         model_name, coalesce(provider, ''), context_min_tokens,
         context_max_tokens, window_kind
       ) *
from ainventory.price_events
order by model_name, coalesce(provider, ''), context_min_tokens,
         context_max_tokens, window_kind,
         effective_date desc nulls last, source_date desc nulls last,
         ingested_at desc, id desc;

grant select on ainventory.v_current_measurements,
                ainventory.v_current_prices
  to anon, authenticated, service_role;
