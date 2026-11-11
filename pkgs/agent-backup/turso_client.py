#!/usr/bin/env python3
"""Shared Turso transport + landing-table DDL for the runtime-db-export
loader, repointed 2026-11-06 from muse-dev2 Postgres (agent.mirror_*)
to the dedicated Turso DB `fleet-mirror` (agent_mirror_* tables;
renamed from `muse-mirror` and converted libsql->TursoDB 2026-11-07,
code#6619).
Decision + evidence: developing-today/code#5691 (repoint, not retire).

Target access:
- HTTP pipeline (libsql /v2/pipeline) at the fleet-mirror host. The
  sandbox reaches it through the AMBIENT proxy environment (urllib's
  default opener honors it); do not override proxy settings.
- Token by NAME only: `turso.fleet-mirror-rw-token`, resolved via
  `fleet secrets get` (laptop broker first). If the DB rejects that
  value ("auth role not found" — the broker served a stale cached
  value after the BWS secret was rotated 2026-11-06; defect recorded
  on #5691), fall back to a direct BWS read of the same secret and
  retry once. The token value is never printed or written to a file.

Staging mode: RDE_TURSO_URL=file:/path/to.db runs the identical SQL
generation path against a local SQLite file, no token involved.
RDE_PRINT_SQL=1 prints the generated statements instead of executing.
"""
import base64
import json
import os
import sqlite3
import subprocess
import sys
import urllib.error
import urllib.request

DB_HTTP_URL = "https://fleet-mirror-dezren39.aws-us-east-1.turso.io/v2/pipeline"
TOKEN_NAME = "turso.fleet-mirror-rw-token"
FLEET = os.path.expanduser("~/tools/fleet")

