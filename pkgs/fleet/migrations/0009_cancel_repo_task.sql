-- 0009: cancel_repo_task — close out a QUEUED or PARKED pilot task that will
-- never be worked (issue closed, item self-assigned elsewhere, superseded).
-- Found missing 2026-10-25 by the first repo-fleet-monitor run: task 1078 sat
-- queued for closed issue code#819 and block_task rightly refused it (block is
-- for claimed tasks owned by the caller). Additive only; no existing function
-- is modified. Also registers the 'fleet-monitor' worker identity used as the
-- acting worker for monitor close-outs.

insert into agent.workers (id, role, display_name) values
  ('fleet-monitor', 'monitor', 'Fleet monitor (queue reconciliation)')
on conflict (id) do nothing;

create or replace function agent.cancel_repo_task(
  p_task_id bigint, p_worker text, p_reason text)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  select * into t from agent.tasks
   where id = p_task_id and status in ('queued', 'parked')
   for update;
  if not found then
    raise exception 'cancel_repo_task: task % is not queued or parked', p_task_id;
  end if;
  update agent.tasks
     set status = 'cancelled',
         worker_id = null, lease_until = null,
         last_error = p_reason,
         result = coalesce(result, '{}'::jsonb)
                  || jsonb_build_object('reason_class', 'cancelled',
                                        'cancel_reason', p_reason),
         updated_at = now()
   where id = p_task_id returning * into t;
  delete from agent.leases where task_id = p_task_id;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'cancelled',
          jsonb_build_object('reason', p_reason, 'via', 'cancel_repo_task'));
  return t;
end $$;

grant execute on function agent.cancel_repo_task(bigint, text, text) to service_role, authenticated;
