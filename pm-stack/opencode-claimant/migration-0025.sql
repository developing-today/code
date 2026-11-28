-- migration-0025: claim must never hand out a payload-less packet
-- (#5444), and payload-less rows stop circulating.
--
-- agent.claim_repo_task (migration-0023) selects queued/due-parked
-- candidates with no payload predicate. A row whose payload is NULL
-- or '{}' hands the worker an unroutable packet — the 2026-11-04
-- incident, where a payload-null claim crashed the worker's claim
-- loop. The live queue runs the Turso backend, whose verb_claim in
-- pkgs/fleet/crates/fleet-queue/src/turso_queue.rs carries the same
-- fix (candidate predicate + validate_packet guard + quarantine
-- sweep); this migration is the Postgres store's parity record, in
-- the same shape, for the store these functions still define.
--
-- Two changes to claim_repo_task (body otherwise the migration-0023
-- definition):
--
-- 1. Quarantine sweep at claim start: this queue's claimable rows
--    with a missing/empty payload move to blocked with the reason in
--    last_error and a 'blocked' event row, so pre-existing
--    payload-less rows stop circulating instead of reaching the
--    next claimant.
--
-- 2. The candidate SELECT excludes payload-less rows outright
--    (payload is not null and payload <> '{}'), so even a row the
--    sweep has not seen cannot be picked.
--
-- Schema-level packet validation (validate_packet) lives in the
-- broker/client layers — verb_claim on the Turso backend and
-- claim_output in the fleet client — which serve both stores; a
-- plpgsql port of the validator is deliberately not duplicated here.

CREATE OR REPLACE FUNCTION agent.claim_repo_task(p_worker text, p_queue text, p_lease_seconds integer DEFAULT 2700, p_lane_id text DEFAULT NULL::text)
 RETURNS agent.tasks
 LANGUAGE plpgsql
AS $function$
declare t agent.tasks; v_was_parked boolean; v_lane agent.lanes; v_old_session text; v_q agent.tasks;
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
  -- #5444 change 1: quarantine this queue's payload-less claimable
  -- rows before picking, each with its event row.
  for v_q in
    update agent.tasks
       set status = 'blocked', worker_id = null, lease_until = null,
           last_error = 'quarantined by claim path: payload missing/empty (#5444)',
           updated_at = now()
     where queue = p_queue
       and status in ('queued', 'parked')
       and (payload is null or payload = '{}'::jsonb)
    returning *
  loop
    delete from agent.leases where task_id = v_q.id;
    insert into agent.events (worker_id, task_id, kind, detail)
    values (p_worker, v_q.id, 'blocked',
            jsonb_build_object(
              'reason', 'quarantined by claim path: payload missing/empty (#5444)',
              'via', 'claim_payload_quarantine'));
  end loop;
  -- #5444 change 2: the candidate predicate excludes payload-less
  -- rows (the payload lines in the WHERE clause are the only other
  -- change from migration-0023).
  select * into t from agent.tasks
   where queue = p_queue
     and payload is not null
     and payload <> '{}'::jsonb
     and ( (status = 'queued'  and attempts < max_attempts
            and (not_before is null or not_before <= now()))
        or (status = 'parked'  and not_before is not null and not_before <= now()) )
   order by priority asc, created_at asc
   for update skip locked limit 1;
  if not found then return null; end if;
  v_was_parked := (t.status = 'parked');
  v_old_session := t.progress ->> 'session_id';
  update agent.tasks
     set status = 'claimed', worker_id = p_worker, claimed_at = now(),
         lease_until = now() + make_interval(secs => p_lease_seconds),
         attempts = attempts + (case when v_was_parked then 0 else 1 end),
         lane_id = p_lane_id,
         park_kind = null, wait_kind = null,
         progress = case when v_old_session is not null
                      then (coalesce(progress, '{}'::jsonb) - 'session_id' - 'launched_by')
                           || jsonb_build_object('session_cleared', jsonb_build_object(
                                'session_id', v_old_session,
                                'cleared_at', to_jsonb(now()),
                                'via', 'claim_repo_task'))
                      else progress end,
         not_before = null, updated_at = now()
   where id = t.id returning * into t;
  insert into agent.leases (task_id, worker_id, lease_until, heartbeat_at)
  values (t.id, p_worker, t.lease_until, now())
  on conflict (task_id) do update
    set worker_id = excluded.worker_id,
        lease_until = excluded.lease_until, heartbeat_at = now();
  if v_old_session is not null then
    update agent.repo_task_lane_ledger
       set session_id = null, updated_at = now()
     where task_id = t.id and session_id is not null;
    insert into agent.events (worker_id, task_id, kind, detail)
    values (p_worker, t.id, 'session_binding_cleared',
            jsonb_build_object('session_id', v_old_session,
                               'via', 'claim_repo_task'));
  end if;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, t.id, 'claimed',
          jsonb_build_object('lease_seconds', p_lease_seconds,
                             'via', 'claim_repo_task',
                             'resumed_from_park', v_was_parked,
                             'cleared_session_id', v_old_session,
                             'lane_id', p_lane_id));
  return t;
end $function$;
