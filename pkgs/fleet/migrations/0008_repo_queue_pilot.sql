-- 0008_repo_queue_pilot.sql — repo-health queue/worker pilot extension.
-- Design: ~/workspace/goals/keep-the-four-repos-healthy/files/
--         queue-worker-pilot-design-2026-10-25.md (v1.1)
-- ISOLATION (v1.1 amendment): additive only. The existing global
-- agent.claim_task / agent.fail_task / agent.complete_task are NOT touched.
-- All pilot behavior lives in NEW functions (claim_repo_task, fail_repo_task,
-- enqueue_repo_task, heartbeat_task, checkpoint_task, park_task, block_task)
-- that only pilot queues call. Existing tasks/queues are preserved as-is.
-- Pre-migration snapshot: migrations/0008_pre_snapshot/.
-- Rollback: migrations/0008_repo_queue_pilot_rollback.sql. Idempotent.

-- ---------------------------------------------------------------- worker ---
insert into agent.workers (id, role, display_name) values
  ('repo-worker-code', 'build', 'Repo worker — developing-today/code pilot')
on conflict (id) do nothing;

-- --------------------------------------------------------------- columns ---
alter table agent.tasks add column if not exists not_before   timestamptz;
alter table agent.tasks add column if not exists max_attempts integer not null default 3;
alter table agent.tasks add column if not exists progress     jsonb not null default '{}'::jsonb;
alter table agent.tasks add column if not exists last_error   text;

-- --------------------------------------------------------- status values ---
-- Original constraint from 0001: tasks_status_check (5 values). Recreate it
-- with the three pilot states added. No existing row uses the new values, so
-- widening the CHECK cannot fail on current data.
alter table agent.tasks drop constraint if exists tasks_status_check;
alter table agent.tasks add constraint tasks_status_check
  check (status in ('queued','claimed','done','failed','cancelled',
                    'parked','blocked','needs_review'));

-- ----------------------------------------------------------------- index ---
-- Claim selection: only rows that can become claimable.
create index if not exists tasks_claimable_idx
  on agent.tasks (queue, status, priority, created_at)
  where status in ('queued','parked');

-- ------------------------------------------------------- enqueue (pilot) ---
-- Insert-or-return-existing on dedupe_key; 'enqueued' event only on a real
-- insert. SQL-level validation is intentionally light (schema tag + repo);
-- the repo-queue.py helper validates the full repo-task/v1 packet.
create or replace function agent.enqueue_repo_task(
  p_queue text, p_title text, p_body text default null,
  p_payload jsonb default '{}'::jsonb, p_dedupe_key text default null,
  p_priority integer default 50, p_not_before timestamptz default null,
  p_max_attempts integer default null)
returns agent.tasks language plpgsql as $$
declare t agent.tasks; inserted boolean := false;
begin
  if p_payload ? 'schema' and p_payload ->> 'schema' <> 'repo-task/v1' then
    raise exception 'enqueue_repo_task: unsupported payload schema %',
      p_payload ->> 'schema';
  end if;
  if p_payload ? 'repo' and p_payload ->> 'repo' <> 'developing-today/code' then
    raise exception 'enqueue_repo_task: pilot repo is developing-today/code, got %',
      p_payload ->> 'repo';
  end if;
  insert into agent.tasks (queue, title, body, payload, dedupe_key, priority,
                           not_before, max_attempts)
  values (p_queue, p_title, p_body, coalesce(p_payload, '{}'::jsonb),
          p_dedupe_key, coalesce(p_priority, 50), p_not_before,
          coalesce(p_max_attempts, 3))
  on conflict (dedupe_key) do nothing
  returning * into t;
  if found then
    inserted := true;
  else
    select * into t from agent.tasks where dedupe_key = p_dedupe_key;
    if not found then return null; end if;  -- null dedupe key race; caller retries
  end if;
  if inserted then
    insert into agent.events (worker_id, task_id, kind, detail)
    values (null, t.id, 'enqueued',
            jsonb_build_object('queue', p_queue, 'dedupe_key', p_dedupe_key,
                               'via', 'enqueue_repo_task'));
  end if;
  return t;
end $$;

