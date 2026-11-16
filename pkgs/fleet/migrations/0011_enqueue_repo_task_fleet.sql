-- 0011_enqueue_repo_task_fleet.sql
-- Phase 3 (approved by Drew 2026-10-26): sweeps become queue producers
-- for the whole fleet. agent.enqueue_repo_task (0008) hard-checks the
-- payload repo = developing-today/code. Per the pilot isolation rule,
-- 0008's functions are NOT modified; this adds a fleet sibling that is
-- byte-identical except the repo check accepts the four fleet repos.
-- The helper keeps using enqueue_repo_task for code (Phase 2 behavior
-- unchanged) and enqueue_repo_task_fleet for the other three repos.
-- Rollback: drop function agent.enqueue_repo_task_fleet.

create or replace function agent.enqueue_repo_task_fleet(
  p_queue text, p_title text, p_body text default null,
  p_payload jsonb default '{}'::jsonb, p_dedupe_key text default null,
  p_priority integer default 50, p_not_before timestamptz default null,
  p_max_attempts integer default null)
returns agent.tasks language plpgsql as $$
declare t agent.tasks; inserted boolean := false;
begin
  if p_payload ? 'schema' and p_payload ->> 'schema' <> 'repo-task/v1' then
    raise exception 'enqueue_repo_task_fleet: unsupported payload schema %',
      p_payload ->> 'schema';
  end if;
  if p_payload ? 'repo' and p_payload ->> 'repo' not in (
       'developing-today/code', 'developing-today/opencode',
       'developing-today/openchamber', 'dezren39/nix') then
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
    select * into t from agent.tasks where dedupe_key = p_dedupe_key;
    if not found then return null; end if;  -- null dedupe key race; caller retries
  end if;
  if inserted then
    insert into agent.events (worker_id, task_id, kind, detail)
    values (null, t.id, 'enqueued',
            jsonb_build_object('queue', p_queue, 'dedupe_key', p_dedupe_key,
                               'via', 'enqueue_repo_task_fleet'));
  end if;
  return t;
end $$;

grant execute on function agent.enqueue_repo_task_fleet(text, text, text, jsonb, text, integer, timestamptz, integer) to service_role, authenticated;