# SQLite DDL for every landing table (translated from schema.sql +
# schema2.sql; PG types mapped per the Turso migration's typemap:
# bigint/integer -> INTEGER, double -> REAL, boolean -> INTEGER,
# date/timestamptz/jsonb -> TEXT). The 20 tables already in fleet-mirror
# make CREATE IF NOT EXISTS a no-op; the 5 history tables and 3 shells
# the migration deliberately did not copy are created here on first use.
DDL = {
    "agent_mirror_export_state": """CREATE TABLE IF NOT EXISTS agent_mirror_export_state (
  table_name TEXT PRIMARY KEY, watermark TEXT, last_run_at TEXT,
  last_row_count INTEGER, last_status TEXT, last_error TEXT)""",
    "agent_mirror_scheduler_jobs": """CREATE TABLE IF NOT EXISTS agent_mirror_scheduler_jobs (
  job_id TEXT PRIMARY KEY,
  is_heartbeat INTEGER, source_path TEXT, schedule_kind TEXT, enabled INTEGER,
  schedule_expr TEXT, timezone TEXT, last_status TEXT,
  consecutive_failures INTEGER, next_run_at_utc INTEGER,
  last_run_at_utc INTEGER, last_success_at_utc INTEGER,
  payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  exported_at TEXT NOT NULL)""",
    "agent_mirror_history_scheduler_jobs": """CREATE TABLE IF NOT EXISTS agent_mirror_history_scheduler_jobs (
  job_id TEXT NOT NULL, payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  valid_from TEXT, valid_to TEXT NOT NULL)""",
    "agent_mirror_scheduler_job_runs": """CREATE TABLE IF NOT EXISTS agent_mirror_scheduler_job_runs (
  run_id TEXT PRIMARY KEY,
  job_id TEXT, job_definition_id INTEGER,
  scheduled_for_utc INTEGER, started_at_utc INTEGER, finished_at_utc INTEGER,
  status TEXT, attempt INTEGER, worker_phase TEXT,
  worker_agent_id TEXT, worker_history_agent_id TEXT,
  payload TEXT NOT NULL, exported_at TEXT NOT NULL)""",
    "agent_mirror_agent_agents": """CREATE TABLE IF NOT EXISTS agent_mirror_agent_agents (
  agent_id TEXT PRIMARY KEY,
  id TEXT, session_id TEXT, kind TEXT, parent_id TEXT, parent_agent_id TEXT,
  root_session_id TEXT, request_id TEXT, model TEXT, status TEXT,
  depth INTEGER, ephemeral INTEGER, agent_type TEXT,
  created_at INTEGER, updated_at INTEGER,
  scheduler_cron_job_id TEXT, scheduler_cron_worker_phase TEXT,
  payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  exported_at TEXT NOT NULL)""",
    "agent_mirror_history_agent_agents": """CREATE TABLE IF NOT EXISTS agent_mirror_history_agent_agents (
  agent_id TEXT NOT NULL, payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  valid_from TEXT, valid_to TEXT NOT NULL)""",
    "agent_mirror_goals_goals": """CREATE TABLE IF NOT EXISTS agent_mirror_goals_goals (
  goal_id TEXT PRIMARY KEY,
  slug TEXT, title TEXT, status TEXT, momentum_status TEXT,
  parent_goal_id TEXT, source TEXT, updated_at TEXT,
  payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  exported_at TEXT NOT NULL)""",
    "agent_mirror_history_goals_goals": """CREATE TABLE IF NOT EXISTS agent_mirror_history_goals_goals (
  goal_id TEXT NOT NULL, payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  valid_from TEXT, valid_to TEXT NOT NULL)""",
    "agent_mirror_goals_updates": """CREATE TABLE IF NOT EXISTS agent_mirror_goals_updates (
  update_id TEXT PRIMARY KEY,
  goal_id TEXT, created_at TEXT, updated_at TEXT, author_source TEXT,
  payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  exported_at TEXT NOT NULL)""",
    "agent_mirror_history_goals_updates": """CREATE TABLE IF NOT EXISTS agent_mirror_history_goals_updates (
  update_id TEXT NOT NULL, payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  valid_from TEXT, valid_to TEXT NOT NULL)""",
    "agent_mirror_scheduler_job_definitions": """CREATE TABLE IF NOT EXISTS agent_mirror_scheduler_job_definitions (
  job_definition_id INTEGER PRIMARY KEY,
  job_id TEXT, version INTEGER, prompt_template_id INTEGER,
  created_at TEXT, retired_at TEXT,
  task_text_bytes INTEGER, task_text_md5 TEXT,
  payload TEXT NOT NULL, exported_at TEXT NOT NULL)""",
    "agent_mirror_token_usage_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_token_usage_daily (
  day TEXT NOT NULL, dimension TEXT NOT NULL, key TEXT NOT NULL,
  input_tokens INTEGER, output_tokens INTEGER, messages INTEGER,
  exported_at TEXT NOT NULL,
  PRIMARY KEY (day, dimension, key))""",
    "agent_mirror_token_usage_hourly": """CREATE TABLE IF NOT EXISTS agent_mirror_token_usage_hourly (
  hour TEXT NOT NULL, dimension TEXT NOT NULL, key TEXT NOT NULL,
  input_tokens INTEGER, output_tokens INTEGER, messages INTEGER,
  exported_at TEXT NOT NULL,
  PRIMARY KEY (hour, dimension, key))""",
    "agent_mirror_session_activity_hourly": """CREATE TABLE IF NOT EXISTS agent_mirror_session_activity_hourly (
  hour TEXT PRIMARY KEY,
  active_sessions INTEGER, new_sessions INTEGER, messages INTEGER,
  input_tokens INTEGER, output_tokens INTEGER,
  exported_at TEXT NOT NULL)""",
    "agent_mirror_session_stats_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_session_stats_daily (
  day TEXT NOT NULL, grain TEXT NOT NULL, key TEXT NOT NULL,
  sessions_active INTEGER, sessions_started INTEGER,
  msgs_min INTEGER, msgs_max INTEGER, msgs_avg REAL,
  msgs_p50 INTEGER, msgs_p90 INTEGER, msgs_p95 INTEGER, msgs_p99 INTEGER,
  toks_min INTEGER, toks_max INTEGER, toks_avg REAL,
  toks_p50 INTEGER, toks_p90 INTEGER, toks_p95 INTEGER, toks_p99 INTEGER,
  dur_min_s INTEGER, dur_max_s INTEGER, dur_avg_s REAL,
  dur_p50_s INTEGER, dur_p90_s INTEGER, dur_p95_s INTEGER, dur_p99_s INTEGER,
  exported_at TEXT NOT NULL,
  PRIMARY KEY (day, grain, key))""",
    "agent_mirror_sessions": """CREATE TABLE IF NOT EXISTS agent_mirror_sessions (
  session_id TEXT PRIMARY KEY,
  root_agent_id TEXT, root_kind TEXT, scheduler_cron_job_id TEXT,
  root_model TEXT, model_count INTEGER, agent_count INTEGER,
  created_at_epoch INTEGER, updated_at_epoch INTEGER,
  message_count INTEGER, input_tokens INTEGER, output_tokens INTEGER,
  payload TEXT NOT NULL, exported_at TEXT NOT NULL, chat_id TEXT)""",
    "agent_mirror_agent_sessions": """CREATE TABLE IF NOT EXISTS agent_mirror_agent_sessions (
  session_id TEXT PRIMARY KEY,
  root_request_id TEXT, created_at TEXT, updated_at TEXT, archived_at TEXT,
  payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  exported_at TEXT NOT NULL)""",
    "agent_mirror_history_agent_sessions": """CREATE TABLE IF NOT EXISTS agent_mirror_history_agent_sessions (
  session_id TEXT NOT NULL, payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  valid_from TEXT, valid_to TEXT NOT NULL)""",
    "agent_mirror_message_token_stats_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_message_token_stats_daily (
  day TEXT NOT NULL, grain TEXT NOT NULL, key TEXT NOT NULL,
  messages INTEGER,
  in_min INTEGER, in_max INTEGER, in_avg REAL,
  in_p50 INTEGER, in_p90 INTEGER, in_p95 INTEGER, in_p99 INTEGER,
  out_min INTEGER, out_max INTEGER, out_avg REAL,
  out_p50 INTEGER, out_p90 INTEGER, out_p95 INTEGER, out_p99 INTEGER,
  exported_at TEXT NOT NULL,
  PRIMARY KEY (day, grain, key))""",
    "agent_mirror_run_stats_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_run_stats_daily (
  day TEXT NOT NULL, job_id TEXT NOT NULL,
  runs INTEGER, succeeded INTEGER, failed INTEGER, timeout INTEGER,
  cancelled INTEGER, other INTEGER, finished_runs INTEGER,
  dur_min_s INTEGER, dur_max_s INTEGER, dur_avg_s REAL,
  dur_p50_s INTEGER, dur_p90_s INTEGER, dur_p95_s INTEGER, dur_p99_s INTEGER,
  exported_at TEXT NOT NULL,
  PRIMARY KEY (day, job_id))""",
    "agent_mirror_histograms_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_histograms_daily (
  day TEXT NOT NULL, grain TEXT NOT NULL, key TEXT NOT NULL,
  metric TEXT NOT NULL, bucket_lo INTEGER NOT NULL, bucket_hi INTEGER,
  count INTEGER, exported_at TEXT NOT NULL,
  PRIMARY KEY (day, grain, key, metric, bucket_lo))""",
    "agent_mirror_concurrency_hourly": """CREATE TABLE IF NOT EXISTS agent_mirror_concurrency_hourly (
  hour TEXT PRIMARY KEY,
  max_concurrent_runs INTEGER, max_concurrent_sessions INTEGER,
  exported_at TEXT NOT NULL)""",
    "agent_mirror_distincts_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_distincts_daily (
  day TEXT NOT NULL, grain TEXT NOT NULL, key TEXT NOT NULL,
  models_used INTEGER, sessions_active INTEGER, agents_active INTEGER,
  chats_active INTEGER, tools_used INTEGER,
  exported_at TEXT NOT NULL,
  PRIMARY KEY (day, grain, key))""",
    "agent_mirror_top_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_top_daily (
  day TEXT NOT NULL, kind TEXT NOT NULL, rank INTEGER NOT NULL,
  ref TEXT, label TEXT, tokens INTEGER, messages INTEGER,
  exported_at TEXT NOT NULL,
  PRIMARY KEY (day, kind, rank))""",
    "agent_mirror_tool_stats_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_tool_stats_daily (
  day TEXT NOT NULL, tool_name TEXT NOT NULL, server_name TEXT NOT NULL,
  calls INTEGER, errors INTEGER,
  dur_avg_s REAL, dur_p95_s REAL, dur_max_s REAL,
  exported_at TEXT NOT NULL,
  PRIMARY KEY (day, tool_name, server_name))""",
    "agent_mirror_compaction_stats_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_compaction_stats_daily (
  day TEXT NOT NULL, grain TEXT NOT NULL, key TEXT NOT NULL,
  compactions INTEGER,
  tokens_before_avg REAL, tokens_before_p95 INTEGER,
  tokens_after_avg REAL, tokens_after_p95 INTEGER,
  saved_avg REAL, exported_at TEXT NOT NULL,
  PRIMARY KEY (day, grain, key))""",
    "agent_mirror_subagent_stats_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_subagent_stats_daily (
  day TEXT PRIMARY KEY,
  spawns INTEGER, parents INTEGER,
  children_avg REAL, children_max INTEGER,
  children_p50 INTEGER, children_p90 INTEGER, children_p95 INTEGER,
  children_p99 INTEGER, exported_at TEXT NOT NULL)""",
    "agent_mirror_conversation_stats_daily": """CREATE TABLE IF NOT EXISTS agent_mirror_conversation_stats_daily (
  day TEXT NOT NULL, chat_id TEXT NOT NULL,
  sessions INTEGER, messages INTEGER,
  input_tokens INTEGER, output_tokens INTEGER,
  first_activity_epoch INTEGER, last_activity_epoch INTEGER,
  sess_msgs_p50 INTEGER, sess_msgs_p90 INTEGER,
  sess_toks_p50 INTEGER, sess_toks_p90 INTEGER,
  exported_at TEXT NOT NULL,
  PRIMARY KEY (day, chat_id))""",
}

