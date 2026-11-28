-- 0022: #3424 P1 — park accepts an absolute not_before datetime.
--
-- Problem (developing-today/code#3424 piece 1; split by task 5311,
-- implementation task 6634): agent.park_task accepts only a relative
-- p_delay_seconds, but external waits have known instants (a quota
-- reset, a review window). Enqueue already accepts an absolute
-- not_before; a mid-work park could not name one. Nothing recorded
-- whether a park was relative or absolute, or what class of thing it
-- is waiting on, in a queryable column.
--
-- Changes (additive, pilot functions only — the global agent.* task
-- verbs stay untouched, per the 0008 isolation rule):
--   1. agent.tasks gains two columns:
--        park_kind — 'relative' | 'absolute': how the current park's
--                    not_before was computed. Stamped by park_task on
--                    every park; NULL on tasks that are not parked
--                    (claim clears it when a park is resumed).
--        wait_kind — 'external' | 'capacity' | 'review' | 'ci' |
--                    'timer': the class of wait, derived at park time
--                    from the effective ready_condition kind:
--                      time                       -> timer
--                      ci, ci_green, pr_merged,
--                        issue_closed, upstream_pr -> ci
--                      review, codex_review,
--                        pr_review, person_answer  -> review
--                      capacity, quota_reset       -> capacity
--                      anything else / none        -> external
--                    (a park with no explicit condition is a pure
--                    timer). This is the column form of the same
--                    classification the 0021 task_waiting_class view
--                    derives from ready_condition; the view is left
--                    unchanged.
--   2. agent.park_task gains p_not_before timestamptz (default NULL,
--      appended last; existing named-arg callers are unaffected).
--      Exactly one of p_not_before / p_delay_seconds may be given:
--      both given -> exception; neither given -> the historical
--      default of 1200 seconds (so callers that relied on the old
--      p_delay_seconds default keep working). To make "given"
--      detectable, p_delay_seconds' default changes from 1200 to
--      NULL in the new signature; the 1200s fallback is applied
--      inside the body. With p_not_before, not_before is stored
--      exactly as named and park_kind = 'absolute'; the derived
--      ready_condition fallback becomes {"kind":"time","until":
--      <p_not_before>}. The park event detail records park_kind,
--      wait_kind, and whichever of delay_seconds / not_before was
--      used.
--   3. Partial index tasks_parked_due_idx on (not_before) where
--      status = 'parked': the claim predicate's parked branch
--      (status = 'parked' and not_before <= now()) and the due-park
--      scan read through it. The claim predicate itself already
--      excludes future not_before for both queued and parked rows
--      (0008 body, verified live 2026-11-03 before applying); this
--      migration makes that contract index-backed and test-covered
--      rather than rewriting a working claim function.
--   4. Backfill: every currently-parked row was parked through the
--      relative mechanism (the only one that existed), so park_kind
--      = 'relative' and wait_kind is derived from its stored
--      ready_condition with the mapping above.
--
-- P0 gate: the migration chain was re-listed 2026-11-03 before
-- applying — 0001..0021 all present in this directory, ending at
-- 0021_park_revalidation.sql — and the live park_task body was read
-- back and confirmed to be exactly the 0021 definition.
--
-- Rollback:
--   begin;
--   drop function agent.park_task(bigint, text, text, integer, jsonb, jsonb, timestamptz);
--   -- restore the 0021 park_task body (6-arg) from
--   -- 0021_park_revalidation.sql, then:
--   grant execute on function agent.park_task(bigint, text, text, integer, jsonb, jsonb)
--     to service_role, authenticated;
--   drop index agent.tasks_parked_due_idx;
--   alter table agent.tasks drop column wait_kind, drop column park_kind;
--   commit;
--   notify pgrst, 'reload schema';

begin;

-- ---------------------------------------------------------------- columns
alter table agent.tasks
  add column if not exists park_kind text,
  add column if not exists wait_kind text;

alter table agent.tasks drop constraint if exists tasks_park_kind_check;
alter table agent.tasks add constraint tasks_park_kind_check
  check (park_kind is null or park_kind in ('relative', 'absolute'));

alter table agent.tasks drop constraint if exists tasks_wait_kind_check;
alter table agent.tasks add constraint tasks_wait_kind_check
  check (wait_kind is null
         or wait_kind in ('external', 'capacity', 'review', 'ci', 'timer'));

