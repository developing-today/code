# F12.10 per-account quotas — design for the ownership slice of #1067

Issue: developing-today/code#1067 (F12.10 fair scheduling and
per-account quotas). Slice 2 (PR #12615, design:
`doc/2026-11-22_design_id1067-admission.md`) landed the process-wide
guest-burn budget at the `run_guest_call` funnel: concurrent guest
calls are bounded process-wide, grants prefer the charge key with
the least attained service, and charges settle by actual fuel
against a per-world charge key. It deliberately left per-account
quotas unimplemented behind the world→account ownership decision,
recorded on the issue with options (a)–(d).

That decision is now made (Drewry, 2026-11-22, decision note on
#1067): **(d) calling account by default, with (b) admin registry
and (a) creator principal as per-world options**. Resolution order:
an explicit registry entry (b) wins where present; a world
configured for creator-principal charging (a) charges its recorded
creator; otherwise the calling account (d) pays. (c)
directory-derived is rejected — it can charge the wrong account
silently. This document covers only this slice: ownership
resolution at the charge path, and a per-account bound enforced
alongside the process-wide bound in the same budget. The hub
scheduler and slow-client back-pressure stay unimplemented behind
it, as the issue's criteria list records, and this slice does not
claim the exit benchmark (the ratio gap is attributed in
`pkgs/id/benches/BASELINES.md` to scheduler contention around the
burn path, which a quota bound does not govern).

## Ownership resolution

A new `id-worlds` module, `account_charge`, owns the model:

- **`ChargeMode`** — a world's configured charging mode:
  `Caller` (the default, option (d)) or `Creator` (option (a)).
- **`WorldChargeConfig`** — one world's registry entry: an optional
  explicit `account` (option (b)) and a `mode`. An entry with only
  an account is a pure (b) override; an entry with only
  `mode: "creator"` opts the world into (a) without naming an
  account.
- **`WorldAccountRegistry`** — the (b) store: world id →
  `WorldChargeConfig`, in a JSON file of the shape
  `{ "worlds": { "<world-id>": { "account": "...", "mode": "..." } } }`.
  This is a config-level mapping maintained by an admin — no admin
  UI, no journaling; like `RuntimeLimits` it is runtime
  configuration, installed once by the serve path
  (`WorldAccountRegistry::install_shared`, first install wins) and
  read through `WorldAccountRegistry::shared`. A world absent from
  the registry uses the instance's local config (default: caller
  mode, no explicit account).
- **`resolve_charge_account(world, config, creator, caller)`** —
  the pure resolution function, in decision order:
  1. `config.account` if present — the registry wins;
  2. else, if `config.mode` is `Creator`, the world's recorded
     creator — and if the world carries no recorded creator this
     is an **error**, `ChargeResolutionError::CreatorNotRecorded`,
     never a silent fall-through to the caller (a world explicitly
     configured for creator charging that charged its callers
     instead would be exactly the silent wrong charge (c) was
     rejected for);
  3. else the calling account, where the composition layer has
     installed one;
  4. else the world's own charge key as its account bucket — the
     call carries no caller identity at this layer (see
     "What the funnel can see" below), so its charge is attributed
     to the world itself, visibly, in the budget's per-account
     counters. Nothing is silently charged to a *different*
     account; the bucket is the world's own name.

The resolution inputs are installed on `WorldInstance` by the
composition layer, in the same setter shape as the existing
`set_charge_key` / `set_burn_budget`: `set_charge_config`,
`set_account_registry`, `set_creator_account`,
`set_caller_account`. `world_store::restore_program` — the
production instantiation path — installs the shared registry on
every guest it restores.

## What the funnel can see

The charge path is `WorldInstance::run_guest_call`. At this layer
a call carries no account identity: the actor's `WorldEvent` knows
only a server-assigned participant id, and a participant is an
opaque id plus a display name — membership in the world's
directory, not an account the hub resolved. The session layer does
resolve a caller (`Principal::Account`) per request, but no field
carries it through the actor to the program today. Threading it
is the hub-identity plumbing the slice-3 scoping note on #1067
already identified as its own piece of work; it is **not** in this
slice. What this slice wires is everything downstream of that
threading: the moment a caller principal reaches the instance
(`set_caller_account`), default-mode charges land on the caller;
until then they land on the world's own bucket, and creator-mode
and registry-mode charges — which need no caller — work in full.
No world records a creator yet either (the journal records joins,
not a creator), so a creator-mode world today always takes the
fail-visible path below until a creation record lands; the mode is
opt-in configuration, so no existing world changes behaviour.