INDEXES = [
    "CREATE INDEX IF NOT EXISTS agent_mirror_history_scheduler_jobs_key_idx ON agent_mirror_history_scheduler_jobs (job_id, valid_to)",
    "CREATE INDEX IF NOT EXISTS agent_mirror_history_agent_agents_key_idx ON agent_mirror_history_agent_agents (agent_id, valid_to)",
    "CREATE INDEX IF NOT EXISTS agent_mirror_history_goals_goals_key_idx ON agent_mirror_history_goals_goals (goal_id, valid_to)",
    "CREATE INDEX IF NOT EXISTS agent_mirror_history_goals_updates_key_idx ON agent_mirror_history_goals_updates (update_id, valid_to)",
    "CREATE INDEX IF NOT EXISTS agent_mirror_history_agent_sessions_key_idx ON agent_mirror_history_agent_sessions (session_id, valid_to)",
    "CREATE INDEX IF NOT EXISTS agent_mirror_scheduler_job_runs_job_idx ON agent_mirror_scheduler_job_runs (job_id, scheduled_for_utc)",
]


def physical_name(spec_table):
    """agent.mirror_scheduler_jobs -> agent_mirror_scheduler_jobs"""
    return spec_table.replace(".", "_")


def ensure_tables_sql(physical_names):
    out = []
    for name in physical_names:
        ddl = DDL.get(name)
        if ddl:
            out.append((ddl, []))
    for idx in INDEXES:
        if any(n in idx for n in physical_names):
            out.append((idx, []))
    return out


