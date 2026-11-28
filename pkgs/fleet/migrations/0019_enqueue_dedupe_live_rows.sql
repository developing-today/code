-- 0019: enqueue dedupe must not match terminal rows (#2323, fix task 5268;
-- semantics pinned by triage task 3478, finding comment 6555129148).
--
-- Defect: agent.enqueue_repo_task and agent.enqueue_repo_task_fleet insert
-- ON CONFLICT (dedupe_key) DO NOTHING against the GLOBAL unique constraint
-- tasks_dedupe_key_key, then fall through to returning whatever row holds
-- the key -- including DONE rows. A dedupe key held by a completed task can
-- therefore never be re-enqueued: follow-up packets silently return the
-- completed row and create/mutate nothing (live-probed 2026-11-01 by task
-- 3478 against completed task 3721's key).
--
-- Fix (atomic: index + both RPCs in one migration):
--   1. Drop the global UNIQUE (dedupe_key) constraint. Replace with a
--      PARTIAL unique index over LIVE rows only:
--        status in ('queued','claimed','parked','blocked','needs_review','cancelled')
--      Live-set note: the pinned set was queued/claimed/parked/blocked;
--      'needs_review' is included because it is a non-terminal state whose
--      work is still owned (a second live row for its key would double the
--      work). 'failed' is treated as terminal like 'done' (fail_task sets
--      it only when not requeueing), so a failed key can be re-enqueued.
--      'cancelled' IS in the index, deliberately: the 0014 revival is
--      conflict-driven (it runs only when the insert does nothing), so the
--      cancelled row must still block the insert for revival to fire --
--      a first dry run of this migration with 'cancelled' outside the
--      index proved the point: the insert simply created a new row and
--      revival never ran. A side benefit: at most one cancelled row per
--      key, so revival stays unambiguous.
--      The replacement is a plain INDEX, not a constraint; uniqueness for
--      live rows is enforced by the index itself.
--   2. Both enqueue RPCs: the INSERT now targets the partial index
--      (ON CONFLICT (dedupe_key) WHERE <the index predicate> DO NOTHING),
--      so a key held only by terminal rows accepts a NEW row (history
--      preserved), while a live key still dedupes. The conflict fallthrough
--      is reordered and made deterministic for the multi-row-per-key world
--      this creates:
--        a. return the live row for the key if one exists (newest first);
--        b. else revive the NEWEST cancelled row in place (0014 semantics
--           preserved exactly: fields reset, leases deleted, event logged);
--           if a concurrent insert wins the live slot first, the revival
--           update's unique_violation is caught and the live row returned;
--        c. else return the newest row for the key (covers the race where
--           the conflicting live row went terminal mid-enqueue -- mirrors
--           the pre-0019 "return the existing row" behaviour); null only
--           when no row holds the key at all.
--
-- Base definitions: BOTH bodies below are the LIVE definitions fetched via
-- pg_get_functiondef on 2026-11-01, edited only in the dedupe machinery.
-- In particular the fleet repo check is the live four-repo list. The local
-- 0015 file (zettel/dezren39 widening) is NOT live -- verified by diffing
-- its body against pg_get_functiondef -- and this migration deliberately
-- does not smuggle it in; applying 0015 remains its own decision.
--
-- Cancelled semantics for the pilot function: its live definition already
-- carries the 0014 revival (verified in the fetched definition), and this
-- migration preserves it unchanged -- no deliberate change to document
-- beyond the deterministic row selection in 2a/2c, which the
-- multi-row-per-key world requires.
--
-- Rollback: restore the two pre-0019 function definitions (pg_get_functiondef
-- snapshots taken 2026-11-01, on file with task 5268's run notes), drop
-- index tasks_dedupe_key_live_key, and re-add the global constraint --
-- only valid once no key holds more than one row, so rollback after
-- re-enqueues have created history requires cancelling/merging the extra
-- live rows first. Treat as forward-fix-first.

begin;

alter table agent.tasks drop constraint tasks_dedupe_key_key;

create unique index tasks_dedupe_key_live_key
  on agent.tasks (dedupe_key)
  where dedupe_key is not null
    and status in ('queued', 'claimed', 'parked', 'blocked', 'needs_review', 'cancelled');

create or replace function agent.enqueue_repo_task(p_queue text, p_title text, p_body text default null::text, p_payload jsonb default '{}'::jsonb, p_dedupe_key text default null::text, p_priority integer default 50, p_not_before timestamp with time zone default null::timestamp with time zone, p_max_attempts integer default null::integer)
 returns agent.tasks
 language plpgsql
as $function$
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
                               'via', 'enqueue_repo_task'));
  elsif revived then
    insert into agent.events (worker_id, task_id, kind, detail)
    values (null, t.id, 'enqueued',
            jsonb_build_object('queue', p_queue, 'dedupe_key', p_dedupe_key,
                               'via', 'enqueue_repo_task',
                               'revived_from_cancelled', true));
  end if;
  return t;
end $function$;

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
       'developing-today/openchamber', 'dezren39/nix') then
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

commit;
