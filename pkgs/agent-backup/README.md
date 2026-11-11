# runtime-db-export

Periodic export of the Muse runtime records DB into the Turso DB
`muse-mirror` (`agent_mirror_*` tables). Built 2026-10-29, approved by
Drew 2026-10-28. Runs as an agent cron job `runtime-db-export` every 6 hours
(America/Chicago) — it must be an agent job because scripts cannot reach the
runtime DB (per-exec mount namespaces hide the socket); the only source
route is the `muse.db` tool inside an agent run.

**Repointed 2026-11-06** (code#5691): the landing side moved from muse-dev2
Supabase (`agent.mirror_*`) to the dedicated Turso DB `muse-mirror`
(`agent_mirror_*`), whose initial contents are the migration copy of the PG
tables (219,105 rows, parity-verified). The job was paused for the migration
and is **safe to unpause** — see the status banner at the top of JOB-BODY.md
for the first-run checks. Nothing in this directory writes to Postgres
anymore; `load_rows_pg.py` / `run_sql_pg.py` are the retired PG versions,
kept for reference/rollback only.

## Files

- `JOB-BODY.md` — the exact cron job body (self-contained: every query,
  loader spec, watermark rule). The stored cron body is character-identical
  to this file (verified by diff against the cron projection).
- `load_rows.py` — the landing loader (spec JSON on stdin; upserts, history
  archiving, epoch→date/hour conversion pinned to UTC; drops `_`-prefixed
  columns). Writes to Turso via `turso_client.py`.
- `turso_client.py` — the Turso transport (libsql HTTP pipeline) + the
  SQLite DDL for all 28 landing tables (created on first use; the 20
  migration-copied tables already exist). Token by BWS name only
  (`turso.muse-mirror-rw-token`), resolved via `fleet secrets get` with an
  automatic direct-BWS fallback when the broker's cached value is rejected.
  Also provides the staging mode: `RDE_TURSO_URL=file:/path.db` runs the
  identical SQL path against a local SQLite file.
- `load_from_output.py` / `page_load.py` — feed saved oversized muse.db
  outputs into the loader without rows passing through agent generation;
  page_load prints the next keyset cursor.
- `run_sql.py` — raw SQL files → Turso muse-mirror (prunes, reconciliation
  counts). Translates the PG dialect the job body uses (`agent.mirror_*`
  names, `now()`, `DATE 'YYYY-MM-DD'`) to SQLite/libsql.
- `set_state.py` — records a unit's outcome in BOTH `state.json` and
  `agent_mirror_export_state`.
- `spec_m1..m8.json`, `spec_r1.json`, `spec_r2.json`, `spec_s5.json` —
  loader specs for the saved-output flow.
- `schema.sql`, `schema2.sql` — the original PG DDL for all landing tables
  (historical; the live SQLite DDL lives in `turso_client.py`).
- `state.json` — per-unit watermarks/status (mirrored in muse-mirror).
- `progress/` — backfill logs per unit (build record).
- `merge_s5.py` — S5 merge helper from the build (superseded by the JOB-BODY
  inline merge; kept for reference).

## Landing tables (28, all in Turso `muse-mirror` as `agent_mirror_*`)

Mirrors: mirror_scheduler_jobs (162), mirror_scheduler_job_runs,
mirror_agent_agents, mirror_goals_goals (49), mirror_goals_updates,
mirror_scheduler_job_definitions (metadata only), mirror_agent_sessions
(source empty), mirror_sessions (derived per-session summaries).
History: mirror_history_{scheduler_jobs, agent_agents, goals_goals,
goals_updates, agent_sessions}. Rollups: mirror_token_usage_daily,
mirror_token_usage_hourly, mirror_session_activity_hourly.
Stats: mirror_message_token_stats_daily, mirror_session_stats_daily,
mirror_run_stats_daily, mirror_histograms_daily, mirror_concurrency_hourly,
mirror_distincts_daily, mirror_top_daily, mirror_tool_stats_daily,
mirror_compaction_stats_daily, mirror_subagent_stats_daily,
mirror_conversation_stats_daily. State: mirror_export_state.
(Counts as of the inaugural backfill are in the build report; they grow
every run.)

## Design decisions and adaptations

- **History granularity = run cadence.** mirror_history_* rows are written
  when a mutable row's payload_hash changes between runs; changes made and
  reverted between two runs collapse into one step. job_runs has NO history
  (immutable-ish; status transitions are refreshed in place by M2 REFRESH).
- **agent.agents lands a REDUCED payload.** spawn_metadata_json averages
  ~29KB/row (max 2.4MB — beyond muse.db's 64KB row cap) and
  last_assistant_message ~4KB; the payload carries all scalar/identity
  columns, the extracted scheduler_cron_job_id / worker phase, and the BYTE
  LENGTHS of the four big text columns. Full-text export of agents is
  impossible through muse.db.
- **scheduler.job_definitions is metadata-only** (task_text byte length +
  md5): bodies are job prompts; length+hash detect changes. Source table
  was empty at build anyway.
- **job_runs payload excludes** the raw device_read_state_json /
  pending_terminal_result_json / onboarding_tour_body /
  worker_approval_wait_json contents (NULL or approval blobs at build);
  the approval blob's byte length is kept.
- **agent.sessions is EMPTY at source**, so mirror_agent_sessions is empty
  and per-session summaries are DERIVED from agent.agents
  (mirror_sessions): session key = coalesce(root_session_id, session_id),
  with chat_id via chat.chats on the root agent's session_id.
- **Percentiles are nearest-rank** (row_number + rank
  greatest(1, ceil(p*n))) — the muse.db vocabulary has no percentile_cont.
  The "median" is the single middle rank ((n+1)/2 integer division): for
  even n it is the lower-middle value, not the average of two middles.
- **Histogram bucket edges are fixed** (listed in JOB-BODY S4; do not
  change). Zero-count buckets are skipped at load, so a recompute cannot
  remove a bucket that dropped to zero (documented caveat).
- **Concurrency is an exact event-sweep** (single-pass peaks + carry-in
  form; per-bucket correlated subqueries blow the 3s statement limit).
  Runs with finished_at set but started_at NULL (never started) contribute
  NO end event. Hour spine = hours with token usage (runs variant adds
  run-scheduled hours). Run-concurrency is only meaningful inside the
  job_runs retention window (~7 days at build); earlier hours read 0.
- **Rollups are upsert-only**: a recompute overwrites values for keys that
  still exist but cannot retract a key that vanished from the source
  grouping (e.g. an agent re-attributed to another job). Windows (last 4
  days daily, last 2 hours hourly) bound the staleness.
- **M3 mirror is append-only**: agents deleted at source are not pruned
  (at final build check the mirror held 12,726 rows vs 12,680 live source
  rows — source-side churn during the scan). M1/M4 snapshot tables DO
  prune (with history archiving).
- **S8 tool stats**: runtime.tool_calls / tool_outputs were EMPTY at
  build; the unit is landed-empty and its error rule (output error_text
  NOT NULL OR status outside ('success','completed','ok')) is PROVISIONAL
  until real data exists to validate against.
- **Anchors are UTC epochs**: first token day_epoch = 20724 (2026-09-28),
  first hour_epoch = 497390. (An early build note used 20728 as the first
  day — that was a wrong anchor, caught and repaired by a gap backfill;
  coverage now spans 2026-09-28 onward for every daily unit.)
- **Pad technique**: short-row aggregate pages return inline from muse.db
  and would have to be retyped; appending a 600-char `_pad` literal forces
  the runtime to save the page to a file, and the loader drops `_` columns.
  Page data then never passes through agent generation.

## Operations

- Inspect health: `agent_mirror_export_state` in muse-mirror
  (`turso db shell muse-mirror`) or `state.json` (last_status / last_error
  per unit; failures are isolated per unit and named in the run's report
  line).
- Manual run: `cron.run runtime-db-export`. Cadence changes need Drew.
- Steady-state cost (measured on the inaugural run, 2026-10-29):
  50,772,671 input tokens / 199,890 output across the run's 6-agent tree
  (worker root + live worker + 4 subagents), 3,067s wall. Usage rows post
  only at run completion. NOTE: subagents do NOT inherit the
  scheduler_cron_job_id spawn-metadata key, so the per-job rollup (R1/R2)
  attributes only the root agents' ~25.6M input to this job — the true
  all-in cost is the tree sum above. This is NOT a cheap job: at 6h
  cadence it projects to ~200M input tokens/day. If that burn is
  unwanted, levers are: longer cadence, narrower recompute windows, or a
  leaner run agent (fewer inline pages in context). Cadence changes need
  Drew.
