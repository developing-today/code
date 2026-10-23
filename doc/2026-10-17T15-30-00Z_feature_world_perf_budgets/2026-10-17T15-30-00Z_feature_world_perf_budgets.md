# Per-call resource metrics and performance budgets

See the [Phase 11 foundations plan](../../.opencode/plans/phase-11-foundations.md) and the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md).

## Intent

Fuel is the platform's main performance ceiling for guests: every guest call
runs on a fresh `SandboxLimits::fuel` (10M by default) and traps, poisoning the
world, when it runs out. Nobody could see how close a guest was to that line
without hand instrumentation. Territory's styled view uses about 7.1M to 7.8M
of the 10M, and that was found by hand. This slice makes the cost

1. **measurable** from the host (`WorldInstance::metrics`),
2. **visible to authors** offline (`id world measure`), and
3. **guarded** against regressions in CI (`checked_in_examples_stay_within_fuel_budgets`).

## What is measured

`WorldInstance` records the fuel each guest call spent, in a fixed array keyed
by `GuestCall { Init, Update, View, Layout, Records, Wants, Snapshot, Restore }`.
Each `CallStats` holds `calls`, `fuel_total`, `fuel_max`, and `fuel_last`.

- Every call already resets fuel with `set_fuel(limits.fuel)` first, so fuel
  spent is `limits.fuel - get_fuel()` afterwards. It is read once the whole
  call is done, so it includes the input allocation, the guest `free`, and the
  `plaza_out_len` read around the main export.
- A trapped call is recorded too. The diagnostic getters (`plaza_error_ptr`,
  `plaza_error_len`) refill the store to 10_000, so `guest_error_suffix`
  captures the trap's fuel first. A call that ran out of fuel leaves the store
  at zero and so records the full limit.
- **Init** covers the module start function plus `plaza_init`; both run on the
  one budget set at instantiation. It is recorded when the instance is built.
- **Layout** is the `plaza_layout` call. A guest with no layout export makes
  `layout_at` fall back to `view`, which is then counted as **View**.
- Calls refused before reaching the guest (a poisoned world, an oversized
  message, an absent optional export) are not recorded.
- The trait-level `snapshot` verifies the snapshot on a probe instance. The
  live instance records its own `snapshot` and `records` calls; the probe's
  calls are not added to it.
- Metrics live in memory on the instance. They are never journaled, never on
  the wire, and never part of a snapshot, so they cannot affect replay or
  determinism. Recording is a few integer updates: no lock, no allocation.

## `id world measure`

```
id world measure --module FILE.wasm [--seed N] [--input HEX]... [--width W --height H]
```

Offline and gated on the `sandbox` feature. It compiles the module with the
default limits and instantiates it. It then runs `view`, `layout_at(W, H)` if
both `--width` and `--height` are given, `records`, `wants`, and `snapshot`.
Then each `--input` runs as participant 1 through `update`, with a `view` after
each. It prints one JSON object: `module_bytes`, `fuel_limit`, `calls` (per
kind: `calls`, `fuel_total`, `fuel_max`), `worst_call`, and
`worst_call_percent_of_limit`. Kinds that never ran are omitted. It exits
nonzero only for a compile failure, a malformed `--input`, or a trap.

The reads run before the inputs, so they show the cost at the initial state.
Use `--input` to see how update cost and later views grow with state.

## How budgets are set

`FUEL_BUDGETS` in `src/sandbox.rs` has one row per (example, call kind). The
test drives each example through a fixed script (`drive_*` in the test
module), then runs `view`, `layout_at(40, 20)`, `records`, `wants`, and
`snapshot` at the final state, and restores the snapshot on a fresh instance.
The observed value is the worst single call of that kind. A budget is the
measured baseline plus 25 percent, rounded up to the next 100_000. The test also
requires every example module to be at most 1 MiB, and fails on a call that
has no budget row, so a new export cannot slip in unguarded.

On failure the message names the example and call, the actual fuel, and the
budget, then says: _re-baseline deliberately: update the table and say why in
the commit_.

### Baseline (fuel)

| Example     |  init | update |      view |    layout | records |   wants | snapshot | restore |
| ----------- | ----: | -----: | --------: | --------: | ------: | ------: | -------: | ------: |
| roc-counter | 1,215 |  3,759 |     1,327 |     1,337 |   1,866 |     733 |      874 |   3,238 |
| tic-tac-toe | 1,242 |  7,839 |     5,329 |     6,265 |   7,347 |   1,664 |      769 |  14,204 |
| roc-screen  | 1,215 | 11,491 |   257,078 | 1,192,583 |   2,795 |     733 |      870 |   3,243 |
| roc-grid    | 1,242 |  5,035 |   981,969 |   981,300 |   1,664 |     737 |    1,143 |   5,688 |
| roc-chat    | 1,271 | 89,763 |     5,997 |     6,949 |  23,438 |  42,766 |    4,767 |   7,884 |
| lounge      | 1,271 | 69,255 |     5,550 |     6,502 |  71,278 | 194,873 |   70,261 |  33,299 |
| territory   | 1,255 | 64,520 | 7,798,929 | 7,798,260 |  56,536 |   2,444 |    7,794 |  21,745 |

