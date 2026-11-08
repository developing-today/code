-- migration-0023.sql — issue #5001 / task 9043 (repo-health queue pilot)
-- Renumbered 0022 -> 0023 for the repo record copy only: this migration was
-- applied live on 2026-11-04 as the claimant session-binding fix, but main
-- landed a different migration-0022.sql for issue #4758 (commit 89e951cc5,
-- 2026-11-04T08:20Z) before this record copy merged. Function bodies below
-- are unchanged from the live-applied SQL.
--
-- Problem: no queue transition cleared a claimant-bound session
-- (agent.tasks.progress.session_id / launched_by and
-- agent.repo_task_lane_ledger.session_id). A task requeued after lease
-- expiry / transient fail / park release kept its old binding, so:
--   * a queued row could carry a LIVE session (double execution), and
--   * a relaunch could never bind (bind refuses a different existing
--     session), and
--   * bind_repo_task_lane_session performed no claim/lease check, so a
--     session created from a pre-expiry poll snapshot could bind and
--     execute a task that had already been requeued.
--
-- Fix (authored against the live function bodies extracted
-- 2026-11-04T05:47-05:52Z, task-9043-live-defs/):
--   1. Every transition OUT of a claim into a claimable state clears
--      the binding in the same statement/transaction and preserves the
--      old session id in progress.session_cleared and in an
--      agent.events row of kind 'session_binding_cleared':
--        expire_leases()            claimed -> queued
--        fail_repo_task()           claimed -> queued (requeue branch)
--        park_task()                claimed -> parked
--        revalidate_parked_tasks()  parked  -> queued (released branch)
--      claim_repo_task() additionally clears any residual binding on
--      the row it claims (defense for rows bound before this
--      migration), so a fresh claim never inherits a dead predecessor.
--   2. bind_repo_task_lane_session() now refuses to bind unless the
--      task is claimed with a live lease (status='claimed' and
--      lease_until > now()) in the same transaction as the bind.
--   Terminal transitions (complete / cancel) intentionally keep the
--   binding: on done/cancelled rows it is the execution record, and
--   those rows are never claimable again.

-- ---------------------------------------------------------------------
-- 1. expire_leases: claimed -> queued clears the binding
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION agent.expire_leases()
 RETURNS integer
 LANGUAGE plpgsql
AS $function$
declare n integer := 0; r record;
begin
  for r in
    select id, worker_id, progress ->> 'session_id' as old_session
      from agent.tasks
     where status = 'claimed' and lease_until < now()
  loop
    update agent.tasks
       set status = 'queued', worker_id = null, lease_until = null,
           updated_at = now(),
           progress = case when r.old_session is not null
             then (coalesce(progress, '{}'::jsonb) - 'session_id' - 'launched_by')
                  || jsonb_build_object('session_cleared', jsonb_build_object(
                       'session_id', r.old_session,
                       'cleared_at', to_jsonb(now()),
                       'via', 'expire_leases'))
             else progress end
     where id = r.id and status = 'claimed' and lease_until < now();
    if not found then
      continue;
    end if;
    n := n + 1;
    insert into agent.events (worker_id, task_id, kind, detail)
    values (r.worker_id, r.id, 'lease_expired',
            jsonb_build_object('via', 'expire_leases',
                               'cleared_session_id', r.old_session));
    if r.old_session is not null then
      update agent.repo_task_lane_ledger
         set session_id = null, updated_at = now()
       where task_id = r.id and session_id is not null;
      insert into agent.events (worker_id, task_id, kind, detail)
      values (r.worker_id, r.id, 'session_binding_cleared',
              jsonb_build_object('session_id', r.old_session,
                                 'via', 'expire_leases'));
    end if;
  end loop;
  delete from agent.leases where lease_until < now();
  return n;
end $function$;

-- ---------------------------------------------------------------------
-- 2. fail_repo_task: the transient requeue branch clears the binding
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION agent.fail_repo_task(p_task_id bigint, p_worker text, p_error text, p_reason_class text DEFAULT 'transient'::text)
 RETURNS agent.tasks
 LANGUAGE plpgsql
