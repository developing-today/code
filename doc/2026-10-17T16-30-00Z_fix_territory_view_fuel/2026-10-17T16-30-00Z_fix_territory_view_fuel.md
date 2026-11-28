# Fix: territory's view ran out of fuel with six or more seats

See the [Phase 11 foundations plan](../../.opencode/plans/phase-11-foundations.md) and the [perf budgets record](../2026-10-17T15-30-00Z_feature_world_perf_budgets/2026-10-17T15-30-00Z_feature_world_perf_budgets.md), which measured the problem and left it open.

## Intent

`examples/territory` allows eight seats. Its styled view cost about 6.5M fuel on
an empty board and about 0.8M more per seat, so with six or more seats it
exhausted the 10M default limit and trapped. A trapped view poisons the world.
The earlier claim that territory fits with all eight seats was wrong. This
change makes the full house fit with room to spare, makes the shared `Screen`
code cheaper for every app, and guards the full house in `FUEL_BUDGETS`.

## Reproduction

`territory_view_fits_the_default_fuel_with_every_seat_taken` joins eight players
by presence events, has each act so each gets a seat, and claims 14 cells per
seat (112 of 128 cells). Before the fix:

```
a full territory view must not trap: guest `plaza_view` failed (trap or fuel exhausted)
  ... territory.wasm!roc__proc_15fb
  ... territory.wasm!host.plaza_view
```

`print_territory_view_fuel_by_seats` (ignored; `TERRITORY_WASM=FILE` measures a
variant build) prints the view fuel by seat count.

## Where the fuel went

I built variants of territory's `view` that stop after each stage and measured
each at 0, 2, and 8 seats. Two facts decide everything:

1. **The checked-in module was built by an older Roc.** The repository pins Roc
   `nightly-2026-10-04-130536d` (`nix/roc.nix`, the example READMEs), but the
   checked-in `territory.wasm` matched the output of the `nightly-2026-08-10`
   compiler exactly (6,544,819 fuel on the empty board, to the instruction).
   The pinned compiler builds the **same, unchanged source** into a module that
   costs 2.34M on the empty board and 3.24M with eight full seats.
2. **The remaining cost is in `Screen`, in three places.**

Fuel by stage, view only, in thousands (stale compiler / pinned compiler; the
unchanged territory source and the original `Screen.roc`):

| Stage added (cumulative)                                           |          Empty, stale |           Empty, pinned | 8 seats, pinned |
| ------------------------------------------------------------------ | --------------------: | ----------------------: | --------------: |
| `Screen.new`, `boxed`, `render_json` of a constant picture         |          not measured |                       1 |               1 |
| build 128 `{ ch, fg, bg, bold }` records, `put_row` x8, plain text |                 4,017 |                     540 |             574 |
| `render_json` instead of `render_plain` (quote + style spans)      |                 5,324 |                   1,976 |           2,208 |
| status line (`put_text`, one list update per character)            |                 6,545 |                   2,343 |           2,324 |
| scoreboard, one `put_text` per seat                                | 6,545 + 0.8M per seat | 2,343 + ~0.11M per seat |           3,239 |

Reading the table:

- **Per-character list updates were the dominant cost under the stale
  compiler.** `put_text` and `put_row` update `cells` with `List.set` while the
  accumulator still holds the list, so each update copied the 456-cell list
  (3.5M for the 128 grid cells alone, about 50k for each scoreboard
  character). The pinned compiler updates in place, which is why rebuilding
  alone removes about 4.2M.
- **Building the per-cell records** costs about 4.1k per cell (526k for 128):
  `bytes_to_str([b])` allocates a list and validates it for every cell.
- **`render_json` costs about 1.4M for a 24x19 screen**, mostly `json_quote`,
  which folds every byte into a list through a second per-byte fold, and
  `row_styles_json`, which builds an indexed record list per row.
- **Each seat adds about 0.11M with the pinned compiler**: roughly 30k for the
  per-cell `cursor_at` scan of the seat list and the rest for the scoreboard
  line (`put_text`, 15 to 29 characters at about 5k each).

## The fix

Three changes, in order of reach:

1. **Rebuild with the pinned Roc.** The checked-in modules for every example
   that imports `Screen` (`territory`, `roc-screen`, `roc-grid`) are rebuilt
   with the pinned nightly, as the READMEs say to. Not a source change; worth
   about 4.2M for territory.
2. **`Screen.json_quote` skips its per-byte fold for text that needs no
   escaping** (the common case). This helps every app that calls
   `render_json`: with the original `Screen.roc` and the same compiler,
   roc-screen's view went 252k to 124k, its layout 560k to 378k, and roc-grid's
   view 696k to 391k. Output is byte for byte the same.
3. **`Screen.render_rows_json` and `Screen.box_edge`** build the screen
   document from whole rows: each row is a list of `{ text, style }` pieces,
   padded or cut to the width. No per-cell record list exists. Equal adjacent
   styles merge and the default style writes no span, so the document equals
   what `render_json` writes for the same picture. Territory uses it: the
   boxed border is two `box_edge` strings, each grid row is `|`, a few style
   runs, `|`, and each scoreboard line is one piece. The existing
   `Screen.new`/`put_text`/`put_row`/`render_json` API is untouched.

