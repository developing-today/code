# placement-record PoC (#4986)

Proof of concept for the placement framework v1 (#2024, accepted as the
framework of record by the orchestrator review on #2024, 2026-11-17): a
dispatcher-consumable evaluator over the machine-readable policy artifact
(`doc/2026-11-03T15-10-56Z_design_task-placement-framework-v1/placement-policy.yaml`,
merged in PR #6766) that emits the **placement record** the policy declares —
`option`, `rule`, `pool_read_at`, `supervision` — for ONE dispatch path, the
**queue-worker handoff**.

## What it does

`placement-record --policy <placement-policy.yaml> --answers <answers.json>`
reads a JSON description of one dispatch (the framework's decision questions
Q1–Q6, the R2 guard trigger, an optional E1 exception, and the quota-read
timestamp) and prints the placement record as JSON. Evaluation follows the
policy itself, not a second copy of the table:

- questions in order, first match wins (R1 → R3 → R5 → R6 → R7 → R8 → the
  policy's default rule);
- the R2 guard, when triggered, excludes `github-action-jules` and `openclaw`
  and makes R3 unreachable; a later rule whose option is excluded is skipped;
- `supervision` is copied from the chosen option's own field in the policy;
- `pool_read_at` is `"n/a"` for options that draw on no pool, and otherwise
  must satisfy the B5 measurement gate (a read at most 60 minutes old,
  from the policy's `burn_rules`) — or the record must carry an explicit B4
  reliance (`pool_read_unobtainable`), because an unknown read never idles a
  pool but is never silent either;
- an exception dispatch records `rule: "exception"` with its `exception_id`
  and the option actually chosen (E1).

The command exits non-zero when no valid record can be emitted: a dispatch
that cannot produce a record does not proceed unrecorded.

## Tests

`go test ./...` evaluates the real policy of record (the tests load the
checked-in `placement-policy.yaml`; they skip only if it is absent): every
rule path, first-match-wins, the guard's exclusion of R8 and unreachability
of R3, the B5 gate (missing read, stale read, B4 reliance), the `"n/a"`
non-pool case, the exception path, and the monthly pace guard (refuse at
≥97% / >1pp/h / hard-stop; B4, missing data, and non-pool seatings
proceed).

## Monthly pace guard (issue #2732, slice 1)

A pool-drawn seating is refused — exit non-zero with the pool, the % read,
and the guard source on stderr — when the fresh quota read behind it shows
`pool_monthly_pct` at/above 97%, `pool_pace_pph` above 1pp/h, or
`pool_hard_stop` set (the steward hard-stop in machine-readable form).
Unknown state never refuses: a B4 reliance proceeds, absent monthly data
seats on the fresh read, and non-pool options ignore the guard inputs.
An E1 exception bypasses the guard — the exception record itself is the
required log line.

## Demonstration

A queue-worker handoff of a triage task — the lane's most frequent dispatch
class — evaluates deterministically: triage verdicts are Q4 judgement in the
policy's own words (R6's condition names them), the work needs the laptop
(Q2 guard, recorded in the record's `guard_r2_excluded_options`), and the
record lands on `Muse` with the option's own supervision text. The emitted
record for a live handoff of exactly that shape is posted on #4986 with the
PR that carries this PoC.

One seam the PoC review should settle: implementation handoffs. The table's
default rule (R4) places implementation work on a dispatched OpenCode
session, while the newer direct-assignment doctrine (#8187, which the #2024
review says wins where the two differ) makes the claim itself the placement
for queue work. This PoC evaluates the table as written; how a claimed
implementation task's record should be emitted — R4 as evaluated, or the
claiming lane's option under a recorded E1 exception citing #8187 — is a
review decision, not something the PoC pre-judges.

## Not in this PoC (the rollout this PoC is reviewed for)

- Wiring into the live fleet queue client or the OpenChamber dispatcher
  (the second named path) — that is the permanent rollout, gated on review
  of this PoC per the #2024 program rule.
- The weekly audit sample (placements vs. the table, exceptions listed) —
  it audits records once dispatches emit them in production.