AS $function$
declare t agent.tasks; requeue boolean; delay_s integer; v_old_session text;
begin
  select * into t from agent.tasks
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   for update;
  if not found then
    raise exception 'fail_repo_task: task % is not claimed by %', p_task_id, p_worker;
  end if;
  v_old_session := t.progress ->> 'session_id';
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
         progress = case when requeue and v_old_session is not null
                       then (coalesce(progress, '{}'::jsonb) - 'session_id' - 'launched_by')
                            || jsonb_build_object('session_cleared', jsonb_build_object(
                                 'session_id', v_old_session,
                                 'cleared_at', to_jsonb(now()),
                                 'via', 'fail_repo_task'))
                       else progress end,
         result = coalesce(result, '{}'::jsonb)
                  || jsonb_build_object('reason_class',
                                        coalesce(p_reason_class, 'transient')),
         updated_at = now()
   where id = p_task_id returning * into t;
  delete from agent.leases where task_id = p_task_id;
  if requeue and v_old_session is not null then
    update agent.repo_task_lane_ledger
       set session_id = null, updated_at = now()
     where task_id = p_task_id and session_id is not null;
    insert into agent.events (worker_id, task_id, kind, detail)
    values (p_worker, p_task_id, 'session_binding_cleared',
            jsonb_build_object('session_id', v_old_session,
                               'via', 'fail_repo_task'));
  end if;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'failed',
          jsonb_build_object('error', p_error,
                             'reason_class', coalesce(p_reason_class, 'transient'),
                             'requeued', requeue,
                             'cleared_session_id', case when requeue then v_old_session end,
                             'via', 'fail_repo_task'));
  return t;
end $function$;

-- ---------------------------------------------------------------------
-- 3. park_task: claimed -> parked clears the binding (the claim that
--    owned the session has ended; a resume is a fresh claim + launch)
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION agent.park_task(p_task_id bigint, p_worker text, p_reason text, p_delay_seconds integer DEFAULT NULL::integer, p_progress jsonb DEFAULT '{}'::jsonb, p_ready_condition jsonb DEFAULT NULL::jsonb, p_not_before timestamp with time zone DEFAULT NULL::timestamp with time zone)
 RETURNS agent.tasks
 LANGUAGE plpgsql
AS $function$
declare t agent.tasks; v_condition jsonb; v_not_before timestamptz;
        v_park_kind text; v_wait_kind text; v_old_session text;
begin
  if p_not_before is not null and p_delay_seconds is not null then
    raise exception
      'park_task: pass exactly one of p_delay_seconds or p_not_before, not both';
  end if;
  if p_not_before is not null then
    v_not_before := p_not_before;
    v_park_kind := 'absolute';
  else
    v_not_before := now() + make_interval(secs => coalesce(p_delay_seconds, 1200));
    v_park_kind := 'relative';
  end if;
  v_condition := coalesce(
    p_ready_condition,
    coalesce(p_progress, '{}'::jsonb) -> 'ready_condition',
    jsonb_build_object('kind', 'time', 'until', to_jsonb(v_not_before)));
  v_wait_kind := case v_condition ->> 'kind'
    when 'time' then 'timer'
    when 'ci' then 'ci'
    when 'ci_green' then 'ci'
    when 'pr_merged' then 'ci'
    when 'issue_closed' then 'ci'
    when 'upstream_pr' then 'ci'
    when 'review' then 'review'
    when 'codex_review' then 'review'
    when 'pr_review' then 'review'
    when 'person_answer' then 'review'
    when 'capacity' then 'capacity'
    when 'quota_reset' then 'capacity'
    else 'external'
  end;
  select progress ->> 'session_id' into v_old_session from agent.tasks
   where id = p_task_id and worker_id = p_worker and status = 'claimed';
  update agent.tasks
     set status = 'parked', worker_id = null, lease_until = null,
         not_before = v_not_before,
         park_kind = v_park_kind, wait_kind = v_wait_kind,
         ready_condition = v_condition,
         last_checked_at = now(), check_outcome = null,
         progress = (case when v_old_session is not null
                       then (coalesce(progress, '{}'::jsonb) - 'session_id' - 'launched_by')
                            || jsonb_build_object('session_cleared', jsonb_build_object(
                                 'session_id', v_old_session,
                                 'cleared_at', to_jsonb(now()),
                                 'via', 'park_task'))
                       else coalesce(progress, '{}'::jsonb) end)
                    || coalesce(p_progress, '{}'::jsonb),
         result = coalesce(result, '{}'::jsonb)
                  || jsonb_build_object('park_reason', p_reason),
         updated_at = now()
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   returning * into t;
  if not found then
    raise exception 'park_task: task % is not claimed by %', p_task_id, p_worker;
  end if;
  delete from agent.leases where task_id = p_task_id;
  if v_old_session is not null then
    update agent.repo_task_lane_ledger
       set session_id = null, updated_at = now()
     where task_id = p_task_id and session_id is not null;
    insert into agent.events (worker_id, task_id, kind, detail)
    values (p_worker, p_task_id, 'session_binding_cleared',
            jsonb_build_object('session_id', v_old_session,
                               'via', 'park_task'));
  end if;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'parked',
          jsonb_build_object('reason', p_reason,
                             'park_kind', v_park_kind,
                             'wait_kind', v_wait_kind,
                             'delay_seconds', p_delay_seconds,
                             'not_before', to_jsonb(v_not_before),
                             'ready_condition', v_condition));
  return t;
