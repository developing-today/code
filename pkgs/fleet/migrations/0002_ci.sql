-- 0002_ci.sql — GitHub CI event ingest. Self-contained in schema `ci`.
-- The github-ci Edge Function inserts one row per workflow_run webhook.
-- dedupe_key makes webhook redeliveries harmless.

create schema if not exists ci;

create table if not exists ci.events (
  id            bigint generated always as identity primary key,
  received_at   timestamptz not null default now(),
  source        text not null default 'github',
  event_type    text not null default 'workflow_run',
  action        text,
  repo          text not null,
  workflow_name text,
  workflow_id   bigint,
  run_id        bigint,
  run_attempt   integer,
  branch        text,
  sha           text,
  status        text,
  conclusion    text,
  actor         text,
  html_url      text,
  ci_started_at timestamptz,
  ci_updated_at timestamptz,
  payload       jsonb not null,
  dedupe_key    text unique
);
create index if not exists ci_events_repo_time_idx on ci.events (repo, received_at desc);
create index if not exists ci_events_conclusion_idx on ci.events (conclusion, received_at desc);

create or replace view ci.recent_failures as
  select id, received_at, repo, workflow_name, branch, sha, run_id,
         run_attempt, actor, html_url
    from ci.events
   where conclusion in ('failure','timed_out','startup_failure','action_required')
   order by received_at desc;

alter table ci.events enable row level security;
