-- 0017_workbook_final_captures.sql
-- Final one-time captures of the deprecated fleet workbooks (Drew,
-- 2026-10-31): Automation Meta, Research & Discovery Operations and
-- Finance Operations were deprecated 2026-10-22 and their Sheets are
-- frozen forever (nothing further is added to them; no refresh job
-- exists or will exist). Each gets a schema in the platform's
-- one-schema-per-domain style:
--   sources       -- the workbook, marked FINAL CAPTURE
--   sheet_rows    -- COMPLETE raw capture: every row of every tab as
--                    JSON cells {cells:[...]}, row_index = physical
--                    1-based sheet row (the engineering convention)
--   events        -- the queryable layer, event-shaped (deviation
--                    from engineering's `records`, by design): every
--                    non-header, non-blank row becomes a typed event
--                    (kind: fact / reference / opinion / note /
--                    record, by tab semantics) with record_id, title
--                    and status picked by the same header-candidate
--                    rules the engineering import used (proven to
--                    reproduce its 331 records exactly), plus
--                    event_date / source_ref / topics extraction and
--                    the full header->value map in `fields` — these
--                    domains are fed by tasks as event streams from
--                    here, so the capture lands in the shape the
--                    streams will use.
--   ingest_batches -- audit row for the capture run (mirrors
--                    engineering.ingest_batches).
-- The Engineering Research Ledger refresh (new batch in the existing
-- engineering schema) is data, not DDL, and lives in the capture
-- run's ingest_batches row.
-- Applied to muse-dev2 2026-10-31; re-runnable.
-- NOTE: the three schemas are SQL-accessible; they are not in the
-- PostgREST exposed-schema list (platform config), matching that
-- nothing at runtime REST-reads them.

create schema if not exists automation;
grant usage on schema automation to anon, authenticated, service_role;
create table if not exists automation.sources (
  id bigint generated always as identity primary key,
  name text not null,
  kind text not null,
  drive_file_id text,
  drive_modified_at timestamptz,
  import_batch uuid not null,
  imported_at timestamptz not null default now(),
  note text
);
create table if not exists automation.sheet_rows (
  id bigint generated always as identity primary key,
  source_id bigint not null,
  import_batch uuid not null,
  sheet_name text not null,
  row_index integer not null,
  data jsonb not null,
  constraint automation_sheet_rows_dedupe_key unique (source_id, import_batch, sheet_name, row_index)
);
create table if not exists automation.events (
  id bigint generated always as identity primary key,
  source_id bigint not null,
  import_batch uuid not null,
  sheet_name text not null,
  row_index integer not null,
  kind text not null,
  record_id text,
  title text,
  status text,
  event_date date,
  source_ref text,
  topics text[] not null default '{}',
  fields jsonb not null,
  dedupe_key text not null,
  created_at timestamptz not null default now(),
  constraint automation_events_batch_row_key unique (import_batch, sheet_name, row_index),
  constraint automation_events_dedupe_key_key unique (dedupe_key)
);
create table if not exists automation.ingest_batches (
  id uuid primary key default gen_random_uuid(),
  source_kind text not null,
  source_ref text,
  ran_at timestamptz not null default now(),
  inserted_counts jsonb not null default '{}'::jsonb,
  note text,
  created_at timestamptz not null default now()
);
alter table automation.sources enable row level security;
grant select on automation.sources to anon, authenticated;
grant all on automation.sources to postgres, service_role;
alter table automation.sheet_rows enable row level security;
grant select on automation.sheet_rows to anon, authenticated;
grant all on automation.sheet_rows to postgres, service_role;
alter table automation.events enable row level security;
grant select on automation.events to anon, authenticated;
grant all on automation.events to postgres, service_role;
alter table automation.ingest_batches enable row level security;
grant select on automation.ingest_batches to anon, authenticated;
grant all on automation.ingest_batches to postgres, service_role;
create index if not exists automation_events_kind_idx on automation.events (kind, sheet_name);
create index if not exists automation_events_date_idx on automation.events (event_date desc);
create index if not exists automation_events_topics_idx on automation.events using gin (topics);
create index if not exists automation_sheet_rows_batch_idx on automation.sheet_rows (import_batch, sheet_name);
create schema if not exists research;
grant usage on schema research to anon, authenticated, service_role;
create table if not exists research.sources (
  id bigint generated always as identity primary key,
  name text not null,
  kind text not null,
  drive_file_id text,
  drive_modified_at timestamptz,
  import_batch uuid not null,
  imported_at timestamptz not null default now(),
  note text
);
create table if not exists research.sheet_rows (
  id bigint generated always as identity primary key,
  source_id bigint not null,
  import_batch uuid not null,
  sheet_name text not null,
  row_index integer not null,
  data jsonb not null,
  constraint research_sheet_rows_dedupe_key unique (source_id, import_batch, sheet_name, row_index)
);
create table if not exists research.events (
  id bigint generated always as identity primary key,
  source_id bigint not null,
  import_batch uuid not null,
  sheet_name text not null,
  row_index integer not null,
  kind text not null,
  record_id text,
  title text,
  status text,
  event_date date,
  source_ref text,
  topics text[] not null default '{}',
  fields jsonb not null,
  dedupe_key text not null,
  created_at timestamptz not null default now(),
  constraint research_events_batch_row_key unique (import_batch, sheet_name, row_index),
  constraint research_events_dedupe_key_key unique (dedupe_key)
);
create table if not exists research.ingest_batches (
  id uuid primary key default gen_random_uuid(),
  source_kind text not null,
  source_ref text,
  ran_at timestamptz not null default now(),
  inserted_counts jsonb not null default '{}'::jsonb,
  note text,
  created_at timestamptz not null default now()
);
alter table research.sources enable row level security;
grant select on research.sources to anon, authenticated;
grant all on research.sources to postgres, service_role;
alter table research.sheet_rows enable row level security;
grant select on research.sheet_rows to anon, authenticated;
grant all on research.sheet_rows to postgres, service_role;
alter table research.events enable row level security;
grant select on research.events to anon, authenticated;
grant all on research.events to postgres, service_role;
alter table research.ingest_batches enable row level security;
grant select on research.ingest_batches to anon, authenticated;
grant all on research.ingest_batches to postgres, service_role;
create index if not exists research_events_kind_idx on research.events (kind, sheet_name);
create index if not exists research_events_date_idx on research.events (event_date desc);
create index if not exists research_events_topics_idx on research.events using gin (topics);
create index if not exists research_sheet_rows_batch_idx on research.sheet_rows (import_batch, sheet_name);
create schema if not exists finance;
grant usage on schema finance to anon, authenticated, service_role;
create table if not exists finance.sources (
  id bigint generated always as identity primary key,
  name text not null,
  kind text not null,
  drive_file_id text,
  drive_modified_at timestamptz,
  import_batch uuid not null,
  imported_at timestamptz not null default now(),
  note text
);
create table if not exists finance.sheet_rows (
  id bigint generated always as identity primary key,
  source_id bigint not null,
  import_batch uuid not null,
  sheet_name text not null,
  row_index integer not null,
  data jsonb not null,
  constraint finance_sheet_rows_dedupe_key unique (source_id, import_batch, sheet_name, row_index)
);
create table if not exists finance.events (
  id bigint generated always as identity primary key,
  source_id bigint not null,
  import_batch uuid not null,
  sheet_name text not null,
  row_index integer not null,
  kind text not null,
  record_id text,
  title text,
  status text,
  event_date date,
  source_ref text,
  topics text[] not null default '{}',
  fields jsonb not null,
  dedupe_key text not null,
  created_at timestamptz not null default now(),
  constraint finance_events_batch_row_key unique (import_batch, sheet_name, row_index),
  constraint finance_events_dedupe_key_key unique (dedupe_key)
);
create table if not exists finance.ingest_batches (
  id uuid primary key default gen_random_uuid(),
  source_kind text not null,
  source_ref text,
  ran_at timestamptz not null default now(),
  inserted_counts jsonb not null default '{}'::jsonb,
  note text,
  created_at timestamptz not null default now()
);
alter table finance.sources enable row level security;
grant select on finance.sources to anon, authenticated;
grant all on finance.sources to postgres, service_role;
alter table finance.sheet_rows enable row level security;
grant select on finance.sheet_rows to anon, authenticated;
grant all on finance.sheet_rows to postgres, service_role;
alter table finance.events enable row level security;
grant select on finance.events to anon, authenticated;
grant all on finance.events to postgres, service_role;
alter table finance.ingest_batches enable row level security;
grant select on finance.ingest_batches to anon, authenticated;
grant all on finance.ingest_batches to postgres, service_role;
create index if not exists finance_events_kind_idx on finance.events (kind, sheet_name);
create index if not exists finance_events_date_idx on finance.events (event_date desc);
create index if not exists finance_events_topics_idx on finance.events using gin (topics);
create index if not exists finance_sheet_rows_batch_idx on finance.sheet_rows (import_batch, sheet_name);