## Fail-visible behaviour for creator-less (a) worlds

When resolution fails, the guest call **does not run**: no permit
is taken, nothing is charged, and `run_guest_call` returns the
`CreatorNotRecorded` error (with a `tracing::warn!` naming the
world and the reason), which propagates as the call's error to the
actor exactly like a guest failure. A creator-mode world with no
recorded creator is therefore loudly broken — every call errors
with the reason — instead of quietly charging the wrong account.
The fix is operational and explicit: record the creator, add a
registry entry, or change the mode.

## Per-account enforcement

`GuestBurnBudget` gains the account dimension beside the existing
per-key one, charged at the same two moments (admission and
settle):

- `acquire_charged(key, account, fuel_allowance)` admits against
  **both** bounds: the process-wide `max_concurrent` and, when
  configured, `max_concurrent_per_account`. The existing
  `acquire(key, ...)` delegates with the key as its own account,
  so budgets and callers that predate accounts behave exactly as
  before (and a dedicated budget with no account bound configured
  is unchanged in every case).
- **Queue, never refuse** is preserved: a call whose account is at
  its bound waits; it is never rejected. It also cannot deadlock
  the pool or starve other accounts: grant eligibility is
  per-waiter — a waiter is grantable only when both bounds have
  room for *its* account — so one account's saturated bound queues
  only that account's calls while other accounts' waiters remain
  grantable. Every admitted call is fuel-bounded, so every permit
  returns and every queue drains.
- **Fairness extends per account**: grant order is least attained
  service by *account* first (settled fuel, same one-second
  half-life decay), then by charge key, then arrival. Within one
  account this reduces to the slice-2 order; across accounts, an
  account that has burned heavily queues behind one that has
  burned almost nothing, whichever world the calls belong to.
- **Charges settle by actual fuel** at both levels, as before: a
  settled call folds its measured spend into its key's and its
  account's counters; a permit dropped unsettled charges its full
  allowance at both levels.
- `snapshot()` gains the per-account counters (same shape as the
  per-key ones) and the configured per-account bound, so the quota
  state is observable through the same read as the burn counters.

The per-account bound follows the `RuntimeLimits` pattern:
`RuntimeLimits::guest_burn_per_account_concurrency` (CLI
`--world-guest-burn-per-account-concurrency`), `0` meaning *no
per-account bound* — the default, so an unconfigured hub runs
exactly the slice-2 admission. A nonzero value is applied to the
shared budget by `restore_program`, next to the existing
`guest_burn_concurrency` wiring.

What kind of quota this is, plainly: a bound on **concurrent**
burns per account, the same resource slice 2 bounded process-wide
— it stops one account's worlds from holding the whole burn pool
at once and gives the fairness rule an account axis. It is not a
fuel-per-second rate and not a worlds/storage count; the
`AccountQuotas` vocabulary for those resources is a separate
contract (proposed in PR #12539's line) and stays out of this
slice.

## Exit evidence

Fail-before: at base, the focused suites fail to compile naming
the missing account APIs (`acquire_charged`, the account snapshot
fields, `account_charge`), and the bench record stands as slice
2's: ratio improved at matched load (4.725 → ~3.8) but above 2,
attributed to spawn/shutdown churn and the blocking round trip.

Pass-after: the new unit tests green on the reference machine —
resolution (all three modes, the precedence, the creator-less
error), the budget (per-account isolation: one account at its
bound queues while another is admitted; per-account fair grant
order; per-account settle-by-actual-fuel), and the instance wiring
(a registry-mode guest's burn lands on the registry account; a
creator-mode guest with no creator fails its call visibly and
charges nothing) — with the pre-existing guest_burn / sandbox /
id-caps suites still green. No new dependencies.