-- --------------------------------------------------------- claim (pilot) ---
-- Same atomic core as claim_task (expire first, SKIP LOCKED) but pilot-only:
-- named queue required, due parked tasks are candidates, not_before and the
-- per-task attempt cap are respected. The global claim_task is untouched.
create or replace function agent.claim_repo_task(
  p_worker text, p_queue text, p_lease_seconds integer default 2700)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  perform agent.expire_leases();
  select * into t from agent.tasks
   where queue = p_queue
     and attempts < max_attempts
     and ( (status = 'queued'  and (not_before is null or not_before <= now()))
        or (status = 'parked'  and not_before is not null and not_before <= now()) )
   order by priority asc, created_at asc
   for update skip locked limit 1;
  if not found then return null; end if;
  update agent.tasks
     set status = 'claimed', worker_id = p_worker, claimed_at = now(),
         lease_until = now() + make_interval(secs => p_lease_seconds),
         attempts = attempts + 1, not_before = null, updated_at = now()
   where id = t.id returning * into t;
  insert into agent.leases (task_id, worker_id, lease_until, heartbeat_at)
  values (t.id, p_worker, t.lease_until, now())
  on conflict (task_id) do update
    set worker_id = excluded.worker_id,
        lease_until = excluded.lease_until, heartbeat_at = now();
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, t.id, 'claimed',
          jsonb_build_object('lease_seconds', p_lease_seconds,
                             'via', 'claim_repo_task'));
  return t;
end $$;

-- ------------------------------------------------------------- heartbeat ---
-- Owner-only. Raises on a task not claimed by p_worker so callers cannot
-- mistake a lost claim for a successful heartbeat.
create or replace function agent.heartbeat_task(
  p_task_id bigint, p_worker text, p_extend_seconds integer default 900,
  p_progress jsonb default null)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  update agent.tasks
     set lease_until = now() + make_interval(secs => p_extend_seconds),
         progress = case when p_progress is null then progress
                         else progress || p_progress end,
         updated_at = now()
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   returning * into t;
  if not found then
    raise exception 'heartbeat_task: task % is not claimed by %', p_task_id, p_worker;
  end if;
  update agent.leases set lease_until = t.lease_until, heartbeat_at = now()
   where task_id = p_task_id;
  update agent.workers set last_heartbeat_at = now(), status = 'working'
   where id = p_worker;
  return t;
end $$;

-- ------------------------------------------------------------ checkpoint ---
create or replace function agent.checkpoint_task(
  p_task_id bigint, p_worker text, p_stage text,
  p_progress jsonb default '{}'::jsonb)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  update agent.tasks
     set progress = progress || coalesce(p_progress, '{}'::jsonb)
                    || jsonb_build_object('stage', p_stage,
                                          'stage_at', to_jsonb(now())),
         updated_at = now()
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   returning * into t;
  if not found then
    raise exception 'checkpoint_task: task % is not claimed by %', p_task_id, p_worker;
  end if;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'checkpoint',
          jsonb_build_object('stage', p_stage,
                             'progress', coalesce(p_progress, '{}'::jsonb)));
  return t;
end $$;

-- ------------------------------------------------------------------ park ---
create or replace function agent.park_task(
  p_task_id bigint, p_worker text, p_reason text,
  p_delay_seconds integer default 1200, p_progress jsonb default '{}'::jsonb)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  update agent.tasks
     set status = 'parked', worker_id = null, lease_until = null,
         not_before = now() + make_interval(secs => p_delay_seconds),
         progress = progress || coalesce(p_progress, '{}'::jsonb),
         result = coalesce(result, '{}'::jsonb)
                  || jsonb_build_object('park_reason', p_reason),
         updated_at = now()
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   returning * into t;
  if not found then
    raise exception 'park_task: task % is not claimed by %', p_task_id, p_worker;
  end if;
  delete from agent.leases where task_id = p_task_id;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'parked',
          jsonb_build_object('reason', p_reason,
                             'delay_seconds', p_delay_seconds));
  return t;
end $$;

-- ----------------------------------------------------------------- block ---
create or replace function agent.block_task(
  p_task_id bigint, p_worker text, p_reason_class text, p_blocker text,
  p_progress jsonb default '{}'::jsonb)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  update agent.tasks
     set status = 'blocked', worker_id = null, lease_until = null,
         progress = progress || coalesce(p_progress, '{}'::jsonb),
         result = coalesce(result, '{}'::jsonb)
                  || jsonb_build_object('reason_class', p_reason_class,
                                        'blocker', p_blocker),
         last_error = p_blocker, updated_at = now()
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   returning * into t;
  if not found then
    raise exception 'block_task: task % is not claimed by %', p_task_id, p_worker;
  end if;
  delete from agent.leases where task_id = p_task_id;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'blocked',
          jsonb_build_object('reason_class', p_reason_class,
                             'blocker', p_blocker));
  return t;
end $$;

