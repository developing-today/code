# Queue metadata, notes, status history, resolution, and label/GitHub tracking — design

- Created: 2026-11-24 · Record: `doc/2026-11-24T07-23-14Z_design_queue-metadata-history/`
- Ordered by: Drew, 2026-11-24 01:18 CST — "consider if there's useful optional or mandatory columns or jsonb Metadata you should add to queue table and access w/fleet subcommands. maybe jsonb Metadata either way that accepts arbitrary key value with certain common items. then separately consider notes etc, add tables for automatically tracking every status change pick up drop link to sessions agents models effort etc, mark resolution status reason substatus categories, attach all issue labels to each queue item and track label add remove, possibly comments add remove, description edits, emoji, linking of issues prs marking parent changing parent etc etc"
- Tracking issue: [#14170](https://github.com/developing-today/code/issues/14170)
- Cross-links: [#14158](https://github.com/developing-today/code/issues/14158) (the reprioritization programme — its census worked around every gap named here), [#14100](https://github.com/developing-today/code/issues/14100) (the reprioritize verb — this design generalises its one-off `reprioritized` event into the full history machinery).
- Status: design of record. Slices Q1–Q8 are named in §11 and are packetised separately; this record authorises nothing by itself beyond the standing rule that a gate counts only where it names a hard blocker.

## 1. Inventory — what exists today (verified from source, 2026-11-24)

All references are to `pkgs/fleet/crates/fleet-queue` on main unless noted. The queue store is the fleet-queue Turso database; the broker (`queue_broker.rs` + `turso_queue/`) is the single write path for lifecycle changes, and the client is the `repo-queue` binary behind `fleet queue`.

### 1.1 The tasks table

`turso_queue/core.rs` (SCHEMA const) defines `tasks`:

- Identity/content: `id`, `queue`, `title`, `body`, `payload` (the packet, JSON text, 16,384-byte ceiling — `PAYLOAD_MAX_BYTES` in `validate.rs`), `dedupe_key`.
- Ordering/state: `priority INTEGER`, `status TEXT` (`queued / claimed / handed_off / parked / blocked / done / failed / cancelled`), `not_before`.
- Claim context (current claim only, overwritten each claim): `worker_id`, `claimed_at`, `lease_until`, `lane_id`, `platform`, `attempts`, `max_attempts`, `claim_provider`, `claim_model`, `claim_variant`, `claim_session_id`. A separate `leases` table mirrors the live lease; `repo_task_lane_ledger` keeps per-task lane assignment/execution provenance (incl. `session_id`).
- Working JSON: `progress`, `result` (both JSON text).
- Waiting: `park_kind`, `wait_kind`, `ready_condition`, `last_checked_at`, `check_outcome`, `required_budget_minutes`. Block reason classes exist (`escalated-to-user`, `drew_decision`, `person-answer` — `person_wait.rs`); park kinds include `question`, `person_answer`, `dependency`, `manual`.
- Dependencies: `blocked_deps` (JSON text on the row) + `dep_next_recheck_at / dep_blocked_until / dep_recheck_seconds`. Dep kinds are `blocked-by-issue / blocked-by-pr / blocked-by-task / blocked-by-ci`. There is **no deps table** — tonight's blocker graph (241 open edges) was assembled client-side by parsing row JSON, and two stored deps were malformed (`task:None`, census on #14158).
- Timestamps: `created_at`, `updated_at`, `completed_at`.

What is **not** there: no metadata column, no resolution fields, no labels, no parent/link fields, no notes.

### 1.2 Events — a history mechanism that is not yet a history

`events(id, worker_id, task_id, kind, detail JSON, created_at)` with indexes on `(task_id, id)`, `(kind, created_at)`, `(created_at)`. Lifecycle paths do write events, and claim/admin paths enrich `detail` with audit marks (`core.rs::audit_marks` / `with_marks`): `task_id, queue, pool, platform, lane_id, model_provider, model_id, model_variant, session_id` — lifted from the task row's **current** claim fields at write time. Generic verbs exist (`/events-append`, `/events-list`, `/events-count`, `/events-seen`, `/events-prune`), and `fleet queue timeline`/`history` render a **curated subset** of kinds (claimed, session_bound, lane_session_bound, model_stamp, session_unbound, session_binding_cleared, completed, lease_expired, lease_expired_at_cap, claim_refused, released, parked, failed, cancelled) — `blocked`, `unblocked`, `enqueued`, and `reprioritized` are not in the timeline's list.

The gaps that matter:

- **No structured transition fields.** No `from_status`/`to_status`, no `reason` column, no actor distinct from `worker_id` (the authenticated caller and the claiming worker are different things; #14100's spec already had to put `actor` inside the reprioritized event's detail JSON). Model/session live only inside detail JSON, copied from the row's current claim — so a release event written after a re-claim can carry the wrong session's marks.
- **Retention is a rolling window, not an archive.** `prune_events` survives from the Postgres era with a 30-day default (`migrations/0006_pruning.sql`); the Turso-era verb exists and the design intent (cutover doc `doc/fleet/agent-events-turso.md`) is that a timer calls it. Any audit older than the window — e.g. "why was this task cancelled in October?" — is unanswerable from the live store.
- **Not every writer is the broker.** Programmes have written the queue by direct SQL (the 2026-11-12 reprioritization; tonight's pass 2 used the same route until #14100's verb lands). Those writes leave whatever events the operator remembered to write — the 2026-11-12 run did write one `reprioritized` event per task, by hand, in SQL.

### 1.3 Labels — a fresh mirror, a stale copy, no history

The github-ci Worker maintains a **webhook-fed** mirror in the same Turso store (`pkgs/github-ci/worker/mirror.sql`): `github_mirror_issues` / `github_mirror_pull_requests` carry `labels` (JSON array of names), `state`, `title`, `body_hash`, `comments` (count), `comments_gh_updated_at`, `assignees`, `author`, `latest_agent_note_id/url`, `gh_created_at/gh_updated_at`, `mirrored_at`; `github_mirror_meta` tracks per-repo webhook/backfill freshness. The mirror is a read model, rebuildable from GitHub, and never gates work (#8575).

Queue-side, labels exist only as an **enqueue-time snapshot**: `label_enrich.rs` fills `payload.labels` from the mirror when a packet identifies an issue but carries no labels of its own (so the `expedite ⇒ 15` / `next-up ⇒ 25` band derivation in `validate.rs` can fire). After enqueue, nothing refreshes them — a label added to the issue on day 2 is invisible to the queue item forever, and **no label add/remove history exists anywhere**: the projector upserts over the old set.

### 1.4 Resolution — the vocabulary exists, the fields don't

There is no resolution column. The traces a cleanup has to mine today:

- `result` JSON free text and triage exits (e.g. `already_solved`);
- the verify-disposition readback (#10611, `disposition.rs`): verdicts `closed_verified / merged_verified / open_draft_with_followup / open_draft_superseded / invalid_no_work` — a *verification* vocabulary, computed at completion, stored inside the result;
- PM-side needs-user state (the block path's needs-drew payload, `needs_drew.rs`) — a question's lifecycle, not the task's resolution.

Measured consequence (census, #14158): 18,556 tasks; terminal 18,191 of which **cancelled 1,476 — with no structured reason recorded for any of them**; 165 blocked tasks with no open dep whose block reasons had to be read one by one out of prose.

### 1.5 The packet is already the de-facto metadata

Producers stuff everything else into `payload`: `kind` (small_fix / triage / feature_slice / second_pass / ci_investigation), `repo`, `issue.number`, the `slice` block (feature_slice packets, `validate.rs::slice_errors`), `labels`, producer-specific keys (e.g. `verification_class` on the darwin-nix packet from tonight's PR-diff review). Payload is the *work order* — set at enqueue, size-capped, claim-guarded. It is the wrong home for mutable, post-enqueue annotation, which is exactly the pressure Drew's ask relieves.

## 2. The split rule — columns vs metadata

One rule decides every placement in this design:

> **A field becomes a column when any consumer filters, sorts, joins, or aggregates on it, or when it carries an invariant the broker must enforce. Everything else is metadata.**

Columns are for the machine's questions; metadata is for the humans' and programmes' annotations. Two corollaries:

- **The broker's own writes graduate.** Anything the broker itself must read to do its job (claim guard, dep recheck, held-item exclusion) is a column or a table, never a metadata key — metadata is advisory and must never gate work (the mirror rule, #8575, applied inward).
- **Graduation is a migration, not a redesign.** A metadata key that proves query-hot graduates to a column by an additive ALTER + backfill from the key, and the key remains as its write-path alias for one release. §3.3 names the first graduation candidates in advance.

A note on types: Turso/SQLite has no JSONB; the codebase's established pattern is JSON in TEXT columns read through json1 (`payload`, `progress`, `result`, mirror `labels`). "JSONB metadata" in this design means exactly that pattern — a `metadata` TEXT column holding a JSON object — with the same 16 KB-class discipline as payload.

## 3. Metadata on tasks

### 3.1 Shape

```sql
ALTER TABLE tasks ADD COLUMN metadata TEXT NOT NULL DEFAULT '{}';
```

- Always a JSON **object** (never array/scalar/null at the top level). Merge semantics are key-level (JSON merge-patch): a set names the keys it changes; unmentioned keys stand; setting a key to `null` via the verb deletes it.
- **Limits** (validated broker-side, in `validate.rs` style): whole-object ≤ 8,192 bytes serialised; ≤ 64 keys; key names `[a-z][a-z0-9_.-]*`, ≤ 64 chars; values any JSON, per-value ≤ 2,048 bytes serialised. Errors name the violated limit, like the payload ceiling does today.
- **Never in metadata:** secrets (by name only, as everywhere — a metadata value is visible to every queue reader), the work order itself (payload's job), lifecycle state (columns), anything the claim path must evaluate (§2).

### 3.2 The well-known key registry

Arbitrary keys are legal; these keys are *registered* — when present, the broker type-checks them and consumers may rely on their shape. The registry lives in `validate.rs` beside `PRIORITY_SCALE` and `KINDS`, and this table is its initial content — every key below is drawn from a real write in tonight's programmes or the standing machinery:

| Key | Type | Meaning / tonight's source |
|---|---|---|
| `programme` | string | Owning programme, e.g. `queue-reprioritize-2026-11-24`, `pr-sweep-2026-11-24`, `audit-7`, `stewardship`. Programmes today identify their tasks only by dedupe-key archaeology. |
| `tracking_issue` | `owner/repo#n` string | The programme's tracking issue (#14158, #14170…). |
| `source_issue` | `owner/repo#n` string | The issue that spawned this task when it differs from the task's own work issue (design → slice spawning). |
| `held` | object `{by, reason, since}` | A human/programme hold mark with its reason — tonight's hard-skip list lives only in prose on #14158. Advisory only; the hard rails stay enforced by their existing routes (§2: a hold that must gate claiming graduates — see §3.3). |
| `verification_class` | string | e.g. `runtime-gap/high-risk` — written verbatim by tonight's pr-diff-review lane into a packet with nowhere proper to go. |
| `external_ids` | object | Foreign identifiers: `pr` (number), `pr_url`, `actions_run_id`, `workflow_id`, `session_handle` (actions-exec), `ledger_id` (PM needs-user item). |
| `coverage_exception` | string | For the estate-coverage floor (#14158): why this issue deliberately has no work task, or why this task is an issue's designated tracker. |
| `question` | object `{ledger_id, state}` | Link to the needs-user question this task waits on, when the wait is a question rather than a dep (the 165 no-dep blocked pile). |
| `lane_hint` | object `{host?, lane?, session?}` | Producer's routing hint — a hint, never a binding (bindings are the `bind` verb's job). |
| `estimate` | object `{minutes?, points?}` | Producer estimate, for programme planning readouts. |
| `x-*` | any | Experimental keys: the `x-` prefix marks unregistered experiments so the registry can adopt or retire them by census, not by guesswork. |

### 3.3 Graduation candidates (named in advance)

- `held` → if a future programme needs the claim scan to *skip* held tasks mechanically, `held` graduates to a `hold_kind` column + claim-scan predicate. Until then it is annotation; tonight's held items are protected by their recorded rails, not by this key.
- `programme` → if programme-scoped list filtering becomes a hot path (the audit skill in §9 is the likely driver), it graduates; the `--meta` filter (§3.4) is the bridge.

### 3.4 Access — `fleet queue meta`

New verb family on the client, broker routes beside `/park`'s pattern:

- `fleet queue meta get <task-id> [key]` — whole object or one key.
- `fleet queue meta set <task-id> <key> <json-value>` / `--json '{...}'` for multi-key merge-patch.
- `fleet queue meta unset <task-id> <key>`.
- `fleet queue meta keys` — the registry (names, types, one-line meanings), so agents discover the vocabulary instead of inventing keys.
- `fleet queue enqueue --meta key=json` — stamp at creation (programmes stop encoding attribution into dedupe keys).
- `fleet queue list --meta programme=pr-sweep-2026-11-24` — equality filter via `json_extract`, repeatable (AND). Deliberately equality-only in v1; range/substring filters are a graduation conversation.
- `fleet queue show` renders metadata as its own block, after payload.

**Writers:** any authenticated broker caller — the same trust boundary as every lifecycle verb today (the broker authenticates the caller identity; there is no per-field ACL anywhere in the queue and this design does not invent one). **Every meta write lands in history** (§5) as a `meta_updated` row whose detail carries the changed keys with old→new values (values size-capped in the event; the row is the audit, the column is the state). Direct-SQL writes bypass this — §10.3 names that as the standing risk this design reduces but cannot abolish.

## 4. Notes — a table, not a metadata key

**Recommendation: a separate append-only `task_notes` table.** Notes are a *stream* (many, ordered, authored, timestamped); metadata is *current state* (one value per key). Stuffing a stream into a state column guarantees lost updates under concurrent writers and destroys authorship. The estate already learned this shape once: GitHub-side agent notes are comments, and the mirror tracks their latest pointer — notes are append-only everywhere else in the system.

```sql
CREATE TABLE IF NOT EXISTS task_notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    author TEXT NOT NULL,            -- authenticated caller identity
    author_kind TEXT NOT NULL DEFAULT 'agent',  -- agent | human | system
    session_id TEXT,                 -- the author's session, when known
    model_provider TEXT, model_id TEXT, model_variant TEXT,  -- audit marks, as in events
    body TEXT NOT NULL,              -- ≤ 4096 chars, validated
    created_at TEXT NOT NULL,
    edited_at TEXT,                  -- set on edit; the original is kept in task_history detail
    deleted_at TEXT                  -- soft delete only; rows are never hard-deleted by a verb
);
CREATE INDEX IF NOT EXISTS task_notes_task ON task_notes(task_id, id);
```

- Verbs: `fleet queue note add <task-id> --text '...'` (with `--kind` for the rare non-agent author), `fleet queue note list <task-id>`, `note edit <note-id> --text '...'` (author-only; the superseded body is preserved in a history row), `note remove <note-id>` (soft; author or programme owner).
- Rendering: `show` prints the latest notes inline (bounded, with a count); `timeline` interleaves notes with history rows by timestamp — a reader of a task sees one chronological story: transitions, notes, resolutions.
- Notes are queue-local working record. Posting a note to the linked GitHub issue stays an explicit, separate act (the creation-outbox route exists for issue writes); v1 does not mirror notes to GitHub automatically — the mirror's `latest_agent_note` pointer already surfaces issue-side agent notes to queue readers, which is the direction that matters.

## 5. Status history — `task_history`

### 5.1 Shape

Extend, don't replace: `events` remains the free-form log (sweeps, prune breadcrumbs, generic `/events-append` writers, non-task rows). Every **task state transition** additionally gets one structured row:

```sql
CREATE TABLE IF NOT EXISTS task_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    queue TEXT NOT NULL,
    kind TEXT NOT NULL,              -- transition kind, §5.2
    from_status TEXT,                -- NULL for the enqueue row
    to_status TEXT NOT NULL,
    reason TEXT,                     -- the verb's reason/blocker text, first-class at last
    actor TEXT,                      -- authenticated caller identity (who invoked the verb)
    worker_id TEXT,                  -- the worker holding/affected by the claim, when any
    session_id TEXT,                 -- session bound at the time of THIS transition
    model_provider TEXT, model_id TEXT, model_variant TEXT,  -- the claim's model marks, copied at write time
    lease_until TEXT,                -- lease context for claim/release/expire rows
    priority_from INTEGER, priority_to INTEGER,  -- set on reprioritized rows (generalises #14100)
    detail TEXT NOT NULL DEFAULT '{}',           -- kind-specific remainder, JSON
    source TEXT NOT NULL DEFAULT 'broker',       -- broker | backfill | sql-programme
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS task_history_task ON task_history(task_id, id);
CREATE INDEX IF NOT EXISTS task_history_kind_ts ON task_history(kind, created_at);
CREATE INDEX IF NOT EXISTS task_history_queue_ts ON task_history(queue, created_at);
```

The marks are **columns copied at write time from the transition's own context** — not, as today's `audit_marks` does, re-read from the task row's current claim fields. That single change fixes the mis-attribution class where a late event inherits a newer claim's session/model.

### 5.2 Kinds

The lifecycle's real transition set, aligned with the verbs and with #14100's established name:

`enqueued, claimed, handed_off, released, lease_expired, parked, woken, blocked, unblocked, dep_recheck (state-neutral rows allowed: from = to), reprioritized (state-neutral), meta_updated (state-neutral), note_added (state-neutral, pointer in detail), completed, failed, cancelled, revived, resolution_cleared (rides revive), quarantined, quarantine_cleared`.

State-neutral rows (from = to) are first-class: a reprioritize or a meta write is history even though the status didn't move — tonight's programmes needed exactly those rows and had to fake them as events.

### 5.3 Capture — broker-side, same transaction

- Every lifecycle verb in `turso_queue/` writes its history row **inside the same transaction** as the task-row update — the pattern the Postgres era had (W1 in the cutover doc: events inserted inside the RPC body) and the Turso port kept for `events`. The history insert joins the existing `event()` call sites; a transition that cannot write its history row does not commit (fail-closed — history is the audit, not a side effect).
- The sweeper/staleness/repair paths (lease expiry, dep recheck) are broker paths too and write with `actor = 'broker:<path>'`, so machine transitions are distinguishable from agent-invoked ones.
- **Retention:** `task_history` is **exempt from the events prune**. Volume is bounded by transitions (~33k events/day at PG-era rates were dominated by checkpoints/heartbeats, which are *not* transitions and stay in `events`); transition volume is an order of magnitude lower. Review at 1M rows: the archive route is the existing R2 backup of the store, and a future `task_history` archive verb is a named non-goal for v1.
- `events` keeps its prune and its generic writers; during migration the broker dual-writes (event + history row) so existing readers (health's terminal-event folds, the timeline's kind list) never break mid-rollout.

### 5.4 Reading

- `fleet queue timeline <id>` reads `task_history` (all kinds, not a curated subset) interleaved with `task_notes`; `history` keeps its current events-based meaning for one release, then converges.
- `fleet queue show <id>` gains a `last_transition` summary line (kind, actor, age) — the staleness question stewardship asks hourly becomes one field.
- Cross-task reads for consumers: `GET /history-since?cursor=<id>` on the broker (id-ordered, page-bounded) — §9's feed.

## 6. Resolution — columns, a taxonomy, and terminal discipline

### 6.1 Why columns

Resolution is the single most aggregated field the programmes produce: every census, sweep report, and stewardship readout groups terminal tasks by *why they ended*. It is filtered, counted, and constrained (a terminal transition without one is tonight's 1,476-row hole) — §2's rule puts it in columns without argument. It does **not** go in metadata (advisory, unenforced) or in `result` (free text, where it hides today).

### 6.2 The fields

```sql
ALTER TABLE tasks ADD COLUMN resolution TEXT;            -- taxonomy, §6.3
ALTER TABLE tasks ADD COLUMN resolution_category TEXT;   -- success | redirection | rejection (stored, derived at write)
ALTER TABLE tasks ADD COLUMN resolution_substatus TEXT;  -- per-resolution refinement, §6.3
ALTER TABLE tasks ADD COLUMN resolution_reason TEXT;     -- free text, REQUIRED for rejection category
ALTER TABLE tasks ADD COLUMN resolution_ref TEXT;        -- the other end of a redirection: task id or owner/repo#n
ALTER TABLE tasks ADD COLUMN resolved_by TEXT;           -- actor
ALTER TABLE tasks ADD COLUMN resolved_at TEXT;
```

### 6.3 The taxonomy (grounded in tonight's cleanup)

| resolution | category | substatuses | tonight's ground |
|---|---|---|---|
| `done` | success | `verified` (disposition closed/merged-verified), `unverified`, `partial_with_followup` | The normal completion; disposition verdicts map onto the substatus instead of dying inside `result`. |
| `already_solved` | success | `solved_elsewhere`, `premise_already_met` | Exists today as a triage exit with no home; AUDIT-7's L2 census re-derived it for 8,161 closed issues by hand. |
| `answered` | success | `answered_from_record`, `answered_by_decision` | Question tasks — the 165 no-dep blocked pile was largely questions waiting for an answer that already existed somewhere. |
| `duplicate` | redirection | — (ref required) | Broker dedupe is enqueue-time only; human/programme dedupe (pass 2's set (c)) had nowhere to write the verdict. |
| `superseded` | redirection | `by_task`, `by_issue`, `by_pr` (ref required) | PR sweep: 3 duplicates marked superseded in AUDIT-7 by label only; re-sliced designs supersede their slice tasks. |
| `split_out` | redirection | — (refs = the children, in `queue_links`) | Drew's split rule from tonight's PR order: the blocked part splits out, the parent closes as split_out, children carry the work, the split-out part's question priority rises. |
| `wontfix` | rejection | `decided_by_drew`, `decided_by_agent`, `out_of_scope` | A decision, recorded as one — distinct from drift. Reason required. |
| `obsolete` | rejection | `issue_closed_elsewhere`, `premise_gone`, `expired` | Stale-state cleanup: tasks whose world ended without them (issue closed by other work, programme dissolved). Reason required. |
| `invalid` | rejection | `malformed_packet`, `wrong_queue`, `not_a_task` | The census's error class: `task:None` deps, off-scale priorities, packets that never described work. Reason required. |

### 6.4 Interaction with terminal states

- `status` stays the lifecycle state; resolution is its *verdict*. Constraints, enforced broker-side at the transition:
  - `done` ⇒ resolution ∈ {`done`, `already_solved`, `answered`} (success category).
  - `cancelled` ⇒ resolution **required**, ∈ redirection ∪ rejection. (This is the 1,476-row fix: after Q2, an unexplained cancellation is unrepresentable through the verbs.)
  - `failed` ⇒ resolution **optional**: failure is machinery state (retry, lease expiry at cap) and most failed tasks are revived; resolution is set when a failed task is *finally* abandoned, at which point the verb moves it to cancelled with its rejection resolution — failed stays resolution-free while genuinely in play.
  - `revive` clears the resolution fields and writes the clearing into history (`resolution_cleared` rides the `revived` row's detail: prior resolution, prior reason) — a revived task's earlier verdict stays auditable, never silently erased.
- Verb surface: `complete --resolution done --substatus verified`, `cancel --resolution wontfix --reason '...' [--ref task:21660]`, `fail` unchanged. Defaults: `complete` with no flag writes `done/unverified` (never blocks tonight's flows); disposition-verified completions auto-stamp `verified` from the #10611 readback the gate already computes.
- The needs-user flow composes: a task blocked on a question that Drew answers resolves its *children* normally; a pure question task completes as `answered`.

### 6.5 Backfill honesty

Resolutions are backfilled **only where the record already proves them**: triage exits in `result` (`already_solved`), disposition verdicts (`done/verified`), dedupe-cancelled pairs whose surviving twin is identifiable. Everything else terminal stays `resolution = NULL`, which after Q2 reads precisely: *ended before verdicts were recorded*. The census number (how many of 18,191) is reported once, on #14170, as the baseline — not laundered into guesses.

## 7. Labels — current set on the item, history at the source

### 7.1 Current labels on the queue item

Two homes, two jobs:

- **The mirror stays the source of truth** for an issue's current GitHub labels (webhook-fed, freshness visible in `github_mirror_meta`). Queue readers that can afford a join read it directly by the task's issue ref.
- **A cache column for the hot paths:** `tasks.labels TEXT NOT NULL DEFAULT '[]'` (JSON array of names, the mirror's exact shape), because list filtering and the claim scan cannot join per row. Refreshed by the broker: **at enqueue** (formalising what `label_enrich` does into payload today — payload keeps its copy for the band derivation's back-compat), **at claim**, and **at every lifecycle transition** — the moments a reader is about to act on the item. Plus a low-frequency broker sweep (hourly, only for actionable statuses) to catch label changes on idle items. `fleet queue list --label user/hold` / `--label priority/high` filter on the cache (json_each), and `show` displays it with its `labels_synced_at` stamp so a reader can see staleness instead of trusting silently.

Cost, stated plainly: one mirror read per transition (same store, indexed PK lookup) — negligible against the transition's existing writes. The sweep is bounded (actionable items only; tonight: 465 rows).

### 7.2 Label add/remove history — captured in the projector, not by polling

The mirror projector (github-ci Worker) is the one component that **already sees every label change**: `issues` webhook deliveries include `labeled` / `unlabeled` actions with the specific label and the sender, and every other action carries the full new set. The design adds, inside the projector's existing upsert path:

```sql
CREATE TABLE IF NOT EXISTS github_label_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo TEXT NOT NULL, number INTEGER NOT NULL, is_pr INTEGER NOT NULL DEFAULT 0,
    added TEXT NOT NULL DEFAULT '[]',     -- label names added by this delivery
    removed TEXT NOT NULL DEFAULT '[]',   -- label names removed by this delivery
    label_set TEXT NOT NULL,              -- full set after the change
    action TEXT NOT NULL,                 -- webhook action (labeled|unlabeled|opened|edited|synchronize|backfill|…)
    actor TEXT,                           -- webhook sender login, when present
    gh_updated_at TEXT, mirrored_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS github_label_history_issue ON github_label_history(repo, number, id);
```

- For `labeled`/`unlabeled` the diff is exact (payload-named). For any other upsert, the projector diffs the stored set against the incoming set — it already reads-before-write for its ordering guards. Backfill runs write one `action='backfill'` row per issue with the full set (added = set, removed = []), so history has a defined start per issue rather than a silent void.
- This is deliberately **mirror-side, not queue-side**: labels belong to the issue, and one issue can back many tasks over its life. Queue items read their label history by joining on the issue ref; a task_history row is *not* written per label change (that would fan one GitHub event into N task rows and drown the transition log). `fleet queue timeline --with-github` is the reader that interleaves them (§8.5).
- The projector is a read model: a missed delivery is repaired by the existing backfill sweep, which writes its own diff row when it lands — history degrades to "as observed", never to fiction. `github_mirror_meta.last_webhook_at` remains the freshness readout.

## 8. GitHub-side change tracking — decided per item

The mirror's projector is the capture point for all of these; the question per item is whether the event is worth a stored row. Storage verdict up front: **one sibling table in the mirror's DDL**, `github_issue_history`, with a `kind` discriminator — not the queue's `events` (different store lifecycle, different writers; the PG era mixed them and the cutover spent a design separating the concerns), and not one table per kind.

```sql
CREATE TABLE IF NOT EXISTS github_issue_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo TEXT NOT NULL, number INTEGER NOT NULL, is_pr INTEGER NOT NULL DEFAULT 0,
    kind TEXT NOT NULL,        -- body_edited | comment_added | comment_removed | comment_count_changed
                               -- | parent_changed | linked | unlinked | state_changed
    detail TEXT NOT NULL DEFAULT '{}',
    actor TEXT, gh_updated_at TEXT, mirrored_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS github_issue_history_issue ON github_issue_history(repo, number, id);
```

### 8.1 Description (body) edits — YES

The mirror already stores `body_hash`; an edit that changes the hash is exactly the "the spec changed under the task" signal programmes currently discover by accident. Projector: on upsert, if the stored `body_hash` differs from the incoming one (and neither is the row's first sight), write `body_edited` with `{old_hash, new_hash}` in detail — **hashes only, never bodies** (bodies live on GitHub; the queue store keeps pointers, per the payload ceiling's standing rule: "evidence and pointers, not full diffs/logs"). Cost: one comparison in an existing path. Value: every design/spec issue edit becomes visible to the task working it.

### 8.2 Comments added/removed — COUNTS ONLY

The mirror already tracks `comments` (count) + `comments_gh_updated_at`, and agent notes get first-class treatment (`latest_agent_note_id/url`, deleted-note tombstones). Full comment capture (bodies, per-comment rows) is rejected: volume (the PG-era stream ran ~33k rows/day with comments-class traffic a large share), duplication of GitHub's own record, and no programme query needs comment bodies from the queue store — they read the issue. What *is* worth a row: the count delta, because "a question was answered / a human replied" is a ready-condition signal lanes already poll for. Projector: on upsert, if the count moved, write `comment_count_changed {from, to}` (with `comment_added`/`comment_removed` used only when the delivery itself is an `issue_comment` action naming the direction). The agent-note pointer changes ride the same row kind with the note id in detail.

### 8.3 Emoji reactions — NO (considered, rejected)

Reactions have no webhook-fed consumer anywhere in the estate: no ready-condition, claim rule, or programme readout keys off them; the mirror doesn't carry them; capturing them means polling the reactions API per issue (the one capture mechanism with real rate-limit cost) for a signal whose only current use is Drew's 👍-on-a-batch in chat — which never touches GitHub. Revisit only if a programme names a query that needs them. Recorded here so the next design doesn't re-litigate from zero.

### 8.4 Issue↔PR linking and parent changes — YES, as first-class queue structure

This is the ask with real machinery behind it, because tonight's programmes *are* link-manipulation programmes (the PR sweep links every PR to issues + tasks; the reprioritization pass splits parents into children). Today links live in prose (`Refs #n`, dedupe keys, pm-metadata `links`/`direct_parent` on the issue) and in the disposition module's three-signal inference. The design gives the queue a link table:

```sql
CREATE TABLE IF NOT EXISTS queue_links (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    other_kind TEXT NOT NULL,        -- issue | pr | task
    other_repo TEXT,                 -- for issue/pr
    other_number INTEGER,            -- for issue/pr
    other_task_id INTEGER,           -- for task
    link_kind TEXT NOT NULL,         -- implements | pr_for | parent | child | split_from | supersedes | duplicate_of | related
    created_by TEXT NOT NULL, created_at TEXT NOT NULL,
    removed_at TEXT, removed_by TEXT,  -- links are soft-removed; the removal is itself the record
    UNIQUE (task_id, other_kind, other_repo, other_number, other_task_id, link_kind)
);
CREATE INDEX IF NOT EXISTS queue_links_task ON queue_links(task_id);
CREATE INDEX IF NOT EXISTS queue_links_other ON queue_links(other_repo, other_number);
```

- **Seeding:** at enqueue, the broker writes the packet's own links (`implements` → its issue; `pr_for` when a packet names its PR). Programmes add the rest with `fleet queue link add/remove <task-id> --kind parent --issue owner/repo#n` (and `--task`, `--pr` forms). Every add/remove writes a task_history state-neutral row (`linked`/`unlinked` kinds join §5.2's set) — parent changes are therefore *in the task's own history*, answering Drew's "marking parent changing parent" on the queue side directly.
- **GitHub-side parents (sub-issues):** GitHub's sub-issue relationships are the issue's truth, the queue's `parent` links are the task's. The projector gains a `parent_number` column on `github_mirror_issues` (from the sub-issues projection in the backfill/API path — webhook payloads for sub-issue changes carry the parent) and writes `parent_changed {from, to}` into `github_issue_history` when it moves. Where the two disagree (queue parent ≠ GitHub parent), that is a *finding for the audit programme*, not an auto-fix — each side keeps its own record and §9's queries surface the divergence.

### 8.5 Deps normalise alongside (the blocker graph becomes SQL)

`queue_links` covers *relationships*; *blocking* gets its own normalisation, because the dep machinery (recheck timers, blocked-until) is stateful in a way links aren't:

```sql
CREATE TABLE IF NOT EXISTS task_deps (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    dep_kind TEXT NOT NULL,          -- blocked-by-issue | blocked-by-pr | blocked-by-task | blocked-by-ci
    ref_repo TEXT, ref_number INTEGER, ref_task_id INTEGER,
    state TEXT NOT NULL DEFAULT 'open',   -- open | cleared | malformed
    created_at TEXT NOT NULL, cleared_at TEXT, cleared_by TEXT,
    detail TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS task_deps_task ON task_deps(task_id, state);
CREATE INDEX IF NOT EXISTS task_deps_ref ON task_deps(ref_repo, ref_number, state);
```

- The broker maintains it at `block` / `unblock` / `dep-recheck` / enqueue-with-deps; `blocked_deps` on the row remains as a **derived cache** during transition (the dep-recheck path reads it today), regenerated from `task_deps` on each write, and retired as a read source once consumers move (§10).
- Malformed deps (tonight's `task:None`) become representable and countable: `state='malformed'` with the raw entry in detail, instead of a silent parse-skip. The blocker graph — "which tasks does issue X block, transitively" — becomes a recursive query over one indexed table instead of 465 client-side JSON parses.

## 9. Consumers — the queries this schema must make cheap

| Consumer | Query | Served by |
|---|---|---|
| Reprioritization audit (pass 1 census) | Band histogram per queue × status; actionable counts | `tasks` (exists) + `resolution IS NULL` terminal audit (Q2) |
| Reprioritization audit (unlock graph) | Direct + transitive unlock counts per issue/task | `task_deps` recursive CTE (Q6) |
| Reprioritization audit (cleanup) | Blocked with no open dep and no question — one query, not 165 prose reads | `tasks` ⋉ `task_deps` + `metadata.question` (Q3/Q6) |
| Reprioritization audit (held check) | All items whose issue carries `user/*` | `tasks.labels` cache (Q5) |
| PR sweep | Every open PR → its tasks; every task → its PRs; PRs with no task | `queue_links` both directions (Q6) + mirror PR table |
| PR sweep (splits) | Parent → children after a split; split-out parts awaiting answers | `queue_links` (split_from) + `resolution='split_out'` (Q2) |
| Stewardship (staleness) | Tasks whose last transition is older than X, by queue; claims near lease cap | `task_history` max(id) per task (Q1) |
| Stewardship (needs-user) | Open questions: blocked/parked with question metadata, age, blocked-by counts per question | metadata `question` + `task_deps` counts (Q3/Q6) |
| PM stack (aggregator/projector) | Everything that changed since cursor C, estate-wide, in order | `GET /history-since` over `task_history.id` (Q1/Q8) |
| PM stack (needs-user board) | Resolution + question state for board items | Q2 columns + Q3 metadata |
| Programmes generally | "Everything programme P created/touched, and what became of it" | `metadata.programme` filter (Q3) + history actor/programme joins |
| Fleet book / audits | Label-change timeline for an issue; body-edit flags on spec issues | `github_label_history`, `github_issue_history` (Q5/Q7) |

The audit skill being written under #14158 should name these queries as its census pack once Q1–Q6 land; until then it keeps the client-side assembly it used tonight, and this table is its migration checklist.

## 10. Migration, backfill, rollout, risks

### 10.1 Migration mechanics

All changes follow the crate's established pattern: new tables join the `SCHEMA` const in `turso_queue/core.rs`; new columns arrive via the existing idempotent `ALTER TABLE … ADD COLUMN` helper (core.rs ~L890–997). No destructive change; every step is reversible by ignoring the new surface. The mirror-side tables/columns land in `pkgs/github-ci/worker/mirror.sql` + `mirror.ts` under the same idempotency rule the Worker already uses (it ensures DDL on every write).

### 10.2 Backfill (18,556 tasks, measured tonight)

- `tasks.metadata`: starts `{}`. One optional pass extracts what payloads already state (programme attribution from dedupe-key families; `verification_class` where a packet carries it) — extraction is lossless copying, never inference.
- `tasks.labels`: one join pass from the mirror by payload issue ref (the `label_enrich::payload_issue_ref` logic, reused); items with no mirror row keep `[]` + a null sync stamp, which reads honestly as "never synced".
- `task_history`: backfilled from `events` where kind maps to §5.2 (claimed/completed/parked/released/failed/cancelled/…), marks lifted from detail JSON into the columns where present, `source='backfill'`. Events whose kind doesn't map stay events-only. The backfill count vs the live count is reported on #14170 — history before the cutover is *reconstructed*, and the record says so.
- `resolution`: §6.5's provable-only rule.
- `task_deps`: parsed from `blocked_deps`; unparseable entries land as `state='malformed'` rows (tonight's two known cases become the first citizens, not silent drops).
- `queue_links`: seeded from packets (issue refs) and dedupe keys; PR links seeded from mirror + disposition records where they exist.

### 10.3 Rollout order and risks

Order is the slice order (§11): history first (everything else writes into it), resolution second (the audit's biggest hole), metadata third, notes fourth, labels fifth, links/deps sixth, projector history seventh, consumers eighth. Each slice deploys by the fleet-static generation pattern; broker slices dual-write before any reader switches.

Risks, named:

- **Claim-path latency.** History adds one indexed insert inside the claim transaction. The claim harness (fleet-queue tests + the broker claim benchmarks) must show no material regression before the generation flips; if it does, the insert moves to a same-transaction batch write, not to an async queue (async loses fail-closed).
- **Direct-SQL programmes bypass capture.** Tonight's pass 2 wrote priorities by SQL until #14100's verb lands. History completeness therefore depends on programmes using verbs — the reprioritization skill (under #14158) already routes its writes through the verb once it exists; this design adds the same requirement to history-affecting writes and gives `source='sql-programme'` as the honest marker when SQL is genuinely unavoidable (the programme writes its own history rows in the same script, as 2026-11-12 did for events).
- **Projector change risk.** The github-ci Worker is production webhook infrastructure; its diff-capture must be fail-open (a history-write failure never fails the mirror upsert — the mirror never gates work, #8575, and neither does its history).
- **Turso size.** History + notes + GitHub history at estate rates: transitions are ~10³/day class, label/body/comment rows bounded by webhook volume on ~1.5k watched issues — years of headroom in the store's current footprint class; the 1M-row review point in §5.3 is the tripwire.
- **Vocabulary drift.** Three taxonomies (resolution, metadata keys, link kinds) are registry constants in `validate.rs` with the same change discipline as `PRIORITY_SCALE`: additions are cheap, renames are migrations.

## 11. Slice map (packet-ready; dependencies in order)

- **Q1 — task_history foundation.** Table + indexes (§5.1); broker dual-write (history row + existing event) inside every lifecycle verb's transaction, marks copied from transition context; backfill from `events` (`source='backfill'`, counts reported); `timeline` reads history + notes-interleave-ready; `/history-since` broker route. *Oracle:* for a scripted lifecycle (enqueue→claim→park→wake→block→unblock→reprioritize→complete), the history rows match the transitions 1:1 with correct from/to, actor, session, model; claim-harness latency within noise; backfill count reconciles against events census. *Depends on:* nothing. (Reprioritize rows arrive with #14100's verb; the kind is reserved now.)
- **Q2 — resolution columns + taxonomy.** Columns (§6.2), registry + validation in `validate.rs`, verb parameters on complete/cancel, disposition→substatus mapping, revive clearing, provable-only backfill + baseline census on #14170. *Oracle:* a cancel without resolution is rejected by the verb; a disposition-verified completion stamps `done/verified` unaided; revive preserves the prior verdict in history. *Depends on:* Q1.
- **Q3 — metadata column + registry + `fleet queue meta`.** Column, limits, registry (§3.2) with type checks, verb family + enqueue `--meta` + list `--meta` filter, `meta_updated` history rows. *Oracle:* merge-patch semantics (untouched keys stand, null deletes); an over-limit write is rejected naming the limit; list filter returns exactly the programme's tasks. *Depends on:* Q1.
- **Q4 — task_notes.** Table, note verbs (add/list/edit/remove-soft), author/session/model marks, show/timeline interleave. *Oracle:* two concurrent note adds both survive in order; an edit preserves the original in history; timeline renders one chronological story. *Depends on:* Q1.
- **Q5 — labels current + history.** `tasks.labels` cache + `labels_synced_at`, broker refresh at enqueue/claim/transitions + actionable-only sweep, list `--label`; projector label-diff into `github_label_history` incl. backfill rows. *Oracle:* label an issue on GitHub → the cache and a history row with exact added/removed appear without any queue write; a `user/*` filter lists exactly the held set. *Depends on:* Q1 (cache stamps ride transitions); projector half is independent and may land first.
- **Q6 — links + deps normalised.** `queue_links` + `task_deps` (§8.4/§8.5), broker maintenance at enqueue/block/unblock/dep-recheck, `fleet queue link` verbs, blocked_deps cache transition, malformed-dep capture, backfill from rows/packets. *Oracle:* tonight's census blocker graph reproduces from SQL alone, counts matching #14158's record (241 open edges, top blockers #3399→7 …); a parent change appears in the child's history. *Depends on:* Q1.
- **Q7 — GitHub-side history in the projector.** `github_issue_history` + `parent_number` projection: body-edit rows (hash-only), comment-count deltas, parent-change rows; fail-open throughout. *Oracle:* editing a watched issue's body, commenting, and re-parenting a sub-issue each produce exactly one correct row; a killed history write still lands the mirror upsert. *Depends on:* nothing (parallel with Q5's projector half; shared review).
- **Q8 — consumers.** Aggregator since-cursor feed consumption (PM projector), stewardship staleness/needs-user queries onto history, the #14158 audit skill's census pack rewritten onto §9's queries, fleet book chapter update (book-live-update). *Oracle:* stewardship's cycle reads staleness from one query; the audit census reproduces tonight's numbers from the canned pack. *Depends on:* Q1–Q6 as applicable per consumer.

## 12. Decisions taken in this design (named deciders)

- **D1 — history as a new table, not events columns.** Decided here: `events` has non-task writers and a prune; transitions deserve a structured, prune-exempt home. (Reversible at Q1 review; the dual-write keeps both alive.)
- **D2 — resolution required for cancelled, optional for failed.** Decided here, from §6.4's reasoning. Drew is the decider if the failed-task rule should tighten.
- **D3 — reactions: no.** Decided here (§8.3); Drew may override with a named consumer.
- **D4 — notes queue-local in v1** (no automatic GitHub mirroring). Decided here; the outbox route exists when a programme needs posting.
- **D5 — metadata write trust = broker caller trust** (no per-key ACL in v1). Decided here, matching every existing verb.
- **D6 — label history lives mirror-side**, queue reads by join. Decided here (§7.2); the alternative (fan-out into task_history) was rejected on volume and ownership grounds.

*End of design.*
