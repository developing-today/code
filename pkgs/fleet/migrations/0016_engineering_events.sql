-- 0016_engineering_events.sql
-- engineering.events: append-only event log for engineering / hardware
-- findings (first writer: the weekly hardware roundup). The ledger mirror
-- (engineering.sources / sheet_rows / records) stays the batch-import copy
-- of the Engineering Research Ledger workbook; runtime findings do NOT
-- belong in the mirror, they land here — the same split ainventory uses
-- (sheet_rows mirror vs measurement_events / price_events logs).
-- engineering.ingest_batches mirrors ainventory.ingest_batches so append
-- writers (engineering_append.py) leave the same audit trail.
-- Applied to muse-dev2 2026-10-31; re-runnable.

create table if not exists engineering.events (
  id bigint generated always as identity primary key,
  run_date date not null,
  topic text not null,
  headline text not null,
  summary text,
  source_url text,
  source_name text,
  event_date date,
  dedupe_key text not null,
  payload jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  constraint engineering_events_dedupe_key_key unique (dedupe_key)
);
alter table engineering.events enable row level security;
grant select on engineering.events to anon, authenticated;
grant all on engineering.events to postgres, service_role;
create index if not exists engineering_events_run_date_idx
  on engineering.events (run_date desc);
create index if not exists engineering_events_topic_idx
  on engineering.events (topic, run_date desc);

create table if not exists engineering.ingest_batches (
  id uuid primary key default gen_random_uuid(),
  source_kind text not null,
  source_ref text,
  ran_at timestamptz not null default now(),
  inserted_counts jsonb not null default '{}'::jsonb,
  note text,
  created_at timestamptz not null default now()
);
alter table engineering.ingest_batches enable row level security;
grant select on engineering.ingest_batches to anon, authenticated;
grant all on engineering.ingest_batches to postgres, service_role;
