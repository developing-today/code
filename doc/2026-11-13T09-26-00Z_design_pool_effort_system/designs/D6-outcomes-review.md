# D6 — Decision record, outcome capture, feedback gauges, PM review surface

- Program: pool & effort system (code#9399), design mill, queue item 15864.
- Component: D6 of `../main.md` §3. Binding: AD7 (every decision and attempt is an
  addressable event; feedback and outcomes are append-only rows that reference the
  ID), with AD5 (decision pools, D5), AD8 (export to the stats store, D3), AD10
  (self-switching is recorded as a decision).
- Status: design draft, detached run, nothing committed. Only this file and
  `../slices/D6-slices.json` were written.
- Measured inputs: `../baseline/v1-data-sources.md` (read-only opencode.db
  inspection, 2026-11-13 06:05–06:30 CST; live counts at drafting time: `route_attempt`
  649,532 rows, `session_v2` sub-sessions 3,423). No store was written to.

---

## 1. Scope

**Themes (inventory numbering as in `../main.md` §3):** T11 (the record part of the
decision/feedback theme), T13 (outcome capture and the learning loop), T16 (PM review
surface).

**Statements covered.** `inventory/statement-map.json` is still empty at drafting
time, so statements are cited by brief or supplement section and line. When the
inventory lands, each row below is re-keyed to its statement id; nothing else changes.

| Source | Sections used by D6 |
| --- | --- |
| `brief.md` | §5 (per-session and per-task data, stats list, days to complete, quota %), §6 (group-by any tag or label, per-sample review), §9 (V1 inputs: PM and roadmap stats extend, not replace) |
| `brief-decisions-supplement.md` | §3 (every decision gets an ID; caller and callee points gauges, separate), §4 (invalid-JSON path), §5 (outcome vocabulary, every-session reporting, linkage, per-session metrics), §6 (tie-break lean vs fallback, recorded distinctly), §7 (clause ids applied), §8 (PM review surface, four bullets), §9 (decision record fields) |
| `../brief-decisions-supplement.md` §3 | "Every decision gets an ID" is the basis of §4.1 below |
| `../../2026-11-13T11-54-00Z_design_pm_stack_prompt_surface/brief.md` §2, §5 | `not_confident` answers and `prompt_surface_ref` on decision records; chat sessions as a later reporter surface |

Out of scope: the decision executor and its prompt text (D5), the stats store, rollups
and quantiles (D3), trial design (D4), categories and self-switching guidance (D2).
D6 defines what is recorded and how outcomes attach; D3 defines how it is aggregated.

---

## 2. V1 — what exists today (cited, not redesigned)

### 2.1 The ledger D6 extends

| Fact | Evidence |
| --- | --- |
| `route_attempt` (one row per model call, incl. failover legs): `id`, `time_started/ended`, `session_id`, `assistant_message_id`, `route_id`, `provider_id`, `model_id`, `variant`, `outcome`, `error_*`, `failed_over_to`, `first_token_ms`, `response_ms`, `tokens_*` (input/output/reasoning/cache), `quota` JSON (reset instants only, no percent), `cost_usd` (nonzero on 50,367 rows), `continuation_of`, `credential_id` | `packages/core/src/database/migration/20261006083034_model_route_log.ts` (fork, origin/HEAD); `baseline/v1-data-sources.md` S1 |
| `route_decision` (per selection: `route_id`, `selection`, `candidates`, `chosen`, `reason`, `detail` with weights and exclusions `quota-skip` / `cooldown-until`); 2.43 M rows | same migration file; `baseline/v1-data-sources.md` S1b |
| `route_health` cooldown and quota events (63 k rows) | `baseline/v1-data-sources.md` S1c |
| `session_v2` with `parent_id` (3,423 sub-sessions at drafting), `agent`, `model` JSON (pool as `providerID` `opencode-route`), token totals, `cost` (zero on subscriptions), `metadata` JSON, `time_idle`, `subagent_policy` | fork `packages/core/src/database/schema.gen.ts`; baseline S2 |
| `session_message` with `content[type=tool]` (name, `state.input`), `reasoning`, `text`, model switches; the live turn stream since 2026-01-18 | baseline S2b. Legacy `message`/`part` frozen 2026-10-06 (G2) |
| Quota % readings per pool window (`usage-YYYY-MM.jsonl`, `quota-current.json`) with no attempt link | baseline S4, G5 |
| Queue `tasks` row: `queue`, `payload`, `status`, `priority`, `claim_provider`, `claim_model`, `claim_variant`, `claim_session_id` (the code#8176 stamp), `created_at`, `claimed_at`, `completed_at`, `blocked_deps` (free text), `ready_condition`, `check_outcome`, `result` JSON | `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs:884–906` |
| Queue `events` table (`worker_id`, `task_id`, `kind`, `detail` JSON, `created_at`) — exists today, unused for priority or outcome | `turso_queue.rs:928–933` |
| Queue CLI verbs: enqueue, list, claim, release, lane, bind, park, wake, block, unblock, cancel, revive, fail, complete, timeline, health, signal, outbox, stack. **No reprioritize verb.** | `pkgs/fleet/crates/fleet-queue/src/client.rs:5314–5340` (dispatch) |
| No notes table; no typed blocker links; no provenance on writes | `doc/2026-11-10_design_queue_task_data_model_3424.md`; baseline G8 |

### 2.2 The PM stack pieces D6 maps onto

| Component | What it does today | Evidence |
| --- | --- | --- |
| Decision sets (human rulings, not model decisions) | Sets of items with questions; first resolution wins; `answered` / `deferred` / `request-info` | `pm-stack/aggregator/src/decisions.ts` (header); routes `/api/decisions/<setId>/submit|defer|request-info|events` in `server.ts` (~lines 718–1189) |
| Decision-set append-only event log with outbox, 16 KB payload cap | Append-only JSONL plus outbox; sync to Turso by `eventId` with `INSERT OR IGNORE` | `pm-stack/aggregator/src/decision-events.ts`, `decision-events-sync.ts`; `pm-stack/decision-events-worker/schema.sql` |
| Item history, weekly complete cutoffs, versioned JSONL | Append-only capture, excludes incomplete weeks rather than reading them as zero | `pm-stack/aggregator/src/item-history.ts` |
| Canonical status | `Todo | In Progress | In Review | Done` with per-source mapping; unmapped is explicit | `pm-stack/aggregator/src/status-taxonomy.ts` |
| Search | `searchWork` over board, roadmap, registry, DNS, services, tiers | `pm-stack/aggregator/src/search-view.ts` |
| Epic graph and epic selector | Per-epic dependency and hierarchy graph from native relations | `pm-stack/aggregator/src/epic-graph.ts` |
| Queue view | Reconciled in-progress and queued view; `/queue` route | `pm-stack/aggregator/src/queue-view.ts`; `server.ts:2507` |
| Dashboard comments | One JSON file keyed by stable object ref, operator-token author binding | `pm-stack/aggregator/src/comments.ts` |
| Needs-Drew ranking | Ranked asks for Drew | `pm-stack/aggregator/src/needs-drew.ts` |
| Issue↔PR links | `issue_pr_links.json` (`repo#n` → PR numbers) | `~/.local/state/pm-aggregator/issue_pr_links.json` (baseline S7) |
| Genealogy design | `primary-parent/*` labels are the only queryable form; `## PM lineage` body block is a generated mirror, never truth | `doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy/note.md` §3 |
| Roadmap | `planning/roadmaps/pm-stack.md` (Now/Next/Later tables) | `planning/roadmaps/pm-stack.md` |
| Trail (precedent for a native record tool) | `trail_add` in the opencode-cockpit system instructions records any created or changed external thing | session system prompt; repo location to confirm (Q6) |

### 2.3 Constraints measured today

- **AGENTS.md is 7,493 bytes against an 8,000-byte budget** (`AGENTS.md`, checked by
  `scripts/research_workflow.py check`). Any guidance added to the index must fit in
  about 500 bytes; the full text goes to a topic file (`doc/agents/…`). This binds
  D6-S13.
- Standing rules that bind D6: GitHub via `gh` only; merge commits only (so commit
  trailers survive); CI on main only (so guards run on main and report, not gate PRs);
  secrets by name only.
- There is no decision-level data: no prompt version, no points gauges, no outcome
  store (baseline G9, G10). The V1 answer to "which prompt made this decision" is
  "unknowable".

---

## 3. Options considered

Each major choice lists at least four options. Assessment axes: architecture,
scalability, performance, reliability, extensibility. "Borrows from" points into
`../research/` by section.

### 3.1 Where the decision record lives

| # | Option | Architecture | Scale | Perf | Reliability | Extensibility | Borrows from |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A | **New tables in `opencode.db` beside `route_attempt`/`route_decision`, one transaction per decision (chosen for the record)** | Joins by id to the ledger with no copy; same migration path as `route_attempt` | Table grows ~1 KB/decision; D3 exports it | Write once at decision end, off the request path's critical reads | Single writer (the decision executor); migration tested on a copy | Schema versioned by `record_version` | AD7; R2 §5.5 record shape |
| B | JSONL file beside `decision-events.jsonl` | No migration; no joins | Fine; joins by script | Cheap | Partial lines on crash; no constraints | Weak | `decision-events.ts` pattern |
| C | Turso `fleet-platform` only | Cross-machine by default | Network in the decision path | Adds a network call per decision | Outage blocks records unless outboxed | Strong | `decision-events-sync.ts` |
| D | Langfuse as system of record | External dependency | Good | Network | Vendor-owned | Good dashboards | R2 §5.3, R3 §4.2 (projection only) |
| E | Postgres in the queue DB (R3 §7.1 item 4) | Couples to queue schema, which #3424 is redesigning | Good | Network | Couples to queue migrations | Relational joins | R3 §7.1 |

**Chosen: A** for the record and its legs. Feedback, outcomes and links are not
decision-local (raters and sweepers run in other sessions and machines), so they go to
the cross-machine sink in §3.2. This is a placement refinement of AD7, not a change to
its rule (see §7.4, challenge 1).

### 3.2 Where feedback, outcomes and links live

| # | Option | Verdict |
| --- | --- | --- |
| A | Append-only tables in the fleet-platform Turso DB, idempotent by id, with a local JSONL outbox (the `decision-events` precedent) — **chosen** | Cross-machine, multi-writer, no queue-schema coupling; `INSERT OR IGNORE` by id makes retries safe |
| B | GitHub labels only | Rejected as system of record: removable, not append-only, no evidence, no severity. Kept as a **projection** for a short list of labels (§4.6) because labels are the only queryable form (genealogy note §3) |
| C | GitHub comments with a fixed grammar | Rejected: parse-fragile, no typed fields |
| D | The dashboard `comments.ts` store | Rejected: keyed by ref but untyped, one JSON file, not a fact table |
| E | Postgres (R3 §7.1 item 4) | Rejected for V2: the queue DB is Postgres by default and Turso optionally, and #3424 is redesigning its schema; coupling outcome capture to it means outcomes wait on queue migrations. Revisit in V3 with D3 |

### 3.3 How feedback gauges are represented

| # | Option | Verdict |
| --- | --- | --- |
| A | One row per rating event with two integer columns, `points_correct` and `points_wrong`, each 0–5, each nullable but not both null — **chosen** | Matches "separate gauges" literally (a 5/5 is valid); append-only; maps to two Langfuse numeric scores on export |
| B | One row per gauge reading (`gauge ∈ {correct, wrong}`) | Equivalent; more rows; rejected for readability |
| C | A signed net score −5…+5 | Rejected: collapses the two gauges the supplement asks to keep apart |
| D | Categorical scale (e.g. good/mixed/bad) | Rejected: loses 0–5 resolution the supplement names |
| E | Langfuse numeric scores as the store | Rejected for SoT; used as export (R2 §5.3) |

### 3.4 Outcome vocabulary and detection

| # | Option | Verdict |
| --- | --- | --- |
| A | Closed registry file (`pm-stack/outcome-labels.json`) with definition, polarity, detection kind, severity guide, projection flag; observations reference a registry version — **chosen** | Closed set, validated writes, versioned; extension is a PR, not a silent new string |
| B | Free text labels | Rejected: no group-by |
| C | GitHub labels only | Rejected (§3.2 B) |
| D | Boolean Langfuse score per label | Rejected as SoT (R2 §5.3 notes it is one value per score); usable as projection |
| E | LLM classifies each outcome | Rejected for automatic labels (R3 §5.3: rework is a file-level test, not a judgement). Allowed later as *suggestions* only (V3) |

### 3.5 Linkage capture

| # | Option | Verdict |
| --- | --- | --- |
| A | Git trailers (`Task-Id:`, `Session-Id:`, `Decision-Id:`, `Assisted-by:`) on commits + a mirrored PR-body block + the queue claim stamp + session metadata — **chosen** | Trailers travel with the commit and survive merge commits (merge-commit policy); `git interpret-trailers` reads them; the claim stamp already exists |
| B | Git notes (`refs/notes/…`) | Rejected: not pushed by default, invisible to `git log` |
| C | Time-window correlation | Rejected as the join: heuristic; kept only as a fallback that is labelled "inferred" |
| D | Langfuse trace ids only | Rejected as the join; carried as an optional field |
| E | Branch-name conventions only | Rejected: branch names are not stable under rename |

Assisted-by follows the kernel convention and the cue issue cited in R3 §5.2
(`https://github.com/cue-lang/cue/issues/4452`), which rejects `Co-authored-by` for AI
attribution.

### 3.6 Report-anything mechanism

| # | Option | Verdict |
| --- | --- | --- |
| A | `fleet outcome report|rate|list` CLI (the write path) + a native tool wrapper with the `trail_add` shape + AGENTS guidance in a topic file — **chosen** | Works from a shell, a subagent, or a tool call; one writer; provenance from the session env |
| B | Only a native tool | Rejected: subagents and shell scripts cannot reach it |
| C | Only the mcpx code-mode namespace | Rejected for V2: mcpx call ledger is missing (code#9374); kept as a later projection |
| D | Only the PM dashboard | Rejected: a session cannot report from inside a conversation |
| E | File a GitHub issue per report | Rejected: floods the tracker; the supplement wants any session to report on any item |

### 3.7 Automatic detection of rework and reverts

| # | Option | Verdict |
| --- | --- | --- |
| A | Mechanical sweeps: `gh` for merged/closed/reverted/duplicate, `git log` trailers and `This reverts commit` for reverts, file-overlap churn in a fixed window for rework — **chosen** | Deterministic, re-runnable, golden-testable (R3 §5.3) |
| B | LLM judgement of "was this rework" | Rejected for automatic labels |
| C | `git blame` line survival | Accurate but heavy; V3 candidate for `work-undone` |
| D | GitClear-style two-week churn as a vendor-defined prior | Used only as the default window (14 days) and as a prior, not as truth (R3 §5.1) |

### 3.8 Review sets (PM review surface)

| # | Option | Verdict |
| --- | --- | --- |
| A | Sets are **views** over items with append-only membership; scores attach to items and decisions, never to sets — **chosen** | Each set is standalone; scores are not duplicated per set; the same item can sit in many sets |
| B | Sets own their scores (container model) | Rejected: a score in one set is invisible in another; duplicates |
| C | Saved searches only | Rejected: no marked set, no custom subset |
| D | Reuse the decision-set container (`decisions.ts`) as-is | Partial reuse: its item/question model and first-wins rule are for human rulings; review needs many ratings per item. Reuse the event-log pattern, not the container |
| E | Kaneo views | Rejected: one board, not cross-repo, not writable with gauges |

### 3.9 Per-session metrics: derived or stored

| # | Option | Verdict |
| --- | --- | --- |
| A | Derived on read from `route_attempt` and `session_message` by pure functions with golden tests; D3 exports the raw rows — **chosen for V2** | No mutable session row; no second store |
| B | Mutable `session_metrics` row updated at session end | Rejected: violates append-only |
| C | Append-only `session_fact` snapshot at session close | Accepted as a V3 option (§5) when D3 needs point-in-time facts |
| D | Materialized view in SQLite | Rejected: a plugin-owned view on a live DB is fragile |

---

## 4. V2 design — the chosen options, precisely

### 4.1 Identifiers

| Entity | Format | Source of truth | Notes |
| --- | --- | --- | --- |
| Decision | `dec_<ULID>` (26-char Crockford base32 after the prefix) | `decision_record.id` | ULID sorts by time; minted at decision start, so it exists before any leg runs |
| Decision leg | `(decision_id, seq, attempt_no)` | `decision_leg` PK | `attempt_no` is 1 or 2 (invalid-JSON retry) |
| Prompt version | `(prompt_id, version)` plus `prompt_date`, `sha256` | `decision_prompt_version` | Full body stored once per version |
| Feedback | `fbk_<ULID>` | `decision_feedback.id` | Append-only |
| Outcome | `out_<ULID>` | `outcome_observation.id` | Append-only |
| Outcome link | `olk_<ULID>` | `outcome_link.id` | Append-only |
| Task | `queue:<queue>#<tasks.id>` | queue `tasks` | Integer id is stable within a queue |
| Item | `<owner>/<repo>#<number>` | GitHub | Issue or PR; epics are items with the `epic` label |
| Session | `ses_…` | `session_v2.id` | Sub-session = row with `parent_id` |
| Attempt | `att:<route_attempt.id>` | `route_attempt` | |
| Tool call | `tc:<session_message.id>#<index>` | `session_message` content index | |
| Route selection | `rd:<route_decision.id>` | `route_decision` | |
| Review set | `rs_<ULID>` | `review_set.id` | |

### 4.2 Decision record (opencode.db, V2 migration)

One migration file under `packages/core/src/database/migration/` (fork), following
`20261006083034_model_route_log.ts`; `schema.gen.ts` regenerated. Tables are additive,
so the migration is safe to run beside the existing writers.

```sql
CREATE TABLE decision_prompt_version (
  prompt_id    TEXT NOT NULL,
  version      TEXT NOT NULL,                      -- semver, e.g. '1.0.0'
  prompt_date  TEXT NOT NULL,                      -- ISO date, UTC, when the version was cut
  sha256       TEXT NOT NULL CHECK (length(sha256) = 64),
  body         TEXT NOT NULL CHECK (length(body) <= 16384),  -- full text, once per version
  created_at   INTEGER NOT NULL,
  PRIMARY KEY (prompt_id, version)
);

CREATE TABLE decision_record (
  id                     TEXT PRIMARY KEY CHECK (id GLOB 'dec_*'),
  record_version         INTEGER NOT NULL DEFAULT 1,
  decision_pool          TEXT NOT NULL,            -- route id, e.g. 'decision'
  decision_type          TEXT NOT NULL,            -- 'category' | 'route' | 'self-switch' | 'classify' | ... (D5 registry)
  caller_session_id      TEXT,                     -- session that asked
  subject_session_id     TEXT,                     -- session whose work is decided (sub-session), nullable
  task_ref               TEXT,                     -- 'queue:<queue>#<id>', nullable
  prompt_id              TEXT,                     -- NULL when no LLM leg ran
  prompt_version         TEXT,
  schema_id              TEXT NOT NULL,            -- e.g. 'system-one'
  schema_version         TEXT NOT NULL,
  request_sha256         TEXT NOT NULL CHECK (length(request_sha256) = 64),
  request_bytes          INTEGER NOT NULL CHECK (request_bytes >= 0),
  started_at             INTEGER NOT NULL,         -- epoch ms
  finished_at            INTEGER NOT NULL,
  time_to_decision_ms    INTEGER NOT NULL CHECK (time_to_decision_ms >= 0),
  status                 TEXT NOT NULL CHECK (status IN ('answered','default','failed')),
  answered_leg_seq       INTEGER,                  -- NULL unless status = 'answered'
  answered_by            TEXT,                     -- 'provider/model' (never a pool name)
  answered_via_pool      TEXT,                     -- route id that contained the answering leg
  default_answer         INTEGER NOT NULL DEFAULT 0 CHECK (default_answer IN (0,1)),
  default_answer_id      TEXT,
  answer_json            TEXT CHECK (length(answer_json) <= 8192),
  probabilities_json     TEXT CHECK (length(probabilities_json) <= 8192),
  not_confident_count    INTEGER NOT NULL DEFAULT 0,
  effort_lean_requested  TEXT CHECK (effort_lean_requested IN ('low','medium','high','none')),
  effort_lean_applied    INTEGER NOT NULL DEFAULT 0 CHECK (effort_lean_applied IN (0,1)),
  effort_lean_delta      REAL,                     -- tie-break magnitude; NULL unless applied
  fallback_pool_used    INTEGER NOT NULL DEFAULT 0 CHECK (fallback_pool_used IN (0,1)),  -- supp §6 axis 2
  json_attempts_total    INTEGER NOT NULL DEFAULT 0,
  invalid_json_legs      INTEGER NOT NULL DEFAULT 0,
  tokens_input           INTEGER NOT NULL DEFAULT 0,
  tokens_output          INTEGER NOT NULL DEFAULT 0,
  tokens_cache_read      INTEGER NOT NULL DEFAULT 0,
  tokens_reasoning       INTEGER NOT NULL DEFAULT 0,
  neurons                INTEGER,                  -- Cloudflare leg, NULL elsewhere
  cost_usd               REAL,                     -- NULL when unknown; subscriptions read 0 (baseline G4)
  quota_snapshot_json    TEXT CHECK (length(quota_snapshot_json) <= 2048),
  route_decision_id      INTEGER,                  -- route_decision.id when the proxy wrote one
  clauses_applied_json   TEXT CHECK (length(clauses_applied_json) <= 2048),  -- D5 clause ids (supp §7)
  prompt_surface_ref     TEXT,                     -- pm-stack prompt-surface request id, nullable
  created_at             INTEGER NOT NULL
);

CREATE TABLE decision_leg (
  decision_id        TEXT NOT NULL REFERENCES decision_record(id),
  seq                INTEGER NOT NULL CHECK (seq >= 1),
  attempt_no         INTEGER NOT NULL DEFAULT 1 CHECK (attempt_no IN (1,2)),
  leg_ref            TEXT NOT NULL,                -- e.g. 'decision/cf-clef-flash'
  leg_kind           TEXT NOT NULL CHECK (leg_kind IN ('native','llm')),
  provider_id        TEXT,
  model_id           TEXT,
  variant            TEXT,                         -- the rung actually sent (AD1), not the requested one
  outcome            TEXT NOT NULL CHECK (outcome IN (
                       'answered','invalid-json-retry','invalid-json-exhausted',
                       'schema-invalid','provider-error','timeout','refused',
                       'unsupported-tier','cycle-skipped','walled-skipped',
                       'budget-skipped','not-configured')),
  json_status        TEXT CHECK (json_status IN ('valid','invalid-json','schema-invalid','repaired')),
  repairs_json       TEXT CHECK (length(repairs_json) <= 512),   -- e.g. ["renormalized probabilities"]
  error_code         TEXT CHECK (length(error_code) <= 64),
  invalid_sample_sha256 TEXT CHECK (length(invalid_sample_sha256) = 64),  -- hash of invalid output, never the text
  route_attempt_id   INTEGER,                      -- route_attempt.id when an HTTP call was made
  tokens_input       INTEGER NOT NULL DEFAULT 0,
  tokens_output      INTEGER NOT NULL DEFAULT 0,
  tokens_cache_read  INTEGER NOT NULL DEFAULT 0,
  tokens_reasoning   INTEGER NOT NULL DEFAULT 0,
  neurons            INTEGER,
  cost_usd           REAL,
  ttft_ms            REAL,
  total_ms           REAL,
  PRIMARY KEY (decision_id, seq, attempt_no)
);

-- Append-only (AD7): rows are written once, in one transaction at decision end.
CREATE TRIGGER decision_record_no_update BEFORE UPDATE ON decision_record
  BEGIN SELECT RAISE(ABORT, 'decision_record is append-only'); END;
CREATE TRIGGER decision_record_no_delete BEFORE DELETE ON decision_record
  BEGIN SELECT RAISE(ABORT, 'decision_record is append-only'); END;
CREATE TRIGGER decision_leg_no_update BEFORE UPDATE ON decision_leg
  BEGIN SELECT RAISE(ABORT, 'decision_leg is append-only'); END;
CREATE TRIGGER decision_leg_no_delete BEFORE DELETE ON decision_leg
  BEGIN SELECT RAISE(ABORT, 'decision_leg is append-only'); END;
CREATE TRIGGER decision_prompt_version_no_update BEFORE UPDATE ON decision_prompt_version
  BEGIN SELECT RAISE(ABORT, 'prompt versions are immutable'); END;
CREATE TRIGGER decision_prompt_version_no_delete BEFORE DELETE ON decision_prompt_version
  BEGIN SELECT RAISE(ABORT, 'prompt versions are immutable'); END;

CREATE INDEX decision_record_caller_idx ON decision_record (caller_session_id, started_at);
CREATE INDEX decision_record_subject_idx ON decision_record (subject_session_id);
CREATE INDEX decision_record_task_idx ON decision_record (task_ref);
CREATE INDEX decision_record_pool_time_idx ON decision_record (decision_pool, started_at);
CREATE INDEX decision_leg_attempt_idx ON decision_leg (route_attempt_id);
```

**Invariants** (checked by the guard in D6-S4, property-tested):

1. `status = 'answered'` ⇔ `answered_leg_seq` is set and that leg row has `outcome = 'answered'`.
2. `default_answer = 1` ⇔ `status = 'default'` and `default_answer_id` is set.
3. `status = 'failed'` ⇔ no leg answered and no default is configured.
4. `attempt_no = 2` only when the same leg's `attempt_no = 1` has `outcome = 'invalid-json-retry'`.
5. A leg's `outcome` is a member of the closed vocabulary; `cycle-skipped` and
   `walled-skipped` legs carry no `route_attempt_id` (they made no call).
6. `finished_at ≥ started_at`, and `time_to_decision_ms = finished_at − started_at`.
7. Legs are ordered by `seq`, and `seq` values are contiguous from 1.
8. Sum of leg tokens equals the record's token totals.

**Write path.** The decision executor (D5) collects leg results in memory while the
decision runs, then in one transaction inserts `decision_prompt_version` (insert-or-
verify), `decision_record`, and all `decision_leg` rows. A crash mid-decision leaves no
record, which is the honest outcome: nothing was decided. The route proxy's
`route_attempt` row is written by the proxy as today; the leg stores its id.

**Prompt version rule.** The executor loads the decider prompt, computes `sha256` over
its exact bytes, and inserts `(prompt_id, version)` if absent. If the same version
exists with a different `sha256`, the executor refuses to run the LLM leg and records
`not-configured` on it. A prompt change must bump the version. The row stores the
version and date in full; the body is stored once.

**Request.** Only `request_sha256` and `request_bytes` are stored. The request can be
recovered only if the caller keeps it (D5 owns that). Debug capture of request bodies
is off by default (§4.7).

**Quota.** `quota_snapshot_json` holds the pool-window percentages read at decision
start from the pm-aggregator's current quota reading, with the reading's `observed_at`.
This closes baseline G5 for decisions only. Attempt-level percent is still inferred
(Q-src-6) and is labelled as such.

**Effort fields.** `effort_lean_requested` is what the caller asked for, `effort_lean_applied`
and `effort_lean_delta` say whether the tie-break actually moved the result, and
`fallback_pool_used` says whether the fallback pool (supplement §6, axis 2) answered.
The three are separate so the analysis never confuses the tie-break with the fallback.

### 4.3 Feedback — `decision_feedback` (fleet-platform Turso, V2)

Feedback rows are written by any session, human or system, through `fleet outcome rate`
(§4.9). Each row is one rating event.

```sql
CREATE TABLE decision_feedback (
  id             TEXT PRIMARY KEY CHECK (id GLOB 'fbk_*'),
  decision_id    TEXT NOT NULL,                    -- dec_… (may be a decision in another machine's opencode.db)
  rater_ref      TEXT NOT NULL,                    -- ses_… | 'human:<login>' | 'system:<job>'
  rater_kind     TEXT NOT NULL CHECK (rater_kind IN ('session','human','system')),
  role           TEXT NOT NULL CHECK (role IN ('caller','callee','other')),
  perspective    TEXT NOT NULL CHECK (perspective IN ('self','parent','subsession','other')),
  points_correct INTEGER CHECK (points_correct IS NULL OR points_correct BETWEEN 0 AND 5),
  points_wrong   INTEGER CHECK (points_wrong   IS NULL OR points_wrong   BETWEEN 0 AND 5),
  CHECK (points_correct IS NOT NULL OR points_wrong IS NOT NULL),
  subject_ref    TEXT,                             -- item/task the rating is grounded in, nullable
  reason         TEXT CHECK (length(reason) <= 500),
  rated_at       TEXT NOT NULL,                    -- ISO-8601 UTC
  provenance     TEXT NOT NULL                     -- JSON: host, pid, session, lane, runtime (#3424 piece 6 shape)
);
```

**Meaning of the axes.**

- `role` is the decision's side: `caller` (the session that asked for the decision),
  `callee` (the session the decision routed work to, i.e. the sub-session), `other`
  (anyone else).
- `perspective` is the rater's relation to the decided subsession: `self` (the
  subsession rates itself), `parent` (its parent rates it), `subsession` (a child rates
  its parent), `other`.
- A caller that is the parent of the subsession rates with `role=caller,
  perspective=parent`. A sub-session rating its own routed outcome is `role=callee,
  perspective=self`. Both are stored, so the supplement's "from the sub-session's own
  perspective" and "from the parent's perspective" are two group-bys over one table.

**Gauges.** `points_correct` and `points_wrong` are two gauges. They are never netted.
A row may carry both, one, or the other; it must carry at least one.

**Current view.** `v_decision_feedback_current` returns the latest row per
`(decision_id, role, rater_ref)`, using `rated_at` with `id` as tie-break. Earlier rows
stay. Reports show current ratings and can show the history.

**Rules.** A rater may not rate its own decision as `callee` unless `perspective =
self`. A rating with no `reason` is allowed but counted separately in D3 exports.

### 4.4 Outcomes — `outcome_observation` and `outcome_link` (fleet-platform Turso, V2)

```sql
CREATE TABLE outcome_observation (
  id                    TEXT PRIMARY KEY CHECK (id GLOB 'out_*'),
  subject_ref           TEXT NOT NULL,            -- '<owner>/<repo>#<n>' | 'queue:…#<id>' | 'dec_…' | 'ses_…' | 'att:…'
  subject_kind          TEXT NOT NULL CHECK (subject_kind IN ('item','task','decision','session','attempt')),
  decision_id           TEXT,                     -- set when the outcome is about a decision (direct or via a link)
  label                 TEXT NOT NULL,            -- must exist in pm-stack/outcome-labels.json (validated by the writer)
  label_registry_version TEXT NOT NULL,
  polarity              TEXT NOT NULL CHECK (polarity IN ('negative','positive','neutral')),
  severity              INTEGER CHECK (severity IS NULL OR severity BETWEEN 0 AND 5),
  detection             TEXT NOT NULL CHECK (detection IN ('auto-git','auto-gh','auto-queue','reported','reviewed')),
  observed_by           TEXT NOT NULL,            -- 'ses_…' | 'human:<login>' | 'system:sweep' | 'system:<job>'
  observer_role         TEXT NOT NULL CHECK (observer_role IN ('self','parent','subsession','reviewer','system','other')),
  observed_at           TEXT NOT NULL,            -- ISO-8601 UTC
  window                TEXT,                     -- e.g. '14d-after-merge'
  evidence_url          TEXT,
  note                  TEXT CHECK (length(note) <= 500),
  confidence            TEXT CHECK (confidence IN ('high','medium','low')),
  chain_id              TEXT,                     -- groups a causal chain (§4.5)
  provenance            TEXT NOT NULL             -- JSON, as in decision_feedback
);

CREATE TABLE outcome_link (
  id           TEXT PRIMARY KEY CHECK (id GLOB 'olk_*'),
  from_id      TEXT NOT NULL,                     -- out_…
  to_id        TEXT NOT NULL,                     -- out_…
  kind         TEXT NOT NULL CHECK (kind IN ('caused','amplified','preceded')),
  observed_by  TEXT NOT NULL,
  observed_at  TEXT NOT NULL,
  note         TEXT CHECK (length(note) <= 500),
  CHECK (from_id <> to_id)
);
```

**Current view.** `v_outcome_current` keeps the latest observation per
`(subject_ref, label, observed_by)`. Automatic and reported observations of the same
label stay as separate rows, and the view shows both with `detection`. A reviewed label
overrides nothing automatically; a reviewer's `reviewed` row sits beside the sweep's
`auto-*` row.

**Negative rule.** A `reported` or `reviewed` negative label with `severity ≥ 4` shows
as "reported" in the review surface until a second observer, who is not the first, adds
a `reviewed` row. This is a display rule, not a block on writing.

### 4.5 Outcome vocabulary and severity

The registry is `pm-stack/outcome-labels.json` (D6-S7). It is the source; this table
is its V2 content. Every label in supplement §5 is present. Detection kinds:
**auto-gh** and **auto-git** come from mechanical sweeps (D6-S8); **auto-queue** from
queue state; **reported** from `fleet outcome report` by any session or human;
**reviewed** from the PM review surface or a reviewer subsession.

| Label | Polarity | Group | Definition | Detection | Automatic rule (when auto) |
| --- | --- | --- | --- | --- | --- |
| `pr-merged` | positive | lifecycle | PR merged into the default branch | auto-gh | `gh pr view --json state,mergedAt` → MERGED |
| `issue-resolved` | positive | lifecycle | Issue closed as completed | auto-gh | `stateReason = COMPLETED` |
| `issue-made` | neutral | lifecycle | Issue created as a result of this decision or task | auto-gh | new issue whose body or trailer carries `Decision-Id` / `Task-Id` |
| `canceled` | neutral | lifecycle | Task or issue canceled before its work landed | auto-queue / auto-gh | queue `status = canceled`; issue `stateReason = NOT_PLANNED` without a duplicate marker |
| `pr-closed-incomplete` | negative | lifecycle | PR closed unmerged with no replacement PR | auto-gh | `state = CLOSED`, `mergedAt` null, no later PR closing the same issue |
| `closed-incomplete` | negative | lifecycle | Issue closed without a landed change and not as duplicate | auto-gh | `stateReason = NOT_PLANNED`, no duplicate marker |
| `duplicate-work` | negative | lifecycle | Work done twice; one copy closed as duplicate | auto-gh (confirm: reviewed) | `stateReason = DUPLICATE`, or two PRs closing one issue where one is closed with a duplicate comment |
| `work-not-pushed` | negative | lifecycle | Task claimed or completed, but no commit carries its `Task-Id` on origin | auto-git | claim older than 24 h, `completed_at` set, zero commits on origin with the trailer |
| `pr-reverted` | negative | lifecycle | A merged PR later reverted | auto-gh | a revert PR whose body names the PR number, or `Revert "<title>"` |
| `commit-reverted` | negative | lifecycle | A commit reverted directly | auto-git | `git log --grep 'This reverts commit <sha>'` |
| `rework` | negative | lifecycle | A follow-up change rewrites the same files within the window for the same item | auto-git | file-overlap churn: ≥ 50 % of the files changed by the item are changed again by a commit ≤ 14 days later, linked to the same issue (provisional, Q5) |
| `work-undone` | negative | lifecycle | Landed work removed or overridden later, not by a revert | reviewed (auto candidate) | candidate when `rework` fires and deleted lines exceed 50 % of the item's added lines |
| `caused-incident` | negative | impact | The change caused an incident or outage | reported (auto hint) | hint: incident-labelled issue created ≤ 72 h after merge, linked |
| `data-loss` | negative | impact | The change caused loss of data | reported | — |
| `significant-expense` | negative | impact | Cost or quota burn well above expectation | reviewed (auto hint) | hint: quota reading delta above the item's expectation band (threshold Q4) |
| `slow` | negative | impact | Made the system measurably slower | reviewed | — (measured form via TTFT change marker in V3) |
| `heavy` | negative | impact | Far more tokens or steps per point than peers | reviewed (auto hint) | hint: session tokens above the p90 of its kind (D3 quantile) |
| `network` | negative | impact | Caused network or egress cost or traffic problems | reported | — |
| `not-scalable` | negative | impact | Works at small scale only | reviewed | — |
| `invalid` | negative | judgement | Premise wrong or out of scope | reported / reviewed | — |
| `unclear` | negative | judgement | Request or result unclear | reported / reviewed | — |
| `misunderstood` | negative | judgement | Solved a different problem from the one asked | reported / reviewed | — |
| `too-big` | negative | judgement | Scope larger than one PR or slice should be | reviewed (auto hint) | hint: PR additions + deletions above the slice norm (threshold Q4) |
| `overcomplicated` | negative | judgement | More complex than needed | reviewed | — |
| `oversimplified` | negative | judgement | Missing needed parts | reviewed | — |
| `simple` | positive | judgement | Appropriately simple | reviewed | — |
| `perfect` | positive | judgement | Matched the expectation with no rework | reviewed | — (an absence of `rework` is not evidence; never automatic) |
| `safe` | positive | judgement | No safety or security regression found | reviewed | — |
| `fixed` | positive | judgement | The problem is verified fixed | reviewed | — (`issue-resolved` is the lifecycle fact; `fixed` is the judgement) |
| `partial` | neutral | judgement | Part done, remainder open | reviewed (auto hint) | hint: `issue-resolved` with an open linked follow-up |
| `cheaper-than-expectation` | positive | judgement | Cost or quota below the estimate | reviewed | — (measured form in V3) |
| `unneeded-worry` | negative | judgement | Extra checks or alarm that were not needed | reviewed | — |
| `overtested` | negative | judgement | Test effort exceeded the risk | reviewed | — |
| `undertested` | negative | judgement | Test effort below the risk; a bug escaped | reviewed | — |

Thirty-three labels in four lifecycle rows plus impact and judgement groups. Labels
that are not in the registry cannot be written. New labels arrive by PR that changes
the registry file, and the registry carries a `version` that every observation records
(`label_registry_version`).

**Severity (0–5, negative polarity only; positive labels carry NULL).** Provisional
defaults, to be calibrated in V3:

| Severity | Meaning | Examples |
| --- | --- | --- |
| 0 | Recorded, no measurable cost | a label added for a clarification |
| 1 | Cosmetic, or under 15 minutes lost | a wrong title |
| 2 | Rework under one hour, or an unclear request | `misunderstood` caught before merge |
| 3 | Rework of an hour or more, a wasted PR, or quota burn above the Q4 band | `pr-reverted` after two days |
| 4 | User-visible degradation or a wasted pool window | `slow` for a workhorse window |
| 5 | Incident, data loss, or outage | `caused-incident`, `data-loss` |

**Causal chains.** A chain is a set of outcomes joined by `outcome_link` with
`chain_id` shared. The worked example in §6.2 is the supplement's sentence: an issue
misunderstood, which caused a PR, which blew a quota, which slowed performance, which
caused an outage. Each step is a separate outcome row with its own observer; the link
records who asserted the causation and with what confidence. Causation is asserted, not
inferred: the system does not guess causal links.

### 4.6 Linkage model

```
item (issue/PR/epic) ──task-item──▶ queue task ──task-session──▶ session (root)
                                                             │ parent_id
                                                             ▼
                                                    sub-session ──▶ attempts (route_attempt)
                                                       │             └─ tool calls (session_message)
                                                       └─ decision (caller/subject) ──▶ legs ──▶ route_attempt
item ◀──decision-item── decision ──▶ feedback (fbk) and outcomes (out) ──▶ item / task / session
commit ──trailers──▶ task, session, decision ;  PR body Trace block mirrors the same ids
```

**`fact_link` (fleet-platform Turso).** An append-only table of joins that are not
already a column:

```sql
CREATE TABLE fact_link (
  id          TEXT PRIMARY KEY CHECK (id GLOB 'fl_*'),
  from_ref    TEXT NOT NULL,
  to_ref      TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('task-item','task-session','session-parent','session-attempt',
                                            'decision-item','decision-session','commit-task','commit-session',
                                            'commit-decision','item-pr','item-epic')),
  source      TEXT NOT NULL CHECK (source IN ('trailer','claim','queue-payload','pr-body','session-metadata',
                                              'route-ledger','issue_pr_links','sweep','manual')),
  observed_at TEXT NOT NULL,
  observed_by TEXT NOT NULL
);
```

**Who writes each link, and how it survives.**

| Link | Written by | Where it lives | Survives |
| --- | --- | --- | --- |
| task → session | queue claim (`claim_session_id`) | `tasks` row | rows persist (#3424 notes the row is the record) |
| task → item | queue enqueue (`payload` carries the item ref) and the PR body `Task-Id` | `tasks.payload`, `fact_link` | payload immutable; link appended |
| session → parent session | OpenCode core | `session_v2.parent_id` | always |
| session → attempt | route proxy | `route_attempt.session_id` | always |
| session → task (reverse) | session metadata written at claim by the launcher (D6-S10) | `session_v2.metadata.task` | with the session |
| decision → session | executor | `decision_record.caller_session_id`, `subject_session_id` | always |
| decision → legs → attempt | executor | `decision_leg.route_attempt_id` | always |
| commit → task / session / decision | git trailers (§4.8) | git history | merge commits keep them; rebase is forbidden by policy |
| PR → issue | `Closes #n` and `issue_pr_links.json` (projection) | GitHub, pm-aggregator | GitHub |
| decision → item | `Decision-Id` trailer on the PR's commits; `fact_link` `decision-item` written by the sweep | git + Turso | yes |

**Per-session metrics (supplement §5).** Derived from the raw rows, never stored
mutable. D3 exports the raw rows; `session-metrics.ts` (D6-S11) computes these
definitions, pure over row fixtures.

| Supplement §5 metric | Definition | V1 source | Status |
| --- | --- | --- | --- |
| Tokens in / out / reasoning / cache read / cache write | Sum over the session's `route_attempt` rows (or `session_v2` totals) | `route_attempt.tokens_*` | exists; cache and reasoning are included in totals per `gen_ai` convention (R3 §4.1) |
| Tokens context | Input tokens of the session's last successful step (context size at the step) | `route_attempt.tokens_input`, ordered by `time_started` | derived |
| User input | Count and characters of `user` messages (characters only, no content) | `session_message` type `user` | derived |
| Thinking | Count and characters of `reasoning` parts | `session_message` content `reasoning` | derived |
| Tool calls and reads | Count per tool name; `read` counted separately | `session_message` content `tool` | exists; `mcpx`/`fleet` subcommands inferred (G1, G3) |
| TTFT (per step) | `first_token_ms` of each attempt | `route_attempt.first_token_ms` | exists |
| First TTFT (per session) | `first_token_ms` of the session's earliest attempt | `route_attempt` ordered by `time_started` | derived |
| Total step time | Sum of `response_ms` across attempts | `route_attempt.response_ms` | derived |
| Total step count | Count of distinct `assistant_message_id` (steps) and count of attempts (calls, incl. failover) | both | derived; both reported |
| Time for a session and for an input | Session: `time_updated − time_created − time_idle` (estimate). Input: from a `user` message to the next completed `assistant` message | `session_v2`, `session_message` | estimate, labelled |
| Number of question prompts | Count of `question` tool invocations | `session_message` tool `question` | derived (9 in the last week; rare) |
| Number of questions | Sum of questions in each `question` call's `state.input` | `session_message` | derived; input shape to confirm (Q12) |
| Time until submitted | Completion time minus creation time of each `question` tool part | `session_message` | derived |
| Errors auto-rerouted | Attempts with `failed_over_to` set, and continuations (`continuation_of`) | `route_attempt` | exists |
| Decision model and decision | `decision_record` joined by caller or subject session | D6 §4.2 | new (V2) |
| Feedback from PRs, issues, parents, subsessions | `decision_feedback` and `outcome_observation` joined through `fact_link` | D6 §4.3–4.4 | new (V2) |
| Sub-session count and depth | Count of `session_v2` rows with `parent_id`, and depth by recursion | `session_v2` | derived (not measured in V1) |
| Diff +/- | `summary_additions/deletions` (468 rows, sparse) and PR `additions/deletions` from GitHub at merge | `session_v2`; `gh pr view` | sparse → PR-side source (G7) |
| Days to complete, actual work time | `created_at` → `completed_at` on the task; actual work = sum of attached sessions' active time | queue `tasks`, `session_v2` | derived; the start event needs queue `events` (G14) |

Statistics (min, max, mean, median, p90, p99, p99.9, quartiles, IQR, stddev, count,
sum) are computed by D3 from these per-sample rows. D6 does not compute them, and every
figure it exports carries `n`.

### 4.7 Privacy and size bounds

| Item | Rule | Enforced by |
| --- | --- | --- |
| Prompt | Full text stored once per `(prompt_id, version)`, ≤ 16 KB. Row stores version, date, sha256 | DDL CHECK; insert-or-verify |
| Request | Only `sha256` and byte count. Body is never stored by default | Writer API has no body parameter |
| Invalid output | Only `invalid_sample_sha256`. Optional redacted sample ≤ 4 KB in a 14-day blob store, **off by default**, enabled per decision type | Writer; blob TTL job (D6-S24) |
| Answer, probabilities | Stored in full; the schema bounds them to ≤ 8 KB | DDL CHECK |
| Quota snapshot | ≤ 2 KB | DDL CHECK |
| Error bodies | Never copied into decision rows. `route_attempt.error_body` remains where it is, truncated by D3 | Writer |
| Secrets | Never stored. Credentials by `credential_id` only | Writer and guard test |
| Notes, reasons | ≤ 500 characters | DDL CHECK |
| Evidence | URL only | Writer validates scheme and host allow-list |
| Rater and observer identity | Session ids or `human:<login>`, from the environment, never user-typed | `fleet outcome` writer (§4.9) |
| Size estimate | Decision + legs ≈ 1.5 KB; feedback ≈ 0.3 KB; outcome ≈ 0.5 KB. At an assumed 2,000 decisions/day with 2 legs, ≈ 4 MB/day including outcomes. Measured for 30 days in V2 before retention is set (R3 §7.1 item 7) | D6-S24 measures |

The owner's standing position that secrets in chat logs do not matter (AGENTS
owner-preferences) does not relax the storage rule: secrets are still stored by name
only.

### 4.8 Git trailers and the PR Trace block

Commit trailers are the join key of record. Format (git trailer syntax, read by
`git interpret-trailers --parse`):

```
Task-Id: queue:developing-today-code#15864
Session-Id: ses_e5e2b2a8d4fffeqBP4FUy4CDVy
Decision-Id: dec_01JDXG7Q3Z8K5W2N6R9T4V0B1C
Assisted-by: opencode (opencode-route/workhorse)
```

- `Task-Id` and `Session-Id` are optional; `Decision-Id` repeats for each decision that
  shaped the commit. `Assisted-by` names the route head, not the serving leaf
  (route refs are identity; leaves vary per step, `subagents.md`).
- The fleet worktree hook (D6-S9) stamps them from the environment (`FLEET_TASK_ID`,
  the session id, decision ids recorded in the session's metadata). Other checkouts
  degrade gracefully: the PR-body block and the claim stamp still join.
- **PR Trace block** (added to the PR body by the `pr` skill and the PR template,
  D6-S13). A generated mirror, never truth:

```
## Trace
- Task: queue:developing-today-code#15864
- Session: ses_e5e2b2a8d4fffeqBP4FUy4CDVy
- Decisions: dec_01JDXG7Q3Z8K5W2N6R9T4V0B1C
- Closes: developing-today/code#9399
```

Invariant (checked by the sweep): the PR body block and the commit trailers agree
for every id in the block; disagreement is a finding, and the commit wins.

### 4.9 The report-anything mechanism (`fleet outcome`)

One writer, three front ends: the CLI, a native tool named `outcome_report` with the
`trail_add` shape, and a code-mode wrapper once mcpx has its call ledger (code#9374).

```
fleet outcome report --item developing-today/code#3424 --label misunderstood \
    --severity 2 --note "Asked for blocker links; the PR shipped only the notes table." \
    [--decision dec_…] [--caused-by out_…] [--chain chn_…] [--evidence <url>]
fleet outcome rate --decision dec_… --role caller --perspective parent \
    --correct 4 --wrong 0 --reason "routed to the right pool; the leg order was fine"
fleet outcome list --item developing-today/code#3424 [--current] [--json]
fleet outcome sweep [--since 14d] [--dry-run]          # D6-S8; registered in fleet-schedule-registry
fleet outcome link --from out_… --to out_… --kind caused --note "…"
```

Rules:

- Any session may report on any item or PR. Observer identity comes from the
  environment (session id, host, lane), not an argument.
- `report` needs `--evidence` or a `--note` of at least 20 characters. Without either
  it is refused with a usage error (operator script contract, `doc/agents/operator-script-argument-contract.md`).
- `report` never edits an automatic row. It adds a row.
- Writes are idempotent by generated id; retries are safe.
- Writes go to the local outbox first, then sync to Turso (`INSERT OR IGNORE` by id),
  the pattern of `decision-events-sync.ts`. A failed sync leaves the outbox entry
  pending and reports it. Outbox path defaults to
  `~/.local/state/fleet/outcome-outbox.jsonl`.
- Rate limit: 50 `report` calls per session per hour, a guard against floods; the
  limit is a config value, not a constant.
- Labels not in the registry are refused, and the error lists the registry version.
- **Projection to GitHub labels.** Only labels with `project: true` in the registry
  are mirrored to the item as `outcome:<label>` (default: `caused-incident`,
  `data-loss`, `pr-reverted`, `misunderstood`, `duplicate-work`). Mirror is add-only
  and rebuildable from the table; the table is truth.

### 4.10 Review surface (PM stack, V2)

Each supplement §8 case maps to a component. Scores attach to items and decisions,
never to sets.

| Supplement §8 case | Implementation | Component touched | Data |
| --- | --- | --- | --- |
| Feedback, labels, scores on **one item** | A "Review" panel on the item detail page: label picker from the registry, severity, the two gauges for any decision linked to the item, evidence, note | `/item/<subject>` (`renderItem`, `render.ts`), dashboard comments store pattern (`comments.ts`) | writes through the same `fleet outcome` writer; reads `v_outcome_current` and `v_decision_feedback_current` |
| A **model-selected set** of items | A set produced by a stored predicate (e.g. terminal items in an epic with a severity ≥ 3 outcome, or decisions whose caller and callee gauges disagree by ≥ 3), recomputed on open, predicate stored on the set | new `/review` page; ranking approach from `needs-drew.ts` | `review_set` (kind `model-selected`, `predicate_json`) |
| A **marked set** (set as a set) | User-marked membership, add and remove both append rows | `/review` | `review_set` (kind `marked`), `review_set_member` (append) |
| **Custom selection within a set** | Checkbox subset of a set; the review action applies to the checked members and records the subset in the action's note | `/review` | action rows reference the set id and the member refs |
| **Multiple sets** | Any number open at once; each is a view with its own predicate or membership; no set depends on another | `/review` tabs | no cross-set state |
| Each item **standalone** | Opening one item outside any set works without a set; scores are per item | `/item/<subject>` | same tables |
| Search terminal items **by epic, roadmap, timeline** | Terminal means canonical `Done` (status-taxonomy) or PR merged/closed, or queue `completed/canceled/failed`. Filters: `epic:<ref>` (epic-graph membership), `roadmap:<ref>` (roadmap annotation), `closed:<from>..<to>` (weekly cutoffs from item-history) | `/search` (`searchWork`, `search-view.ts`), `/program` (`epic-graph.ts`) | filters on existing pool and history; reviews from the tables |
| Review **queued work** and **reprioritize** | `/queue` lists queued tasks with a priority control; the change calls `repo-queue reprioritize <task> --priority N --reason …`, a new verb on the `client.rs` dispatch, which appends a `priority` row to the existing `events` table | `/queue` (`queue-view.ts`, `server.ts:2507`); `turso_queue.rs` events table; `client.rs:5314–5340` | `events` (kind `priority`, detail before/after, reason) |
| **Add labels** to queued or terminal work | Labels land on the linked GitHub item (queryable form). A review label is a registry label with `project: true`; queue tasks have no labels of their own | item label write via `gh` | `outcome_observation` plus the GitHub label |

**Reprioritize rule.** Changing a queued task's priority is recorded with a reason. It
does not change the claim order for tasks already claimed. It is an operator action, not
a model's: a session may suggest a priority (`outcome report --label …` is not a
priority change) and the human or the orchestrator applies it.

### 4.11 Learning loop, V2 readout

V2 is capture only. A reader (D3, V3) computes, per decision type and per answering
leg: the distribution of gauges, the rate of negative labels, the median and p90 of
time-to-decision, and the second-try success rate of invalid JSON (R2 §3.4). Those
numbers are not used to change routing in V2. The first use is V3, after 14 days of
records per decision type (`../main.md` §0 gate).

---

## 5. V3 and V4 — what each adds and what it learns from

### 5.1 V3 — measured and automatic

Entry gate: V2 records exist for ≥ 14 days per decision type being tuned, and the
30-day storage measurement is in.

- **Nightly export** of `decision_record`, `decision_leg`, `decision_feedback`,
  `outcome_observation`, `outcome_link`, `fact_link` to Parquet partitioned by day
  (D3's export manifest; D6 supplies the table definitions and the `schema_version`
  of each file).
- **Current-view exports** and the Langfuse projection: gauges as two numeric scores,
  each label as a boolean score (projection only, R2 §5.3).
- **Calibration of severity and of labels** against the first 14 days of `reviewed`
  rows: base rates per label and per severity, with `n` and an interval. Base rates
  feed D4 trials as priors, never as proof.
- **Suggest-only labels.** An LLM reviewer subsession proposes labels for an item with
  evidence. Suggestions are stored as `reported` with `observer_role = other` and a
  `provenance.source = "suggestion"`. They never write `reviewed`.
- **Snapshot at session close** (option 3.9 C): an append-only `session_fact` row for
  point-in-time facts that D3 needs.
- **Churn and undone detection** upgraded from the file-overlap test to hunk survival
  (`git blame` on the item's added lines) for `work-undone`.
- **Measured `slow` and `cheaper-than-expectation`** from TTFT and quota change markers
  around a change (D3 change markers).
- **Causal-chain report**: chains longer than two steps, with the share of chains
  asserted by a reviewer versus reported.
- **Registry drift guard** (D6-S23): the registry file, the DDL and the writer's label
  list must agree; CI on main reports drift.

**Learns from:** the labelled rows, the gauges, and invalid-JSON second-try rates. Output:
per-decision-type priors for D4 trials and a dashboard for the review surface.

### 5.2 V4 — learned and adaptive

Entry gate: V3 metrics show the hand-tuned decision baseline is the limiting factor.

- **Counterfactual evaluation** of other decision policies using stored probability
  vectors and the logged `request_sha256` (only if D6 retains request bodies for the
  replayed decision types; this needs an explicit privacy decision by the owner, §7.2).
  Inverse-propensity scoring is useless while the pool choice is deterministic (R2
  §5.1); deliberate exploration within a trial's safe set (AD9) is the way to get
  propensities.
- **Outcome-weighted decision priors** feeding D5's default answers, inside the safe
  set only.
- **Labelling assistant** that proposes the review set order (the model-selected set
  ranked by expected information), human-confirmed.
- **Hosted editions** of the review surface, with the same schema.

---

## 6. Worked examples (concrete pools from V1)

### 6.1 A category decision where the first leg is walled and the second answers

Setting: an OpenChamber category `research` on Auto asks the decision pool `decision`
(D5 classifier) which route to use. The decision pool's legs, in the free → plan → PAYG
order of supplement §2, are:

1. `decision/cf-clef-flash` (native, Cloudflare Workers AI). Its included allowance is
   used up for the UTC day, so the budget guard skips it before any call.
2. `decision/opencode-zen-jev-free` (native, Jev 1.13 free door on OpenCode Zen; the
   promo was live-probed 200 on 2026-11-13, per `../questions.md` Q5).
3. `decision/luna-100k-last-resort` (LLM, `gpt-6-luna` #medium with the 100k cap,
   pinned directly, per `../questions.md` Q4).

Record written at the end of the decision (illustrative values):

```
decision_record
  id                    dec_01JDXG7Q3Z8K5W2N6R9T4V0B1C
  decision_pool         decision             decision_type  category
  caller_session_id     ses_<auto session>   subject_session_id ses_<research sub-session>
  prompt_id             NULL                 -- no LLM leg answered
  schema_id             system-one           schema_version 1
  request_sha256        9f2c…(64)            request_bytes  1834
  time_to_decision_ms   1640                 status         answered
  answered_leg_seq      2                    answered_by    opencode-zen/jev-1.13-free
  answered_via_pool     decision             default_answer 0
  probabilities_json    {"route":{"workhorse":0.61,"scout":0.27,"summit":0.12}}
  effort_lean_requested medium               effort_lean_applied 1   effort_lean_delta 0.02
  fallback_pool_used    0                    json_attempts_total 0
  neurons               NULL                 cost_usd NULL   (subscription/free door)
  quota_snapshot_json   {"decision/cf-clef-flash":{"unified-7d":"…","pct":100}, "read_at":"…"}

decision_leg
  (dec, 1, 1)  decision/cf-clef-flash  native  budget-skipped        route_attempt_id NULL
  (dec, 2, 1)  decision/opencode-zen-jev native  answered            route_attempt_id 1234567 ttft 210 ms
```

Notes. Leg 1 is not an attempt, so it has no `route_attempt_id` and no tokens; it is
still recorded, which is why the budget guard's skips show up in the analysis. The
`effort_lean_*` fields say the tie-break was applied and by how much; `fallback_pool_used
= 0` says the fallback pool did not answer. Later, the research sub-session's parent
rates the outcome: `fbk`, `role=caller`, `perspective=parent`, `points_correct=4`,
`points_wrong=0`. The item's PR merges (auto `pr-merged`, sweep). Nothing else is
written.

### 6.2 The causal chain from supplement §5

Setting: `developing-today/code#3424` (queue data model) is claimed by a workhorse
session. The issue body was misread, and the PR built a notes table the design did not
ask for, which then ran a Luna trial across all queue tasks: quota burn on the `limited`
pool rose sharply, the queue slowed for other lanes, and a subsequent outage followed.

The chain (chain id `chn_01JDXK…`):

| Step | Outcome row | Observer, detection | Severity | Link |
| --- | --- | --- | --- | --- |
| 1 | `misunderstood` on `…/code#3424` | Drew via review panel, `reviewed` | 2 | — |
| 2 | `pr-merged` on the PR that followed | sweep, `auto-gh` | null | `caused` from step 1 (asserted by Drew) |
| 3 | `significant-expense` on the PR's session tree (pool `limited`, quota delta) | reviewer subsession, `reviewed`, hint from the quota band | 3 | `amplified` from step 2 |
| 4 | `slow` on the queue's lane health | Drew, `reviewed` | 4 | `caused` from step 3 |
| 5 | `caused-incident` on the outage issue | reported by the on-call session via `fleet outcome report`, `reported` | 5 | `caused` from step 4 |

Every step is one append. The chain is a read over `outcome_link`; the causal words are
the observer's assertions, and the report shows who asserted each one. A reader sees
that step 1 is a human judgement and step 3's cost figure is inferred from the quota
band, with the band's threshold recorded in the registry.

### 6.3 Report-anything from a sub-session

A sub-session launched by a `workhorse` head to fix a flaky test notices that a merged
PR on the same area was the cause. It runs:

```
fleet outcome report --item developing-today/code#9399 --label undertested --severity 1 \
  --evidence https://github.com/developing-today/code/pull/9385 \
  --note "The route-guard test added in #9385 did not cover the cycle-skipped leg; found while fixing the flake."
```

The row lands in the outbox, syncs to Turso, and appears on the item's review panel as
`reported`. No issue is filed (3.6 E). The sub-session's `Session-Id` is in its own
commits, so the link is already there. Its parent can add a `fact_link` if needed.

---

## 7. Risks, open questions, challenges

### 7.1 Risks

| Risk | Mitigation | Default chosen |
| --- | --- | --- |
| Migration on the live `opencode.db` (21.5 GB, WAL) | Additive tables; test the migration on a copy of the DB (`.bak` snapshot pattern), never on the live file in the design phase | Run on a copy; ship behind the executor's flag |
| Trailers missing on commits from non-fleet checkouts | PR Trace block and claim stamp still join; sweep reports orphan commits | Degrade, do not block |
| Label floods from reporters | Rate limit, evidence-or-note rule, registry-only labels | 50 reports per session per hour |
| Reported labels are wrong | Display rule (§4.4), second-observer rule for severity ≥ 4 | Display rule only; no write block |
| Outcome lag (merge, revert and incident arrive weeks later) | Stable join keys (not timestamps); windows recorded on each row | 14-day window for `rework` |
| Gauges used as ground truth before they are calibrated | Gauges are per rater and per role; D3 reports them with `n`; no routing change in V2 | No routing use before V3 |
| Turso outage | Outbox; sync leg retries; `INSERT OR IGNORE` | Pending entries reported |
| Decision volume far above assumption | Measured for 30 days before retention; D3 partitioning | 2,000/day assumed |

### 7.2 Open questions (recorded with defaults; detached run, no question tool)

| # | Question | Default chosen | Why | Where applied |
| --- | --- | --- | --- | --- |
| Q1 | Put feedback, outcomes and links in fleet-platform Turso, or in the decision-events-worker D1 instead? | fleet-platform Turso | Same token-by-name pattern as `decision_set_events`; the worker is a fan-out for the decision sets | §4.4, §4.6 |
| Q2 | Who owns the opencode.db migration? | the fork (`developing-today-forks/opencode`), core package | The `route_attempt` migration lives there | D6-S1 |
| Q3 | Enforce commit trailers? | fleet worktree hook stamps them; no enforcement on other checkouts | Standing rules: CI on main only, so enforcement could only report | D6-S9 |
| Q4 | Severity and size thresholds (quota band, PR size norm) | provisional; set from first 14 days (V3) | No labelled data yet | §4.5 |
| Q5 | Rework window and overlap | 14 days, 50 % file overlap, same linked issue | GitClear-style prior, not a standard (R3 §5.1) | §4.5 `rework` |
| Q6 | Which repo ships the opencode-cockpit `trail_add` tool, where `outcome_report` belongs | the repo that ships `trail_add`; confirm before D6-S12 | Location not verified in this run | D6-S12 |
| Q7 | Does a second observer need to be a different session, or may a human be the second? | any different observer identity | Keeps the rule simple | §4.4 |
| Q8 | Retention of blob samples | 14 days, off by default | Privacy bound §4.7 | D6-S24 |
| Q9 | Gauge per decision or per leg? | per decision; leg-level correctness derived in D3 | Ratings are about the routed outcome | §4.3 |
| Q10 | Record requested and applied effort separately? | yes, both, plus delta | Supplement §6 asks for the distinction | §4.2 |
| Q11 | Expected decision volume | 2,000/day for sizing, measured after 30 days | No measured decision volume exists | §4.7 |
| Q12 | Shape of `question` tool input (count of questions) | assume `questions` array length; confirm on live rows | Only 9 calls in the last week | D6-S11 |
| Q13 | Should review labels also project to queued items (not just linked items)? | no; labels on the linked item only | Queue tasks have no label surface; #3424 may add one | §4.10 |
| Q14 | Does the reprioritize verb need the orchestrator's approval for lanes other than the caller's? | yes, as an operator action; the verb records the actor | Supplement §8 says "reprioritize", not "reprioritize anything" | §4.10 |

### 7.3 Dependencies outside D6 (edges to resolve when merging the program)

- **D5** (decision executor): D6-S3 exposes the writer the executor calls. The executor's
  slices that produce legs and records must be blocked by D6-S1…S4. The D5 slice ids
  are not known from this run; the edge is named in the slice bodies, not in
  `blocked_by`.
- **D3** (stats store): the nightly export manifest lists the D6 tables (D6-S19 is
  blocked by the D3 export slice). D3 aggregates; D6 defines the per-session facts.
- **#3424 pieces 4–6** (notes, events, provenance): D6 uses the existing `events` table
  for priority rows and the same provenance shape. It does not wait for #3424.
- **code#9373 and code#9374** (fleet and mcpx ledgers): per-subcommand usage is still
  inferred; the `fleet outcome` writer does not depend on them.

### 7.4 Challenges to main.md and to the research

1. **AD7 placement.** AD7 says the decision record sits in `opencode.db` beside the
   ledger, and that outcomes are append-only rows that reference the ID. This design
   keeps both, and adds that feedback, outcomes and links live in the cross-machine
   sink, because raters and sweeps run outside the decision's machine. This refines
   placement; it does not change the append-only rule.
2. **R3 §7.1 item 4** recommended Postgres (queue DB) for outcome and review tables. This
   design picks Turso fleet-platform for V2 (§3.2 E) because the queue schema is being
   changed by #3424, and outcome capture should not wait on queue migrations. Revisit
   in V3 with D3.
3. **Name collision.** "Decision set" in the PM stack (`decisions.ts`, #8766, #8568) is a
   **human ruling** batch. The model-side object is `decision_record`. The two must not
   share a table or an id prefix (`set`/`rs_` vs `dec_`). Reuse of the event-log pattern
   is intentional; reuse of the container is not (§3.8 D).
4. **Supplement §9 phrase "a successful session goal"** is operationalized as the
   feedback gauges plus the outcome labels, not as a single boolean. A boolean would
   collapse the separate gauges the supplement names in §3.

---

## 8. Statement coverage

| Statement (source §) | Section of this design | Slice(s) |
| --- | --- | --- |
| brief §5 — per-session and per-task data (attempts, triages, days to complete, success, time and quota per session/task/PR/attempt) | §4.6 per-session table; §4.2 quota snapshot | D6-S11, D6-S1, D6-S3 |
| brief §5 — stats list (min…IQR) | §4.6 last paragraph (D3 owns; D6 keeps `n`) | D6-S11 (n carried) |
| brief §6 — group by any tag or label; per-sample review | §4.2 keys, §4.4 current views, §4.5 registry | D6-S1, D6-S7 |
| brief §6 — mark record for time-since-inject etc. | §4.2 `prompt_version` and `record_version` | D6-S2 |
| brief §9 — extend the stats pipeline, do not replace it | §2.1, §4.6 (D6 extends the ledger; D3 exports) | D6-S19 |
| supp §2 — Cloudflare included allowance burned first, disengage at cap | §4.2 `neurons`, `budget-skipped` | D6-S1, D6-S3 |
| supp §3 — every decision gets an ID; caller and callee gauges, separate | §4.1, §4.2, §4.3 | D6-S1, D6-S5, D6-S6 |
| supp §3 — effort default has no impact and is not used | §4.2 `effort_lean_*` (recorded, default off) | D6-S1 |
| supp §4 — invalid-JSON path, two tries, then next leg, then default | §4.2 `attempt_no`, `json_status`, `invalid_sample_sha256`, invariant 4 | D6-S1, D6-S4 |
| supp §5 — outcome vocabulary (every label) | §4.5 table, `outcome-labels.json` | D6-S7, D6-S8 |
| supp §5 — every session can report, on any issue or PR | §4.9, Appendix A | D6-S12, D6-S13 |
| supp §5 — linkage: item → task → sessions/attempts/subsessions → tool calls, tokens, pools, models → decision | §4.1, §4.6 | D6-S9, D6-S10 |
| supp §5 — per-session metrics (TTFT, first TTFT, step time/count, questions, time to submit, reroutes) | §4.6 table | D6-S11 |
| supp §5 — feedback from PRs, issues, parents, subsessions | §4.3, §4.4 | D6-S6, D6-S8 |
| supp §6 — tie-break lean vs fallback pool choice, kept distinct | §4.2 `effort_lean_*`, `fallback_pool_used` | D6-S1 |
| supp §7 — clause ids applied | §4.2 `clauses_applied_json` | D6-S1 (column), D5 owns clauses |
| supp §8 — review one item, model-selected set, marked set, custom selection, multiple sets, standalone | §4.10 | D6-S14, D6-S15 |
| supp §8 — search terminal items by epic, roadmap, timeline | §4.10 | D6-S16 |
| supp §8 — review queued work, reprioritize, add labels | §4.10 | D6-S17 |
| supp §9 — prompt with version and date; which prompt for every decision | §4.2 prompt rule | D6-S2 |
| supp §9 — tokens, percentages, quota, time to decision | §4.2 | D6-S1, D6-S3 |
| supp §9 — perspective of success (sub-session, parent, anyone) | §4.3 role and perspective | D6-S6 |
| supp §9 — invalid-JSON path and the pool/model that answered | §4.2 legs, `answered_by`, `answered_via_pool` | D6-S1, D6-S3 |
| prompt-surface brief §2 — "not confident" answers, prompt-surface reference | §4.2 `not_confident_count`, `prompt_surface_ref` | D6-S1 |
| AD7 — every decision and attempt is an addressable event; append-only | §3.1, §4.2 triggers, §7.4 challenge 1 | D6-S1, D6-S4 |
| AD10 — self-switch recorded as a decision | §4.2 `decision_type = self-switch` | D6-S3 (type registry from D5) |
| Standing rule — CI on main only; merge commits only; gh only | §4.8, §4.9, §7.1 | D6-S8, D6-S9, D6-S23 |
| AGENTS.md budget (8,000 bytes) | §2.3 | D6-S13 |
| Privacy and size (inferred from supp §9 and AD7; no direct statement) | §4.7 | D6-S5, D6-S24 |

---

## Appendix A — Guidance text for agents

### A.1 `doc/agents/outcome-reporting.md` (full topic file, drafted)

```markdown
# Reporting outcomes on issues and PRs

Any session may report an outcome on any issue or PR, and should when it
notices one: work that was undone, an issue that was misunderstood and led to a
PR that cost more than it should have, a test that was missing, a duplicate.

Use the writer; do not file an issue to say it.

    fleet outcome report --item <owner>/<repo>#<n> --label <label> \
        --severity <0-5> --note "<what happened, in one or two sentences>" \
        [--evidence <url>] [--decision <dec_…>] [--caused-by <out_…>]

- Labels come from `pm-stack/outcome-labels.json`. A label that is not there is
  refused; list them with `fleet outcome labels`.
- A note of at least 20 characters, or `--evidence`, is required.
- Do not add severity above 3 unless you can name the evidence.
- To rate a decision you were part of, use `fleet outcome rate` with your role
  (caller or callee) and your perspective (self, parent, subsession, other).
  Give points for correct and for wrong separately, 0 to 5 each.
- Reports are append-only. To correct one, report again and say so in the note.
- Your session id and host are recorded from the environment. Do not pass them.

Automatic outcomes (merged, reverted, closed incomplete, duplicate, rework) are
written by `fleet outcome sweep`. Do not write them by hand.
```

### A.2 The AGENTS.md index pointer (≤ 300 bytes, to fit the budget)

```markdown
## Outcome reporting

Any session may report an outcome on any issue or PR with `fleet outcome report`
and rate a decision with `fleet outcome rate`. Full rules:
[`doc/agents/outcome-reporting.md`](doc/agents/outcome-reporting.md).
```

The current AGENTS.md is 7,493 bytes; this pointer adds about 260 bytes. D6-S13 must
re-check the budget with `scripts/research_workflow.py check` before merge.

### A.3 The `fleet outcome` CLI contract (for the writer's header comment)

Every flag is declared, unknown flags are a usage error with a non-zero exit, and
the command validates all arguments before any network, file write, or sync (the
operator-script argument contract, `doc/agents/operator-script-argument-contract.md`).

### A.4 Trailer snippet for the `pr` skill and PR template

```markdown
## Trace
- Task: <queue:…#n or none>
- Session: <ses_…>
- Decisions: <dec_… or none>
- Closes: <owner/repo#n>
```

---

## Appendix B — DDL summary (fleet-platform Turso)

`outcome_observation`, `outcome_link`, `decision_feedback`, `fact_link`, `review_set`,
`review_set_member`, plus the views `v_outcome_current` and
`v_decision_feedback_current`. The full DDL is in §4.3–4.4 and §4.6, and the review
tables are:

```sql
CREATE TABLE review_set (
  id             TEXT PRIMARY KEY CHECK (id GLOB 'rs_*'),
  kind           TEXT NOT NULL CHECK (kind IN ('query','marked','model-selected')),
  title          TEXT NOT NULL CHECK (length(title) <= 200),
  predicate_json TEXT CHECK (length(predicate_json) <= 4096),
  created_by     TEXT NOT NULL,
  created_at     TEXT NOT NULL
);
CREATE TABLE review_set_member (
  id         TEXT PRIMARY KEY CHECK (id GLOB 'rsm_*'),
  set_id     TEXT NOT NULL REFERENCES review_set(id),
  item_ref   TEXT NOT NULL,
  action     TEXT NOT NULL CHECK (action IN ('add','remove')),
  by_ref     TEXT NOT NULL,
  at         TEXT NOT NULL
);
```

`review_set` rows are immutable, so a changed predicate is a new set. Membership is
append-only; the current membership is the latest action per `item_ref`.
