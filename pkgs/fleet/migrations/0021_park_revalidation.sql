-- 0021: #3424 P3 — scheduled long-park revalidation.
--
-- Problem (developing-today/code#3424 piece 3; triage task 6636 confirmed
-- the scope 2026-11-02, implementation task 6640): parked queue tasks are
-- never re-verified. A park whose ready condition cleared weeks ago sits
-- parked until a human notices, and nothing records when a park was last
-- checked or what it is waiting on.
--
-- Changes (additive, pilot functions only — the global agent.* task verbs
-- stay untouched, per the 0008 isolation rule):
--   1. agent.tasks gains three columns:
--        last_checked_at  — when the sweeper last re-verified this park
--                           (park_task stamps it at park time: a fresh park
--                           counts as checked at park time, so the sweeper
--                           only picks it up once it is >1 day old).
--        check_outcome    — 'confirmed' | 'stale' | 'released', the result
--                           of the last revalidation (NULL = never checked).
--        ready_condition  — JSONB written at park time saying what "done
--                           waiting" means for this park. Consumed whole by
--                           the sweeper and the waiting_class view, never
--                           queried by field from outside — the same
--                           sanctioned-pocket shape as the task payload
--                           (#3424 storage ruling, 2026-11-01). Shape:
--                           {"kind": <kind>, ...kind-specific refs}.
--                           Kinds the sweeper can verify from live DB
--                           state: 'task_terminal' {task_id},
--                           'task_status' {task_id, status},
--                           'event_seen' {task_id?, event_kind},
--                           'time' {until}. External kinds ('ci',
--                           'pr_merged', 'issue_closed', 'person_answer',
--                           'review', ...) are recorded faithfully and DO
--                           drive waiting_class, but a DB sweeper cannot
--                           verify them — see outcome 'stale' below.
--   2. agent.park_task gains p_ready_condition (default NULL, appended
--      last; existing named-arg callers are unaffected). When no explicit
--      condition is passed, park_task derives one from the progress
--      payload's 'ready_condition' key, else falls back to
--      {"kind": "time", "until": <not_before>} — so every new park carries
--      a condition. Parking also stamps last_checked_at = now() and
--      resets check_outcome to NULL.
--   3. agent.revalidate_parked_tasks(p_min_age, p_limit): the sweeper.
--      Daily runner: pg_cron (measured enabled on muse-dev, migration
--      0007 — its several-minutes slop is irrelevant at a daily cadence),
--      scheduled below as 'park-revalidation-daily' at 05:17 UTC.
--      Semantics per parked row whose last check is older than p_min_age
--      (default 1 day):
--        condition met      -> 'released': status='queued',
--                              not_before=NULL, a 'park_released' event
--                              carrying the condition + the finding note,
--                              and the same note in progress (the finding
--                              note lives in progress until the P4 notes
--                              table lands — P4 ordering decided here:
--                              progress first, notes table mirrors later).
--        condition unmet    -> 'confirmed': ONLY last_checked_at and
--                              check_outcome change; the park (status,
--                              not_before, condition) is untouched.
--        cannot verify      -> 'stale': no ready_condition, a malformed
--                              condition, a missing target row, or an
--                              external kind the DB cannot check. Park
--                              untouched except the timestamps, plus a
--                              'park_revalidated' event so the park
--                              surfaces daily instead of rotting silently.
--      Elapsed time is never evidence: a 'time' condition is always
--      'confirmed', never released by the sweeper — time parks become
--      claimable through the normal claim path at not_before.
--   4. agent.task_waiting_class view: derived waiting_class per
--      non-terminal task, by precedence (highest first):
--        person-answer > review > blocked > ci/external > capacity >
--        queued
--      computed from facts that exist today — status, ready_condition
--      kind, block reason_class, progress stage, an open attention
--      marker in progress (progress.attention.state = 'open') — never
--      from last-note-type alone. P1 (links) and P4 (notes table) have
--      not landed yet; when they do, a later migration adds open-blocker
--      links and open attention notes as inputs UNDER THE SAME
--      precedence (an open attention note forces person-answer even
--      when the latest finding is stale — acceptance case 2).
--      Claimed tasks report 'working'; the view's job is waiting tasks.
--
-- P0 gate (packet): the migration chain was re-listed 2026-11-02 before
-- applying — 0001..0020 all present in this directory, ending at
-- 0020_lane_identity.sql; the chain restored 2026-11-01 is complete, so
-- 0021 builds on the live claim_repo_task (0020) and park_task (0008)
-- definitions.
--
-- Rollback:
--   begin;
--   select cron.unschedule('park-revalidation-daily');  -- if pg_cron ran
--   drop view agent.task_waiting_class;
--   drop function agent.revalidate_parked_tasks(interval, integer);
--   drop function agent.park_task(bigint, text, text, integer, jsonb, jsonb);
--   -- restore the 0008 park_task body (5-arg) from
--   -- 0008_repo_queue_pilot.sql, then:
--   grant execute on function agent.park_task(bigint, text, text, integer, jsonb)
--     to service_role, authenticated;
--   alter table agent.tasks drop column ready_condition,
--                           drop column check_outcome,
--                           drop column last_checked_at;
--   commit;
--   notify pgrst, 'reload schema';

begin;

-- ---------------------------------------------------------------- columns
alter table agent.tasks
  add column if not exists last_checked_at timestamptz,
  add column if not exists check_outcome   text,
  add column if not exists ready_condition jsonb;

alter table agent.tasks drop constraint if exists tasks_check_outcome_check;
alter table agent.tasks add constraint tasks_check_outcome_check
  check (check_outcome is null
         or check_outcome in ('confirmed', 'stale', 'released'));

create index if not exists tasks_parked_revalidation_idx
  on agent.tasks (last_checked_at)
  where status = 'parked';

-- ------------------------------------------------------- park (extended)
-- 0008 body + ready_condition capture + last_checked_at stamp. The old
-- 5-arg signature is dropped; the 6-arg form defaults p_ready_condition
-- so named-arg callers passing the original five arguments keep working.
drop function if exists agent.park_task(bigint, text, text, integer, jsonb);

create or replace function agent.park_task(
  p_task_id bigint, p_worker text, p_reason text,
  p_delay_seconds integer default 1200, p_progress jsonb default '{}'::jsonb,
  p_ready_condition jsonb default null)
returns agent.tasks language plpgsql as $$
declare t agent.tasks; v_condition jsonb; v_not_before timestamptz;
begin
  v_not_before := now() + make_interval(secs => p_delay_seconds);
  v_condition := coalesce(
    p_ready_condition,
    coalesce(p_progress, '{}'::jsonb) -> 'ready_condition',
    jsonb_build_object('kind', 'time', 'until', to_jsonb(v_not_before)));
  update agent.tasks
     set status = 'parked', worker_id = null, lease_until = null,
         not_before = v_not_before,
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
                             'delay_seconds', p_delay_seconds,
                             'ready_condition', v_condition));
  return t;
