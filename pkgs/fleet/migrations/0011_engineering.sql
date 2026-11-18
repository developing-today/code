-- 0011_engineering.sql — Engineering Research Ledger import.
-- Self-contained in schema `engineering`. Append-only history with full
-- provenance: every row carries its source file + import batch.
-- Raw sheet archive (sheet_rows, jsonb {"cells":[...]}) preserves the
-- Ledger workbook exactly; the typed extract (records) is built from it
-- by the import script for sheets whose first column is a stable ID,
-- and can be rebuilt at any time from sheet_rows.
-- Source: Engineering Research Ledger (Drive spreadsheet
-- 1PWLW4Rv7P1VIHKo9NGNMDNoK8lFdrV1QR18dXRHgCNE), imported 2026-10-25
-- under agent task 1018, ahead of the 2026-11-05 Gemini/ChatGPT cleanup.
-- The Ledger itself is NOT archived/renamed by this migration; that
-- rename is held until this import verifies (task 1018).

create schema if not exists engineering;

create table if not exists engineering.sources (
  id               bigint generated always as identity primary key,
  name             text not null,
  kind             text not null default 'drive-file',
  drive_file_id    text,
  drive_modified_at timestamptz,
  import_batch     uuid not null,
  imported_at      timestamptz not null default now(),
  note             text
);

create table if not exists engineering.sheet_rows (
  id           bigint generated always as identity primary key,
  source_id    bigint not null references engineering.sources(id),
  import_batch uuid not null,
  sheet_name   text not null,
  row_index    integer not null,
  data         jsonb not null,
  unique (source_id, import_batch, sheet_name, row_index)
);
create index if not exists eng_sheet_rows_sheet_idx
  on engineering.sheet_rows (sheet_name, import_batch);

-- Typed extract: one row per data row (header excluded) for sheets with
-- a stable ID in column 1. `fields` is the header->value map for the row;
-- title/status are lifted from the per-sheet columns named in the import
-- script (TITLE_COL / STATUS_COL) where those columns exist cleanly.
create table if not exists engineering.records (
  id           bigint generated always as identity primary key,
  source_id    bigint not null references engineering.sources(id),
  import_batch uuid not null,
  sheet_name   text not null,
  row_index    integer not null,
  record_id    text not null,
  title        text,
  status       text,
  fields       jsonb not null,
  unique (source_id, import_batch, sheet_name, row_index)
);
create index if not exists eng_records_sheet_idx
  on engineering.records (sheet_name, import_batch);
create index if not exists eng_records_record_id_idx
  on engineering.records (record_id);

alter table engineering.sources    enable row level security;
alter table engineering.sheet_rows enable row level security;
alter table engineering.records    enable row level security;

-- Grants (same shape as 0005, for this schema only)
grant usage on schema engineering to anon, authenticated, service_role;
grant all on all tables in schema engineering to service_role;
grant select, insert, update, delete on all tables in schema engineering to authenticated;
grant select on all tables in schema engineering to anon;
grant all on all sequences in schema engineering to service_role;
grant usage, select on all sequences in schema engineering to authenticated, anon;
alter default privileges in schema engineering grant all on tables to service_role;
alter default privileges in schema engineering grant all on sequences to service_role;
