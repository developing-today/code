# Platform world: current state, findings, and open work

Transcribed from context on 2026-10-18, before compaction. No new research was
done for this file. Where a number or SHA appears it is what the context holds;
verify against the repo before relying on it.

## 1. What has landed (merged into `main`)

Merged as one stack top (#234) plus siblings and independents. Verification on
`main` at `60f7a634`: **1988 tests, 0 failures, 36 suites**, no disk pressure.

| PR   | Adds                                            | Enables                                                                             |
| ---- | ----------------------------------------------- | ----------------------------------------------------------------------------------- |
| #209 | Roc source compilation from the world page      | Worlds built from source in the browser                                             |
| #214 | Portable screen presenters (web + SSH)          | One structured grid renders identically in browser and terminal                     |
| #215 | Source-only install policy                      | Server can refuse raw artifacts, forcing build-from-source                          |
| #216 | Terminal keyboard input for web worlds          | Games playable from the browser with real key events                                |
| #219 | Per-session viewport and paste                  | Connections size independently; bounded clipboard paste                             |
| #220 | Terminal buffer capture interface               | Ratatui-style buffers feed the same screen contract                                 |
| #221 | Bounded pure Roc `RocJson`                      | Apps parse/encode JSON without host-specific shortcuts                              |
| #224 | LogSync deadline + CI disk cap + `--keep-going` | Tests no longer race 30s; CI reports every check                                    |
| #225 | Module build provenance                         | Recorded compiler/platform/source digests on install (observation, not attestation) |
| #226 | Per-world resource profiles                     | Optional requests clamped to server hard ceilings                                   |
| #228 | Durable per-world resource profiles             | Profile survives restart; applies on next install                                   |
| #231 | Capability ceilings                             | Server and per-world ceilings bound which capabilities reach authority              |
| #233 | Operator CLI for ceilings and profiles          | `id world caps --ceiling`, `id world profile`                                       |
| #234 | Structured RocJson/Screen app migration         | `territory`, `lounge`, `roc-grid`, `roc-screen` on shared libraries                 |
| #239 | Session recording and replay                    | `id world record` / `replay` fixtures; record-match verdict                         |
| #243 | Journaled scheduled events                      | `schedule.at` / `schedule.cancel`, restart-safe, `late:true` on catch-up            |
| #244 | Journal refuses unreadable complete lines       | Fail-loud rollback instead of silent truncation                                     |
| #246 | Phase 11/12 plans                               | Foundations and platform-flexibility roadmaps                                       |
| #247 | Per-call fuel metrics and budgets               | `id world measure`; budget tests guard regressions                                  |
| #248 | Territory view fits 8 seats                     | 1.6M fuel (16% of limit) vs 7.1M; `Screen.render_rows_json`                         |
| #256 | Chat visibility + `history` scope               | Join-only viewers no longer see chat; `history` opts in to read                     |

Independents also merged: #210 (signed doc revisions), #212 (home discovery),
#213 (Workers pkarr doc). #255 is the owner's own sweep and was left alone.

## 2. SSH input: the one open design question

The platform has one open PR, **#211** (SSH chat and document viewing), which
predates the keyboard work and builds a line-buffered text interface for SSH
(`/chat hello`, `/records`, `/documents`). `main` now forwards raw keystrokes so
games work in an SSH terminal. The two input models conflict. Reconciling them
is a product question.

### Three things that get conflated

- **Input routing**: raw keystrokes forwarded one by one, versus line-buffered
  text collected until Enter.
- **Terminal screen buffer**: primary screen (normal scrollback) versus
  alternate screen (full-screen app mode, no scrollback). Display, not input.
- **Dual channel**: two input paths live at once in one SSH session. Not a mode
  switch, not a screen buffer choice.

### Typing: does the local client update the screen?

| Mode                       | Who draws while you type                      | What the server receives                   |
| -------------------------- | --------------------------------------------- | ------------------------------------------ |
| Raw (world default)        | Nobody; the world redraws on each input       | every keystroke                            |
| Buffered line, local echo  | the local client, like a shell prompt         | the whole line on Enter                    |
| Buffered line, server echo | the server draws the prompt and echoes        | keystrokes, echoed and drawn by the server |
| PTY canonical / cooked     | the kernel line discipline buffers and echoes | the line on Enter                          |

Local echo is snappier but the server does not know what is being typed, so
autocomplete and validation need streaming anyway. Server echo gives the server
everything as it is typed, at one round trip per keystroke unless batched. The
hybrid most modern TUI line editors use over SSH: stream keystrokes for
intelligence, let the client draw locally.

### Raw keystrokes in primary mode with scrollback

Yes, they work. The screen buffer is independent of input routing. What matters
is what is on screen:

- **Alternate screen**: the normal choice for a game. No scrollback, full-screen
  redraw.
- **Primary screen, in-place redraw**: works, scrollback preserved above, same
  keystroke semantics. Write at the cursor, move back up.
- **Primary screen, appended log output**: raw input works but the display
  becomes a transcript. Fine for a chat or a log-following world.

The real cost of primary mode is not input; every redraw competes with
scrollback and anything else printed to the terminal.

### The four dual-channel shapes

1. **Split UI on one PTY.** World grid above, command line pinned at the bottom
   of the same buffer. One channel, one PTY, server draws both regions;
   keystrokes route by focus. Gains: one connection, one screen, no client
   changes, commands and play coexist visibly. Losses: server owns more UI
   state; world redraw must not clobber the command line; focus switching is
   state the user must notice; round-trip latency for command echo.

2. **Two channels on one SSH connection.** SSH multiplexes independent channels
   over one connection. One channel is the raw game stream; another runs a
   shell-like command line. The client decides which channel a keystroke goes
   to, usually by which pane has focus. Gains: clean separation; the command
   channel can be scripted, tab-completed, and piped into without touching the
   game stream; one connection, one auth. Losses: needs client-side support
   (most off-the-shelf SSH clients open one session channel, so this needs our
   own client or a wrapper); more moving parts and more to test. Best for power
   users with our own client, or automation that drives a world while watching
   the game stream.

3. **Prefix key and a command buffer.** Raw keystrokes everywhere. A prefix key
   (for example `Ctrl-P` or `~ Enter`) switches the keyboard to a line buffer
   shown in a prompt line. Still one channel and one screen; no permanent
   command region. Gains: smallest screen change, game stays dominant, no focus
   ambiguity when the prompt is hidden. Losses: mode state; a keystroke means
   different things depending on hidden state, and a mis-set mode silently eats
   input; needs a visible indicator; escape handling gets fiddly.

4. **Primary mode, line oriented.** Classic terminal mode: scrollback, buffered
   commands, line-oriented output. The world prints text lines; you type
   commands at a prompt. Gains: trivially scriptable — pipe in commands, parse
   output, tab-complete, autocomplete, error hints, retry logic. This is what an
   agent or CI job wants. Losses: no real-time grid, no game input; output is a
   transcript.

### Why allow both, and why not

Games need raw; operators and agents need commands. The risk is only in how they
coexist:

- **Bad**: auto-detect ("if it looks like text, buffer it") — non-deterministic
  and untestable.
- **Good**: explicit, one-way routing. A prefix key or a startup flag chooses
  the path; mode is visible; exit returns to raw.

Reasons not to do both (costs, not blockers): two input paths mean more protocol
surface and more tests; mode state can leak or mis-route input under load; more
than one way to send the same message complicates auditing.

### Mode-switch triggers

| Trigger                            | Deterministic | Needs client support | Notes                                                  |
| ---------------------------------- | ------------- | -------------------- | ------------------------------------------------------ |
| Prefix key (`Ctrl-P`, `~ Enter`)   | yes           | no                   | standard TUI choice; needs visible mode indicator      |
| Dedicated key (`Ctrl-]`, `Ctrl-\`) | yes           | no                   | no collision with editing keys, undiscoverable         |
| Startup flag or subcommand         | yes           | no                   | `id world join --command-mode` picks the mode up front |
| Type `/` in raw mode               | no            | no                   | a game that wants to type `/` breaks                   |
| Ctrl+C / interrupt                 | ambiguous     | no                   | already used for cancel; do not overload               |
| Client capability negotiation      | yes           | yes                  | client tells the server what it supports               |

Two distinct switches to plan for: **session start** (primary line mode for
automation and agents versus alternate interactive mode for humans) and
**in-session** (entering and leaving command mode while interactive mode stays
up). The second is where the prefix-key and mode-indicator work lives.

### Buffered output while staying interactive

Separate axis from input routing. You can keep raw keystrokes and the alternate
screen and still let the server batch and buffer its output:

- **Batching**: the server accumulates redraws for a few milliseconds and sends
  one frame instead of many. Server-side, no client change.
- **Buffered output to a region**: with shape (1), the world screen redraws live
  while the bottom command region holds a buffered line the server knows about,
  giving server-side autocomplete and suggestions without giving up the game.
- **Extended to Roc apps**: if the platform passes "the viewer is typing this
  partial line" to the program, a Roc app could render live autocomplete or a
  validating form. New guest-facing surface; needs its own design: what event
  carries the partial line, that it is not journaled, and what its bounds are.

### Current shape that fits this repo

- Raw in alternate mode as the interactive default. Already on `main`. Keep it.
- Primary line mode as a separate entry path, driven by the same command
  grammar as the CLI. This is the automation and agent surface: pipe in
  commands, one-off invocations, tab completion. No world screen needed. This
  is #211's commands with a different front door.
- One trigger for the mode switch: the startup flag or subcommand. Deterministic,
  no hidden state, no client support needed. Add a prefix-key switch later only
  if people want to switch mid-session.
- Defer in-session buffered typing to Roc apps until (2) exists and the command
  grammar is known.

Do not auto-detect mode from content. Do not build two SSH channels on one
connection until someone has a real client for it.

## 3. What is done

- Portable screens: one grid contract, web + SSH + captured buffers, no guest
  HTML/CSS/escape sequences.
- Shared Roc libraries: `RocJson`, `RocGrid`, `RocCap`, `RocText`.
- Live examples: `territory` (multi-player grid, per-seat colours, inverted
  cursor, scoreboard), `lounge`, `roc-chat`, `roc-grid`, `roc-screen`,
  `tic-tac-toe`, `roc-counter`.
- Policy: source-only installs, per-world profiles (request clamped to server
  ceilings), capability ceilings (known ∩ server ceiling ∩ world ceiling ∩
  granted), roles (`spectator`, `observer`, `player`, `guest`, `delegator`).
- Security posture: install seed redacted in join projections; chat hidden from
  join-only viewers with an explicit `history` scope to opt in to reading.
- Cost: per-call fuel metrics, `id world measure`, budget tests. Territory's
  full house is 1.6M of 10M.
- State: scheduled events (journaled, restart-safe, `late:true` on catch-up),
  session fixtures and replay with verdicts, journal fail-loud on unreadable
  complete lines, module provenance.
- Operator CLI: `id world caps --ceiling`, `id world profile`, `id world new`,
  `id world stats`, `id world logs`, `id world record`, `id world replay`,
  `id world measure`.
- Phase 11 (foundations) and Phase 12 (platform flexibility) plans written.

## 4. What is not done, planned, or proposed

- **#211 (SSH worlds)**: the input model choice above. Open.
- **CI**: `main`'s Nix check is still red on Firefox E2E (#229). The owner said
  leave E2E alone.
- **Journal rollback (#227)**: binaries built before the #244 fix truncate an
  unreadable last line. #244 fixed future binaries; pre-existing ones still
  truncate. Restore a journal backup before rolling back.
- **LogSync flakes (#241)**: the bulk test hits its 30s deadline under load; #224
  aligned it to 60s. `test_an_interrupted_session_restarts_without_duplicates`
  flakes the same way. Why a 23s bulk transfer and the mid-stream reset under
  `--keep-going` are unexplained.
- **Host events (#242)**: presence, capability results and ticks reach the
  program but are not in `events_after`, so fixtures cannot contain them.
- **Phase 11 (foundations)**: CI repair (relay kit, E2E isolation, LogSync
  explanation, a fast lane, injected time), cost visibility (fuel metrics done,
  host metrics and tracing, benchmarks, bytes per frame), structure (split the
  large modules — `world.rs` 6,120 lines, `world_session.rs` 4,017, `cli.rs`
  3,958 — one client projection done in #256, journal format tooling, error
  taxonomy), guest cost (cheaper `Screen`, module size, replay self-check,
  view traps should degrade to an error screen instead of poisoning the world,
  pinned toolchain for checked-in artifacts).
- **Phase 12 (platform flexibility)**: world manifest and `apply`, forking and
  templates, roles v2 (builder, moderator), provider registry (`kv`, `blob`,
  `http.fetch`, `notify`), ABI spec and conformance suite, more guest SDKs
  (Rust, TypeScript/AssemblyScript or Zig), program upgrade with state
  migration, fair scheduling and quotas, delta and binary frames, many worlds
  per node, journal replication, portals, `id world dev`, `id world doctor`,
  module trust and signing, denial-of-service tests.
- **RocText** exists with only proven string operations; Unicode-aware casing
  and trimming deferred until a Roc nightly bump.
- **No table/list/meter `Screen` widgets** yet; deferred until two apps need the
  same one.

## 5. Open GitHub issues

- **#227** Define rollback compatibility for world profile journal entries.
- **#229** Stabilize shared-state failures in Firefox E2E CI. Owner said leave
  E2E alone.
- **#238** Nix CI cannot read the relay kit from the repo-root doc tree. Settled
  on `main` by #249 (vendored copy under `tests/fixtures/relay`, drift test).
- **#240** Install seed is readable by every joiner. Fixed on `main` by #250
  (seed redacted in `events_after` and the snapshot) and #256 routes it through
  one projection.
- **#241** Bulk LogSync test retries a mid-stream reset instead of explaining
  it. Live failure on `main` is `Timeout(30s)`; #224 aligns the deadline to 60s.
  Still open: why a 23s bulk transfer, and the reset under load.
- **#242** Host-originated events are not retained in the event stream.
- **PR #255** (owner's sweep) and **PR #211** (SSH worlds) are the only
  non-bot PRs still open.

## 6. SHAs and worktrees held in context

Merge state: `main` at `60f7a634` (Merge pull request #212).
Integration worktrees and heads referenced during the merge-down:
`platform-world-apps-migration` (chain top, merged as #234, `1dcd5fd4` merge of
main into it), `platform-world-journal-tail` (#244), `platform-next-stage`
(#246), `platform-perf-budgets` (#247), `platform-world-schedule` (#243),
`platform-world-fixtures` (#239), `platform-territory-fuel` (#248),
`platform-doc-events` (#210), `platform-home` (#212), `platform-p7-2` (#213),
`platform-ssh-world` (#211, merge aborted, still conflicting),
`fix-log-sync-ci` (#224), `fix-chat-visibility` (#256).
Cherry-pick SHAs during the integration: `5323cf9e`, `d4061072` (rollback
correction doc commits), `13c1b1d0`, `a9252ec3`, `840e1a6e` (the three commits
kept on #224's rebuild).

## 7. Testing and CI notes

- `cargo test --lib`: 1988 passed, 36 suites on the merged state.
- `cargo test --all-features -- --skip serve_tests`: 1988 passed in the home
  worktree run after #212; web `bun run test` (vitest) 349 passed / 9 skipped.
- Plain `cargo clippy` fails with E0514 (toolchain mismatch between the system
  rustc and the pinned one); the rustup 1.97.0 cargo in a separate
  `CARGO_TARGET_DIR` exits 0. Not a code problem.
- `cargo check --all-targets --all-features` needs `web/dist` built first
  (`bun install` + `bun run build`).
- Full `cargo test --lib` needs `RUST_MIN_STACK` (set in
  `pkgs/id/.cargo/config.toml`) and cargo run from `pkgs/id`.
- The `id` workflow's `nix flake check` runs `nix flake check ./pkgs/id/ -L
--keep-going` (the `--keep-going` is the #224 change).
- `mkCheck` sets `CARGO_INCREMENTAL=0` and `line-tables-only` debuginfo so
  parallel debug builds do not fill the runner's disk.
- Running `cargo test` from the repo root instead of `pkgs/id` overflows the
  clap stack in tests; this is why the `RUST_MIN_STACK` config exists.

## 8. Repo rules held in context (from `pkgs/id/AGENTS.md`)

- Never `git restore` / `git checkout --` over unstaged work. Ask first.
- Never delete `rust-toolchain.toml`.
- Any new just recipe needs a matching `nix run .#<command>` app in `flake.nix`.
- Lints deny `unwrap_used`, `expect_used`, `panic`, `todo`, `dbg_macro` in
  non-test code; test modules carry `#[allow(clippy::unwrap_used,
clippy::expect_used, clippy::panic)]`.
- Documentation protocol: `doc/<UTC_RFC_DATETIME>_<kind>_<name>/` folder with a
  matching `.md`; plans live in `.opencode/plans/`.
- Link to source plans from the top of the doc; References at the bottom.

## 9. Corrections made during this work

- Said territory's styled view fit all eight seats. It trapped at six or more.
  The measurement used few seats. Fixed in #248 (1.6M fuel for a full house).
- Said rollback to a pre-change binary fails loudly. Only non-final unreadable
  entries fail loudly; a final one is truncated as a torn append. Fixed for
  future binaries in #244; pre-existing ones still truncate.
- Said join-only participants cannot read chat. They could, via the snapshot
  and `events_after`. Fixed in #256.
- Said a join-only watcher heard chat live on `main`. It did not; the session
  loop never forwarded the raw event. The raw broadcast does carry the real
  seed and chat, and #256 pins that so nobody forwards it unprojected.