end $$;

-- ---------------------------------------------------------------- sweeper
create or replace function agent.revalidate_parked_tasks(
  p_min_age interval default interval '1 day',
  p_limit integer default 500)
returns table(task_id bigint, outcome text) language plpgsql as $$
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
             progress = progress || jsonb_build_object(
               'park_revalidation', jsonb_build_object(
                 'outcome', 'released', 'at', to_jsonb(now()),
                 'condition', r.ready_condition,
                 'note', 'ready_condition met; released to queued by the '
                         'park revalidation sweeper (migration 0021)')),
             updated_at = now()
       where id = r.id;
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
end $$;

-- ----------------------------------------------------- waiting_class view
create or replace view agent.task_waiting_class as
select
  t.id as task_id,
  t.queue,
  t.status,
  (t.ready_condition ->> 'kind') as wait_kind,
  t.check_outcome,
  t.last_checked_at,
  case
    -- person-answer: an explicit person condition, an escalation block,
    -- or an open attention marker beats every other class — including a
    -- stale finding (check_outcome = 'stale' never lowers this class).
    when (t.ready_condition ->> 'kind') = 'person_answer'
      or (t.status = 'blocked'
          and t.result ->> 'reason_class' in
              ('escalated-to-user', 'drew_decision', 'person-answer'))
      or (t.progress -> 'attention' ->> 'state') = 'open'
      then 'person-answer'
    when t.status = 'needs_review'
      or (t.ready_condition ->> 'kind') in ('review', 'codex_review',
                                            'pr_review')
      then 'review'
    when t.status = 'blocked'
      or (t.ready_condition ->> 'kind') = 'blocked_on_task'
      or ((t.ready_condition ->> 'kind') = 'task_terminal'
          and coalesce(bt.status, '') not in ('done', 'cancelled'))
      or ((t.ready_condition ->> 'kind') = 'task_status'
          and bt.status is distinct from
              (t.ready_condition ->> 'status'))
      then 'blocked'
    when (t.ready_condition ->> 'kind') in
           ('ci', 'ci_green', 'external', 'upstream_pr', 'pr_merged',
            'issue_closed')
      or (t.progress ->> 'stage') = 'awaiting_ci'
      then 'ci/external'
    when (t.ready_condition ->> 'kind') in ('capacity', 'quota_reset',
                                            'time')
      or (t.status = 'queued' and t.not_before is not null
          and t.not_before > now())
      then 'capacity'
    when t.status = 'claimed' then 'working'
    else 'queued'
  end as waiting_class
from agent.tasks t
left join agent.tasks bt
  on (t.ready_condition ->> 'kind') in
       ('blocked_on_task', 'task_terminal', 'task_status')
 and (t.ready_condition ->> 'task_id') ~ '^[0-9]+$'
 and bt.id = (t.ready_condition ->> 'task_id')::bigint
where t.status not in ('done', 'failed', 'cancelled');

-- ------------------------------------------------------------- daily cron
-- pg_cron is enabled on muse-dev (migration 0007, measured). Schedule
-- only when the cron schema is present so this migration stays portable
-- to a pg_cron-less restore; the DO block is a no-op there and the
-- runner choice falls back to calling revalidate_parked_tasks() from
-- the fleet-monitor beat.
do $$
begin
  if exists (select 1 from pg_extension where extname = 'pg_cron') then
    if exists (select 1 from cron.job
                where jobname = 'park-revalidation-daily') then
      perform cron.unschedule('park-revalidation-daily');
    end if;
    perform cron.schedule('park-revalidation-daily', '17 5 * * *',
                          'select agent.revalidate_parked_tasks()');
  end if;
end $$;

-- ----------------------------------------------------------------- grants
grant execute on function agent.park_task(bigint, text, text, integer, jsonb, jsonb)
  to service_role, authenticated;
grant execute on function agent.revalidate_parked_tasks(interval, integer)
  to service_role, authenticated;
grant select on agent.task_waiting_class to service_role, authenticated, anon;

commit;

notify pgrst, 'reload schema';
