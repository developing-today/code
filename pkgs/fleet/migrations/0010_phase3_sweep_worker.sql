-- 0010_phase3_sweep_worker.sql
-- Phase 3 (approved by Drew 2026-10-26): repo sweeps become queue
-- producers; Option B — the rotating sweep also claims from the per-repo
-- queues it produces into. This migration ONLY registers the sweep's
-- worker identity. No function changes; global claim/fail untouched
-- (pilot isolation rule from 0008). Queues are plain names on
-- agent.tasks (repo-health-opencode-pilot, repo-health-openchamber-pilot,
-- repo-health-nix-pilot); claim_repo_task is already queue-generic.
-- Rollback: delete the worker row (delete from agent.workers where
-- id = 'fleet-sweep-rotating') once no live task references it.

insert into agent.workers (id, role, display_name) values
  ('fleet-sweep-rotating', 'build', 'Rotating sweep — Phase 3 producer/consumer (opencode/openchamber/nix)')
on conflict (id) do nothing;
