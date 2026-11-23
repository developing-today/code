# Controlled pool test — run record

Copy this template per run. One filled copy per run, at
`~/workspace/goals/keep-the-four-repos-healthy/hidden_files/controlled-tests/<flag>-<YYYY-MM-DD>.md`.
A record missing any per-arm measurement cannot support a verdict
(see `doc/agents/controlled-pool-tests.md`).

## Test

- Flag issue: #
- Candidate route:
- Incumbent route:
- Date / window:
- Tasks per arm: (3 = directional read, 6 = config-change bar)

## Headroom evidence

- Quota read at (≤60 min before dispatch):
- Pools under test and headroom:
- Authorizing note: Drew 2026-11-22 — tests run when headroom allows.

## Arms

- Candidate arm tasks (issue numbers):
- Incumbent arm tasks (issue numbers):
- Route pin: `fleet agent next --pool <arm-route>` per arm.
- Arm identity: claim stamps (`claim_provider` / `claim_model` /
  `claim_variant`) plus `route_usage.lastServing` — never the
  `pool` field.

## Measurements (per arm)

| Arm | Completion | Wall-clock | Tokens (in+out, context-inclusive) | Stalls/nudges | Rework within 48h |
| --- | --- | --- | --- | --- | --- |
| Candidate | | | | | |
| Incumbent | | | | | |

## Verdict

- Verdict: (pass / fail / inconclusive — 3/arm cannot land a change)
- Numbers attached:
- Recorded on: flag issue #___; interpretive model record (pool-intelligence job).
- Follow-up: (steward PR link, or flag retired with numbers)