end $function$;

-- ---------------------------------------------------------------------
-- 4. claim_repo_task: a fresh claim never inherits a residual binding
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION agent.claim_repo_task(p_worker text, p_queue text, p_lease_seconds integer DEFAULT 2700, p_lane_id text DEFAULT NULL::text)
 RETURNS agent.tasks
 LANGUAGE plpgsql
AS $function$
declare t agent.tasks; v_was_parked boolean; v_lane agent.lanes; v_old_session text;
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

-- ---------------------------------------------------------------------
-- 5. revalidate_parked_tasks: the parked -> queued release branch
--    clears the binding (body otherwise identical to the live def)
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION agent.revalidate_parked_tasks(p_min_age interval DEFAULT '1 day'::interval, p_limit integer DEFAULT 500)
 RETURNS TABLE(task_id bigint, outcome text)
 LANGUAGE plpgsql
AS $function$
declare r agent.tasks; v_kind text; v_outcome text; v_target agent.tasks;
begin
  for r in
    select * from agent.tasks t
     where t.status = 'parked'
       and coalesce(t.last_checked_at, t.updated_at) < now() - p_min_age
     order by coalesce(t.last_checked_at, t.updated_at) asc
     limit p_limit
     for update skip locked
  loop
    v_kind := r.ready_condition ->> 'kind';
    if r.ready_condition is null or v_kind is null then
      v_outcome := 'stale';
    elsif v_kind = 'time' then
      -- Elapsed time is never evidence: time parks release through the
      -- claim path at not_before, not through the sweeper.
      v_outcome := 'confirmed';
    elsif v_kind = 'task_terminal' then
      select * into v_target from agent.tasks
       where id = (r.ready_condition ->> 'task_id')::bigint;
      if not found then
        v_outcome := 'stale';
      elsif v_target.status in ('done', 'cancelled') then
        v_outcome := 'released';
      else
        v_outcome := 'confirmed';
      end if;
    elsif v_kind = 'task_status' then
      select * into v_target from agent.tasks
       where id = (r.ready_condition ->> 'task_id')::bigint;
      if not found then
        v_outcome := 'stale';
      elsif v_target.status = (r.ready_condition ->> 'status') then
        v_outcome := 'released';
      else
        v_outcome := 'confirmed';
      end if;
    elsif v_kind = 'event_seen' then
      if exists (select 1 from agent.events e
                  where e.task_id = coalesce(
                          (r.ready_condition ->> 'task_id')::bigint, r.id)
                    and e.kind = (r.ready_condition ->> 'event_kind')) then
        v_outcome := 'released';
      else
        v_outcome := 'confirmed';
      end if;
    else
      -- External kinds (ci, pr_merged, issue_closed, person_answer,
      -- review, ...): not verifiable from DB state. Mark stale so the
      -- park surfaces for an agent with external access instead of
      -- sitting silently; the park itself is untouched.
      v_outcome := 'stale';
    end if;

    if v_outcome = 'released' then
      update agent.tasks
         set status = 'queued', not_before = null,
             last_checked_at = now(), check_outcome = 'released',
             progress = (case when r.progress ->> 'session_id' is not null
                           then (coalesce(r.progress, '{}'::jsonb) - 'session_id' - 'launched_by')
                                || jsonb_build_object('session_cleared', jsonb_build_object(
                                     'session_id', r.progress ->> 'session_id',
                                     'cleared_at', to_jsonb(now()),
                                     'via', 'revalidate_parked_tasks'))
                           else coalesce(r.progress, '{}'::jsonb) end)
               || jsonb_build_object(
               'park_revalidation', jsonb_build_object(
                 'outcome', 'released', 'at', to_jsonb(now()),
                 'condition', r.ready_condition,
                 'note', 'ready_condition met; released to queued by the '
                         'park revalidation sweeper (migration 0021)')),
             updated_at = now()
       where id = r.id;
      if r.progress ->> 'session_id' is not null then
        update agent.repo_task_lane_ledger
           set session_id = null, updated_at = now()
         where task_id = r.id and session_id is not null;
        insert into agent.events (worker_id, task_id, kind, detail)
        values ('park-revalidation-sweeper', r.id, 'session_binding_cleared',
                jsonb_build_object('session_id', r.progress ->> 'session_id',
                                   'via', 'revalidate_parked_tasks'));
      end if;
      insert into agent.events (worker_id, task_id, kind, detail)
      values ('park-revalidation-sweeper', r.id, 'park_released',
              jsonb_build_object('condition', r.ready_condition,
                                 'via', 'revalidate_parked_tasks'));
    else
      update agent.tasks
         set last_checked_at = now(), check_outcome = v_outcome,
             updated_at = now()
       where id = r.id;
      insert into agent.events (worker_id, task_id, kind, detail)
      values ('park-revalidation-sweeper', r.id, 'park_revalidated',
              jsonb_build_object('outcome', v_outcome,
                                 'condition_kind', v_kind,
                                 'via', 'revalidate_parked_tasks'));
    end if;

    task_id := r.id; outcome := v_outcome; return next;
  end loop;