Fuel counts Wasm instructions, so these numbers do not depend on the host or
on a debug or release build. They are exact for these module bytes and scripts.

### Headroom against the 10M default

Each example's worst call as a percent of the limit:

| Example       | Worst call |          Fuel | % of 10M |
| ------------- | ---------- | ------------: | -------: |
| roc-counter   | update     |         3,759 |     0.04 |
| tic-tac-toe   | restore    |        14,204 |     0.14 |
| roc-screen    | layout     |     1,192,583 |    11.93 |
| roc-grid      | view       |       981,969 |     9.82 |
| roc-chat      | update     |        89,763 |     0.90 |
| lounge        | wants      |       194,873 |     1.95 |
| **territory** | **view**   | **7,798,929** | **78.0** |

**Above 50 percent: territory only.** An empty territory board costs 6.54M for
a view (65 percent); the script's two seats and a few claimed cells take it to
7.80M. A hand run with one seat and one cell gave 7.1M (71 percent).

**Territory's view exceeds the default limit at six or more seats**, which its
own `max_seats = 8` allows. Measured in a scratch test with every seat joined
and active, view fuel was:

| Seats     |     1 |     2 |     4 |           6 |           8 |
| --------- | ----: | ----: | ----: | ----------: | ----------: |
| View fuel | 7.33M | 8.10M | 9.74M | out of fuel | out of fuel |

Each seat adds about 0.8M, and claimed cells add little (16 cells per seat
moved the 2-seat view by 0.03M). A view that runs out of fuel traps and poisons
the world, so a full territory world is broken under the default profile today.
This slice records the problem; it does not fix it (non-goals below). The
`FUEL_BUDGETS` row guards the two-seat baseline, so it will not catch this by
itself.

## How to re-baseline

Re-baseline when a guest or the platform legitimately changes cost:

1. Rebuild the example module as usual.
2. Run `cargo test --lib -- --ignored print_fuel_baseline --nocapture` from
   `pkgs/id`. It prints one `FUEL_BUDGETS` row per call kind, with the new
   budget and the measured baseline in a comment.
3. Paste the rows over `FUEL_BUDGETS` and update the baseline table above.
4. Say why in the commit message. A rise without a reason is the regression the
   test exists to catch.

## Limits

- Budgets are 25 percent over a scripted state, not a worst case. Cost grows
  with state, as territory shows, so a pass does not mean a world cannot run
  out of fuel.
- Rounding up to 100_000 makes budgets for tiny calls loose: any call under
  80,000 fuel has a 100,000 budget, and 100,000 is 1 percent of the limit. They
  guard against order-of-magnitude regressions only.
- A budget near the limit guards little. A budget cannot exceed what the guest
  can spend, which is the limit, and territory's 9.8M budget sits close to it.
- Metrics are per instance and in memory. A restarted or replaced world starts
  from zero, and nothing aggregates across worlds.
- Only guest fuel is measured. Host-side time (JSON parsing, compression, the
  network) and guest memory are not.

## Non-goals

No wire change and no `stats` frame change; no host-side metrics endpoint; no
change to the default limits; no optimisation of the guests.

## Follow-ups

- **Host-side metrics endpoint.** Expose `WorldInstance::metrics` through the
  world host (and `id world stats`) so a live world's headroom is visible, next
  to the execution profile it already prints.
- **Fix territory's fuel use.** The view costs about 6.5M on an empty board
  and 0.8M per seat. Options: a cheaper render path, or a higher world
  profile for it. Either needs its own decision.
- **Look at the shared screen code.** The two grid-like views cost heavily for
  what they draw: roc-grid's view is 0.98M and roc-screen's layout is 1.19M.
  Hypothesis, unmeasured: the cost is mostly in the shared `Screen` rendering
  rather than in each app. Measure with `id world measure` before changing it.
- **Crowded-state budgets.** Add a second territory script with all eight
  seats once the view fits, so the budget covers the worst case.
- **Warn on high usage.** Have `id world install` or `compile` run the
  measurement and warn when a call uses more than half the profile's fuel.
- **Per-world budgets.** Compare against a world's own execution profile
  rather than the 10M default.

## References

- [Phase 11 foundations plan](../../.opencode/plans/phase-11-foundations.md)
- [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md)
- `src/sandbox.rs`: `GuestCall`, `CallStats`, `CallMetrics`, `FUEL_BUDGETS`, `print_fuel_baseline`
- `src/commands/world.rs`: `measure_module`; `src/cli.rs`: `WorldCommand::Measure`
