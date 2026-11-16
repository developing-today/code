-- 0004_ainventory.sql — AInventory (AI model benchmark inventory).
-- Self-contained in schema `ainventory`. Append-only history with full
-- provenance: every row carries its source file + import batch.
-- Raw sheet archive (sheet_rows, jsonb) preserves the workbooks exactly;
-- typed tables (models, model_value, benchmark_results) are built from it
-- by the import script and can be rebuilt at any time from sheet_rows.

create schema if not exists ainventory;

create table if not exists ainventory.sources (
  id               bigint generated always as identity primary key,
  name             text not null,
  kind             text not null default 'drive-file',
  drive_file_id    text,
  drive_modified_at timestamptz,
  import_batch     uuid not null,
  imported_at      timestamptz not null default now(),
  note             text
);

create table if not exists ainventory.models (
  id          bigint generated always as identity primary key,
  name        text not null unique,
  provider    text,
  first_seen  timestamptz not null default now(),
  last_seen   timestamptz not null default now()
);

create table if not exists ainventory.sheet_rows (
  id           bigint generated always as identity primary key,
  source_id    bigint not null references ainventory.sources(id),
  import_batch uuid not null,
  sheet_name   text not null,
  row_index    integer not null,
  data         jsonb not null,
  unique (source_id, import_batch, sheet_name, row_index)
);
create index if not exists sheet_rows_sheet_idx
  on ainventory.sheet_rows (sheet_name, import_batch);

-- Typed long-form results: one row per (model, benchmark, metric, effort).
-- Built from Benchmark_Comparison / Coding_Agent_Results / model_value_data.
create table if not exists ainventory.benchmark_results (
  id           bigint generated always as identity primary key,
  source_id    bigint not null references ainventory.sources(id),
  import_batch uuid not null,
  model_name   text not null,
  benchmark    text not null,
  metric       text not null,
  effort       text,
  value_num    numeric,
  value_text   text,
  cost_per_task_usd numeric,
  speed_tps    numeric,
  raw          jsonb,
  unique (source_id, import_batch, model_name, benchmark, metric, effort)
);
create index if not exists benchmark_results_model_idx
  on ainventory.benchmark_results (model_name, benchmark);

alter table ainventory.sources           enable row level security;
alter table ainventory.models            enable row level security;
alter table ainventory.sheet_rows        enable row level security;
alter table ainventory.benchmark_results enable row level security;