def _last_line(text):
    lines = [l for l in text.strip().splitlines() if l.strip()]
    return lines[-1].strip() if lines else ""


def token_from_fleet():
    out = subprocess.run([FLEET, "secrets", "get", TOKEN_NAME],
                         capture_output=True, text=True, timeout=60)
    if out.returncode != 0:
        raise RuntimeError(f"fleet secrets get failed rc={out.returncode}")
    return _last_line(out.stdout)


def token_from_bws():
    env = dict(os.environ)
    helper = os.path.expanduser("~/tools/bws-env.sh")
    script = (
        f"source {helper} >/dev/null 2>&1; "
        "SID=$(bws secret list 2>/dev/null | jq -r "
        f"'.[] | select(.key==\"{TOKEN_NAME}\") | .id'); "
        "bws secret get \"$SID\" | jq -r .value"
    )
    out = subprocess.run(["bash", "-c", script], capture_output=True,
                         text=True, timeout=60, env=env)
    if out.returncode != 0:
        raise RuntimeError(f"bws fallback failed rc={out.returncode}")
    return _last_line(out.stdout)


def _typed(value):
    if value is None:
        return {"type": "null"}
    if isinstance(value, bool):
        return {"type": "integer", "value": str(1 if value else 0)}
    if isinstance(value, int):
        return {"type": "integer", "value": str(value)}
    if isinstance(value, float):
        return {"type": "float", "value": value}
    if isinstance(value, (bytes, bytearray)):
        return {"type": "blob", "base64": base64.b64encode(bytes(value)).decode()}
    return {"type": "text", "value": str(value)}


