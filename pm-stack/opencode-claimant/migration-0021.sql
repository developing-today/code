CREATE OR REPLACE FUNCTION agent.assign_repo_task_lane(p_task_id bigint, p_worker text, p_exclusion_reason text DEFAULT NULL::text)
 RETURNS agent.repo_task_lane_ledger
 LANGUAGE plpgsql
AS $function$
declare
  t agent.tasks;
  row_out agent.repo_task_lane_ledger;
  o_count bigint;
  d_count bigint;
  lane text;
begin
  select * into t from agent.tasks where id = p_task_id;
  if not found then
    raise exception 'assign_repo_task_lane: task % does not exist', p_task_id;
  end if;
  perform pg_advisory_xact_lock(hashtext('repo_task_lane:' || t.queue));
  -- Idempotent: a task that already has a row keeps its assignment.
  select * into row_out from agent.repo_task_lane_ledger
   where task_id = p_task_id;
  if found then
    -- 0021: keep progress.lane in step with the ledger (self-heal for
    -- rows assigned before progress stamping existed).
    update agent.tasks
       set progress = coalesce(progress, '{}'::jsonb)
                      || jsonb_build_object('lane', row_out.assigned_lane),
           updated_at = now()
     where id = p_task_id
       and coalesce(progress ->> 'lane', '') <> row_out.assigned_lane;
    return row_out;
  end if;
  select
    count(*) filter (where coalesce(executed_lane, assigned_lane) = 'opencode'),
    count(*) filter (where coalesce(executed_lane, assigned_lane) = 'direct')
    into o_count, d_count
    from agent.repo_task_lane_ledger
   where queue = t.queue and exclusion_reason is null;
  if p_exclusion_reason is not null then
    lane := 'direct';
  elsif o_count <= d_count then
    lane := 'opencode';
  else
    lane := 'direct';
  end if;
  insert into agent.repo_task_lane_ledger
    (queue, task_id, assigned_lane, exclusion_reason, assigned_by)
  values (t.queue, p_task_id, lane, p_exclusion_reason, p_worker)
  returning * into row_out;
  -- 0021 (#3026): stamp the assignment into tasks.progress so the
  -- laptop claimant's candidate discovery (progress.lane = 'opencode',
  -- no progress.session_id) can see it. Before this, the assignment
  -- lived only in the ledger and discovery saw zero candidates.
  update agent.tasks
     set progress = coalesce(progress, '{}'::jsonb)
                    || jsonb_build_object('lane', lane),
         updated_at = now()
   where id = p_task_id;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'lane_assigned',
          jsonb_build_object('assigned_lane', lane,
                             'exclusion_reason', p_exclusion_reason,
                             'opencode_count', o_count,
                             'direct_count', d_count,
                             'via', 'assign_repo_task_lane'));
  return row_out;
end $function$;
CREATE OR REPLACE FUNCTION agent.bind_repo_task_lane_session(p_task_id bigint, p_worker text, p_session_id text, p_launched_by text DEFAULT 'opencode-claimant')
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
update agent.tasks t
   set progress = coalesce(t.progress, '{}'::jsonb)
                  || jsonb_build_object('lane', l.assigned_lane),
       updated_at = now()
  from agent.repo_task_lane_ledger l
 where l.task_id = t.id
   and t.status = 'claimed'
   and (t.progress ->> 'lane') is null
returning t.id, l.assigned_lane;
