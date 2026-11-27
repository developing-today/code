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

## Live quota read (#2732)

The monthly guard needs a live monthly percent, not a hand-supplied one.
Pass `--quota-current <quota-current.json> --pool <serviceId>` and the
tool fills `pool_monthly_pct`, `pool_read_at`, and `pool_id` from the
quota gatherer's monthly window for that pool — the gatherer
(`pm-stack/aggregator/scripts/gather-quota.ts`) writes `quota-current.json`
every 15 minutes, and the reader (`ReadQuotaCurrent`) takes the newest
`ok` entry for the serviceId under the policy's B5 bound (60 minutes).
An out-of-range monthly percent drops instead of clamping. An entry the
gatherer retained after a failed tick (`retainedFromFailedTick`) is stale,
never a live read. If a fresh read fails, the tool retries once and then
uses the latest valid last-known monthly measurement under B4, marking
`b4_unknown_read_relied_on` in the record and reporting that reliance on
stderr. It exits non-zero if no last-known monthly measurement exists. A
successful fresh read clears any `pool_read_unobtainable` B4 fallback
recorded in the answers file, and explicitly empty `--quota-current` /
`--pool` values are a usage error, never a silent fall back to the
hand-supplied inputs. A seating against a fresh pool read at or above 97%
monthly then refuses with the `monthly-guard refusal` line the weekly audit counts
(`placement-audit --log`), so the refusal is countable from
dispatcher-log data:

```sh
placement-record --policy placement-policy.yaml --answers answers.json \
  --quota-current quota-current.json --pool chatgpt-plan
# placement-record: monthly-guard refusal: option "Muse" on pool "chatgpt-plan" refused (monthly 98.4% at/above 97% threshold [source: monthly-guard])
```

## Steward hard-stop file (#2732 proposal step 2)

The monthly percent covers the threshold guards; the steward hard-stop
covers the judgement call — a pool the steward declares stopped before
it walls. Pass `--hard-stops <hard-stops.json>` and the tool checks the
steward-written registry for the seating's pool before evaluating:

```json
{ "version": 1, "stops": { "chatgpt-plan": { "stopped": true, "note": "monthly 97% hard stop", "issuedAt": "2026-10-29T13:22:00Z" } } }
```

A stop for that pool refuses the seating with the hard-stop refusal
line the weekly audit already counts (`[source: hard-stop]`); a pool
with no entry — or an explicit `stopped: false` — seats unaffected,
and the file never clears a `pool_hard_stop` recorded in the answers.
A missing or malformed registry fails the dispatch loudly instead of
seating unguarded. Follow-up (not this slice): the steward writer that
maintains `hard-stops.json` beside `quota-current.json`, and the
trailing-pace input, which still has no live producer.

## Tests

`go test ./...` evaluates the real policy of record (the tests load the
checked-in `placement-policy.yaml`; they skip only if it is absent): every
rule path, first-match-wins, the guard's exclusion of R8 and unreachability
of R3, the B5 gate (missing read, stale read, B4 reliance), the `"n/a"`
non-pool case, the exception path, the monthly pace guard (refuse at
≥97% / >1pp/h / hard-stop; B4, missing data, and non-pool seatings
proceed), the hard-stop registry (stopped/absent/explicit-false reads,
sticky stop, fail-closed I/O, pool identity required), and the weekly
audit sample (emitted records re-checked against
the table, tampered records named by finding kind, deterministic
sampling, refusal counting).

## Monthly pace guard (issue #2732, slice 1)

A pool-drawn seating is refused — exit non-zero with the pool, the % read,
and the guard source on stderr — when the fresh quota read behind it shows
`pool_monthly_pct` at/above 97%, `pool_pace_pph` above 1pp/h, or
`pool_hard_stop` set (the steward hard-stop in machine-readable form).
Unknown state never refuses: a B4 reliance proceeds, absent monthly data
seats on the fresh read, and non-pool options ignore the guard inputs.
An E1 exception bypasses the guard — the exception record itself is the
required log line.

## Weekly audit sample (issue #2732, slice 2)

`placement-audit --policy <placement-policy.yaml> --records <records.json>`
re-checks emitted placement records against the policy table — the named
rule must select the recorded option, supervision must be the option's
own, exception records must carry their exception id, `pool_read_at`
must be a timestamp exactly for pool-drawn options and `"n/a"`
otherwise, and the record's policy version must match — lists every
exception record in the sample, and prints the audit as JSON.
`--sample N` audits N evenly spaced records (deterministically); the
command exits non-zero when any sampled record disagrees with the
table. With `--log <dispatcher.log>` it also counts the monthly pace
guard's refusal lines — total, per pool, per guard source — so the
steward daily pass can cite refusal counts from data (the issue's
second acceptance criterion) instead of reconstructing seatings after
a wall. Refusals are counted, never treated as disagreements: they are
the guard working. The audit runs on records wherever they are
emitted; today that is `placement-record` output, and the same audit
covers production records once the wiring below lands.

## Demonstration

A queue-worker handoff of a triage task — the lane's most frequent dispatch
class — evaluates deterministically: triage verdicts are Q4 judgement in the
policy's own words (R6's condition names them), the work needs the laptop
(Q2 guard, recorded in the record's `guard_r2_excluded_options`), and the
record lands on `Muse` with the option's own supervision text. The emitted
record for a live handoff of exactly that shape is posted on #4986 with the
PR that carries this PoC.

For a claimed queue-work implementation task, record the claiming lane's
option under `rule: "exception"` with an `exception_id` citing #8187; the
claim itself is the placement.

## Not in this PoC (the rollout this PoC is reviewed for)

- Wiring into the live fleet queue client or the OpenChamber dispatcher
  (the second named path) — that is the permanent rollout, gated on review
  of this PoC per the #2024 program rule. The producer and the call shape
  for that wiring are now named: the quota-current producer is the
  gatherer's `quota-current.json` (read by `ReadQuotaCurrent`, with the
  `--quota-current` / `--pool` flags on `placement-record`), and the
  dispatcher invokes `placement-record` before seating and captures its
  refusal stderr into the dispatcher log, where `placement-audit --log`
  counts it. What remains is the invocation itself — the queue client or
  dispatcher calling the tool on the seating path.
