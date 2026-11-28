-- 0006_pruning.sql — event retention for the coordination + CI schemas.
-- History is NOT lost by pruning: the full database (events included) is
-- backed up to R2 every ~12h by the DIY content-addressed backup system
-- (~/workspace/supabase/muse-dev/backup/), which is the archive of record.
-- Pruning only keeps the live tables lean on the 500 MB Free instance.
-- Idempotent.

create index if not exists events_ts_idx on agent.events (ts);
create index if not exists ci_events_ts_idx on ci.events (received_at);

-- Delete agent.events older than p_keep_days; returns rows deleted.
-- Leaves one 'events_pruned' breadcrumb per run (itself prunable later).
create or replace function agent.prune_events(p_keep_days integer default 30)
returns integer language plpgsql as $$
declare n integer;
begin
  delete from agent.events
   where ts < now() - make_interval(days => p_keep_days)
     and kind <> 'events_pruned';
  get diagnostics n = row_count;
  delete from agent.events
   where ts < now() - make_interval(days => p_keep_days)
     and kind = 'events_pruned';
  insert into agent.events (worker_id, kind, detail)
  values ('system', 'events_pruned',
          jsonb_build_object('deleted', n, 'keep_days', p_keep_days));
  return n;
end $$;

-- Delete ci.events older than p_keep_days (CI history kept longer: 90d).
create or replace function ci.prune_events(p_keep_days integer default 90)
returns integer language plpgsql as $$
declare n integer;
begin
  delete from ci.events
   where received_at < now() - make_interval(days => p_keep_days);
  get diagnostics n = row_count;
  return n;
end $$;

grant execute on function agent.prune_events(integer) to service_role;
grant execute on function ci.prune_events(integer) to service_role;
