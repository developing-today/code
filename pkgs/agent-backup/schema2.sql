-- runtime-db-export expansion (2026-10-28, Drew): full sessions mirror,
-- per-conversation rollups, and the export-time-only statistics family.
-- Percentiles are nearest-rank picks via row_number() windows (no
-- percentile_cont in the muse.db vocabulary).

-- reshape session stats: add grain + full percentile/duration columns
DROP TABLE IF EXISTS agent.mirror_session_stats_daily;
CREATE TABLE agent.mirror_session_stats_daily (
  day date NOT NULL, grain text NOT NULL, key text NOT NULL,
  sessions_active bigint, sessions_started bigint,
  msgs_min bigint, msgs_max bigint, msgs_avg double precision,
  msgs_p50 bigint, msgs_p90 bigint, msgs_p95 bigint, msgs_p99 bigint,
  toks_min bigint, toks_max bigint, toks_avg double precision,
  toks_p50 bigint, toks_p90 bigint, toks_p95 bigint, toks_p99 bigint,
  dur_min_s bigint, dur_max_s bigint, dur_avg_s double precision,
  dur_p50_s bigint, dur_p90_s bigint, dur_p95_s bigint, dur_p99_s bigint,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, grain, key)
);

-- full mirror of agent.sessions (EMPTY at source at build time; the
-- derived per-session summary lives in agent.mirror_sessions)
CREATE TABLE IF NOT EXISTS agent.mirror_agent_sessions (
  session_id text PRIMARY KEY,
  root_request_id text,
  created_at timestamptz, updated_at timestamptz, archived_at timestamptz,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  exported_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS agent.mirror_history_agent_sessions (
  session_id text NOT NULL,
  payload jsonb NOT NULL, payload_hash text NOT NULL,
  valid_from timestamptz, valid_to timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS mirror_history_agent_sessions_key_idx
  ON agent.mirror_history_agent_sessions (session_id, valid_to);

-- derived sessions gain their chat link (chat_id = root agent's session_id
-- when a chat.chats row exists; only 38 chats exist at build time)
ALTER TABLE agent.mirror_sessions ADD COLUMN IF NOT EXISTS chat_id text;

-- per-message token distribution (global + per scheduler job)
CREATE TABLE IF NOT EXISTS agent.mirror_message_token_stats_daily (
  day date NOT NULL, grain text NOT NULL, key text NOT NULL,
  messages bigint,
  in_min bigint, in_max bigint, in_avg double precision,
  in_p50 bigint, in_p90 bigint, in_p95 bigint, in_p99 bigint,
  out_min bigint, out_max bigint, out_avg double precision,
  out_p50 bigint, out_p90 bigint, out_p95 bigint, out_p99 bigint,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, grain, key)
);

-- job-run outcomes + duration distribution (job_id '' = global)
CREATE TABLE IF NOT EXISTS agent.mirror_run_stats_daily (
  day date NOT NULL, job_id text NOT NULL,
  runs bigint, succeeded bigint, failed bigint, timeout bigint,
  cancelled bigint, other bigint,
  finished_runs bigint,
  dur_min_s bigint, dur_max_s bigint, dur_avg_s double precision,
  dur_p50_s bigint, dur_p90_s bigint, dur_p95_s bigint, dur_p99_s bigint,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, job_id)
);

-- fixed-bucket histograms (bucket_hi NULL = open-ended top bucket)
CREATE TABLE IF NOT EXISTS agent.mirror_histograms_daily (
  day date NOT NULL, grain text NOT NULL, key text NOT NULL,
  metric text NOT NULL, bucket_lo bigint NOT NULL, bucket_hi bigint,
  count bigint, exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, grain, key, metric, bucket_lo)
);

-- peak concurrency per hour (exact event-sweep maxima)
CREATE TABLE IF NOT EXISTS agent.mirror_concurrency_hourly (
  hour timestamptz PRIMARY KEY,
  max_concurrent_runs bigint, max_concurrent_sessions bigint,
  exported_at timestamptz NOT NULL DEFAULT now()
);

-- distinct counts per day (global + per job)
CREATE TABLE IF NOT EXISTS agent.mirror_distincts_daily (
  day date NOT NULL, grain text NOT NULL, key text NOT NULL,
  models_used bigint, sessions_active bigint, agents_active bigint,
  chats_active bigint, tools_used bigint,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, grain, key)
);

-- top-10 lists per day
CREATE TABLE IF NOT EXISTS agent.mirror_top_daily (
  day date NOT NULL, kind text NOT NULL, rank integer NOT NULL,
  ref text, label text, tokens bigint, messages bigint,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, kind, rank)
);

-- tool-call aggregates (source runtime.tool_calls/outputs EMPTY at build;
-- error rule is provisional until data exists: output error_text NOT NULL
-- or output status not in ('success','completed','ok'))
CREATE TABLE IF NOT EXISTS agent.mirror_tool_stats_daily (
  day date NOT NULL, tool_name text NOT NULL, server_name text NOT NULL,
  calls bigint, errors bigint,
  dur_avg_s double precision, dur_p95_s double precision, dur_max_s double precision,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, tool_name, server_name)
);

-- compaction stats (global + per job)
CREATE TABLE IF NOT EXISTS agent.mirror_compaction_stats_daily (
  day date NOT NULL, grain text NOT NULL, key text NOT NULL,
  compactions bigint,
  tokens_before_avg double precision, tokens_before_p95 bigint,
  tokens_after_avg double precision, tokens_after_p95 bigint,
  saved_avg double precision,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, grain, key)
);

-- subagent shape per day (global; depth + children distributions also
-- land in mirror_histograms_daily as spawn_depth / children_per_parent)
CREATE TABLE IF NOT EXISTS agent.mirror_subagent_stats_daily (
  day date PRIMARY KEY,
  spawns bigint, parents bigint,
  children_avg double precision, children_max bigint,
  children_p50 bigint, children_p90 bigint, children_p95 bigint, children_p99 bigint,
  exported_at timestamptz NOT NULL DEFAULT now()
);

-- per-conversation (per chat) daily rollup
CREATE TABLE IF NOT EXISTS agent.mirror_conversation_stats_daily (
  day date NOT NULL, chat_id text NOT NULL,
  sessions bigint, messages bigint,
  input_tokens bigint, output_tokens bigint,
  first_activity_epoch bigint, last_activity_epoch bigint,
  sess_msgs_p50 bigint, sess_msgs_p90 bigint,
  sess_toks_p50 bigint, sess_toks_p90 bigint,
  exported_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (day, chat_id)
);
