# roc-cap-demo — a capability-dependent demo library + app (#5957)

A small worked example of the Roc library import model for worlds:
a **library that is not pure** (it depends on a platform capability)
and an **app** that imports it and runs on a platform providing that
capability.

- `lib/` — the demo library, a Roc package. `lib/Greeter.roc` greets
  world participants by name. The names live in the world records, so
  the library issues a `records.read` capability request through the
  platform's `RocCap` protocol helpers: it cannot do its job on a
  platform that does not provide that capability. `lib/main.roc` is
  the package root and binds the library to
  `../roc-world/platform.roc`.
- `main.roc` — the demo app. On init it asks for the names through
  the library; its view greets every participant once the result
  arrives. All protocol handling (issuing, announcing in `wants`,
  routing the result by id, retiring it, snapshot/restore of the
  session) lives in the library.

## Tests

`roc-cap-demo-test.sh` (wired into `scripts/roc-test.sh`, i.e. the `roc-test`
just recipe and flake check) runs:

1. The library's `expect` tests (`roc test lib/main.roc`) and a
   typecheck of the demo app (`roc check main.roc`) — the positive
   path.
2. **Negative test A** — `main-missing-cap.roc`, the same app and
   library bound to the reduced platform
   `../roc-world/platform-no-caps.roc`, which does not expose
   `RocCap`. Compilation must fail, and the failure must name the
   missing `pf.RocCap` module. (`lib-no-caps/` is the library package
   bound to the reduced platform; its `Greeter.roc` is a
   byte-identical copy of `lib/Greeter.roc` — Roc package modules
   cannot be symlinked in from outside the package root — and
   `roc-cap-demo-test.sh` fails if the copies drift.)
3. **Negative test B** — `main-wrong-usage.roc`, the app against the
   full platform but calling `RocCap.schedule_at_request` with a
   wrong signature. Compilation must fail with a type mismatch at
   that call.

Each negative test pins *why* it fails (asserted error content), not
just that it fails.