Territory's picture did not change. The boxed grid, per-seat colours, inverted
cursor, free count, and one scoreboard line per seat remain, and I compared the
view JSON of the old source (pinned compiler) and the new source byte for byte
across six states (0 to 8 seats, 0 to 14 cells per seat): identical.
`territory_scoreboard_lines_are_cut_at_the_screen_width` pins the rows for a
seat with a long name, which the screen clips to 24 columns.

## Results

View fuel, default limit 10,000,000:

| Territory view    | Before (checked in) | Pinned compiler, old source | After (this change) |
| ----------------- | ------------------: | --------------------------: | ------------------: |
| empty board       |           6,544,819 |                   2,343,322 |             964,367 |
| 1 seat, 1 cell    |           7,112,842 |                   2,438,218 |           1,034,195 |
| 2 seats, 14 cells |           7,798,938 |                   2,572,204 |           1,132,099 |
| 4 seats, 14 cells |           9,069,083 |                   2,793,480 |           1,293,209 |
| 8 seats, 1 cell   |          trap (10M) |                   3,071,268 |           1,468,278 |
| 8 seats, 14 cells |          trap (10M) |                   3,239,531 |           1,611,425 |

The full-house view is **16 percent** of the limit (target: at most 40
percent), and the empty board is 9.6 percent.

Shared `Screen` apps (worst view / layout, scripted state):

| Example    | View before | View after | Layout before | Layout after |
| ---------- | ----------: | ---------: | ------------: | -----------: |
| roc-screen |     257,078 |    123,675 |     1,192,583 |      378,384 |
| roc-grid   |     981,969 |    391,196 |       981,300 |      389,763 |

Their drop is partly the compiler (original `Screen.roc`, pinned compiler:
252k / 560k and 696k / 695k) and partly `json_quote`.

## Guarding it

- `territory_view_fits_the_default_fuel_with_every_seat_taken` fails if the
  full-house view traps or uses more than 40 percent of the limit.
- `checked_in_examples_stay_within_fuel_budgets` gets a `territory-full`
  entry (`drive_territory_full`: eight seats, 14 cells each) alongside the
  two-seat `territory` entry. Same rule: baseline plus 25 percent, rounded up
  to 100,000. Its view budget is 2.1M, its worst update 1.1M (one 29-key
  input), records 0.6M.
- Re-baselined rows, and why: `territory` (stale to pinned compiler plus the
  new view), `roc-screen` and `roc-grid` (pinned compiler plus `json_quote`).
  Small calls (init, wants, snapshot, restore) moved by a few hundred fuel with
  the compiler and keep their 100,000 budget.

## Compiler hazard found on the way

Using a `Str` again after passing it to `Str.to_utf8` corrupts memory in the
pinned nightly's wasm backend: a first version of the `json_quote` fast path
returned the original `text`, and roc-screen and roc-grid then rendered rows
made of other allocations (`roc test` on `Screen.roc` passed, the sandbox
tests failed). The code now reads the bytes once and rebuilds the `Str` from
them. This is recorded in `examples/roc-world/README.md`. Territory's earlier
rows did not show it because small strings are stored inline.

## Limits

- Fuel scales with state. The view costs about 0.96M on an empty board and
  about 0.08M per seat after that; a larger board or longer scoreboard would add more. The
  budget covers eight seats and 112 of 128 cells, not every possible state.
- Records is now the second-largest call at the full house (437k), and update
  with a long key string reaches 831k. Both are far from the limit.
- `render_rows_json` counts columns as Unicode scalar values, like `chars`.
  It does not measure double-width glyphs; the host still validates the
  document and rejects a row whose display width does not fill the screen.
- Builds of the same source gave different bytes on some runs here (the first
  build after other builds differed from later, mutually identical ones), so do
  not compare the checked-in `.wasm` files by hash. Fuel counts are exact for
  the checked-in bytes.
- The other examples (`roc-counter`, `tic-tac-toe`, `roc-chat`, `lounge`) do
  not import `Screen`; I did not rebuild them, and they may also have been
  built by the older compiler.

## Follow-ups

- **A view trap should degrade to an error screen instead of poisoning the
  world.** Not changed here: the poisoning behaviour is unchanged.
- Check that the other checked-in modules were built with the pinned Roc and
  add a check that fails when a module's compiler differs from `nix/roc.nix`.
- Consider moving `roc-screen` and `roc-grid` to `render_rows_json`, and
  `put_grid` to a row path; both still build per-cell records.
- Crowded-state budgets for the other examples (the earlier record's
  follow-up now holds for territory only).

## Progress

- 2026-10-17: reproduced with a failing eight-seat test; attributed the cost;
  rebuilt `territory`, `roc-screen`, and `roc-grid` with the pinned compiler;
  added `json_quote` fast path, `render_rows_json`, `box_edge`; moved
  territory's view to the row path; added the full-house guard and
  re-baselined. Phase 11: territory's fuel follow-up from the perf budgets
  record is closed; the view-trap degradation follow-up is open.

## References

- [Phase 11 foundations plan](../../.opencode/plans/phase-11-foundations.md) (not in this worktree; its "Phase 11 progress" list takes the Progress entry above)
- [Perf budgets record](../2026-10-17T15-30-00Z_feature_world_perf_budgets/2026-10-17T15-30-00Z_feature_world_perf_budgets.md)
- `pkgs/id/examples/roc-world/Screen.roc`, `pkgs/id/examples/territory/main.roc`
- `pkgs/id/src/sandbox.rs`: `crowd_territory`, `drive_territory_full`, `FUEL_BUDGETS`
