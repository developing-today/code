# The central compile coordinator

<!-- id: compile-coordinator since: pre-release -->

Placement today is a per-seat decision: `fleet queue dispatch` runs
`decide()` in `crates/fleet-queue/src/placement.rs`, reads the load
its own seat can see, picks the least-loaded surface, and dispatches.
The central compile coordinator (#13765) is the designed replacement
for the decision's _admission_ half: a coordinator living in the
queue broker that claims slots atomically, holds one true load view,
enforces budgets globally, and queues jobs fairly across seats — with
the existing singular per-seat dispatch kept, by contract, as the
fallback whenever the coordinator does not answer.

This chapter is written from the fleet side: what the coordinator
coordinates, where its state lives, and how the fallback works.
[Placement and remote dispatch](placement.md) covers the engine the
coordinator sits under; it stays accurate for the singular path,
which is still the only path that runs.

**Designed versus built, as of 2026-11-23 — stated once, plainly:**
the design is complete and of record on #13765 (Drew's order,
2026-11-23 15:09 CST); the build has not landed. There is no
`placement_coord` module and no coordinator tables in the code on
main (grep-verified for this chapter, 2026-11-23). The build was
routed as queue task 21517 to a laptop OpenCode session after the
first build attempt stopped on a platform execution-risk flag, and
task 21517 was then **parked** — its record states the four-slice
build was too large for one unattended run, that no code changed, and
that the next step is a dedicated slice-1 task (backend verbs, DDL,
tests 1–3). The one piece of the design already in the shipped code
is the client-side claim attempt inside Actions exec (below). Every
statement in this chapter about coordinator behaviour is a statement
about the design, dated here, not about running software.

## The four reasons

The design answers four defects of per-seat placement, each with a
named proof in the issue's proof bar:

1. **Atomic slots.** Read-then-dispatch is racy: two seats see the
   same least-loaded pool and both pile in. The observed case in the
   design record: 62 active org jobs against a cap of 20 — caps
   steer, they do not gate. The coordinator claims slots atomically.
   _Proof:_ two simultaneous claims for the last slot — exactly one
   wins.
