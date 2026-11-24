-- runtime-db-export landing tables (muse-dev Supabase, agent schema).
-- Created 2026-10-28. Mirrors of the Muse runtime records DB, exported
-- through the muse.db window by the runtime-db-export cron job.
-- payload_hash = md5(payload::text) computed in SQL on write.

CREATE TABLE IF NOT EXISTS agent.mirror_export_state (
  table_name     text PRIMARY KEY,
  watermark      text,
  last_run_at    timestamptz,
  last_row_count bigint,
  last_status    text,
  last_error     text
);

-- scheduler.jobs (snapshot + history)
CREATE TABLE IF NOT EXISTS agent.mirror_scheduler_jobs (
  job_id text PRIMARY KEY,
  is_heartbeat boolean, source_path text, schedule_kind text, enabled boolean,
  schedule_expr text, timezone text, last_status text,
  consecutive_failures bigint, next_run_at_utc bigint,
  last_run_at_utc bigint, last_success_at_utc bigint,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  exported_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS agent.mirror_history_scheduler_jobs (
  job_id text NOT NULL,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  valid_from timestamptz, valid_to timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS mirror_history_scheduler_jobs_key_idx
  ON agent.mirror_history_scheduler_jobs (job_id, valid_to);

-- scheduler.job_runs (incremental; mutable status, no history per design)
CREATE TABLE IF NOT EXISTS agent.mirror_scheduler_job_runs (
  run_id text PRIMARY KEY,
  job_id text, job_definition_id bigint,
  scheduled_for_utc bigint, started_at_utc bigint, finished_at_utc bigint,
  status text, attempt integer, worker_phase text,
  worker_agent_id text, worker_history_agent_id text,
  payload jsonb NOT NULL,
  exported_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS mirror_scheduler_job_runs_job_idx
  ON agent.mirror_scheduler_job_runs (job_id, scheduled_for_utc);

-- agent.agents (incremental, REDUCED payload + history: spawn_metadata_json
-- averages ~29KB/row and last_assistant_message ~4KB/row at source, so the
-- payload carries scalar/identity columns, the extracted scheduler-cron
-- attribution, and byte lengths of the big text columns instead of content)
CREATE TABLE IF NOT EXISTS agent.mirror_agent_agents (
  agent_id text PRIMARY KEY,
  id text, session_id text, kind text, parent_id text, parent_agent_id text,
  root_session_id text, request_id text, model text, status text,
  depth integer, ephemeral boolean, agent_type text,
  created_at bigint, updated_at bigint,
  scheduler_cron_job_id text, scheduler_cron_worker_phase text,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  exported_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS agent.mirror_history_agent_agents (
  agent_id text NOT NULL,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  valid_from timestamptz, valid_to timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS mirror_history_agent_agents_key_idx
  ON agent.mirror_history_agent_agents (agent_id, valid_to);

-- goals.goals (snapshot + history)
CREATE TABLE IF NOT EXISTS agent.mirror_goals_goals (
  goal_id text PRIMARY KEY,
  slug text, title text, status text, momentum_status text,
  parent_goal_id text, source text, updated_at text,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  exported_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS agent.mirror_history_goals_goals (
  goal_id text NOT NULL,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  valid_from timestamptz, valid_to timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS mirror_history_goals_goals_key_idx
  ON agent.mirror_history_goals_goals (goal_id, valid_to);

-- goals.updates (incremental + history)
CREATE TABLE IF NOT EXISTS agent.mirror_goals_updates (
  update_id text PRIMARY KEY,
  goal_id text, created_at text, updated_at text, author_source text,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  exported_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS agent.mirror_history_goals_updates (
  update_id text NOT NULL,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  valid_from timestamptz, valid_to timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS mirror_history_goals_updates_key_idx
  ON agent.mirror_history_goals_updates (update_id, valid_to);

-- scheduler.job_definitions (incremental, metadata only: job_definition_id,
-- job_id, version, timestamps + task_text byte length and md5; the prompt
-- bodies themselves are not mirrored. Source rows are immutable versions,
-- so upserts are DO NOTHING. Source table was EMPTY at build time.)
CREATE TABLE IF NOT EXISTS agent.mirror_scheduler_job_definitions (
  job_definition_id bigint PRIMARY KEY,
  job_id text, version integer, prompt_template_id bigint,
  created_at timestamptz, retired_at timestamptz,
  task_text_bytes integer, task_text_md5 text,
  payload jsonb NOT NULL,
  exported_at timestamptz NOT NULL DEFAULT now()
);

-- token usage rollups (recomputed over a rolling window each run)
CREATE TABLE IF NOT EXISTS agent.mirror_token_usage_daily (
  day date NOT NULL, dimension text NOT NULL, key text NOT NULL,
  input_tokens bigint, output_tokens bigint, messages bigint,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, dimension, key)
);
CREATE TABLE IF NOT EXISTS agent.mirror_token_usage_hourly (
  hour timestamptz NOT NULL, dimension text NOT NULL, key text NOT NULL,
  input_tokens bigint, output_tokens bigint, messages bigint,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (hour, dimension, key)
);

-- session rollups
CREATE TABLE IF NOT EXISTS agent.mirror_session_activity_hourly (
  hour timestamptz PRIMARY KEY,
  active_sessions bigint, new_sessions bigint, messages bigint,
  input_tokens bigint, output_tokens bigint,
  exported_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS agent.mirror_session_stats_daily (
  day date PRIMARY KEY,
  sessions bigint,
  msgs_min bigint, msgs_max bigint, msgs_avg double precision, msgs_median double precision,
  toks_min bigint, toks_max bigint, toks_avg double precision, toks_median double precision,
  exported_at timestamptz NOT NULL DEFAULT now()
);

-- per-session summaries (derived from agent.agents + token usage;
-- agent.sessions is EMPTY at source, so sessions are derived from
-- coalesce(root_session_id, session_id) over agent.agents)
CREATE TABLE IF NOT EXISTS agent.mirror_sessions (
  session_id text PRIMARY KEY,
  root_agent_id text, root_kind text, scheduler_cron_job_id text,
  root_model text, model_count bigint, agent_count bigint,
  created_at_epoch bigint, updated_at_epoch bigint,
  message_count bigint, input_tokens bigint, output_tokens bigint,
  payload jsonb NOT NULL,
  exported_at timestamptz NOT NULL DEFAULT now()
);
