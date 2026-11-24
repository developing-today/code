# Completed-cohort usage measurement (#2140)

This is the first measurement slice, **not a production before/after result**.
It does not change lane schedules or activate supervision.

```sh
python3 -I scripts/fleet_completion_usage.py --snapshot cohort.json \
  --start 2026-11-01T00:00:00Z --end 2026-11-02T00:00:00Z
```

`cohort.json` is one JSON object containing four arrays:

| Array | Native fields consumed | Source |
| --- | --- | --- |
| `tasks` | `id`, `queue`, `status`, `progress.session_id` (object or JSON string); optional normalized `session_id` | rich broker task export; `pkgs/fleet/hosted-worker/src/queue-store.ts` |
| `events` | `task_id`, `kind`, `created_at` | complete broker event export for those tasks |
| `ledger` | `queue`, `task_id`, `session_id` | broker execution ledger, including retries |
| `sessions` | `id`, `tokens.input` | cumulative OpenCode session export; `pm-stack/aggregator/src/sources/opencode.ts` |

Export all relevant task rows, finishing events, execution bindings and sessions
before deleting sessions. This command intentionally does not query the broker
or OpenCode: collection/pagination and check-in publication are subsequent slices.
An absent history/binding cannot be reconstructed by this report.

## Meaning and limits

- The denominator follows `pm-stack/console/src/trends.ts`'s completion oracle:
  current status `done`, latest `completed` or `staleness_autocomplete` event in
  the half-open interval `[start,end)`. Reopened tasks do not count. Repeated
  finishing events count once, at the latest timestamp.
- The numerator is **cumulative input tokens of the bound execution sessions for
  those completed tasks**, including distinct retry sessions from the ledger.
  It is not input spent during the time window, total lane-shift overhead,
  cache-read tokens, output tokens, or the average of per-task ratios.
- `claim_session_id` is not an execution binding: it may identify a supervisor.
  The report deliberately does not use it. A session bound to multiple tasks or
  conflicting ledger queue is an error, rather than double-attributed spend.
- Each queue has `completed_tasks`, `measured_tasks`, `missing_task_ids`,
  `known_input_tokens` and `input_tokens_per_completed_task`. Missing session
  usage/bindings produce a null ratio, not a misleading zero or partial ratio.
  Explicit zero input is measured. No completed cohort produces no queue row.
- `done_tasks_without_completion_event` identifies rows whose window cannot be
  established; any such row suppresses its queue's ratio. The report cannot
  discover tasks/events/sessions omitted entirely from an export. Full source
  coverage is a prerequisite for interpreting its numbers.

Two complete exports can support a completed-cohort comparison. A causal cadence
evaluation still needs comparable cohorts, actual full-shift/window spend,
throughput and coverage, and daily check-in integration. Do not infer a 40m→2h
recommendation from this report alone.

## Acceptance inventory at base `076667840ef`

1. **Snapshots: checked.** #2158 closed; the two-week operational handoff evidence
   is on #2140 (2026-11-13). Shared external instructions, not copied into this
   repository: `repo-queue-worker-instruction.md:757–789` and
   `lane-shift-instruction.md:96–103,375+`.
2. **Binding-aware supervision substrate: checked.** #1656 closed, PRs #12879
   (retry), #12859 (separate intervention budget), #13625 (binding on every
   verdict) merged. Repository evidence: `pm-stack/opencode-claimant/babysitter.py`
   `public_binding`/`build_bindings` and `test_babysitter.py` binding regression.
   Sustained 15–20 sessions/shift throughput is **unmeasured here**.
3. **Cadence evaluation and per-queue tokens/completed task: unchecked/unmeasured.**
   #2135 is quota-steward snapshot/probe/follow-up work; its closure does **not**
   prove this reporting criterion despite repeated earlier reconciliation notes.
   This PR supplies only the offline completed-cohort measurement primitive.

Oracle/tests: exact integer sums and the console completion model, plus seeded
permutation-invariance checks. Run:

```sh
python3 -I -m unittest discover -s scripts -p test_fleet_completion_usage.py -v
```

The same focused suite is registered in nightly/manual `scripts-test.yml`.