def _plain(value):
    if isinstance(value, bool):
        return 1 if value else 0
    return value


class AuthError(RuntimeError):
    pass


class Turso:
    """Executes batches of (sql, [python values]) against fleet-mirror.

    A batch runs as one pipeline request sequence; statements in a batch
    share a session (temp tables work inside a batch)."""

    def __init__(self):
        self.url = os.environ.get("RDE_TURSO_URL", "")
        self.file_mode = self.url.startswith("file:")
        self.print_only = os.environ.get("RDE_PRINT_SQL") == "1"
        self._token = None
        self._token_source = None
        self._conn = None
        if self.file_mode:
            self._conn = sqlite3.connect(self.url[5:])
        if self.print_only:
            self.file_mode = False

    # -- remote ------------------------------------------------------
    def _pipeline(self, statements):
        reqs = [{"type": "execute", "stmt": {"sql": sql, "args": [_typed(v) for v in args]}}
                for sql, args in statements]
        reqs.append({"type": "close"})
        body = json.dumps({"requests": reqs}).encode()
        r = urllib.request.Request(
            DB_HTTP_URL, data=body,
            headers={"Authorization": f"Bearer {self._token}",
                     "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(r, timeout=180) as resp:
                data = json.load(resp)
        except urllib.error.HTTPError as e:
            detail = e.read().decode()[:300]
            if e.code in (401, 404) and "auth" in detail:
                raise AuthError(f"HTTP {e.code}: {detail}")
            raise RuntimeError(f"pipeline HTTP {e.code}: {detail}")
        results = []
        for item in data.get("results", []):
            if item.get("type") == "error":
                raise RuntimeError(f"pipeline error: {item['error'].get('message')}")
            res = item.get("response", {}).get("result")
            if res is not None:
                results.append(res)
        return results

    def _get_token(self, source):
        if source == "fleet":
            return token_from_fleet()
        return token_from_bws()

    # -- local file --------------------------------------------------
    def _file_exec(self, statements):
        results = []
        cur = self._conn.cursor()
        for sql, args in statements:
            cur.execute(sql, [_plain(v) for v in args])
            if cur.description:
                cols = [d[0] for d in cur.description]
                rows = [{c: v for c, v in zip(cols, row)} for row in cur.fetchall()]
                results.append({"cols": cols, "rows": rows})
            else:
                results.append({"rows_affected": cur.rowcount})
        self._conn.commit()
        return results

    # -- public ------------------------------------------------------
    def execute(self, statements):
        """statements: list of (sql, args). Returns per-statement result
        dicts (remote results normalized to {cols, rows} like file mode)."""
        if self.print_only:
            for sql, args in statements:
                print("SQL>", sql, "ARGS>", json.dumps([str(a)[:60] for a in args]))
            return []
        if self.file_mode:
            return self._file_exec(statements)
        # chunk into pipeline calls of at most 100 statements
        out = []
        for i in range(0, len(statements), 100):
            chunk = statements[i:i + 100]
            if self._token is None:
                self._token_source = "fleet"
                self._token = self._get_token("fleet")
            try:
                raw = self._pipeline(chunk)
            except AuthError:
                if self._token_source == "bws":
                    raise
                # broker value rejected: rotate to the direct BWS read
                self._token_source = "bws"
                self._token = self._get_token("bws")
                raw = self._pipeline(chunk)
            for res in raw:
                cols = [c.get("name") for c in res.get("cols", [])]
                rows = []
                for row in res.get("rows", []):
                    rows.append({c: (cell.get("value") if isinstance(cell, dict) else cell)
                                 for c, cell in zip(cols, row)})
                out.append({"cols": cols, "rows": rows,
                            "rows_affected": res.get("affected_row_count")})
        return out

    def note_token_source(self):
        return self._token_source


_CLIENT = None


def client():
    global _CLIENT
    if _CLIENT is None:
        _CLIENT = Turso()
    return _CLIENT
