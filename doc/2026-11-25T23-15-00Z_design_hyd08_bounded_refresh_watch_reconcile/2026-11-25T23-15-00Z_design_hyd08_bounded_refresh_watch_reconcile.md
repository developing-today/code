# HYD-08 — Bounded refresh, watch and reconcile commands and capability contract

Issue: [#9510](https://github.com/developing-today/code/issues/9510)
(slice `HYD-08` of [#9207](https://github.com/developing-today/code/issues/9207),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the HYD-08 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225): "implement bounded `hydrate`, `refresh`, `changes`,
`activity reconcile`, `watch` service behavior mapped to shared
contracts", first oracle
`pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt` ("tests
deadline/cancel without worker spawn"), negatives "ambiguous resource,
unsupported include advertised, timeout returns success or fictional
cancellation, watch spawns hidden poller". The merged CLI-09 façade
rows are folded in here. HYD-04 decided what one hydration fetches,
HYD-05 how a walk pages and commits, HYD-06 how edits are detected,
HYD-07 who made a change. This record decides the commands that
drive them: what each one calls, its flags, its deadline and
interrupt behaviour, what it returns, and what the capability listing
may claim. It authorizes no code, registration or deployment (§9).

Governing decisions consumed, not re-opened:

- **Component #9207 §3:** the proposed spellings
  `fleet work hydrate|refresh|changes` and
  `fleet activity reconcile|watch`; "`watch` must be bounded and
  interruptible, with a durable cursor; it does not spawn background
  scheduler"; detailed outcomes map onto the shared outcomes; unknown
  work, ambiguous ref, permissions, rate limits, truncation, stale PR
  head and cursor gaps are distinct.
- **HYD-04** (`doc/2026-11-25T21-40-00Z_design_hyd04_bounded_hydration_adapter_summary/`):
  the source plan and its `endpoint` names, the outcome → per-source
  status table, budgets (proposed 200 requests, 120 s, 64 MiB) with
  "HYD-08 maps [`time_budget`] to a resumable timeout outcome", and
  the single entry point `hydrate(inspection, plan_options, budget,
adapter, read_policy, publisher) -> HydrationResult`. Review-thread
  resolved state is `unknown` until an adapter path exists.
- **HYD-05** (`doc/2026-11-23T13-02-35Z_design_hyd05_pagination_api_caps_resumable_durable_reads/`):
  cursor key and states (`absent`, `in-progress`, `complete`,
  `unknown`); a walk ends `complete`, `truncated` or `incomplete`;
  page and cursor commit together; callers drive walks, "this slice
  adds no scheduler and no polling loop".
- **HYD-06** (`doc/2026-11-23T19-40-12Z_design_hyd06_activity_revisions_tombstones/`):
  updated-since sweep with an overlap window plus bounded
  authoritative reconciliation "driven by the same caller that drives
  hydration — no new scheduler"; coverage-limited endpoints render
  "no _observed_ changes".
- **HYD-07** (`doc/2026-11-25T20-59-50Z_design_hyd07_operation_session_attribution/`):
  `fleet`/`external`/`uncertain` classification; `--external-only`
  keeps `uncertain`; `--strict-external` reserved for CLI-13;
  `--require-clean-reconciliation` fails with `uncertain` on
  `map_stale`.
- **HYD-03** (`doc/2026-11-25T22-15-00Z_design_hyd03_immutable_generation_atomic_publication/`):
  staging and atomic publication; the prior generation stays current
  until the new one publishes.
- **HYD-01** (PR #15184, **open at writing**): manifest and result
  envelope. This record adds no manifest key; if #15184 changes, this
  record follows it.
- **CLI-02** (`doc/2026-11-23T12-33-39Z_design_cli02_shared_command_contract_strict_parser/`):
  strict parse first, typed locators, shared flag families, outcome
  set, usage error exit 2. **CLI-13** (#9442) owns registration,
  help and the golden help/JSON test.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- No `fleet work`, `fleet activity` or `fleet sync` namespace is
  registered; every allocated row (`work.refresh`, `work.resume`,
  `activity.list|since|reconcile|watch`, and the flags) is `absent` in
  `census/commands.jsonl`.
- `work/context.rs` and `work/activity.rs` do not exist; `work/mod.rs`
  lists `authority`, `completion`, `contracts`, `degraded`, `journal`,
  `lifecycle`, `operations`, `priority`, `routing`, `selection`.
- `work/journal.rs` (JRN-03) has checkpoint and handoff entries with
  `HydrationRef` and `CursorRef`.
- The only GitHub boundary is `pkgs/fleet/crates/fleet/src/gh.rs`.

## 1. Intent

An agent can pull a work item's context, ask what changed since it
last looked, and keep watching for a bounded time, and every one of
those calls ends on time, says exactly which sources it saw fully,
and leaves cursors from which the next call resumes. Nothing keeps
running after the command returns, and a deadline is never reported
as success or as a cancellation of something that already happened.

## 2. Ownership

| Concern                                           | Owner                                 | This slice                 |
| ------------------------------------------------- | ------------------------------------- | -------------------------- |
| Command service functions over hydration/activity | `work/context.rs`, `work/activity.rs` | **owns** (service + tests) |
| What one hydration fetches, statuses, budgets     | HYD-04                                | consumer                   |
| Page walk, cursor commit                          | HYD-05                                | consumer                   |
| Revisions, sweep, reconciliation semantics        | HYD-06                                | consumer                   |
| Attribution and `--external-only` meaning         | HYD-07                                | consumer                   |
| Generation staging and publication                | HYD-03                                | consumer                   |
| Registration, help, golden help/JSON test         | CLI-13                                | not here                   |
| Claim during `resume`                             | SEL-02 claim revalidation             | consumer                   |

## 3. Commands

Each command is a function in `work/context.rs` or `work/activity.rs`
taking a parsed CLI-02 envelope and a deadline, returning the shared
`fleet.result/v1`. CLI-13 binds spellings to them.

| Command (census key)                                                                                | Calls                                                                                                    | Writes                                   |
| --------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ---------------------------------------- |
| `fleet work hydrate <work> [--full\|--incremental] [--include S,…] [--output DIR]` (`work.hydrate`) | HYD-04 `hydrate`                                                                                         | a staged then published generation       |
| `fleet work refresh <work> [--include S,…] [--since-checkpoint\|--since T]` (`work.refresh`)        | HYD-06 sweep for the selected sources, then HYD-04 for sources whose cursor is `unknown` or that changed | a new generation; the checkpoint is kept |
| `fleet work changes <work> [--since-start\|--since-checkpoint\|--since T] [--external-only]`        | HYD-06 observation projection filtered by window, HYD-07 classification                                  | nothing                                  |
| `fleet activity since <work> …` (`activity.since`)                                                  | the same function as `work changes`; identical output                                                    | nothing                                  |
| `fleet activity list <work>` (`activity.list`)                                                      | HYD-05 cursor rows for the work                                                                          | nothing                                  |
| `fleet activity reconcile <work> [--source REF] [--dry-run]` (`activity.reconcile`)                 | HYD-06 authoritative re-walk for each cursor key                                                         | pages and cursors, committed together    |
| `fleet activity watch <work> [--timeout D]` (`activity.watch`)                                      | repeated sweep passes in the caller's process (§5)                                                       | pages and cursors, committed per page    |
| `fleet work resume <work> [--claim] [--hydrate]` (`work.resume`)                                    | `changes --since-checkpoint`, blocker check, then optional SEL-02 claim and incremental hydrate          | only what the composed calls write       |

Rules shared by all:

1. **Resource resolution first.** `<work>` and `--source REF` are
   typed locators (CLI-02). An ambiguous work ref is HYD-01's
   top-level identity error and fetches nothing. A `--source` that
   matches more than one linked resource is `link_target_ambiguous`;
   one that is not linked to the work is `source_not_linked`. A
   number in text is never matched.
2. **Reads only.** Every remote request is a GET classified as a read
   by `gh.rs` before it is sent (HYD-04 §8).
3. **`work.resume` is not queue wake.** It composes the primitives in
   order and adds no state of its own; `--claim` goes through SEL-02's
   revalidation and may be refused there.

### 3.1 Flags

| Flag (census key)                              | Decision                                                                                                                                                                                              |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--full` (`flag.full`)                         | fresh generation from every source, ignoring prior cursors; budgets still apply and a budget hit is still `truncated`                                                                                 |
| `--incremental`                                | default when a complete prior generation exists; otherwise behaves as `--full` and says so                                                                                                            |
| `--include` (`flag.include`)                   | a subset of HYD-04 §3.2 `endpoint` names; each listed with its cursor and completeness. A name the adapter cannot fetch is refused with `include_unsupported` before any read                         |
| `--output DIR` (`flag.output`)                 | exports a copy of the **published** generation; write to a temp directory under DIR then rename; a write failure is `output_write_failed` and changes no cursor or generation                         |
| `--since T` (`flag.since`)                     | RFC 3339 with an explicit offset; no offset is a usage error. Selects revisions with `source_updated_at ≥ T` or `occurred_at ≥ T`; items first observed after T but older are flagged `late_observed` |
| `--since-checkpoint` (`flag.since-checkpoint`) | the cursors recorded in the latest journal checkpoint's `CursorRef`s; none recorded → `since_checkpoint_unknown`, never "everything"                                                                  |
| `--since-start` (`flag.since-start`)           | the current session's start entry in the journal; none → `since_start_unknown`, never the current time                                                                                                |
| `--start` (`flag.start`)                       | settled as **not** an alias: refused as a usage error that names `--since-start` and `--since`, until an owner defines a distinct meaning                                                             |
| `--external-only` (`flag.external-only`)       | HYD-07 §5 exactly: `uncertain` stays in, `fleet` is counted and listed as excluded                                                                                                                    |
| `--timeout D`                                  | §4                                                                                                                                                                                                    |
| `--dry-run` on `reconcile`                     | reports which cursor keys would be walked and which are `unknown` (full snapshot); makes **no** remote request and writes nothing                                                                     |

## 4. Deadlines, interrupts and outcomes

- **One deadline per command**, from `--timeout` or the command's
  default (proposed: hydrate and refresh 120 s, matching HYD-04's
  time budget; changes and list 30 s; reconcile 120 s; watch has no
  default and requires `--timeout`). The effective time budget passed
  to HYD-04/HYD-05 is the smaller of the deadline and the configured
  budget.
- **On deadline or interrupt** (SIGINT, SIGTERM, closed stdout) the
  walk stops before the next request; an uncommitted page is
  discarded, never half-written; every committed page keeps its
  cursor. The command returns a **cursor receipt**: per source, the
  cursor key, state and position, plus the exact command to resume.
- **Never success, never fictional cancellation.** A command stopped
  by its deadline returns `partial` (sources `truncated/time_budget`),
  never `verified_complete`. The words "cancelled" or "aborted" never
  describe remote state; the commands only read.

| Situation                                                       | `outcome`           | Notes                                                        |
| --------------------------------------------------------------- | ------------------- | ------------------------------------------------------------ |
| every requested source `complete`, no cursor `unknown`          | `verified_complete` |                                                              |
| any source `truncated`, `partial`, `unavailable` or `forbidden` | `partial`           | per-source detail and receipt                                |
| deadline or interrupt                                           | `partial`           | `time_budget` / `interrupted`; receipt                       |
| a new generation failed to build                                | `rejected`          | `hydration_failed`; names the still-current prior generation |
| ambiguous or unknown work                                       | `rejected`          | HYD-01 identity error                                        |
| `map_stale` with `--require-clean-reconciliation`               | `uncertain`         | HYD-07 §5                                                    |
| `changes` over a coverage-limited endpoint                      | as above            | the endpoint reads "no observed changes", never "no changes" |

The exit-status values belong to CLI-02's matrix; this record fixes
only that `partial` and `uncertain` are never exit 0 for hydrate,
refresh, changes and reconcile.

## 5. `watch` without a hidden poller

- `watch` runs in the calling process and thread: a loop of HYD-06
  sweep passes over the work's cursor keys, each pass committing page
  by page. No thread, task, child process, daemon, timer file or
  queue row is created; when the command returns, nothing of it is
  left running.
- Between passes it sleeps for the longest of the source's
  rate-limit reset, its `Retry-After`, and a minimum pass gap
  (proposed 30 s), and never past the deadline.
- It emits one line (or one JSON object with `--json`) per observed
  change as it is committed, then the cursor receipt at the end.
- A watch that ends at its own deadline or on interrupt returns
  `partial` with `stop_reason: deadline | interrupted` and the
  receipt; its exit status is the matrix's row for a requested stop,
  proposed 0, because the stop was asked for.
- Anything recurring is not a watch: a scheduled reconcile is a new
  schedule and lands with its row in `doc/fleet-schedule-registry.md`
  in the same PR. This slice adds none.

## 6. Capability contract

`fleet.capabilities` (CLI-02) reports, for hydration and activity:
the `endpoint` names the adapter can actually fetch, each source's
coverage limitation (HYD-06 §4), the budget profile and its version,
and the reserved-but-unavailable spellings. A source is advertised
only if `hydrate` would fetch it; review-thread resolved state is
listed as `unknown`, not as supported. `--include` validates against
this same list, so the capability output and the refusal can never
disagree.

## 7. Tests

In `work/context.rs` and `work/activity.rs`, against a fake adapter
that counts requests and a spawn probe at the service seam:

| Test                                                              | Asserts                                                                                                |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt`        | deadline mid-walk → `partial`, `time_budget`, cursors at last committed page, resume command; no spawn |
| `pmo_hyd_refresh_resume_from_receipt_refetches_nothing_committed` | second call from the receipt starts at the stored continuation                                         |
| `pmo_hyd_interrupt_discards_uncommitted_page`                     | interrupt between fetch and commit → page absent, cursor unchanged                                     |
| `pmo_hyd_timeout_never_reports_success_or_cancellation`           | outcome is never `verified_complete`; no remote state described as cancelled                           |
| `pmo_hyd_watch_spawns_no_worker_and_leaves_nothing_running`       | spawn probe zero; no thread or timer alive after return                                                |
| `pmo_hyd_watch_requires_timeout`                                  | missing `--timeout` → usage error                                                                      |
| `pmo_hyd_ambiguous_work_fetches_nothing`                          | ambiguous ref → identity error, request count zero                                                     |
| `pmo_hyd_include_unsupported_refused_before_reads`                | unknown or unavailable source → `include_unsupported`, zero requests                                   |
| `pmo_hyd_capabilities_advertise_only_fetchable_sources`           | advertised set equals the set `hydrate` fetches                                                        |
| `pmo_hyd_since_start_unknown_refused_not_now`                     | no session start → `since_start_unknown`                                                               |
| `pmo_hyd_start_flag_not_aliased`                                  | `--start` → usage error naming `--since-start` and `--since`                                           |
| `pmo_hyd_output_write_failure_keeps_generation_and_cursors`       | unwritable DIR → `output_write_failed`, generation and cursors unchanged                               |
| `pmo_hyd_reconcile_dry_run_makes_no_request`                      | dry-run → plan only, request count zero, no writes                                                     |
| `pmo_hyd_changes_coverage_limited_reads_no_observed_changes`      | coverage-limited endpoint renders "no observed changes"                                                |

Done command (remote runner):

```sh
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_refresh_timeout_returns_resumable_cursor_receipt -- --list
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_
```

The golden help and JSON output test belongs to CLI-13.

## 8. Persisted fields

None. Cursors and pages are HYD-05's, observations HYD-06's,
generations HYD-03's, checkpoints JRN-03's. Proposed reason codes for
the `ReasonCode` registry owner: `include_unsupported`,
`output_write_failed`, `since_start_unknown`,
`since_checkpoint_unknown`, `source_not_linked`, `hydration_failed`.

## 9. Gates

- Registration waits for CLI-13 (#9442); implementation waits for
  HYD-01 (#15184) to merge and for HYD-04/05/06 implementations.
- **Owner decisions asked:** the default deadlines and the minimum
  watch pass gap; whether `--start` ever gets its own meaning; the
  exit status for a watch that stops at its own deadline.
- **Rollback:** unregister the commands; published generations,
  cursors and observations stay readable and the queue is untouched.

## 10. Requirement trace (design-level)

| IDs                                                                                                                    | Outcome                                                                         | Where            |
| ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- | ---------------- |
| C-R118, P-R018                                                                                                         | changes since start, checkpoint or time; external-only keeps uncertain          | §3.1             |
| H-R014, O-R021, P-R013                                                                                                 | durable hydration and per-source cursors driven by bounded commands             | §3, §4           |
| C-R153, P-R037, P-R039, O-R050, P-R036                                                                                 | near-timeout and resume: receipts and checkpoint cursors, never a false success | §4, `resume`     |
| P-R031, P-R035, C-R110, C-R150, C-R151                                                                                 | open/resume compose claim and hydration from primitives                         | §3 `resume`      |
| C-R005, C-R009, C-R107, C-R108, H-R001, H-R016, H-R017, H-R024, H-R031, O-R022, O-R023, O-R043, P-R009, P-R027, P-R029 | cohesive CLI; inventory kept; distinct outcomes and failure cases               | §3, §3.1, §6, §7 |

## 11. Non-goals

- No registration, help text or alias approval (CLI-13, CLI-02).
- No new fetch, status, budget, cursor or attribution rule (HYD-03 to
  HYD-07).
- No scheduler, background worker or recurring job.
- No remote write.
