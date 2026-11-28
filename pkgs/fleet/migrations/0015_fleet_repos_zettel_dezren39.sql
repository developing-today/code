-- 0015: widen agent.enqueue_repo_task_fleet's fleet-repo check for the
-- #2672 Part 5a expansion: developing-today/zettel and dezren39/dezren39
-- join the fleet (pm-dispatch FLEET_REPOS now lists six repos). Body is
-- byte-identical to the live definition (incl. the 0014 revive-on-enqueue
-- behaviour) except for the two added repo names in the payload.repo
-- check. enqueue_repo_task (the developing-today/code pilot function) is
-- deliberately NOT modified, per the pilot isolation rule from 0011.
create or replace function agent.enqueue_repo_task_fleet(p_queue text, p_title text, p_body text default null::text, p_payload jsonb default '{}'::jsonb, p_dedupe_key text default null::text, p_priority integer default 50, p_not_before timestamp with time zone default null::timestamp with time zone, p_max_attempts integer default null::integer)
 returns agent.tasks
 language plpgsql
as $function$
declare t agent.tasks; inserted boolean := false; revived boolean := false;
begin
  if p_payload ? 'schema' and p_payload ->> 'schema' <> 'repo-task/v1' then
    raise exception 'enqueue_repo_task_fleet: unsupported payload schema %',
      p_payload ->> 'schema';
  end if;
  if p_payload ? 'repo' and p_payload ->> 'repo' not in (
       'developing-today/code', 'developing-today/opencode',
       'developing-today/openchamber', 'dezren39/nix',
       'developing-today/zettel', 'dezren39/dezren39') then
    raise exception 'enqueue_repo_task_fleet: not a fleet repo: %',
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
    -- 0014: a cancelled row revives in place with the new packet.
    update agent.tasks
       set status = 'queued', queue = p_queue, title = p_title,
           body = p_body, payload = coalesce(p_payload, '{}'::jsonb),
           priority = coalesce(p_priority, 50), not_before = p_not_before,
           max_attempts = coalesce(p_max_attempts, 3),
           attempts = 0, result = null, last_error = null,
           worker_id = null, lease_until = null,
           claimed_at = null, completed_at = null,
           progress = '{}'::jsonb, updated_at = now()
     where dedupe_key = p_dedupe_key and status = 'cancelled'
     returning * into t;
    if found then
      revived := true;
      delete from agent.leases where task_id = t.id;
    else
      select * into t from agent.tasks where dedupe_key = p_dedupe_key;
      if not found then return null; end if;  -- null dedupe key race; caller retries
    end if;
  end if;
  if inserted then
    insert into agent.events (worker_id, task_id, kind, detail)
    values (null, t.id, 'enqueued',
            jsonb_build_object('queue', p_queue, 'dedupe_key', p_dedupe_key,
                               'via', 'enqueue_repo_task_fleet'));
  elsif revived then
    insert into agent.events (worker_id, task_id, kind, detail)
    values (null, t.id, 'enqueued',
            jsonb_build_object('queue', p_queue, 'dedupe_key', p_dedupe_key,
                               'via', 'enqueue_repo_task_fleet',
                               'revived_from_cancelled', true));
  end if;
  return t;
end $function$;
