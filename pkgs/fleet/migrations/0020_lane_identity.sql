-- 0020: per-lane queue identity for completion attribution.
--
-- Lanes get their own ID so completions can be attributed to the lane
-- that claimed the work, WITHOUT changing claim routing: lanes keep
-- claiming as the shared per-repo worker (tasks.worker_id stays the
-- shared worker). A past incident (code#1534) FK-failed every claim
-- when a lane claimed under its own id, because tasks.worker_id has
-- an FK to agent.workers -- lane identity therefore lives in its own
-- column, never in worker_id.
--
-- Changes:
--   1. agent.lanes: registry mapping lane_id -> (worker_id, queue).
--      worker_id references agent.workers(id); a lane claims only as
--      that worker, only on that queue.
--   2. agent.tasks.lane_id (nullable, FK -> agent.lanes): the lane of
--      the CURRENT claim. Historical rows stay NULL. Every claim sets
--      it unconditionally, so a legacy re-claim with no lane clears it.
--   3. agent.claim_repo_task gains p_lane_id (default null). When it
--      is not null, the lane is validated BEFORE any task is selected:
--      unknown lane, worker mismatch, disabled lane, or queue mismatch
--      each raise and claim nothing. The rest of the body is the LIVE
--      definition (pg_get_functiondef, fetched 2026-11-02; the 0012
--      park/resume accounting version) unchanged in behaviour. The
--      'claimed' event detail gains 'lane_id'.
--   4. Seed the Muse cron surface registry (surface='claude-cron',
--      source_ref = cron job id). OpenChamber-surface lanes are NOT
--      seeded here; another change handles those.
--
-- Rollback:
--   begin;
--   drop function agent.claim_repo_task(text, text, integer, text);
--   -- restore the 0012 claim body:
--   create or replace function agent.claim_repo_task(
--     p_worker text, p_queue text, p_lease_seconds integer default 2700)
--   returns agent.tasks language plpgsql as $$
--   declare t agent.tasks; v_was_parked boolean;
--   begin
--     perform agent.expire_leases();
--     select * into t from agent.tasks
--      where queue = p_queue
--        and ( (status = 'queued'  and attempts < max_attempts
--               and (not_before is null or not_before <= now()))
--           or (status = 'parked'  and not_before is not null and not_before <= now()) )
--      order by priority asc, created_at asc
--      for update skip locked limit 1;
--     if not found then return null; end if;
--     v_was_parked := (t.status = 'parked');
--     update agent.tasks
--        set status = 'claimed', worker_id = p_worker, claimed_at = now(),
--            lease_until = now() + make_interval(secs => p_lease_seconds),
--            attempts = attempts + (case when v_was_parked then 0 else 1 end),
--            not_before = null, updated_at = now()
--      where id = t.id returning * into t;
--     insert into agent.leases (task_id, worker_id, lease_until, heartbeat_at)
--     values (t.id, p_worker, t.lease_until, now())
--     on conflict (task_id) do update
--       set worker_id = excluded.worker_id,
--           lease_until = excluded.lease_until, heartbeat_at = now();
--     insert into agent.events (worker_id, task_id, kind, detail)
--     values (p_worker, t.id, 'claimed',
--             jsonb_build_object('lease_seconds', p_lease_seconds,
--                                'via', 'claim_repo_task',
--                                'resumed_from_park', v_was_parked));
--     return t;
--   end $$;
--   alter table agent.tasks drop column lane_id;
--   drop table agent.lanes;
--   commit;
--   notify pgrst, 'reload schema';

begin;

create table agent.lanes (
  lane_id    text primary key,
  worker_id  text not null references agent.workers(id),
  queue      text not null,
  surface    text not null,
  source_ref text,
  enabled    boolean not null default true,
  created_at timestamptz not null default now()
);

alter table agent.tasks
  add column lane_id text references agent.lanes(lane_id);

drop function agent.claim_repo_task(text, text, integer);

create function agent.claim_repo_task(
  p_worker text, p_queue text, p_lease_seconds integer default 2700,
  p_lane_id text default null)
returns agent.tasks language plpgsql as $$
declare t agent.tasks; v_was_parked boolean; v_lane agent.lanes;
begin
  if p_lane_id is not null then
    select * into v_lane from agent.lanes where lane_id = p_lane_id;
    if not found then
      raise exception 'claim_repo_task: unknown lane_id %', p_lane_id;
    end if;
    if v_lane.worker_id <> p_worker then
      raise exception 'claim_repo_task: lane % belongs to worker %, not %',
        p_lane_id, v_lane.worker_id, p_worker;
    end if;
    if not v_lane.enabled then
      raise exception 'claim_repo_task: lane % is disabled', p_lane_id;
    end if;
    if v_lane.queue <> p_queue then
      raise exception 'claim_repo_task: lane % is for queue %, not %',
        p_lane_id, v_lane.queue, p_queue;
    end if;
  end if;
  perform agent.expire_leases();
  select * into t from agent.tasks
   where queue = p_queue
     and ( (status = 'queued'  and attempts < max_attempts
            and (not_before is null or not_before <= now()))
        or (status = 'parked'  and not_before is not null and not_before <= now()) )
   order by priority asc, created_at asc
   for update skip locked limit 1;
  if not found then return null; end if;
  v_was_parked := (t.status = 'parked');
  update agent.tasks
     set status = 'claimed', worker_id = p_worker, claimed_at = now(),
         lease_until = now() + make_interval(secs => p_lease_seconds),
         attempts = attempts + (case when v_was_parked then 0 else 1 end),
         lane_id = p_lane_id,
         not_before = null, updated_at = now()
   where id = t.id returning * into t;
  insert into agent.leases (task_id, worker_id, lease_until, heartbeat_at)
  values (t.id, p_worker, t.lease_until, now())
  on conflict (task_id) do update
    set worker_id = excluded.worker_id,
        lease_until = excluded.lease_until, heartbeat_at = now();
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, t.id, 'claimed',
          jsonb_build_object('lease_seconds', p_lease_seconds,
                             'via', 'claim_repo_task',
                             'resumed_from_park', v_was_parked,
                             'lane_id', p_lane_id));
  return t;
end $$;

insert into agent.lanes (lane_id, worker_id, queue, surface, source_ref) values
  ('code-a', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift'),
  ('code-b', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-b'),
  ('code-c', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-c'),
  ('code-d', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-d'),
  ('code-e', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-e'),
  ('code-f', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-f'),
  ('code-g', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-g'),
  ('code-h', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-h'),
  ('code-i', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-i'),
  ('code-j', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-j'),
  ('code-k', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-k'),
  ('code-l', 'repo-worker-code', 'repo-health-code-pilot', 'claude-cron', 'repo-queue-worker-shift-l'),
  ('opencode-a', 'repo-worker-opencode', 'repo-health-opencode-pilot', 'claude-cron', 'repo-queue-worker-shift-opencode'),
  ('openchamber-a', 'repo-worker-openchamber', 'repo-health-openchamber-pilot', 'claude-cron', 'repo-queue-worker-shift-openchamber'),
  ('openchamber-b', 'repo-worker-openchamber', 'repo-health-openchamber-pilot', 'claude-cron', 'repo-queue-worker-shift-openchamber-b'),
  ('nix-a', 'repo-worker-nix', 'repo-health-nix-pilot', 'claude-cron', 'repo-queue-worker-shift-nix');

commit;

notify pgrst, 'reload schema';
