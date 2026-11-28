-- 0014: revive-on-enqueue for cancelled tasks (queue dedupe one-way door)
--
-- Problem (issue developing-today/code#1298, found 2026-10-26 by the fleet
-- monitor): agent.enqueue_repo_task (0008) and its fleet sibling
-- agent.enqueue_repo_task_fleet (0011) insert with
-- `on conflict (dedupe_key) do nothing` and return the existing row for ANY
-- status, including 'cancelled'. A cancelled task therefore permanently
-- owns its issue's dedupe identity: the issue can never be enqueued again
-- and the caller cannot tell the enqueue silently no-opped (live instance:
-- shadow task 1064 for code#551, cancelled as superseded, blocked every
-- later enqueue of #551's triage packet).
--
-- Fix — option (a) from #1298, revive on enqueue: on conflict where the
-- existing row is 'cancelled', update that same row back to 'queued' with
-- the new queue/title/body/payload/priority/not_before/max_attempts,
-- reset attempts to 0 and clear result/last_error/worker/lease/claim and
-- completion timestamps and progress, delete any stale lease row, and
-- write an 'enqueued' event whose detail carries
-- revived_from_cancelled: true. One row per identity is preserved; the
-- event log keeps the full audit trail (enqueued -> cancelled -> enqueued
-- with the revival flag), so helper/monitor can distinguish
-- enqueued-new (event without the flag), already-live (no new event) and
-- revived (event with the flag) from the returned row + events.
--
-- FAILED tasks deliberately do NOT revive: failure is a worked outcome
-- with failure accounting (attempts, last_error, result) that producers
-- and the monitor rely on; silently resurrecting a failed row would erase
-- that record and could re-loop work a worker already judged. Cancellation
-- is an operator decision (superseded / self-assigned / issue-closed
-- cleanup), so reviving on a fresh producer enqueue matches operator
-- intent. Re-enqueueing a failed identity remains an explicit operator
-- action (cancel is not available for failed rows by design in 0009).
--
-- Both enqueue functions get the identical revival block; their repo
-- checks are unchanged (0008: developing-today/code only; 0011: the four
-- fleet repos). No other function is modified.
-- Rollback: re-apply the enqueue function bodies from
-- 0008_repo_queue_pilot.sql and 0011_enqueue_repo_task_fleet.sql (no data
-- changes to undo; revived rows stay queued, which is their intent).

create or replace function agent.enqueue_repo_task(
  p_queue text, p_title text, p_body text default null,
  p_payload jsonb default '{}'::jsonb, p_dedupe_key text default null,
  p_priority integer default 50, p_not_before timestamptz default null,
  p_max_attempts integer default null)
returns agent.tasks language plpgsql as $$
declare t agent.tasks; inserted boolean := false; revived boolean := false;
begin
  if p_payload ? 'schema' and p_payload ->> 'schema' <> 'repo-task/v1' then
    raise exception 'enqueue_repo_task: unsupported payload schema %',
      p_payload ->> 'schema';
  end if;
  if p_payload ? 'repo' and p_payload ->> 'repo' <> 'developing-today/code' then
    raise exception 'enqueue_repo_task: pilot repo is developing-today/code, got %',
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
                               'via', 'enqueue_repo_task'));
  elsif revived then
    insert into agent.events (worker_id, task_id, kind, detail)
    values (null, t.id, 'enqueued',
            jsonb_build_object('queue', p_queue, 'dedupe_key', p_dedupe_key,
                               'via', 'enqueue_repo_task',
                               'revived_from_cancelled', true));
  end if;
  return t;
end $$;

create or replace function agent.enqueue_repo_task_fleet(
  p_queue text, p_title text, p_body text default null,
  p_payload jsonb default '{}'::jsonb, p_dedupe_key text default null,
  p_priority integer default 50, p_not_before timestamptz default null,
  p_max_attempts integer default null)
returns agent.tasks language plpgsql as $$
declare t agent.tasks; inserted boolean := false; revived boolean := false;
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
end $$;

grant execute on function agent.enqueue_repo_task(text, text, text, jsonb, text, integer, timestamptz, integer) to service_role, authenticated;
grant execute on function agent.enqueue_repo_task_fleet(text, text, text, jsonb, text, integer, timestamptz, integer) to service_role, authenticated;
