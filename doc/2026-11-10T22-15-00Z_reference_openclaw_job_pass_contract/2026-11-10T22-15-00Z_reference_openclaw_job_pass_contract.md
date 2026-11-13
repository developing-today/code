# The OpenClaw job-pass contract

Issue: [#2171](https://github.com/developing-today/code/issues/2171)
(OpenClaw: finish setup, begin passing jobs, review results
against OpenCode)
Companion: the fixture comparison recorded on #2171 (2026-10-29,
comment 6492773383) stays on the issue until the observability
work ([#2072](https://github.com/developing-today/code/issues/2072))
exists; this document is the durable contract and the
evidence-bounded recommendation the issue's acceptance asks for.
Status: contract v1. This document changes no routing default, no
live OpenClaw configuration, and no gateway state.

## Why a contract

Jobs have been passed to OpenClaw since 2026-10-29, on ad-hoc
packets. The comparison those jobs produced is only as good as the
packets are complete: a result that omits its outcome, its
artifacts, or its cost cannot be reviewed against an OpenCode run,
and an incomplete packet that *looks* complete (a missing token
count recorded as 0) actively corrupts the comparison. This
contract fixes the packet shapes so every future job — on either
executor — produces reviewable evidence.

## Job input contract

A job passed to OpenClaw (or recorded for comparison against one)
is a packet with these fields, all required unless marked
otherwise:

| Field | Content |
|---|---|
| Scope | What the job is to do, stated as a bounded question or change. One job, one scope. |
| Job class | One of: read-only investigation · CI diagnosis · design review · small implementation. The class decides the review burden (§ Recommendation). |
| Repo / worktree | The repository and the exact worktree or snapshot the job runs against. Read-only classes name a commit; implementations name a dedicated worktree and branch. |
| Evidence links | The inputs the job may rely on: issue/PR links, logs, frozen prompts, source snapshots. Anything not linked is out of scope unless the job class says otherwise. |
| Definition of done | The deliverable, in checkable form: the questions answered, the artifact produced, the tests that must pass. "Investigate X" is not a definition of done; "name the failing job, step, and root cause with log lines" is. |
| Authority posture | Read-only, or the exact write surface (branch, files) an implementation job may touch. Default is read-only. |

## Result contract

A result counts as done only when its packet carries every field
below. **A measurement that was not captured is recorded as
`unavailable`, never as zero** — zero is a measurement, and a
false zero poisons every aggregate built on the comparison.

| Field | Content |
|---|---|
| Outcome | `accepted` · `needed rework` · `failed`, judged by the reviewer against the definition of done — not self-reported by the executor. A run that produced no deliverable is `failed` with the failure mode named (e.g. provider stream error), however good its partial reasoning looked. |
| Artifacts | PR, branch, files, or the answer text itself — links, not descriptions. |
| Tokens | Input / output / cache-read as the executing system reports them, with the system named. The two systems count differently today (OpenClaw's input includes its full harness prompt, uncached; OpenCode's rides in cache reads), so token figures compare **cost-relatively within a route**, never as absolute cross-system totals. Unreported is `unavailable`. |
| Wall time | End-to-end wall time; gateway/executor time too when the platform reports it separately (the two diverged by 6× on the first smoke job under laptop load). |
| Review / supervision | Who reviewed, against what ground truth or definition of done, and the verdict per criterion when the job has several. |

## Evidence on record (what the recommendation may use)

All of it is on #2171; summarised here so the recommendation's
bounds are explicit.

- **Smoke job (2026-10-29).** Scheduled-tasks study question via
  the gateway: outcome ok, 13,746 input / 237 output tokens, 11.8s
  gateway (74s wall incl. CLI startup under load). One honest gap
  (fleet peers described from docs instead of enumerated live).
- **Five frozen fixtures (2026-10-29), one paired run each,**
  identical prompt + source snapshot to both executors, all
  read-only: F1 issue triage, F2 CI diagnosis, F3 config audit,
  F4 design review, F5 evidence review. OpenClaw completed 5/5,
  first try, on its primary route (Command Code → DeepSeek V4
  Flash). The OpenCode leg completed 4/5 (F5 failed: provider
  stream error after 374s, no deliverable — a platform failure,
  tokens `unavailable`) after routing around two non-viable
  surfaces. Answer quality was at parity: ties on F1/F2, OpenCode
  deeper on the judgment-heavy F3/F4, OpenClaw cleaner in
  classification discipline and the only completion on F5.
- **Limits of this evidence.** One paired run per fixture — no
  variance estimate exists. The two legs drew **different
  routes**: OpenClaw's five runs all took one primary route,
  while the OpenCode leg drew four different provider doors from
  the workhorse composition across five runs, so "OpenCode" here
  is a pool, not a route. Token accounting differs by system
  (above). Every fixture was read-only and frozen: no production
  design review and **no implementation job of any size has been
  piloted** through OpenClaw.

## Recommendation (evidence-bounded)

**Do not move any job class to OpenClaw by default yet.**

- **Read-only investigation / triage and CI diagnosis:** the
  tested classes (F1, F2, and the smoke job, at parity with 5/5
  completion reliability). Approved as **opt-in pilot classes**:
  jobs may be passed deliberately, with a human reviewer and a
  complete result packet under this contract. Not a default —
  one paired run per fixture cannot carry a default.
- **Design reviews:** F4 gives fixture-level evidence only (a
  strong paired result, one run). Production design reviews are
  untested on OpenClaw. Opt-in with human review; no default.
- **Implementations (any size):** untested — no evidence exists.
  No default, and no pilot expansion until the read-only pilot
  has run under this contract and a round-2 paired pass on stable
  routes exists.
- **Revisit when:** the contract above has been the packet shape
  for a real pilot batch; a round-2 comparison runs both legs on
  named, stable routes (so route variance is controlled); and
  the #2072 exporter makes the token axis measurable across
  systems. Until then, any claim that one executor is cheaper or
  faster than the other exceeds the evidence.

## What this document does not do

- Change any routing default, OpenClaw configuration, or gateway
  state (per #2171's remainder scope; the setup/dispatch items
  #1711, #2012, #2257 and PRs #3275, #5056 are their own tracks
  and are untouched).
- Move the fixture comparison off #2171 — it stays there until
  #2072 exists.
- Authorise volume growth: the issue's ordering (review
  justifies growth) stands.
