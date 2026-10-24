# Authoring CLI: `id world new` and `id world stats`

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md) and the [world CLI controls record](../2026-10-16T12-00-00Z_feature_world_cli_controls/2026-10-16T12-00-00Z_feature_world_cli_controls.md).

## Intent

Two gaps slowed app authors down: starting a world app meant copying an
example by hand, and diagnosing a live world meant three separate commands
(`info`-style frames are not even exposed by the CLI). Add local scaffolding
and a single read-only status object.

## Decisions

- **`id world new DIR [--name NAME]` writes files, no network.** It creates
  `main.roc`, an executable `build.sh`, and `README.md`. The app name is
  restricted to ASCII letters, digits, `-`, and `_` so it is safe in a Wasm
  filename and YAML-free text. The directory must be absent or empty. The
  scaffold is a real, buildable app: a boxed count screen using `pf.Screen`,
  faithful snapshot/restore, and an empty `wants`. The platform path is
  `../roc-world/platform.roc`, so the scaffold lives next to a platform
  checkout; the README says so.
- **`build.sh` resolves `wasm-tools` locally.** It prefers `$WASM_TOOLS`, then
  `wasm-tools` on `PATH`, then a Nix store copy, then `nix run`, so a fresh
  scaffold builds on a NixOS host without configuration.
- **`id world stats NODE --capability CAP` reads three frames.** It composes
  the module info (hash, provenance, sequence), the capability report, and the
  current execution profile into one JSON object. It is read-only and needs
  only a guest capability; no admin token.
- **`id world logs NODE --capability CAP [--seconds N]`** joins with
  `after: 0` and prints `event` frames until the deadline (default five
  seconds) or the host closes. Retention bounds the tail; the deadline bounds
  the command. Errors from the host stop it with the host's message.

## Acceptance

- `scaffolding_writes_a_buildable_app_skeleton` checks the files, the program
  signature, the build output name, the executable bit on Unix, and refusals
  for a non-empty directory and a path-like name.
- The scaffold was built during this slice (`world new` then `./build.sh`) and
  produced a validated import-free Wasm module; the probe was removed.
- `test_cli_parse_world_new_and_stats` covers `new`, `stats`, and `logs`
  (including `--seconds`).
- `id world stats` composes existing client calls; its output shape is the
  three documented frames, and the client calls already have protocol tests.

## Follow-ups

- A `--json` output variant for `logs` and `stats`.
- Template selection (`--template game|chat|dashboard`) once more than one
  scaffold exists.
