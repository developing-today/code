# F12.10 guest-burn admission — design for the second slice of #1067

Issue: developing-today/code#1067 (F12.10 fair scheduling and
per-account quotas). Slice 1 (PR #12572) profiled the missed exit
benchmark and attributed it: in `fuel_spin_isolation`, 15 spinner
worlds burn synchronously on the shared multi-thread runtime and
occupy every core; the victim world's own service is negligible
(command mean ~55 µs calm), and its spinning-phase p99 is runtime
wake starvation. The parallel placement fix (#12571, guest calls
under `block_in_place`) was measured on the reference machine in the
same PR: the ratio did not move and the victim's own calls got ~20×
slower (spinning command mean 67/103 µs → 1,651/2,191 µs), because
placement cannot create CPU — 15 burns saturate 16 cores from
blocking threads just as well. The recorded next slice is therefore
admission, not placement:

> Bound how many guest calls burn concurrently, charged per
> world/account — which is also the quota hook the per-account
> quota criterion needs. Exit shape: the same bench command's
> profile line shows the spinning ratio below 2 at comparable load
> with the victim's spinning command mean back at its pre-#12571
> level.

This document covers only the admission slice. Per-account quota
enforcement, the hub scheduler, and slow-client back-pressure stay
unimplemented behind it, as the issue's criteria list records.

## Where guest calls execute

Every synchronous guest execution funnels through one function:
`run_guest_call` in `id-worlds`' `sandbox.rs`. The seven
`WorldInstance` methods (`update`, `view`, `layout`, `records`,
`wants`, `snapshot`, `restore`) each wrap their inner call in it;
since #12571 it runs the call under `block_in_place` on a
multi-thread runtime and directly elsewhere. The actor's serial
command loop drives the program through these methods, and so do
restore/replay (`world_store::restore_program`) and the dev/fixture
paths. Admission placed at this funnel covers every guest burn
regardless of driver, and native programs — which have no fuel and
cannot burn — never touch it.

## Mechanism: a shared burn budget with fair grants

A new `id-worlds` module, `guest_burn`, provides `GuestBurnBudget`:
a semaphore-class concurrency budget over synchronous guest calls,
shared process-wide (the hub is the process; `GuestBurnBudget::
shared()` is the default every `WorldInstance` draws from).

- **Bound.** At most `max_concurrent` guest calls execute at once.
  The default sizing is `available_parallelism − 2` (floor 1): one
  core is reserved for the async runtime and actors, one for the
  driver, journal, and OS. On the 16-thread reference machine the
  bound is 14, so even with every permit held by burns, the
  victim's driver and actor always have a core to wake on — the
  resource whose absence slice 1 measured.
- **Queue, never refuse.** A call that arrives past the bound waits
  for a permit; it is never refused. The codebase's refusal
  vocabulary (capability, ceiling, and bound refusals in the actor's
  `execute`) rejects *requests a world is not allowed to make*; a
  guest call is an execution the world is already entitled to, and
  refusing it under contention would reject the victim's inputs
  precisely because *other* worlds are burning — the opposite of
  the fairness criterion, and a semantics change to every program
  call. Queueing is the host-side back-pressure this criterion
  calls for, and it cannot deadlock: every admitted call is
  fuel-bounded, so every permit is released within one fuel budget
  of work, and the queue always drains.
- **Fair grant order.** Permits are not granted FIFO. When a permit
  frees, it goes to the waiting call whose charge key has the least
  *attained service* — settled fuel, exponentially decayed with a
  one-second half-life — with arrival order breaking ties. A world
  that has just burned 20M fuel goes behind a world that has burned
  almost nothing, so the victim's wait is bounded by roughly one
  permit turnover (bound ÷ burn duration), not by a rotation
  through every spinner. A spinner's own throughput degrades to
  its fair share of the burn pool instead of the whole machine.
- **Where the wait happens.** Admission is acquired inside
  `run_guest_call`, after the `block_in_place` conversion on a
  multi-thread runtime, so a waiting call parks a blocking thread —
  never a shared runtime worker (the #557 rule). On plain threads
  and current-thread runtimes the call path is synchronous already
  and no other task on that thread can hold a permit across an
  await (permits are never held across awaits), so the wait can
  only be on another thread's fuel-bounded call.

## The charge, and the quota hook

Each call is charged to a **charge key** at admission and settled
with its *actual* spent fuel afterwards — the figure
`WorldInstance::settle` already measures for `CallMetrics`. The
budget keeps per-key counters (`admitted`, `settled_fuel`,
`waits`, current and high-water in-flight) behind a `snapshot()`
read, which is the same counter the per-account quota criterion
will hang off: when the ownership question recorded on the issue
(creator principal, admin registry, or caller account) is decided,
keys are aggregated or re-keyed by account without touching the
admission path.

The charge key is the guest's identity as the composition layer
knows it:

- `world_store::restore_program` — the production path that
  instantiates a world's guest — sets the key to the `world_id`
  its caller passes, so a live world's charge accumulates under
  its world identity across restarts.
- Everywhere else (dev, fixture, audit, bench worlds) the key
  defaults to a unique per-instance label. A live world has
  exactly one live instance in its actor, so per-instance and
  per-world attribution coincide while a world is live; the bench
  labels its instances with the world names it already uses, so
  the spinner *workers'* charges persist across their per-spin
  re-instantiations.

A permit that is dropped without settling (a panic path) charges
the call's full fuel allowance instead of a measured spend: when
the spend is unknown, the conservative figure is the worst case
the admission decision priced in.

## Configuration

The bound follows the `RuntimeLimits` pattern (runtime
configuration, never journaled, validated positive at parse):

- `RuntimeLimits` gains `guest_burn_concurrency: usize`, `0`
  meaning *auto* (the `available_parallelism − 2` default). The
  serve path configures the shared budget from it once at startup,
  before any world opens; a later configuration attempt is a no-op
  (first configuration wins, like the budget itself: one hub, one
  budget).
- `WorldInstance::set_burn_budget` lets a composition root or a
  test substitute a dedicated budget for one guest, bypassing the
  shared default. Tests use this to run small budgets without
  perturbing the process-wide one other tests share.

Native (`world_native`) workers are out of scope: they are
separate processes under `RLIMIT_CPU` and a per-call deadline, not
synchronous burns on the hub runtime. The many-worlds budget of
#557 (per-world memory) is likewise a different resource; this
slice bounds only concurrent guest CPU.

## Exit evidence

Fail-before is slice 1's merged record (BASELINES.md): reference
ratio 2.517 (2026-11-10) and, on current main, light-load pairs at
ratio 6.049/6.711 (pre-#12571) and 6.826/6.085 (post-#12571) with
the victim's spinning command mean at 1,651/2,191 µs post-#12571.

Pass-after is the same command on the reference machine —
`cargo bench --bench world_bench -- --only fuel_spin_isolation` —
showing ratio below 2 at comparable load with the victim's
spinning command mean back at its pre-#12571 level (tens of µs),
plus unit tests for the budget itself: the bound is never
exceeded, charges are attributed per key and settled with actual
spend (allowance when unsettled), grant order prefers the
least-attained key over an earlier-arriving burn-heavy key, and
two guests sharing a budget of one serialize their burns.
