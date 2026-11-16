-- Focused tests for migration-0026 (#5400): the restored guarded
-- recovery modes in agent.bind_repo_task_lane_session.
--
-- Run via test_migration_0026.sh, which starts a scratch Postgres,
-- applies THIS schema file first, then ../migration-0026.sql, then
-- migration-0026.test.sql (the scenarios) with ON_ERROR_STOP.
--
-- The schema here is the minimal subset of columns the function
-- touches (the base agent schema predates the record-copy
-- migrations in this directory and lives with the live DB).

create schema agent;

create table agent.tasks (
  id bigint primary key,
  queue text,
  status text,
  lease_until timestamptz,
  progress jsonb not null default '{}'::jsonb,
  attempts integer not null default 0,
  max_attempts integer,
  worker_id text,
  updated_at timestamptz not null default now()
);

create table agent.repo_task_lane_ledger (
  task_id bigint primary key,
  queue text,
  assigned_lane text,
  executed_lane text,
  exclusion_reason text,
  assigned_by text,
  session_id text,
  updated_at timestamptz not null default now()
);

create table agent.events (
  id bigserial primary key,
  worker_id text,
  task_id bigint,
  kind text,
  detail jsonb,
  created_at timestamptz not null default now()
);

-- Seed one claimed opencode-lane task (id 1) bound to ses_old in
-- both stores; p_lease is relative to now() ('-10 minutes' expired,
-- '+10 minutes' live).
create or replace function agent.test_seed(p_lease interval, p_attempts integer, p_max_attempts integer)
returns void language plpgsql as $$
begin
  truncate agent.events, agent.repo_task_lane_ledger, agent.tasks;
  insert into agent.tasks (id, queue, status, lease_until, progress, attempts, max_attempts, worker_id)
  values (1, 'repo-health-code-pilot', 'claimed', now() + p_lease,
          '{"lane": "opencode", "session_id": "ses_old"}'::jsonb,
          p_attempts, p_max_attempts, 'repo-worker-code');
  insert into agent.repo_task_lane_ledger (task_id, queue, assigned_lane, assigned_by, session_id)
  values (1, 'repo-health-code-pilot', 'opencode', 'repo-worker-code', 'ses_old');
end $$;