-- ------------------------------------------------------------- due index
create index if not exists tasks_parked_due_idx
  on agent.tasks (not_before)
  where status = 'parked';

-- --------------------------------------------------------------- backfill
update agent.tasks
   set park_kind = 'relative',
       wait_kind = case ready_condition ->> 'kind'
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
       end
 where status = 'parked' and park_kind is null;

-- -------------------------------------------------------- park (extended)
-- 0021 body + p_not_before. The old 6-arg signature is dropped; the
-- 7-arg form defaults p_not_before so named-arg callers passing the
-- original six arguments keep working. p_delay_seconds' default
-- becomes NULL purely so "caller passed a delay" is detectable; the
-- body applies the historical 1200s default when neither is given.
drop function if exists agent.park_task(bigint, text, text, integer, jsonb, jsonb);

create or replace function agent.park_task(
  p_task_id bigint, p_worker text, p_reason text,
  p_delay_seconds integer default null, p_progress jsonb default '{}'::jsonb,
  p_ready_condition jsonb default null,
  p_not_before timestamptz default null)
returns agent.tasks language plpgsql as $$
declare t agent.tasks; v_condition jsonb; v_not_before timestamptz;
        v_park_kind text; v_wait_kind text;
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
  update agent.tasks
     set status = 'parked', worker_id = null, lease_until = null,
         not_before = v_not_before,
         park_kind = v_park_kind, wait_kind = v_wait_kind,
         ready_condition = v_condition,
         last_checked_at = now(), check_outcome = null,
         progress = progress || coalesce(p_progress, '{}'::jsonb),
         result = coalesce(result, '{}'::jsonb)
                  || jsonb_build_object('park_reason', p_reason),
         updated_at = now()
   where id = p_task_id and worker_id = p_worker and status = 'claimed'
   returning * into t;
  if not found then
    raise exception 'park_task: task % is not claimed by %', p_task_id, p_worker;
  end if;
  delete from agent.leases where task_id = p_task_id;
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, p_task_id, 'parked',
          jsonb_build_object('reason', p_reason,
                             'park_kind', v_park_kind,
                             'wait_kind', v_wait_kind,
                             'delay_seconds', p_delay_seconds,
                             'not_before', to_jsonb(v_not_before),
                             'ready_condition', v_condition));
  return t;
end $$;

-- ------------------------------------------------- claim clears park_kind
-- claim_repo_task already predicates on not_before for queued and
-- parked rows (verified live 2026-11-03); the only change here is
-- clearing park_kind/wait_kind when a claim resumes a parked task, so
-- the columns describe the current park only. Body is the live 0020
-- definition plus that one assignment.
create or replace function agent.claim_repo_task(
  p_worker text, p_queue text,
  p_lease_seconds integer default 2700, p_lane_id text default null)
returns agent.tasks language plpgsql as $$
declare t agent.tasks; v_was_parked boolean; v_lane agent.lanes;
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
  update agent.tasks
     set status = 'claimed', worker_id = p_worker, claimed_at = now(),
         lease_until = now() + make_interval(secs => p_lease_seconds),
         attempts = attempts + (case when v_was_parked then 0 else 1 end),
         lane_id = p_lane_id,
         park_kind = null, wait_kind = null,
         not_before = null, updated_at = now()
   where id = t.id returning * into t;
  insert into agent.leases (task_id, worker_id, lease_until, heartbeat_at)
  values (t.id, p_worker, t.lease_until, now())
  on conflict (task_id) do update
    set worker_id = excluded.worker_id,
        lease_until = excluded.lease_until, heartbeat_at = now();
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, t.id, 'claimed',
          jsonb_build_object('lease_seconds', p_lease_seconds,
                             'via', 'claim_repo_task',
                             'resumed_from_park', v_was_parked,
                             'lane_id', p_lane_id));
  return t;
end $$;

-- ----------------------------------------------------------------- grants
grant execute on function agent.park_task(bigint, text, text, integer, jsonb, jsonb, timestamptz)
  to service_role, authenticated;
grant execute on function agent.claim_repo_task(text, text, integer, text)
  to service_role, authenticated;

commit;

notify pgrst, 'reload schema';
