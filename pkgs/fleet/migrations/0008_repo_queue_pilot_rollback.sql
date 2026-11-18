-- 0008_repo_queue_pilot_rollback.sql — undo 0008_repo_queue_pilot.sql.
-- PREPARED 2026-10-25 alongside the pre-migration snapshot in
-- 0008_pre_snapshot/. DO NOT RUN casually: this removes the pilot queue
-- extension. It deliberately ABORTS if any task is sitting in one of the
-- pilot-only statuses (parked / blocked / needs_review) — move or cancel
-- those tasks first, because the restored status CHECK cannot hold them.
-- It also aborts if the pilot worker still owns/references any task.

do $$
begin
  if exists (select 1 from agent.tasks
              where status in ('parked','blocked','needs_review')) then
    raise exception 'rollback aborted: tasks exist in pilot-only statuses (parked/blocked/needs_review); resolve them first';
  end if;
  if exists (select 1 from agent.tasks where worker_id = 'repo-worker-code') then
    raise exception 'rollback aborted: repo-worker-code still referenced by tasks';
  end if;
  if exists (select 1 from agent.leases where worker_id = 'repo-worker-code') then
    raise exception 'rollback aborted: repo-worker-code still holds a lease';
  end if;
end $$;

-- Pilot-specific functions (signatures must match 0008 exactly).
drop function if exists agent.enqueue_repo_task(text, text, text, jsonb, text, integer, timestamptz, integer);
drop function if exists agent.claim_repo_task(text, text, integer);
drop function if exists agent.heartbeat_task(bigint, text, integer, jsonb);
drop function if exists agent.checkpoint_task(bigint, text, text, jsonb);
drop function if exists agent.park_task(bigint, text, text, integer, jsonb);
drop function if exists agent.block_task(bigint, text, text, text, jsonb);
drop function if exists agent.fail_repo_task(bigint, text, text, text);

-- Health view.
drop view if exists agent.repo_queue_health;

-- Claim-selection index.
drop index if exists agent.tasks_claimable_idx;

-- Restore the original five-value status CHECK from 0001_agent.sql.
alter table agent.tasks drop constraint if exists tasks_status_check;
alter table agent.tasks add constraint tasks_status_check
  check (status in ('queued','claimed','done','failed','cancelled'));

-- Added columns (dropping them discards pilot progress/not_before data).
alter table agent.tasks drop column if exists not_before;
alter table agent.tasks drop column if exists max_attempts;
alter table agent.tasks drop column if exists progress;
alter table agent.tasks drop column if exists last_error;

-- Pilot worker row (guarded above: only removed when unreferenced).
delete from agent.workers where id = 'repo-worker-code';

-- Note: agent.events rows written by pilot functions are audit history and
-- are intentionally left in place, matching the design's rollback section.