end $function$;

-- ---------------------------------------------------------------------
-- 6. bind_repo_task_lane_session: bind-time claim guard — the task
--    must be claimed with a live lease in the same transaction
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION agent.bind_repo_task_lane_session(p_task_id bigint, p_worker text, p_session_id text, p_launched_by text DEFAULT 'opencode-claimant'::text)
 RETURNS agent.repo_task_lane_ledger
 LANGUAGE plpgsql
AS $function$
declare
  t agent.tasks;
  row_out agent.repo_task_lane_ledger;
begin
  select * into t from agent.tasks where id = p_task_id;
  if not found then
    raise exception 'bind_repo_task_lane_session: task % does not exist', p_task_id;
  end if;
  if t.status <> 'claimed' or t.lease_until is null or t.lease_until <= now() then
    raise exception 'bind_repo_task_lane_session: task % is not claimed with a live lease (status %, lease_until %)',
      p_task_id, t.status, t.lease_until;
  end if;
  select * into row_out from agent.repo_task_lane_ledger
   where task_id = p_task_id;
  if not found then
    raise exception 'bind_repo_task_lane_session: no lane ledger row for task %', p_task_id;
  end if;
  if row_out.session_id is not null and row_out.session_id <> p_session_id then
    raise exception 'bind_repo_task_lane_session: task % already bound to session %', p_task_id, row_out.session_id;
  end if;
  update agent.repo_task_lane_ledger
     set session_id = p_session_id, updated_at = now()
   where task_id = p_task_id
  returning * into row_out;
  update agent.tasks
     set progress = coalesce(progress, '{}'::jsonb)
                    || jsonb_build_object('session_id', p_session_id,
                                          'launched_by', p_launched_by),
         updated_at = now()
   where id = p_task_id;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'lane_session_bound',
          jsonb_build_object('session_id', p_session_id,
                             'launched_by', p_launched_by,
                             'via', 'bind_repo_task_lane_session'));
  return row_out;
end $function$;
