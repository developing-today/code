-- migration-0024: fix the two Codex P2 findings against the live
-- session-binding functions (#5393; findings raised on PR #5349, the
-- migration-0023 record copy, review comments 4465191796 and
-- 4465191807).
--
-- Bodies are the CURRENT LIVE definitions, re-extracted with
-- pg_get_functiondef on 2026-11-04 before this migration was written
-- (they match the migration-0023 record copy). Only the two flagged
-- statements change:
--
-- 1. bind_repo_task_lane_session (finding 4465191796): the task row is
--    now locked FOR UPDATE before the lane ledger is accessed. The
--    binding-clearing transitions (park/fail/expire/revalidate) lock
--    the task row first; the old order (ledger update before the task
--    lock is ever taken) created a lock-order cycle that could
--    deadlock a same-session bind retry overlapping one of those
--    transitions.
--
-- 2. revalidate_parked_tasks (finding 4465191807): the release branch
--    now aliases agent.repo_task_lane_ledger as l and qualifies
--    l.task_id. The function RETURNS TABLE(task_id bigint, ...), so
--    the unqualified task_id was ambiguous (SQLSTATE 42702) and
--    errored the whole sweep whenever a released parked task still
--    carried progress.session_id. Reproduced live before this fix:
--    'column reference "task_id" is ambiguous' at the ledger update.

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
        update agent.repo_task_lane_ledger l
           set session_id = null, updated_at = now()
         where l.task_id = r.id and l.session_id is not null;
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

CREATE OR REPLACE FUNCTION agent.bind_repo_task_lane_session(p_task_id bigint, p_worker text, p_session_id text, p_launched_by text DEFAULT 'opencode-claimant'::text)
 RETURNS agent.repo_task_lane_ledger
 LANGUAGE plpgsql
AS $function$
declare
  t agent.tasks;
  row_out agent.repo_task_lane_ledger;
begin
  select * into t from agent.tasks where id = p_task_id for update;
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
