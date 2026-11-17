-- 0009_repo_snapshots.sql — Phase B part 1: repo snapshot layer.
-- Maintained by ~/tools/fleet-fetch.py (deterministic, zero agent tokens):
-- one row per repo with current head, open PRs/issues (capped jsonb), and a
-- CI summary, refreshed on a systemd timer. Additive only; idempotent.

create table if not exists agent.repo_snapshots (
  repo             text primary key,
  head_sha         text,
  default_branch   text,
  open_prs         jsonb not null default '[]'::jsonb,
  open_issues      jsonb not null default '[]'::jsonb,
  open_pr_count    integer not null default 0,
  open_issue_count integer not null default 0,
  ci_summary       jsonb not null default '{}'::jsonb,
  fetched_at       timestamptz not null default now(),
  fetch_source     text not null default 'fetcher'
);

create table if not exists agent.watermarks (
  key        text primary key,
  value      jsonb not null default '{}'::jsonb,
  updated_at timestamptz not null default now()
);

alter table agent.repo_snapshots enable row level security;
alter table agent.watermarks     enable row level security;

-- Grants, following the 0005 pattern for schema `agent`.
grant usage on schema agent to anon, authenticated, service_role;
grant all on agent.repo_snapshots, agent.watermarks to service_role;
grant select, insert, update, delete on agent.repo_snapshots, agent.watermarks to authenticated;
grant select on agent.repo_snapshots, agent.watermarks to anon;
alter default privileges in schema agent grant all on tables to service_role;
alter default privileges in schema agent grant all on sequences to service_role;
