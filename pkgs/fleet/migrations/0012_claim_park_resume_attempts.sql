-- 0012: park/resume must not consume attempts (queue-pilot accounting fix)
--
-- Problem (found 2026-10-26, fleet operations): agent.claim_repo_task (0008)
-- incremented attempts on EVERY claim, including resumes of tasks that had
-- parked properly to wait on CI. Faithfully shepherded tasks exhausted their
-- caps by being resumed: tasks 1109 (#229, 6/6), 1173, 1175, 1210 sat parked
-- at cap -- one park-lapse away from dead-in-queue -- and tasks 1149/1157 hit
-- their caps through repeated claim evaporation compounded by the same
-- accounting. Operator cap raises were the only remedy.
--
-- Fix, two parts, both inside the pilot's own claim function (the global
-- agent.claim_task / agent.fail_task remain untouched, per the v1.1
-- isolation rule):
--   1. A due parked task is claimable regardless of attempts: the
--      attempts < max_attempts gate now applies only to fresh (queued)
--      claims. Parking is the prescribed wait state, not a failure.
--   2. Claiming a parked task does not increment attempts. Fresh claims
--      from queued still do, so evaporation/failure accounting is
--      unchanged: a claim that ends without complete/park/block/fail has
--      still consumed exactly one attempt.
-- The 'claimed' event detail gains resumed_from_park so resumes stay
-- distinguishable from fresh claims in the event log.
--
-- Rollback: re-apply the claim_repo_task body from
-- 0008_repo_queue_pilot.sql (no data changes to undo; attempts already
-- consumed under the old rule are not refunded).

create or replace function agent.claim_repo_task(
  p_worker text, p_queue text, p_lease_seconds integer default 2700)
returns agent.tasks language plpgsql as $$
declare t agent.tasks; v_was_parked boolean;
begin
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
                             'resumed_from_park', v_was_parked));
  return t;
end $$;