2. **One true load view.** Load is read per runner-host repo with the
   seat's own tokens; a repo the seat has no token for counts as 0
   running. (#13611 made the decision logic treat unread load as
   unknown, but the seat's view is still only its own.) The
   coordinator holds the true cross-owner load view, and admission
   uses it. _Proof:_ a seat with zero coverage for an owner is
   refused or queued when the coordinator's view shows the owner
   full, admitted when it shows room, and `/placement-load` returns
   the true counts to any seat.
3. **Global budgets.** The Blacksmith budget in the surface registry
   (`fleet-org-blacksmith`: `budget.free_minutes_monthly = 3000`,
   `paid_cap_minutes = 0`) is settled today from each seat's local
   store — a per-seat honour system. The coordinator enforces it
   fleet-wide and atomically: reserve at claim, settle at completion.
   _Proof:_ reservations past the fleet-wide free-minute budget are
   refused; settle converts a reservation to actual use.
4. **Cross-seat visibility and queueing.** Sessions become visible to
   every seat natively, and when all pools saturate, jobs wait in a
   real queue with fairness — FIFO per owner — instead of every seat
   dispatching into the pile. _Proof:_ jobs queue, drain in fair
   order as slots free, and a claim or session made on one seat
   resolves from another.

Plus the fallback proof: coordinator down — dispatch still succeeds
via singular handling, and the output reports the fallback.

## What it coordinates: the three tables

The coordinator's home is the queue broker's store — the
`fleet-queue` Turso database, already the fleet's coordination point,
where `placement_sessions` (#13146) already lives as a best-effort
visibility mirror. The design adds three tables there, named in the
#13844 Turso audit as "designed to be added to" that store:

| Table                   | What it holds                                                                                                                                                                                                                                                 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `placement_owner_state` | The coordinator's per-owner state: the true load view rows and the active-claim counters admission checks against the owner's cap (caps from the surface registry: developing-today 20 concurrent jobs, developing-today-forks 20, dezren39 40, macOS 5 each) |
| `placement_claims`      | One row per claim: the atomic admission record for a session's slot on a surface, from grant through confirm to release, including waitlisted claims waiting for capacity                                                                                     |
| `placement_budgets`     | The global budget ledger: reservations taken at claim time and settled to actual use at completion, per budget-bearing surface                                                                                                                                |

Admission is the house atomicity pattern — the single conditional
UPDATE whose affected-row count decides the winner, the same pattern
as task claim in `turso_queue/claim.rs` — applied to the owner
counter in `placement_owner_state`, with the Blacksmith budget
reservation taken in the same flow.

## The verbs and the sweep

The backend verbs are designed as a new module,
`turso_queue/placement_coord.rs`, routed in `turso_queue::dispatch`
beside the Actions exec verbs:

| Verb                      | Job                                                                                                                                                           |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `/placement-claim`        | Atomic admission: conditional owner-counter UPDATE against the cap, using the coordinator's load rows plus active claims; budget reservation in the same flow |
| `/placement-confirm`      | Confirm a granted claim with its dispatched run id                                                                                                            |
| `/placement-release`      | Release a claim, with budget settle                                                                                                                           |
| `/placement-claim-status` | Read a claim's state — including from a different seat than the one that made it                                                                              |
| `/placement-load`         | The true load view, readable by any seat                                                                                                                      |

Behind the verbs, a coordinator sweep — a background thread in the
queue broker, on the #12247 startup-thread pattern — refreshes the
true load view from GitHub on an interval (broker-side reads, the
same host-repo counts `build_ctx` makes, from the seat with full
token coverage), expires stale claims and grants, settles confirmed
claims whose runs finished, recomputes the active-claim counters from
the claims table (self-healing), and grants the waitlist fairly as
capacity appears.

The client contract leaves the command surface unchanged. `fleet
queue dispatch` asks the coordinator for a claim on the decided
surface before dispatching, in a bounded roundtrip with the same
discipline as the #13146 mirror. Granted: dispatch as today, then
confirm with the run id. Saturated: the session is committed in state
`queued` with its claim id and full job record — the shape the
all-tiers-exhausted path already writes — and `fleet queue session
status` completes the dispatch when the claim is granted.

## The fallback: singular per-seat dispatch

The fallback is a contract, not an accident. When the coordinator is
unreachable or times out, the command proceeds with today's singular
per-seat dispatch and **says so in its output**: `placement_path`
names `coordinator` or `singular-fallback`, per the estate's
route-visibility doctrine. A down coordinator never hard-fails a
build. Scope is the compile/dispatch path only; the agent task queue
is untouched, and the two coordinate through the shared broker store.

One consumer already runs this contract. Actions exec
([Temporary builds and custom connections](temporary-builds.md))
attempts the coordinator claim first, in
`pkgs/fleet/crates/fleet-queue/src/client/actions_exec.rs`:
`ae_placement_claim` posts to `/placement-claim` and returns granted
only when a coordinator actually answers; every other outcome — the
verbs not live, an error, an `ok:false` — proceeds on the singular
path, named via `placement_path`. Until the coordinator lands, that
is the path every Actions exec session takes.

## Dependency map and working order

Prioritised, in the order the build itself must go:

1. **Slice 1 — backend verbs, DDL, proofs 1–3.** The
   `placement_coord` module, the three tables, and the concurrency,
   load-view and budget tests. This is the dedicated task the parked
   task 21517 names as its next step.
2. **Slice 2 — the coordinator sweep.** Load refresh, claim expiry,
   settle, counter recompute, fair waitlist grants.
3. **Slice 3 — client integration.** The claim attempt inside
   `fleet queue dispatch`, the `queued`-session completion through
   `fleet queue session status`, and `placement_path` in the output.
4. **Slice 4 — the saturation and fallback proofs.** Queueing drains
   FIFO per owner; coordinator down dispatches singular and reports
   it.

Dependencies: the broker store and its conditional-UPDATE pattern
(landed, #13146); the surface registry's caps and budgets (landed);
broker-side GitHub reads (landed, #8276's dep probe is the precedent).
Nothing in the coordinator blocks Actions exec, which composes with
the verbs when they answer and proceeds singular when they do not.
A design-review note stands on #13765 (AUDIT-7, #13876,
2026-11-24T00:11:05Z): the design is strong, and one record gap is
named — the design of record lives only in the issue body, where
comparable programmes keep a dated record under `doc/`.

## Sources

Issue #13765 (Drew's order, premise, architecture, proof bar; the
COMPILE-COORD flag stop, the task-21517 routing and parking records,
the AUDIT-7 review); the #13844 Turso audit's coordinator-table
lines; `pkgs/fleet/crates/fleet-queue/src/placement.rs`;
`pkgs/fleet/crates/fleet-queue/src/client/actions_exec.rs`;
[Placement and remote dispatch](placement.md). State verified against
main on 2026-11-23: no `placement_coord` module, no
`placement_owner_state` / `placement_claims` / `placement_budgets`
DDL in code — the tables appear only in the audit doc as designed.
