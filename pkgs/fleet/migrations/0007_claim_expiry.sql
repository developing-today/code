-- 0007_claim_expiry.sql — opportunistic lease expiry inside claim_task.
-- Measured 2026-10-21: pg_cron IS enabled and its jobs DO run, but on this
-- Free (shared/Nano) instance the per-minute job fired ~once per several
-- minutes, not every minute. Lease expiry therefore cannot depend on
-- pg_cron alone. claim_task now runs agent.expire_leases() first, so
-- abandoned tasks become claimable again at the exact moment any worker
-- looks for work. The pg_cron schedule stays as a backstop. Idempotent.

create or replace function agent.claim_task(
  p_worker text, p_queue text default null, p_lease_seconds integer default 900)
returns agent.tasks language plpgsql as $$
declare t agent.tasks;
begin
  perform agent.expire_leases();
  select * into t from agent.tasks
   where status = 'queued' and (p_queue is null or queue = p_queue)
   order by priority asc, created_at asc
   for update skip locked limit 1;
  if not found then return null; end if;
  update agent.tasks
     set status = 'claimed', worker_id = p_worker, claimed_at = now(),
         lease_until = now() + make_interval(secs => p_lease_seconds),
         attempts = attempts + 1, updated_at = now()
   where id = t.id returning * into t;
  insert into agent.leases (task_id, worker_id, lease_until, heartbeat_at)
  values (t.id, p_worker, t.lease_until, now())
  on conflict (task_id) do update
    set worker_id = excluded.worker_id,
        lease_until = excluded.lease_until, heartbeat_at = now();
  insert into agent.events (worker_id, task_id, kind, detail)
  values (p_worker, t.id, 'claimed',
          jsonb_build_object('lease_seconds', p_lease_seconds));
  return t;
end $$;
