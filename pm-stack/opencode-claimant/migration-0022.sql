-- #4758: recovery must release the progress and lane-ledger bindings
-- together. The old session id is a compare-and-clear guard: a stale
-- claimant cannot unbind a session that another launch has replaced.
CREATE OR REPLACE FUNCTION agent.bind_repo_task_lane_session(p_task_id bigint, p_worker text, p_session_id text, p_launched_by text DEFAULT 'opencode-claimant')
 RETURNS agent.repo_task_lane_ledger
 LANGUAGE plpgsql
AS $function$
declare
  t agent.tasks;
  row_out agent.repo_task_lane_ledger;
  recovery_reason text;
begin
  select * into t from agent.tasks where id = p_task_id for update;
  if not found then
    raise exception 'bind_repo_task_lane_session: task % does not exist', p_task_id;
  end if;
  select * into row_out from agent.repo_task_lane_ledger
   where task_id = p_task_id for update;
  if not found then
    raise exception 'bind_repo_task_lane_session: no lane ledger row for task %', p_task_id;
  end if;

  -- The claimant uses this reserved launched_by value only after the
  -- old session is verified absent (or verified empty and deleted).
  -- Matching both stores before changing either makes recovery an
  -- atomic compare-and-clear rather than a progress-only checkpoint.
  if p_launched_by in (
       'opencode-claimant-recovery:zero-message-deleted',
       'opencode-claimant-recovery:gone-session'
     ) then
    if p_worker is distinct from 'opencode-claimant' then
      raise exception 'bind_repo_task_lane_session: recovery must be performed by opencode-claimant';
    end if;
    if p_session_id is null or btrim(p_session_id) = '' then
      raise exception 'bind_repo_task_lane_session: recovery requires the prior session id';
    end if;
    if t.status is distinct from 'claimed'
       or coalesce(t.progress ->> 'lane', '') <> 'opencode'
       or t.progress ->> 'session_id' is distinct from p_session_id
       or row_out.assigned_lane is distinct from 'opencode'
       or row_out.session_id is distinct from p_session_id then
      raise exception 'bind_repo_task_lane_session: recovery binding mismatch for task %', p_task_id;
    end if;
    if t.max_attempts is not null
       and coalesce(t.attempts, 0) >= t.max_attempts then
      raise exception 'bind_repo_task_lane_session: recovery attempts exhausted for task %', p_task_id;
    end if;

    recovery_reason := case p_launched_by
      when 'opencode-claimant-recovery:zero-message-deleted'
        then 'verified zero-message session deleted before recovery'
      else 'bound session verified absent before recovery'
    end;

    update agent.repo_task_lane_ledger
       set session_id = null, updated_at = now()
     where task_id = p_task_id
    returning * into row_out;
    update agent.tasks
       set attempts = coalesce(attempts, 0) + 1,
           progress = coalesce(progress, '{}'::jsonb)
                      || jsonb_build_object(
                           'session_id', null::text,
                           'launched_by', null::text,
                           'stage', 'launch_failed',
                           'launch_failed_session_id', p_session_id,
                           'launch_failed_at', now(),
                           'launch_failed_reason', recovery_reason),
           updated_at = now()
     where id = p_task_id;
    insert into agent.events (worker_id, task_id, kind, detail)
    values (p_worker, p_task_id, 'lane_session_recovered',
            jsonb_build_object('session_id', p_session_id,
                               'reason', recovery_reason,
                               'stage', 'launch_failed',
                               'via', 'bind_repo_task_lane_session'));
    return row_out;
  end if;

  if p_session_id is null or btrim(p_session_id) = '' then
    raise exception 'bind_repo_task_lane_session: session id must not be empty';
  end if;
  if row_out.session_id is not null and row_out.session_id <> p_session_id then
    raise exception 'bind_repo_task_lane_session: task % already bound to session %', p_task_id, row_out.session_id;
  end if;
  if t.progress ->> 'session_id' is not null
     and t.progress ->> 'session_id' <> p_session_id then
    raise exception 'bind_repo_task_lane_session: task % progress already bound to session %', p_task_id, t.progress ->> 'session_id';
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
end;
$function$;
