-- 0001_agent.sql — shared agent state: the authoritative coordination layer.
-- Self-contained in schema `agent`. No cross-schema dependencies.
-- Design rule (2026-10-20): exactly-one-owner state lives in Postgres.
-- Leasing uses SELECT ... FOR UPDATE SKIP LOCKED so two workers can never
-- claim the same task. Mirrors the Turso muse-meta coord_* v1 shape
-- (workers/tasks/leases/events/checkpoints) for a clean cutover mapping.

create schema if not exists agent;

create table if not exists agent.workers (
  id            text primary key,
  role          text not null,
  display_name  text,
  status        text not null default 'idle'
                check (status in ('idle','working','blocked','offline')),
  last_heartbeat_at timestamptz,
  created_at    timestamptz not null default now()
);

create table if not exists agent.tasks (
  id            bigint generated always as identity primary key,
  queue         text not null default 'general',
  title         text not null,
  body          text,
  payload       jsonb not null default '{}'::jsonb,
  dedupe_key    text unique,
  status        text not null default 'queued'
                check (status in ('queued','claimed','done','failed','cancelled')),
  priority      integer not null default 100,
  worker_id     text references agent.workers(id),
  lease_until   timestamptz,
  attempts      integer not null default 0,
  result        jsonb,
  created_at    timestamptz not null default now(),
  updated_at    timestamptz not null default now(),
  claimed_at    timestamptz,
  completed_at  timestamptz
);
create index if not exists tasks_queue_status_idx
  on agent.tasks (queue, status, priority, created_at);

create table if not exists agent.leases (
  task_id       bigint primary key references agent.tasks(id) on delete cascade,
  worker_id     text not null references agent.workers(id),
  lease_until   timestamptz not null,
  heartbeat_at  timestamptz not null default now()
);

-- Append-only event history: nothing is silently lost, failures surface here.
create table if not exists agent.events (
  id            bigint generated always as identity primary key,
  ts            timestamptz not null default now(),
  worker_id     text,
  task_id       bigint references agent.tasks(id),
  kind          text not null,
  detail        jsonb not null default '{}'::jsonb
);
create index if not exists events_task_idx on agent.events (task_id, ts);

create table if not exists agent.checkpoints (
  worker_id     text not null references agent.workers(id),
  key           text not null,
  value         jsonb not null default '{}'::jsonb,
  updated_at    timestamptz not null default now(),
  primary key (worker_id, key)
);

create or replace function agent.claim_task(
  p_worker text, p_queue text default null, p_lease_seconds integer default 900)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  select * into t from agent.tasks
   where status = 'queued' and (p_queue is null or queue = p_queue)
   order by priority asc, created_at asc
   for update skip locked limit 1;
  if not found then return null; end if;
  update agent.tasks
     set status = 'claimed', worker_id = p_worker, claimed_at = now(),
         lease_until = now() + make_interval(secs => p_lease_seconds),
         attempts = attempts + 1, updated_at = now()
   where id = t.id returning * into t;
  insert into agent.leases (task_id, worker_id, lease_until, heartbeat_at)
  values (t.id, p_worker, t.lease_until, now())
  on conflict (task_id) do update
    set worker_id = excluded.worker_id,
        lease_until = excluded.lease_until, heartbeat_at = now();
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, t.id, 'claimed',
          jsonb_build_object('lease_seconds', p_lease_seconds));
  return t;
end $$;

create or replace function agent.complete_task(
  p_task_id bigint, p_worker text, p_result jsonb default '{}'::jsonb)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  update agent.tasks
     set status = 'done', result = p_result, completed_at = now(),
         updated_at = now(), lease_until = null
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   returning * into t;
  if not found then return null; end if;
  delete from agent.leases where task_id = p_task_id;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'completed', p_result);
  return t;
end $$;

create or replace function agent.fail_task(
  p_task_id bigint, p_worker text, p_error text, p_requeue boolean default true)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  update agent.tasks
     set status = case when p_requeue then 'queued' else 'failed' end,
         worker_id = null, lease_until = null, updated_at = now()
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   returning * into t;
  if not found then return null; end if;
  delete from agent.leases where task_id = p_task_id;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'failed',
          jsonb_build_object('error', p_error, 'requeued', p_requeue));
  return t;
end $$;

-- Requeue tasks whose lease expired (crashed/abandoned workers). Safe to
-- call from any cron; later this can move in-DB via pg_cron (see cutover plan).
create or replace function agent.expire_leases()
returns integer language plpgsql as $$
declare n integer;
begin
  with expired as (
    update agent.tasks set status = 'queued', worker_id = null,
           lease_until = null, updated_at = now()
     where status = 'claimed' and lease_until < now()
     returning id, worker_id)
  insert into agent.events (worker_id, task_id, kind)
  select worker_id, id, 'lease_expired' from expired;
  get diagnostics n = row_count;
  delete from agent.leases where lease_until < now();
  return n;
end $$;

alter table agent.workers     enable row level security;
alter table agent.tasks       enable row level security;
alter table agent.leases      enable row level security;
alter table agent.events      enable row level security;
alter table agent.checkpoints enable row level security;

-- Seed: the four designated workers from Drew's automation system + muse as coordinator.
insert into agent.workers (id, role, display_name) values
  ('muse',       'coordinator', 'Muse (chief of staff)'),
  ('sentinel',   'intake',      'Sentinel — intake / triage'),
  ('rnd',        'research',    'R&D — research'),
  ('engineering','build',       'Engineering — build / fix'),
  ('finance',    'money',       'Finance — money watch')
on conflict (id) do nothing;
