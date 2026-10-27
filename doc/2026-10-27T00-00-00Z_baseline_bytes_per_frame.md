# F11.9 — Bytes-per-frame baseline table

- Status: baseline record
- Date: 2026-10-27
- Issue: developing-today/code#542
- Measured at commit: `37418e56339c315cf0ddf3e61c9c55c1e9d0c0a2` (main, 2026-10-27)
- Measurement: `pkgs/id/src/sandbox.rs` test `bytes_per_frame_baseline_for_checked_in_examples`

## What is measured

For each checked-in example wasm in `pkgs/id/examples/`, the guest is
instantiated in the `Sandbox` with a fixed seed (7) and the full-screen
frame byte length at rest is recorded via `layout_at(width, height)` at
the two reference viewports, 80x24 and 200x60.

The baseline is the guest frame bytes **at the sandbox boundary** — the
number Phase 12 delta frames (F12.11) and server-side redraw batching
must beat at the source. Presenter wire encodings (surface JSON / ANSI)
are out of scope for this baseline and are not measured here.

## Baseline table (seed 7, at rest)

| example     | plaza_layout | bytes @ 80x24 | bytes @ 200x60 |
| ----------- | ------------ | ------------: | -------------: |
| lounge      | yes          |            29 |             29 |
| roc-chat    | yes          |            24 |             24 |
| roc-counter | yes          |             7 |              7 |
| roc-grid    | yes          |           383 |            383 |
| roc-screen  | yes          |           179 |            179 |
| territory   | yes          |          1010 |           1010 |
| tic-tac-toe | yes          |            25 |             25 |

Notes:

- All seven examples export `plaza_layout`, so no guest used the
  `view()` fallback in this run. The measurement test asserts the
  fallback invariant (identical bytes at both viewports, equal to
  `view()`) for any future guest without the export, and the printed
  table marks such guests `no (view fallback)`.
- At rest, every example's frame is the same byte length at both
  viewports: the initial frames do not fill the larger viewport. That
  equality is itself a baseline datum — it is the starting point delta
  frames are compared against, not a measurement error (the test calls
  `layout_at` separately for each viewport).

## How to rerun

```sh
cd pkgs/id && cargo test --lib bytes_per_frame_baseline -- --nocapture
```

The test prints the table above (one row per example) and passes when
every example instantiates and renders within the default sandbox
limits. Update this file's table and measured-at commit when a rerun
produces different numbers.
