-- migration-0026: restore the guarded lane-session recovery modes in
-- bind_repo_task_lane_session (#5400).
--
-- migration-0022 introduced two narrowly reserved recovery modes for
-- the opencode claimant: when a bound session is verified deleted
-- while still zero-message, or verified absent from the server, the
-- claimant calls bind with p_launched_by set to
-- 'opencode-claimant-recovery:zero-message-deleted' or
-- 'opencode-claimant-recovery:gone-session' and p_session_id set to
-- the PRIOR session id. The function compare-and-clears that id
-- against BOTH agent.tasks.progress and agent.repo_task_lane_ledger
-- before clearing either binding, records stage 'launch_failed' on
-- the task, and consumes one attempt.
--
-- The migration-0023 record copy rewrote the function without the
-- recovery branches, and migration-0024 (the current definition)
-- kept that shape: its claimed + live-lease guard now rejects every
-- recovery call, because prompt.md only invokes recovery AFTER the
-- lease has expired. The claimant's documented recovery operation
-- (#4758) therefore fails against the recorded function contract.
--
-- This migration is migration-0024's bind_repo_task_lane_session
-- body with the migration-0022 recovery branch restored ahead of
-- the normal-bind guard. Nothing else changes:
--
-- * Task-row-first lock order (0024, Codex finding 4465191796) is
--   preserved in both branches: the task row is locked FOR UPDATE
--   before the ledger row is touched; the recovery branch then
--   locks the ledger row FOR UPDATE, as 0022 did.
-- * The normal branch is migration-0024 unchanged: a normal bind
--   still requires a claimed task with a live lease.
-- * Recovery-mode decisions (the point triage task 10034 left open):
--   recovery requires status 'claimed' AND an expired lease
--   (lease_until null or <= now()). prompt.md invokes recovery only
--   after lease expiry, and a live lease means the bound session's
--   launch window is still owned — a recovery call against a live
--   lease is refused, so a stale claimant can never clear a binding
--   whose replacement may still legitimately launch. Recovery also
--   still requires p_worker = 'opencode-claimant', the prior
--   session id present in BOTH stores (any mismatch raises and
--   leaves both stores unchanged), and attempts remaining.
--
-- revalidate_parked_tasks is not redefined here: migration-0024's
-- definition stands.

CREATE OR REPLACE FUNCTION agent.bind_repo_task_lane_session(p_task_id bigint, p_worker text, p_session_id text, p_launched_by text DEFAULT 'opencode-claimant'::text)
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

  -- Recovery branch (restored from migration-0022): the claimant
  -- uses these reserved launched_by values only after the old
  -- session is verified deleted-while-zero-message or verified
  -- absent, and only after the task lease has expired. Matching
  -- both stores before changing either makes recovery an atomic
  -- compare-and-clear rather than a progress-only checkpoint.
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
    select * into row_out from agent.repo_task_lane_ledger
     where task_id = p_task_id for update;
    if not found then
      raise exception 'bind_repo_task_lane_session: no lane ledger row for task %', p_task_id;
    end if;
    if t.lease_until is not null and t.lease_until > now() then
      raise exception 'bind_repo_task_lane_session: recovery for task % requires an expired lease (lease_until %)', p_task_id, t.lease_until;
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

  if t.status <> 'claimed' or t.lease_until is null or t.lease_until <= now() then
    raise exception 'bind_repo_task_lane_session: task % is not claimed with a live lease (status %, lease_until %)',
      p_task_id, t.status, t.lease_until;
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
