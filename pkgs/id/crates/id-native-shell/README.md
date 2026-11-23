# id-native-shell

Native reference shell for `id` — F13.6, design
`doc/2026-11-01_design_id788.md` (issue #788). A Bevy client speaking
the F13.1 shell contract (`pkgs/id/contract/shell-contract.v1.json`):
it renders the host's `view` frames and emits only the client frames
the contract already names (`input`, `paste`, `resize`, `chat`). It
adds no frame kinds and needs no host change.

This crate is a workspace member but deliberately **not** a default
member, so the ordinary `id` build never acquires Bevy or
desktop-platform dependencies:

```sh
cargo build -p id                # no Bevy — verified by the isolation rule
cargo build -p id-native-shell   # the native shell
cargo test -p id-native-shell    # unit + transcript + parity tests
```

## Status (design steps)

- **Step 1 — done** (PR #5573): isolated app package, identity /
  connection bootstrap (the CLI's own `CLIENT_KEY_FILE` +
  `load_or_create_keypair`), contract frame builders, view fallback
  rule, transcript fixture checked against the machine-readable
  contract (`tests/transcript.rs`).
- **Step 2 — done**: this crate now carries
  - the Bevy window/render feature set and the Nix system-library
    wiring it needs (below),
  - a styled render model + Bevy UI renderer for validated surfaces
    (`src/render.rs`): `Screen` surfaces render per-cell with
    xterm-256 colors; `Text`/`Form` surfaces and surface-less views
    fall back to the deterministic text projection / legacy `data`,
    exactly as the terminal shell presents them,
  - native event mapping (`src/events.rs`): key bytes, clipboard
    paste, and window-pixel resizes become the same contract frames
    the terminal builders produce (`tests/parity.rs` is the oracle),
  - the live `WorldClient` session loop (`run_session` in
    `src/session.rs`): join, validated host frames folded into
    `SessionState`, every view published to the renderer
    (`PresentedView` in the binary), outbound frames routed to the
    session's world — plus the live-host durable-outcome parity
    test that comes with it (`tests/live_session.rs`: one game
    played through both the native loop and a terminal-style raw
    client, with the host journal as the oracle).
- Steps 3–4 (rich 3D/audio/widget surfaces) are conditional on the
  design's Q1 decision for Drew and are **not** in this crate.

## Capability statement

Per the shells v1 record (`doc/2026-11-10T20-40-00Z_design_more_shells_v1`,
§2 and "The one gate"), this shell's capability statement is recorded
in [`capability-statement.json`](capability-statement.json): the client
frames it sends (`join`, `input`, `paste`, `resize`, `chat`,
`screen_subscribe`, `form_result`, `provider_offer`, `provider_reply`),
the server frames it presents (the `SessionState` fold, with `error`
surfaced as a session refusal), the one capability provider it
declares (`client.file.read`, F13.7), and what it excludes by name
(rich surfaces, offline reading, mobile-native). The conformance run
is wired into the check — the `test-native-shell` flake check runs
this crate's whole test suite — and
`tests/capability_statement.rs` pins the statement to the contract
and to the code's actual builders, session fold, and provider
adapter, so the record cannot drift from the shell it describes.
Recording the statement completes issue #10928's Done alongside the
already-landed conformance wiring and the behavioral checks
(`tests/parity.rs`, `tests/live_session.rs`).

## Build prerequisites

The window/render features link system libraries:

- Wayland: `wayland-client` (via `pkg-config`)
- X11: `libX11`, `libXcursor`, `libXrandr`, `libXi`
- `libxkbcommon`

Inside the repo, `nix develop` (pkgs/id flake) provides all of them —
they are in `buildInputs` in `pkgs/id/nix-common.nix`, with the
runtime library path exported by the shell hook. Outside Nix, install
the equivalent `-dev` packages for your distribution first.

## Running

```sh
cargo run -p id-native-shell -- --node <node-id> --capability <cap> [--world <name>] [--addr <ip:port>] [--no-relay]
```

opens the Bevy window with the renderer live and joins the world:
host views feed the presented view as they arrive. Without
`--node`/`--capability` the window opens without a session and the
startup line on stdout says so.

## Recording / checking a session

The step-1 transcript harness stands in for a recorded session:
`tests/fixtures/transcript_step1.json` is a scripted frame sequence
that `tests/transcript.rs` checks against the F13.1 contract checker
(`id world contract --check` is the CLI equivalent for a transcript
file).
