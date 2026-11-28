-- 0024: widen agent.enqueue_repo_task_fleet's fleet-repo check to the six
-- repos of the Build 1 dispatch estate: developing-today/zettel and
-- dezren39/dezren39 join (developing-today/code#5130, 2026-11-03).
-- Supersedes 0015, which was written against the 0014-era body and never
-- landed live; this body is the CURRENT live definition (incl. the 0019
-- dedupe-live-rows behaviour) with only the repo list changed.
-- enqueue_repo_task (the code pilot function) is deliberately NOT modified.
CREATE OR REPLACE FUNCTION agent.enqueue_repo_task_fleet(p_queue text, p_title text, p_body text DEFAULT NULL::text, p_payload jsonb DEFAULT '{}'::jsonb, p_dedupe_key text DEFAULT NULL::text, p_priority integer DEFAULT 50, p_not_before timestamp with time zone DEFAULT NULL::timestamp with time zone, p_max_attempts integer DEFAULT NULL::integer)
 RETURNS agent.tasks
 LANGUAGE plpgsql
AS $function$
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
  on conflict (dedupe_key)
    where dedupe_key is not null
      and status in ('queued', 'claimed', 'parked', 'blocked', 'needs_review', 'cancelled')
  do nothing
  returning * into t;
  if found then
    inserted := true;
  else
    -- 0019: a live row for the key wins (the conflict that brought us here).
    select * into t from agent.tasks
     where dedupe_key = p_dedupe_key
       and status in ('queued', 'claimed', 'parked', 'blocked', 'needs_review')
     order by created_at desc, id desc
     limit 1;
    if t is null then
      -- 0014: a cancelled row revives in place with the new packet.
      -- 0019: at most one cancelled row per key (it is in the index);
      -- the newest-first selection is belt-and-braces.
      begin
        update agent.tasks
           set status = 'queued', queue = p_queue, title = p_title,
               body = p_body, payload = coalesce(p_payload, '{}'::jsonb),
               priority = coalesce(p_priority, 50), not_before = p_not_before,
               max_attempts = coalesce(p_max_attempts, 3),
               attempts = 0, result = null, last_error = null,
               worker_id = null, lease_until = null,
               claimed_at = null, completed_at = null,
               progress = '{}'::jsonb, updated_at = now()
         where id = (select id from agent.tasks
                      where dedupe_key = p_dedupe_key and status = 'cancelled'
                      order by updated_at desc, id desc
                      limit 1)
         returning * into t;
        if found then revived := true; end if;
      exception when unique_violation then
        -- A concurrent enqueue created the live row first; return it below.
        t := null;
      end;
      if revived then
        delete from agent.leases where task_id = t.id;
      elsif t is null then
        -- No live or cancelled row: the conflicting row went terminal
        -- mid-enqueue (or the key is held only by terminal rows whose
        -- insert race we lost). Return the newest row for the key.
        select * into t from agent.tasks
         where dedupe_key = p_dedupe_key
         order by created_at desc, id desc
         limit 1;
        if t is null then return null; end if;  -- null dedupe key race; caller retries
      end if;
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
