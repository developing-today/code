# Controlled pool tests (#1342)

A pool change proposed by a pool-intelligence flag or a steward hunch
does not land on vibes — it runs a controlled test first. This is the
standing mechanism: a small batch of comparable real tasks run on the
candidate configuration vs the incumbent, measured per arm, with a
verdict recorded on the flag's issue.

Child of the pool & routing intelligence epic (#1334). This doc plus
the [results template](controlled-pool-test-template.md) is #1342
criterion 1 (procedure + template exist); the first executed test is
criterion 2, and folding in #1191's trial results is criterion 3.

## Authorization gate

Controlled A/B pool tests run only when quota headroom allows. Drew,
2026-11-22 ~20:27 CST (recorded on #1342): **yes, when headroom
allows.**

- Headroom is the measurement gate from #1339: a quota picture
  ≤60 min old per pool under test is the license for any decision.
- Pools burn to 100% at reset; the test never spends funds, never
  pays PAYG, never changes a credential, never uses a pool Drew
  hasn't approved. Astra stays gated.
- Record the headroom evidence (fresh quota read, timestamp, pools)
  on the run record **before** dispatching either arm. A run with no
  headroom evidence is not a controlled test.

## Batch design

- A test is a small batch of comparable real tasks — same kind and
  effort class, drawn from the live queues — run on the candidate
  configuration vs the incumbent, in the same window where possible.
- Minimum viable batch: **3 tasks per arm** for a directional read,
  **6 per arm** for a config-change recommendation.
- The candidate comes from the first pool-intelligence flag or the
  #1191 routing trial's leftovers. Candidate on record: #1409 (add
  Kimi K3.1 Pro via opencode-go to summit) — no arms dispatched yet;
  its Go short window is exhausted, so the first test re-picks its
  window from current headroom rather than re-running the expired
  one.

## Route pinning

Pin by **pool route**, not by model pin. The routing policy
(`.opencode/instructions/routing-policy.md`) prefers managed pool
routes over model pins; workhorse is the default route; there is no
in-session model switch — escalation is a fresh launch on a higher
route with the reason recorded.

- Claim each arm with `fleet agent next --pool <arm-route>` — one
  arm on the candidate route, one on the incumbent.
- What the pin leaves behind: the claim stamps
  `claim_provider` / `claim_model` / `claim_variant` /
  `claim_session_id` on the task row, written by `stamp_claim_model`
  (`pkgs/fleet/crates/fleet-queue/src/turso_queue/claim.rs:652`)
  with a `model_stamp` event. `parse_claim_model` (same file:546)
  requires provider and model together — a claim carrying no model
  leaves the columns untouched, so an arm with no stamp is an
  unpinned arm, not a third arm.
- Arm identity at read time comes from `derive_completion_telemetry`
  (`pkgs/fleet/crates/fleet-queue/src/client/support.rs:1329`),
  which prefers `route_usage.lastServing` (providerID + modelID +
  variant, the #1891 shape) when the session record carries it.
  That readout landed on session records via the opencode fork
  (PR #2754, merged; noted on #1340, 2026-11-23).
- **Never identify an arm by the `pool` field.** The claim event's
  `pool` is the queue name (e.g. `repo-health-code-pilot`), not a
  POOL MAP pool, and completion `pool` is the session's declared
  route — in the 2026-11-22→23 production window every non-null
  value was the single string `workhorse`, so the field carries no
  per-pool spread (#1340 re-grade, 2026-11-23). Arm identity is the
  claim stamp plus `route_usage.lastServing`, nothing else.

## Measurements

Measured per arm, from the queue DB / completion telemetry:

- Completion (done vs stalled / failed / handed off).
- Wall-clock per task.
- Tokens: context-inclusive input + output.
- Stalls / nudges.
- Rework within 48h: follow-up fixes on the same issues.

A run record missing any of these per arm is incomplete — it cannot
support a verdict.

## Verdict

- Verdicts are recorded on the flag's issue and in the interpretive
  model record maintained by the pool-intelligence job (#1341).
- A passed test hands the steward a PR-ready change. Config changes
  land ONLY via PR to the canonical repo config — never a hand edit.
- A failed test retires the flag with its numbers attached.
- No pool config change happens without a passed test at the 6/arm
  bar. A 3/arm directional read can retire a flag or justify a
  bigger batch; it cannot land a change.

## Records

- One filled copy of the results template per run, at
  `~/workspace/goals/keep-the-four-repos-healthy/hidden_files/controlled-tests/<flag>-<YYYY-MM-DD>.md`
  (the goal workspace on the Muse host that runs the pool jobs).
- #1191's trial results are folded in as the mechanism's first
  historical dataset rather than re-run (#1342 criterion 3).

## Maintenance

This doc, the template, and the reference check
(`.github/scripts/check-controlled-pool-tests.sh`, run in
`workflow-lint.yml`) move together: the check fails if a
load-bearing code reference in Route pinning moves or a required
template field goes missing, so the procedure cannot silently go
stale against the queue code.