-- ------------------------------------------------------------ fail (pilot) ---
-- Attempt policy (design): transient failures requeue with backoff — 15 min
-- after attempt 1, 45 min after attempt 2 — while attempts remain. When the
-- cap is hit, or the failure is deterministic, the task ends 'failed', or
-- 'blocked' when p_reason_class = 'blocked'. The global fail_task is untouched.
create or replace function agent.fail_repo_task(
  p_task_id bigint, p_worker text, p_error text,
  p_reason_class text default 'transient')
returns agent.tasks language plpgsql as $$
declare t agent.tasks; requeue boolean; delay_s integer;
begin
  select * into t from agent.tasks
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   for update;
  if not found then
    raise exception 'fail_repo_task: task % is not claimed by %', p_task_id, p_worker;
  end if;
  requeue := (coalesce(p_reason_class, 'transient') = 'transient')
             and t.attempts < t.max_attempts;
  delay_s := case when t.attempts <= 1 then 900 else 2700 end;
  update agent.tasks
     set status = case when requeue then 'queued'
                       when p_reason_class = 'blocked' then 'blocked'
                       else 'failed' end,
         worker_id = null, lease_until = null,
         not_before = case when requeue
                           then now() + make_interval(secs => delay_s)
                           else not_before end,
         last_error = p_error,
         result = coalesce(result, '{}'::jsonb)
                  || jsonb_build_object('reason_class',
                                        coalesce(p_reason_class, 'transient')),
         updated_at = now()
   where id = p_task_id returning * into t;
  delete from agent.leases where task_id = p_task_id;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'failed',
          jsonb_build_object('error', p_error,
                             'reason_class', coalesce(p_reason_class, 'transient'),
                             'requeued', requeue, 'via', 'fail_repo_task'));
  return t;
end $$;

-- ------------------------------------------------------------ health view ---
-- One row per queue. Simplification: terminal outcome counts come from the
-- events stream (kinds 'completed'/'failed') joined back to the task's queue,
-- because failed/cancelled tasks have no completed_at timestamp.
create or replace view agent.repo_queue_health as
select
  t.queue,
  count(*) filter (where t.status = 'queued')       as queued_count,
  count(*) filter (where t.status = 'claimed')      as claimed_count,
  count(*) filter (where t.status = 'parked')       as parked_count,
  count(*) filter (where t.status = 'blocked')      as blocked_count,
  count(*) filter (where t.status = 'needs_review') as needs_review_count,
  count(*) filter (where t.status = 'done')         as done_count,
  count(*) filter (where t.status = 'failed')       as failed_count,
  count(*) filter (where t.status = 'cancelled')    as cancelled_count,
  max(now() - t.created_at) filter (where t.status = 'queued')
    as oldest_queued_age,
  max(now() - t.created_at) filter (where t.status = 'parked')
    as oldest_parked_age,
  count(*) filter (where t.status = 'claimed'
                   and t.lease_until < now() + interval '5 minutes')
    as claimed_expiring_within_5m,
  (select jsonb_object_agg(a.attempts, a.n)
     from (select attempts, count(*) as n from agent.tasks x
            where x.queue = t.queue group by attempts) a)
    as attempts_distribution,
  (select count(*) from agent.events e join agent.tasks x on x.id = e.task_id
    where x.queue = t.queue and e.kind in ('completed','failed')
      and e.ts > now() - interval '24 hours')
    as terminal_events_24h,
  (select count(*) from agent.events e join agent.tasks x on x.id = e.task_id
    where x.queue = t.queue and e.kind in ('completed','failed')
      and e.ts > now() - interval '7 days')
    as terminal_events_7d
from agent.tasks t
group by t.queue;

-- ----------------------------------------------------------------- grants ---
-- Match 0005_grants.sql: service_role + authenticated may execute functions;
-- the view gets table-style select grants.
grant execute on function agent.enqueue_repo_task(text, text, text, jsonb, text, integer, timestamptz, integer) to service_role, authenticated;
grant execute on function agent.claim_repo_task(text, text, integer)       to service_role, authenticated;
grant execute on function agent.heartbeat_task(bigint, text, integer, jsonb) to service_role, authenticated;
grant execute on function agent.checkpoint_task(bigint, text, text, jsonb) to service_role, authenticated;
grant execute on function agent.park_task(bigint, text, text, integer, jsonb) to service_role, authenticated;
grant execute on function agent.block_task(bigint, text, text, text, jsonb) to service_role, authenticated;
grant execute on function agent.fail_repo_task(bigint, text, text, text)   to service_role, authenticated;
grant select on agent.repo_queue_health to service_role, authenticated, anon;
