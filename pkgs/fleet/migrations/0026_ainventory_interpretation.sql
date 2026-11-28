-- 0026_ainventory_interpretation.sql — interpretation pass + findings event
-- store for the model-data pipeline (issue developing-today/code#1248,
-- Phase 2 slice 2; slice tracker #10040). Design:
-- workspace/goals/supabase-muse-dev-platform-backups/files/
--   aa-event-log-design-2026-10-26.md (not on this host; #1248's body
--   restates the requirements).
--
-- Slice 2 has two halves, both additive beside the event log from
-- 0013_aa_event_log.sql:
--
--   1. INTERPRETATION PASS. Every non-empty gather batch gets one
--      interpretation run: the batch note (an `ainventory.notes` row of
--      kind 'agent_note'), the structured record of what was filed, and
--      the follow-up research references. A batch with nothing inserted
--      is never interpreted — there is nothing to read.
--
--   2. FINDINGS EVENT STORE. #1248 carries a 2026-10-27 pointer: a
--      permanent append-only findings store lives at
--      files/findings-event-store.md in the keep-the-four-repos-healthy
--      goal workspace, with structured entries (subject, finding,
--      evidence, significance, action) written by pool-intelligence,
--      pool-composition-review and the steward. This pipeline must
--      SUBSUME that store, not start a parallel one, so the same entry
--      shape lands here as rows: subject, finding, evidence,
--      significance, action, plus provenance and the batch it came from.
--
-- Append-only is enforced the same way as the event log: BEFORE
-- UPDATE/DELETE triggers raise, and UPDATE/DELETE grants are revoked
-- from every API role. An interpretation is a dated record of what an
-- agent concluded from a batch at that moment; correcting one means
-- writing a new row, never editing the old one.
--
-- Re-runnable: CREATE IF NOT EXISTS / CREATE OR REPLACE throughout.

create schema if not exists ainventory;

-- ------------------------------------------------------------- findings
-- One row per finding. Field names follow the findings-event-store
-- entry format verbatim so an entry can be lifted from that store into
-- this table without translation (source_ref carries the store path and
-- entry heading it came from).
create table if not exists ainventory.findings (
  id              bigint generated always as identity primary key,
  subject         text not null,      -- model, provider, pool or topic
  finding         text not null,      -- the claim, one sentence or two
  evidence        text,               -- where it came from / how it was seen
  significance    text,               -- why it matters to the fleet
  action          text,               -- what was or should be done
  severity        text
                  check (severity in ('info','watch','risk') or severity is null),
  model_name      text,
  provider        text,
  ingest_batch_id uuid references ainventory.ingest_batches(id),
  source_kind     text,               -- aa | modelsdev | firstparty |
                                      -- thirdparty | probe | provider-doc
                                      -- | findings-event-store | agent
  source_ref      text,
  author          text not null,      -- agent identity or 'steward'
  created_at      timestamptz not null default now(),
  raw             jsonb
);
create index if not exists findings_subject_idx
  on ainventory.findings (subject, created_at desc);
create index if not exists findings_model_idx
  on ainventory.findings (model_name, created_at desc);
create index if not exists findings_batch_idx
  on ainventory.findings (ingest_batch_id);

-- --------------------------------------------------- interpretation runs
-- One row per completed interpretation pass over one batch. note_id
-- points at the batch note in ainventory.notes; followups lists the
-- research trail filed for genuinely new models, each entry
-- {model, task_id, issue_url, status} where status is 'filed' or
-- 'not_filed' WITH a reason — a follow-up that was not filed is still
-- recorded, never silently dropped.
create table if not exists ainventory.interpretation_runs (
  id               bigint generated always as identity primary key,
  ingest_batch_id  uuid not null references ainventory.ingest_batches(id),
  note_id          bigint references ainventory.notes(id),
  author           text not null,     -- agent identity that ran the pass
  model_used       text,              -- model that produced the note
  changed          jsonb not null default '{}'::jsonb,
                                      -- what changed in the batch:
                                      -- inserted counts, new models,
                                      -- superseded values
  new_models       jsonb not null default '[]'::jsonb,
  followups        jsonb not null default '[]'::jsonb,
  findings_count   integer not null default 0,
  created_at       timestamptz not null default now(),
  raw              jsonb
);
create index if not exists interpretation_runs_batch_idx
  on ainventory.interpretation_runs (ingest_batch_id, created_at desc);

-- ------------------------------------------------ awaiting view
-- The event log's own non-empty batches that nobody has read yet.
--
-- Scoped to batches that inserted event-log facts (measurement_events,
-- price_events, provider_policies). Batches from other pipelines that
-- share ingest_batches — coding-plan-watch's plan_events, a workbook
-- backfill — are not the gather loop's work and never appear here.
--
-- A batch note alone is not a completed pass: the run row records the
-- follow-up research trail and findings too.
create or replace view ainventory.v_batches_awaiting_interpretation as
select b.*
from ainventory.ingest_batches b
where b.source_kind = 'gatherer'
  and exists (
        select 1 from jsonb_each_text(b.inserted_counts) as e(k, v)
         where e.k in ('measurement_events', 'price_events',
                       'provider_policies')
           and e.v ~ '^[0-9]+(\.[0-9]+)?$'
           and e.v::numeric > 0)
  and not exists (
        select 1 from ainventory.interpretation_runs r
         where r.ingest_batch_id = b.id);

-- ------------------------------------------- append-only enforcement
-- ainventory.reject_event_mutation() was created in 0013; reuse it so
-- the refusal message names the table that refused.
drop trigger if exists findings_no_mutate on ainventory.findings;
create trigger findings_no_mutate
  before update or delete on ainventory.findings
  for each row execute function ainventory.reject_event_mutation();

drop trigger if exists interpretation_runs_no_mutate
  on ainventory.interpretation_runs;
create trigger interpretation_runs_no_mutate
  before update or delete on ainventory.interpretation_runs
  for each row execute function ainventory.reject_event_mutation();

-- ------------------------------------------------------------- RLS/grants
alter table ainventory.findings             enable row level security;
alter table ainventory.interpretation_runs  enable row level security;

grant select, insert on ainventory.findings,
                        ainventory.interpretation_runs
  to service_role;
grant usage, select on all sequences in schema ainventory to service_role;
revoke update, delete on ainventory.findings,
                         ainventory.interpretation_runs
  from service_role, authenticated, anon;

grant select on ainventory.v_batches_awaiting_interpretation
  to service_role;
