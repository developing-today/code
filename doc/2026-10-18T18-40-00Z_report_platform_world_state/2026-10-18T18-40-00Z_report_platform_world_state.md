# Platform world: state report

**Date:** 2026-10-18
**Scope:** the `id` platform world stack, its example apps, the SSH input
decision and how to implement it, and the open GitHub work.
**Basis:** session context, the companion transcription
[`2026-10-18T00-00-00Z_reference_platform_state`](../2026-10-18T00-00-00Z_reference_platform_state/2026-10-18T00-00-00Z_reference_platform_state.md),
live repo/GitHub checks at 18:36–19:00Z, and four code-level research passes
(SSH I/O path, PR #211 diff, CLI/frame protocol, plans + apps).

Source plans (committed on `origin/main`, `.opencode/plans/`):

- [Phase 11 — foundations](../../.opencode/plans/phase-11-foundations.md)
- [Phase 12 — platform flexibility](../../.opencode/plans/phase-12-platform-flexibility.md)
- [Phase 9/10 — portable platform](../../.opencode/plans/phase-9-portable-platform.md)

> **Checkout note:** the verified state is `origin/main` at `60f7a634`. The
> local `main` ref and the `platform-world-apps-migration` worktree lag it by
> ~145 commits (they lack `Record`/`Replay`/`Measure` and the plan files).
> Facts below are stated against `origin/main` unless noted.

---

## Executive summary

1. **The platform stack is merged into `main`** (#209–#256 plus #210, #212,
   #213, #224). Verified at `60f7a634`: **1988 Rust tests, 0 failures, 36
   suites**; web vitest **349 passed, 9 skipped**; fmt clean.
2. **One world PR remains: #211** (SSH chat and documents) — `OPEN`,
   `CONFLICTING`. Research shows exactly what it adds (§7) and where it
   conflicts: four regions in `SshIo`, all reconcilable. It is line-buffered
   input vs `main`'s raw keystroke forwarding. This is a product decision,
   not a technical blocker.
3. **The input decision is made (§5–§7): raw keystrokes are the interactive
   default; a line mode becomes a second, explicit entry path** for automation,
   agents and quick checks — selected deterministically (an SSH username prefix
   like the existing `explore:`, or a CLI flag), sharing one command grammar
   with `id world`. No content auto-detection. Long-term, agreed: every
   variation — raw/buffered × primary/alternate, split UI, dual channel —
   lands eventually, each for a different context (§13–§15). §7 is a concrete
   implementation plan grounded in file/line.
4. **What the modes are for (§6):** play, quick checks, terminal chat, doc
   reading, scripting and piping, agent/CI transcripts, fixture recording.
   The web client already runs the "split UI" shape (screen + chat form).
5. **Phase 11/12 plans are committed and detailed** — 18 F11 tasks and 18 F12
   tasks with measurable exit criteria (§8). Input modes appear in _neither_
   plan; SSH lives in Phase 9. Adding the modes is a new work item worth
   recording in the Phase 11 plan.
6. **CI:** only the Firefox E2E check is red on `main` (#229 — leave alone).
   The `id` PR gate is the slowest in the repo: ~116 min average per PR
   (#292), with a hard-coded check list (#300) and intermittent disk
   exhaustion (#257).
7. **Where next (§12):** decide the mode shape → reconcile #211 in three
   steps (keep-stack, then line mode as a separate entry, then optional
   toggle) → Phase 11 order F11.17, F11.18, F11.14, F11.5, F11.10 → CI slice
   (#292/#300/#257) → Phase 12 first slice (F12.4, F12.1, F12.7).

---

## 1. What has landed

Merged as one stack top (#234) plus siblings merged individually with `main`
merged in and tested first.

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
| #225 | Module build provenance                         | Compiler/platform/source digests recorded on install (observation, not attestation) |
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
| #248 | Territory view fits 8 seats                     | 1.6M fuel (16% of 10M) vs 7.1M before; `Screen.render_rows_json`                    |
| #256 | Chat visibility + `history` scope               | Join-only viewers no longer see chat; `history` opts in to read                     |

Independents also merged: #210 (signed doc revisions), #212 (home discovery),
#213 (Workers pkarr doc). Resolved and closed: #238 (relay kit, settled by
#249 — vendored under `pkgs/id/tests/fixtures/relay` with a drift test),
#240 (install seed leak, fixed by #250/#256).

## 2. The apps' present state

All eight example dirs sit on the shared platform
(`pkgs/id/examples/roc-world/platform.roc`, freestanding Zig host adapter).

### Shared guest surface

- **Screen contract:** one structured grid; web, SSH and captured buffers
  present the same structure. No guest HTML/CSS/escape sequences.
- **Libraries:** `RocJson`, `RocGrid`, `RocCap`, `RocText` (proven string
  operations only — Unicode casing/trimming deferred to a Roc nightly bump).
  **`RocText` is currently imported by no example** — it exists in the
  platform only (`platform.roc:14`, `roc-world/README.md:99`).
- **`platform.roc`** requires `layout : model, U64, U64 -> Str` (guest-aware
  reflow). `Key.parse` decodes raw terminal input bytes into a `Key` tag union
  (`Char/Ctrl/Enter/Tab/Backspace/Escape/Delete/arrows/Home/End/PageUp/
PageDown/Unknown`) matching the browser encoder's sequences.
- **Not built:** table/list/meter `Screen` widgets — deferred until two apps
  need the same one.

### The examples

| App           | What it is                                                                                                                       | Shared libs                           | Notes / gaps                                                                                                                                   |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `territory`   | Multiplayer 16×8 grid, 8 seats (A–H + cursor), persistent seats; journal replays grid/seats                                      | `Key`, `Screen`, `RocJson`, `RocGrid` | Reference app. Full house = 1.6M fuel (16% of 10M) after #248. `territory.wasm` checked in (`--wasm-memory=8454144` to fit the 16 MiB sandbox) |
| `lounge`      | Capability showcase: `players` + `time.tick:5000`, greets arrivals via `chat.say`, refusals counted `denied=N`, polls `time.now` | `RocJson`, `RocCap`                   | 178 lines                                                                                                                                      |
| `roc-chat`    | Minimal chat on `chat.history`: type a line + Enter → `chat.say`; `hist` returns newest 8 lines                                  | `Key`, `RocCap`, `RocJson`            | 149 lines; matches the #256 visibility rules                                                                                                   |
| `roc-grid`    | `RocGrid` demo: 4×3 grid, `set` writes `A` at centre; reports untouched count / cursor cell / OOB read                           | `RocGrid`, `Screen`                   | 61 lines                                                                                                                                       |
| `roc-screen`  | Portable screen + `Key.parse` demo: counter on `+`, cell grid in a bordered `"counter"` box                                      | `Screen`, `Key`                       | 47 lines                                                                                                                                       |
| `tic-tac-toe` | Two-player game, digits 0–8, ply parity X/O, pure `update` for exact replay; publishes records                                   | none                                  | **Older pattern:** records JSON hand-built by string interpolation (`main.roc:62`) — no `RocJson`                                              |
| `roc-counter` | Tiny pure counter (`inc` → `count=N`), `records` export                                                                          | none                                  | **Older pattern:** legacy plain-text view (`main.roc:14`). Needs Zig 0.16 + pinned Roc `nightly-2026-10-04-130536d`                            |
| `roc-world`   | The shared platform + Zig host adapter (`platform.roc`, `Screen/Key/RocJson/RocGrid/RocCap/RocText.roc`)                         | —                                     | ABI docs require the pinned compiler for `roc_platform_abi.zig`                                                                                |

Migration leftovers worth doing eventually: `tic-tac-toe` → `RocJson`;
`roc-counter` → `Screen`; adopt `RocText` where a real use appears.

## 3. How input works today (code level)

The full keystroke path, from terminal to guest:

1. **SSH channel bytes** arrive at `SshClient::data` (`world_ssh.rs:310-330`)
   → `SshInputFrame::Bytes` into a 256-slot `mpsc` queue via `try_send` —
   never waits; **input is dropped on a full queue** (`world_ssh.rs:323-324`).
2. **`SshIo::recv`** (`world_ssh.rs:371-389`) wraps each batch as
   `{"type":"input","data_hex":<hex>}` (or `{"type":"resize",…}`); the join
   frame is synthesized from the SSH login (`world_ssh.rs:351-357`) —
   **username = world name, password = join capability**; an `explore:` user
   prefix opens the directory explorer instead (`world_ssh.rs:143-146`).
3. **`ClientFrame::Input`** (`world_session.rs:233-235`) is hex-decoded and
   dispatched in `handle_client_frame` (`world_session.rs:2256-2468`) to
   `world.input(token, data)`; `paste` (≤4096 bytes, no control chars) and
   `chat` take sibling paths; `resize` (1..=500 × 1..=200) is **ephemeral,
   never journaled** and re-sends a fitted `view`.
4. **The journal** stores `WorldEventKind::Input(Vec<u8>)` as **opaque bytes**
   (`world.rs:275`, validation `world.rs:1649-1667`); a rejected input is
   never journaled.
5. **The guest** receives the bytes as **valid UTF-8 text**
   (`sandbox.rs:677`) copied into linear memory and passed to
   `plaza_update(model, participant, ptr, len)` (`sandbox.rs:343-358`).
   `System` host events (presence, capability results, ticks) arrive the same
   way with participant 0 (`sandbox.rs:683-685`); `Chat` and `ProgramInstalled`
   are **not** delivered to the guest. Key parsing is guest-side (`Key.parse`).

Output path and batching:

- Every committed event → each session immediately sends `Event` **plus a full
  `View`** (`world_session.rs:2110-2115`); `send_view` re-renders the whole
  presentation fitted to the viewport (`world_session.rs:2166-2209`).
- Host events signal a `tokio::watch` that **coalesces bursts** to one repaint
  per wake-up (`world.rs:2033, 2124-2125, 3117-3122`); a lagging broadcast
  subscriber resnapshots instead of receiving the backlog
  (`world_session.rs:2117-2126`).
- **No output batching otherwise.** SSH writes each frame immediately as a
  **full repaint on the primary screen** — `\x1b[H\x1b[2J` + SGR row runs via
  `ScreenSurface::render_ansi` (`world_presentation.rs:321-405`). There is
  **no alternate screen** (`\x1b[?1049` appears nowhere in the tree), no
  cursor save/restore, no incremental diffing. `error` frames print
  control-stripped text (`world_ssh.rs:681-686`) so no participant can inject
  escapes into another's terminal.
- The browser client (`web/src/world.ts`) encodes keydowns to terminal bytes
  (`world-keyboard.ts:26-53`), sends `input`/`paste`/`resize` frames, and has
  a **separate chat form** (`world.ts:524-533`) — i.e. the web client already
  runs the split-UI shape.

Existing seams a line mode can use:

- **`SessionIo`/`Inbound`** (`world_session.rs:66-88`) — transport abstraction
  SSH already implements.
- **`WorldClient`** (`world_net.rs:157-964`) — wraps every request/response
  pair for out-of-band queries (`logs`, `stats`, `record`, `replay` use it).
- **The explorer's line editor** — `Typing` (`world_ssh.rs:615-677`) with
  echo, backspace, Enter, ESC-discard, 1024-byte cap, and the
  `directory_view::parse_line` grammar (`directory_view.rs:612-738`) is a
  working in-repo precedent for a line shell over the same channel.
- **`id world join`'s pump** (`commands/world.rs:781-798`) — a tiny client-side
  line grammar today: `/quit`, `/input <hex>`, else chat.
- `exec_request`/`subsystem_request` are **rejected** today
  (`world_ssh.rs:209-225`) — the hook point if a second SSH channel is ever
  wanted.

## 4. PR #211: exactly what it adds

Branch `origin/platform-ssh-world`, +761/−45 across 4 files (5 commits over
merge-base `292d3c7a`; PR claims "17 passed" in the focused `world_ssh` suite).

- **`world_ssh.rs` (+714/−16):** `WorldLineBuffer` (submit on CR/LF, CRLF once,
  backspace editing, 1024-byte cap with overlong-line discard, **no echo**),
  an escape-sequence swallowing state machine (`WorldEscape`: CSI/SS3/OSC/DCS/
  APC/PM/SOS — kills OSC 52 clipboard injection), `world_command` parser, a
  records renderer with paging, and a **local read-only document viewer** over
  the server's blob store (`list_public_documents`, `read_public_document`:
  public names only, `.txt/.md/.markdown`, blob `Complete`, ≤64 KiB, valid
  UTF-8).
- **Commands — correction:** there is **no `/chat`**. Any plain line becomes a
  `chat` frame (implicit chat). The set is: `/input <hex>`, `/records
[prefix]`, `/documents [prefix]`, `/document <name>`, `/help`, `/quit`.
- **`commands/serve.rs` (+9/−3):** passes the blobs `store_handle` into
  `world_ssh::serve`. `blob_gate.rs` is rustfmt-only. One doc record gets a
  rewritten SSH section.
- **Tests (8):** e2e chat + records on a vt100 terminal; fragmented-packet and
  CRLF handling; command mapping and `/input` hex rules; overlong discard;
  OSC 52 drop; JSON-escaped bounded records rendering; document name/size/
  binary limits (incl. `../`, newline-in-name); e2e document listing/viewing
  with escape neutralization. The sandbox shell test was changed to send
  `/input 696e63\r` — proof the PR changes the input contract.

**Conflicts with `main` (merge-tree dry run): four regions, all in `SshIo`:**

1. Construction in `shell_request`: `main`'s `first_resize` vs the PR's
   `capability/lines/commands/records_prefix`.
2. `struct SshIo`: `main`'s `mpsc::Receiver<SshInputFrame>` (Bytes/Resize) vs
   the PR's `Receiver<Vec<u8>>` + command fields.
3. `SessionIo::recv`: `main` forwards resize frames (per-session viewport);
   the PR's command loop would **drop them** — semantic conflict, not just
   textual.
4. `SessionIo::send`'s `"view"` arm: `main` renders `value["surface"]` via
   `WorldSurface::Screen(...).render_ansi()`; the PR replaces it with a
   clear-repaint + prompt banner and adds `"snapshot"`/`"records"` arms. A
   naive merge loses surface rendering.

**Keep regardless of the decision:** the whole document stack, the records
renderer and paging arm, the blobs plumbing, `WorldEscape` + length/UTF-8
safety + the injection test, `world_command`'s validation rules.

**Inherently line-mode:** `WorldLineBuffer`, implicit chat (needs an explicit
`/chat` in raw mode), `/input <hex>` (a workaround for line mode — in raw mode
keystrokes go direct), the `world> ` prompt banners appended to view repaints
(collide with full-screen surface repaints), Ctrl-C/Ctrl-D quit semantics.

Known gaps to fix while reconciling: `/records` auto-drains **all** pages with
only a per-page 384 KiB cap (no total cap); document handlers write to the
channel inside `recv` rather than through `send`; documents read the server's
local blob store, not a per-world store.

## 5. SSH input: the design decision

### Three things that get conflated

- **Input routing** — raw keystrokes forwarded one by one, versus line-buffered
  text collected until Enter.
- **Terminal screen buffer** — primary (scrollback) versus alternate
  (full-screen app). Display, not input. (`main` today uses neither properly:
  primary screen with clear-and-repaint, so scrollback accumulates cleared
  frames. Moving the interactive default to the alternate screen is an
  improvement to make, not the status quo.)
- **Dual channel** — two live input paths in one SSH session. Not a mode
  switch, not a screen buffer choice.

### Does the local client update the screen while you type?

| Mode                       | Who draws while you type                      | What the server receives                   |
| -------------------------- | --------------------------------------------- | ------------------------------------------ |
| Raw (world default)        | Nobody; the world redraws on each input       | every keystroke                            |
| Buffered line, local echo  | the local client, like a shell prompt         | the whole line on Enter                    |
| Buffered line, server echo | the server draws the prompt and echoes        | keystrokes, echoed and drawn by the server |
| PTY canonical / cooked     | the kernel line discipline buffers and echoes | the line on Enter                          |

Local echo is snappier but the server does not know what is typed, so
autocomplete and validation need streaming anyway. Server echo gives the
server everything as typed, at one round trip per keystroke unless batched.
The hybrid modern TUI line editors use over SSH: stream keystrokes for
intelligence, let the client draw locally. (PR #211 uses **no echo at all** —
it relies on the remote terminal's own echo, which is why it needs no
round-trips and why overlong/edited lines can desync the display.)

### Raw keystrokes in primary mode with scrollback

Yes — input routing is independent of the screen buffer. Three render styles:

- **Alternate screen** — the normal choice for a game. No scrollback,
  full-screen redraw. (`\x1b[?1049h/l` around the session.)
- **Primary screen, in-place redraw** — works; scrollback preserved above;
  write at the cursor, move back up.
- **Primary screen, appended log output** — raw input works but the display
  becomes a transcript. Fine for chat or log-following worlds.

The real cost of primary mode is that every redraw competes with scrollback
and anything else printed — which is exactly today's clear-and-repaint
behavior.

### The four dual-channel shapes

1. **Split UI on one PTY.** World grid above, command line pinned at the
   bottom. One channel; the server draws both regions; keystrokes route by
   focus. **Gains:** one connection, one screen, no client changes, commands
   and play coexist visibly. **Losses:** the server owns more UI state; world
   redraw must not clobber the command line; focus state must be visible;
   round-trip latency for command echo. **Best for:** interactive SSH where
   you type `/records` mid-game. _The web client is already this shape._
2. **Two channels on one SSH connection.** SSH multiplexes channels; one is
   the game stream, another a shell-like command line; the client routes by
   pane focus. **Gains:** clean separation; the command channel can be
   scripted, tab-completed, piped into without touching the game stream; one
   auth. **Losses:** needs client-side support (off-the-shelf SSH clients open
   one session channel; `exec_request`/`subsystem_request` are rejected today);
   more moving parts. **Best for:** power users with our own client, or
   automation that drives a world while watching the game stream.
3. **Prefix key and a command buffer.** Raw everywhere; `Ctrl-P` (or `~ Enter`)
   switches to a line buffer in an on-demand prompt. **Gains:** smallest screen
   change, game stays dominant, no focus ambiguity when hidden. **Losses:**
   hidden mode state silently eats input; needs a visible indicator; escape
   handling gets fiddly. **Best for:** minimal change to the raw default —
   i.e. the later, optional in-session toggle.
4. **Primary mode, line oriented.** Scrollback, buffered commands,
   line-oriented output; the world prints text lines, you type at a prompt.
   **Gains:** trivially scriptable — pipe in commands, parse output, tab
   complete, autocomplete, error hints, retries. What an agent or CI job
   wants. **Losses:** no real-time grid; output is a transcript.

### Mode-switch triggers

| Trigger                                      | Deterministic | Needs client support | Notes                                                          |
| -------------------------------------------- | ------------- | -------------------- | -------------------------------------------------------------- |
| SSH username prefix (`cmd:` like `explore:`) | yes           | no                   | mirrors the existing explorer pattern; picks the mode at login |
| Startup flag or subcommand                   | yes           | no                   | `id world join --command-mode` for the CLI client              |
| Prefix key (`Ctrl-P`, `~ Enter`)             | yes           | no                   | the in-session toggle; needs visible mode indicator            |
| Dedicated key (`Ctrl-]`, `Ctrl-\`)           | yes           | no                   | no collision with editing keys, undiscoverable                 |
| Type `/` in raw mode                         | no            | no                   | a game that wants to type `/` breaks — do not                  |
| Ctrl+C / interrupt                           | ambiguous     | no                   | already used for cancel; do not overload                       |
| Client capability negotiation                | yes           | yes                  | client tells the server what it supports                       |

Two distinct switches: **session start** (line mode for automation/agents vs
raw interactive for humans) and **in-session** (entering/leaving command mode
while play stays up — the prefix-key and indicator work).

### Buffered output while staying interactive

Separate axis from input routing:

- **Batching** — accumulate redraws for a few milliseconds, send one frame.
  Server-side; no client change. (Today: one full repaint per event.)
- **Buffered output to a region** — shape (1): the world redraws live while a
  bottom command region holds a buffered line the server knows about →
  server-side autocomplete without giving up the game.
- **Extended to Roc apps** — pass "the viewer is typing this partial line" to
  the program; a Roc app could render live autocomplete or a validating form.
  New guest-facing surface; needs its own design: what event carries the
  partial line, that it is **not** journaled, and its bounds.

### Why allow both — and why not

Games need raw; operators and agents need commands. The risk is only in
coexistence:

- **Bad:** auto-detect ("if it looks like text, buffer it") — non-deterministic
  and untestable.
- **Good:** explicit, one-way routing. Mode chosen at login (or by a visible
  prefix-key toggle later); mode is visible; exit returns to raw.

Costs of both (not blockers): more protocol surface and tests; mode state can
mis-route input under load; two ways to send the same message complicates
auditing.

### Recommendation

1. **Raw keystrokes stay the interactive default.** Consider moving the
   renderer to the alternate screen as a small, separate improvement.
2. **Line mode as a separate entry path**, same command grammar as the CLI:
   pipe in commands, one-offs, tab completion. No world screen required.
3. **One trigger at first:** the login-time choice (SSH username prefix or CLI
   flag). Add the prefix-key toggle only if people want to switch mid-session.
4. **Defer in-session buffered typing (the Roc partial-line surface)** until
   the line mode exists and the command grammar is known.

Do not auto-detect from content. Do not build two SSH channels until someone
has a real client for it.

## 6. What the modes are for

| Use case                               | Mode                                               | What it looks like                                                                                                               |
| -------------------------------------- | -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Play a game (`territory`, tic-tac-toe) | Raw                                                | Keystrokes go straight to the program; full-screen grid redraws                                                                  |
| Quick check ("what's the state?")      | Line                                               | `stats`, `records`, `logs` at a prompt — or a CLI one-off `id world stats`                                                       |
| Chat from a terminal without playing   | Line                                               | Plain line = chat (PR #211's implicit chat is exactly right here)                                                                |
| Read the docs of a world               | Line                                               | `/documents`, `/document guide.md` — read-only, bounded                                                                          |
| Power user mid-game                    | Raw + later toggle (shape 3) or split UI (shape 1) | Prefix key for a command, back to play                                                                                           |
| Scripting / piping                     | Line                                               | `printf 'records\nquit\n' \| id world join …`, or CLI verbs run once and exit                                                    |
| Agents and CI                          | Line                                               | Deterministic transcript output; same grammar as the CLI; later `--json` output; perfect for smoke tests against a running world |
| Fixture recording / replay             | CLI                                                | `id world record` / `replay` (on `origin/main`) driven by the same grammar                                                       |
| Accessibility / slow links             | Line                                               | Transcript output works where a 60 fps grid does not                                                                             |

Design rule that follows: **the line grammar should mirror the `id world`
subcommands** (`chat`, `records`, `stats`, `logs`, `caps`, `quit`) so the same
sentence works as a CLI one-off, a piped script line, and an SSH command.

## 7. How to add the modes (implementation plan)

Grounded in the code surveyed in §3–§4. Three steps land #211's value without
pre-committing to dual channel.

### Step 1 — Land the mode-neutral keep-stack (small PR)

- Move PR #211's document stack (`list_public_documents`,
  `read_public_document`, `DocumentError`, name filters) and
  `render_records_value` + the `"records"` reply arm into `world_ssh.rs` on
  top of `main`, keeping `main`'s `"view"` arm untouched
  (`WorldSurface::Screen.render_ansi`).
- Keep `WorldEscape` (escape swallowing) and the length/UTF-8 bounds for any
  text path; keep the injection tests.
- Fix the known gaps while moving: a **total** records render cap (not just
  per-page 384 KiB), and route document output through `send`.
- Decision to record: documents read the **server's** blob store today. If
  web and CLI should see them too, a `documents`/`document` **frame pair** (like
  `records`) belongs in `world_session.rs`; if SSH-only is fine, keep it local.
  Recommend the frame pair for parity, but it can wait.

### Step 2 — Line mode as an explicit second entry (the core PR)

- **Mode selection at login.** Mirror `explore:` (`world_ssh.rs:143-146`):
  a `cmd:` username prefix (or bare-name default staying raw) selects
  `ShellMode::Command`; `explore:` keeps the directory. For the CLI client,
  `id world join --command-mode` (or plain stdin detection by **explicit
  flag only**).
- **Where the parser sits.** Before `SshIo::recv`'s JSON wrapping
  (`world_ssh.rs:380-383`): a `WorldLineBuffer`-style accumulator converts
  lines into **real `ClientFrame`s** on the existing session — plain line →
  `chat{text}`; `/records [prefix]` → `records{prefix}`; `/input <hex>` →
  `input{data_hex}` (kept as an escape hatch for sending game input from a
  script); `/document …` → the local blob path (or the new frame);
  `/stats`, `/logs` → the same frames `WorldClient` uses. **Dispatch through
  the session driver, not around it** — that keeps the #256 projection
  (chat hidden without `history`, seed redaction) authoritative for every
  path.
- **Keep `resize` frames flowing in command mode** (PR #211's semantic bug);
  the transcript can still fit line widths to the viewport.
- **Output is transcript mode:** no `\x1b[H\x1b[2J`; `view` frames print
  appended lines (or are suppressed in favor of query replies); prompt `id> `
  or `world> ` after each reply; `error` frames keep the control-stripping
  sanitizer (`world_ssh.rs:681-686`).
- **Bounds and journaling:** line buffer ≤1024 bytes (PR #211's discard rule);
  document 64 KiB / list 512 items / 32 KiB; records total cap new. Chat and
  input lines become durable events (existing paths); **partial lines and
  mode state are never journaled** (same rule as `resize`).
- **Tests:** port PR #211's 8 tests onto the reconciled code; add one that
  asserts resize works in command mode, one that the `history` scope still
  gates chat in line-mode `records`, and the existing vt100 e2e. Keep
  `a_shell_plays_the_world_and_repaints_each_view` sending raw bytes (the
  contract difference between the modes _is_ the feature).
- **Docs:** `world_ssh.rs` module doc (username grammar: world name /
  `explore:` / `cmd:`), the arch doc record, and the Phase 11 plan gains a
  task (see §8).

### Step 3 — Optional later, in this order

1. **In-session toggle** (shape 3): prefix key intercepted in `SshIo::recv`
   only in raw mode; a visible mode line; `/quit` or the prefix key returns to
   raw. Only if users ask.
2. **Split UI** (shape 1): pinned command row + focus routing; much more
   server UI state; do it only with a concrete demand.
3. **`--json` transcript output** for agents/CI; then scripted round-trips
   need no terminal at all.
4. **Roc partial-line surface** (buffered typing to guests): new non-journaled
   notification, bounded; needs its own design record.
5. **Two SSH channels** (shape 2): would attach at `exec_request` /
   `subsystem_request` (`world_ssh.rs:209-225`); only with a real client.

### What the CLI gains along the way

The `id world join` pump (`commands/world.rs:781-798`) grows the **same
grammar** (`/records`, `/stats`, `/chat …`, `/quit`) so piped scripts and the
SSH line mode are interchangeable; one-off verbs stay as subcommands
(`id world stats NODE` already exists).

## 8. What is done / not done / planned / proposed

### Done

Portable screens (web + SSH + captured buffers); shared Roc libraries;
seven apps on the shared platform (two still on older patterns); source-only
installs; resource profiles and capability ceilings; roles and scopes;
install-seed redaction and chat visibility via one projection (`#250/#256`);
fuel metrics and budgets (`#247`); territory full house at 16% (`#248`);
journaled scheduled events (`#243`); session record/replay (`#239`); journal
fail-loud (`#244`); module provenance (`#225`); operator CLI incl. `record`,
`replay`, `measure` (on `origin/main`); Phase 11/12 plans (committed).

### Not done (open work)

- **#211** — SSH worlds: awaiting the mode decision; reconcilable per §7.
- **CI** — Firefox E2E (#229, leave alone); `id` gate ~116 min/PR (#292);
  hard-coded check list (#300); disk exhaustion (#257); world test flake
  (#190).
- **#227** — journal rollback compatibility: pre-#244 binaries truncate an
  unreadable last line; back up before rolling back.
- **#241** — LogSync: deadline aligned to 60s (#224), but the ~23s bulk
  transfer and the mid-stream reset under `--keep-going` are unexplained;
  #255 (your sweep) is the open fix attempt.
- **#242** — host-originated events (presence, capability results, ticks) not
  retained in `events_after`; `territory` fixtures cannot contain them.
- **#230** — capability ceilings: mechanism landed (#231/#233); issue still
  open — close or scope the remainder.
- **Input modes** — not in either plan; add as a Phase 11 task when landed.

### Planned — Phase 11 (foundations, F11 tasks with exit criteria)

- **CI repair:** F11.1 relay kit (**done** via #249; drift only caught in a
  full checkout); F11.2 Firefox E2E isolation (per-spec-file `id serve
--ephemeral`, browser restart per file; E2E derivation <10 min, two
  consecutive greens); F11.3 explain LogSync (bulk <10s, 50 consecutive runs
  without retry); F11.4 fast lane (median PR feedback <15 min over ten runs);
  F11.5 injected `Clock` (no `sleep` in `world.rs` tests; schedule tests <1s).
- **Cost visibility:** F11.6 fuel budgets (**done** #247, budget per example,
  fails >25% growth); F11.7 host metrics + tracing (<2% of the F11.8
  benchmark); F11.8 benchmarks (committed baselines; >20% regression reported
  non-blocking); F11.9 bytes-per-frame table (80×24 and 200×60 per example —
  the baseline F12.11 must beat).
- **Structure:** F11.10 split `world.rs` (6,120), `world_session.rs` (4,017),
  `cli.rs` (3,958) — no file >2,500 lines, moves only (`cmd_world` has a
  730 KB stack frame — box large command variants); F11.11 one client
  projection (mostly **done** #250/#256; host events #242 open); F11.12
  journal format management (future-version journal → clear refusal; `backup`
  restores byte-for-byte); F11.13 error taxonomy (every `CapError` documented,
  incl. why `schedule.at` drops).
- **Guest cost:** F11.14 cheaper `Screen` (territory view ≤40% of the fuel
  limit — row strings + style runs); F11.15 modules ≤400 KB with instantiate
  time measured (`wasm-opt -Os` + strip in each `build.sh`); F11.16 replay
  self-check (`id world verify` replays twice, a nondeterministic guest must
  fail); F11.17 a view trap must not poison the world (error screen naming the
  failing call, state kept, retry on next event); F11.18 pinned toolchain for
  checked-in artifacts (the checked-in `territory.wasm` was built by an older
  system `roc` than `nix/roc.nix` pins, costing ~4.2M fuel/view; rebuild all
  examples; `build.sh` refuses a foreign compiler; compare **behavior, never
  hashes**).

### Proposed — Phase 12 (platform flexibility, F12 tasks)

Whole-phase exit: <5 min empty-dir→joinable world; a new capability = provider

- manifest with **no `world.rs` edits**; ≥3 guest languages pass conformance;
  state-preserving upgrade with rollback; 1,000 idle / 100 active worlds per
  node (20 ev/s, p99 <50 ms); `world.toml` + module hash reproduces a world;
  redraw bandwidth <10% of a full frame; no new authority path outside named
  ceilinged providers.

F12.1 declarative `world.toml` manifest + `id world apply`/`diff`; F12.2
forking and templates; F12.3 roles v2 (builder, moderator); F12.4 provider
registry (test provider added without editing `world.rs`); F12.5 first
providers `kv`, `blob`, `http.fetch`, `notify` (bounds/refusal/replay tests;
`http.fetch` refuses unlisted hosts); F12.6 observability/audit (admin frames
→ audit lines, replayable); F12.7 ABI spec + conformance suite
(`plaza_abi_version`); F12.8 second/third guest SDKs (Rust + TS/Zig); F12.9
program upgrade with state migration; F12.10 fair scheduling and quotas
(fuel-spinning world raises no other world's p99 >10%); F12.11 delta and
binary frames; F12.12 many worlds per node; F12.13 journal replication
(tampered stream refused); F12.14 portals (both sides grant); F12.15 `id world
dev` (edit-to-run <3s; fuzz harness); F12.16 `id world doctor` (toolchain,
platform build, ceilings, profile, journal format, clock — each check with a
failing-case test); F12.17 module trust and signing; F12.18 DoS tests.

Sequencing per the plans: Phase 11 first needs F11.1, F11.4, F11.10, F11.11,
F11.12; recommended first Phase 12 slice: **F12.4, F12.1, F12.7**.

## 9. Open GitHub issues (live, 2026-10-18)

### Platform and `id`

| Item    | Title                                                                                           | State / note                                                             |
| ------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| PR #211 | feat(ssh): add chat and document viewing to worlds                                              | OPEN, CONFLICTING; four `SshIo` regions; reconcilable per §7             |
| PR #255 | fix(id): coalesce LogSync ingest batches (#241)                                                 | Your sweep; left alone                                                   |
| #227    | Define rollback compatibility for world profile journal entries                                 | Open; backup before rollback                                             |
| #229    | Stabilize shared-state failures in Firefox E2E CI                                               | Open; leave alone per instruction                                        |
| #230    | Define server and per-world world capability ceilings                                           | Open though #231/#233 landed; close or re-scope                          |
| #241    | Bulk LogSync test retries a mid-stream reset instead of explaining it                           | Open; 7 comments; deadline aligned, cause unexplained                    |
| #242    | Host-originated events are not retained in the event stream                                     | Open; blocks `territory` fixtures with presence/ticks                    |
| #190    | Serve stalls before printing node in `test_world_module_shared_over_iroh_and_web_presentations` | Open; times out after 30s waiting for the `node:` line; predates Phase 4 |

### CI infrastructure (affects the `id` PR gate)

| Item | Title                                                                             | Note                                                                                   |
| ---- | --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| #292 | The `id` workflow gates PRs for ~2 hours — split e2e out and fail fast            | Last 4 runs averaged 116 min/PR (max 137), 465 min total; every other workflow ~11 min |
| #300 | PR gate's hard-coded flake-check list can silently drift from `pkgs/id/flake.nix` | New checks get no PR coverage                                                          |
| #257 | `nix flake check` intermittently dies with 'No space left on device'              | Partially mitigated by `mkCheck` settings                                              |
| #293 | Add `timeout-minutes` caps to every job                                           |                                                                                        |
| #288 | Cache Go build cache by `go.sum` hash                                             |                                                                                        |
| #287 | Consolidate the three Jules workflows into one                                    |                                                                                        |
| #176 | Nix Playwright E2E suite fails after vendoring dependencies                       | Adjacent to #229                                                                       |

### Long tail (open, outside this report's scope)

Phase 2–5 sync/auth/tag correctness (#159–#195: membership fold, grants, tag
claims, sync batch bounds, blob gate wiring, alias display rules, …); mcpx
(#237, #290, #301); Renovate security PRs (#200–#204); a long queue of draft
editor-bot PRs (Palette/Bolt). Recorded here so the backlog is visible.

## 10. Verification and CI state

- `cargo test --all-features --no-fail-fast -- --skip serve_tests`: 1988
  passed, 0 failed, 36 suites (at `60f7a634`).
- `bun run test` in `pkgs/id/web`: 349 passed, 9 skipped. `cargo fmt --check`
  clean.
- Plain `cargo clippy` fails with E0514 (system rustc vs pinned toolchain);
  the rustup 1.97.0 cargo in a separate `CARGO_TARGET_DIR` exits 0 — not a
  code problem.
- `cargo check --all-targets --all-features` needs `web/dist` first
  (`bun install && bun run build`); full `cargo test --lib` needs
  `RUST_MIN_STACK` (`pkgs/id/.cargo/config.toml`) and must run from `pkgs/id`.
- Nix checks: `CARGO_INCREMENTAL=0`, `*_DEBUG=line-tables-only`,
  `nix flake check ./pkgs/id/ -L --keep-going`.
- **Housekeeping found during this research:** local `main` and several
  worktrees lag `origin/main` by ~145 commits. `git fetch` + fast-forward, or
  re-create worktrees from `origin/main`, before building on them.

## 11. Corrections made during this work

1. Said territory's styled view fit all eight seats. It trapped at six or
   more. Fixed in #248 (1.6M fuel full house; the plan also records 78% of
   the limit at two seats pre-#248).
2. Said rollback to a pre-change binary fails loudly. Only non-final
   unreadable entries fail loudly; a final one is truncated. Fixed for future
   binaries in #244; pre-existing ones still truncate.
3. Said join-only participants cannot read chat. They could, via the snapshot
   and `events_after`. Fixed in #256.
4. Said a join-only watcher heard chat live on `main`. It did not. The raw
   broadcast does carry the real seed and chat; #256 pins that.
5. Said PR #211 has a `/chat` command. It does not: plain lines chat
   implicitly; commands are `/input`, `/records`, `/documents`, `/document`,
   `/help`, `/quit`.
6. Said raw keystrokes in alternate mode were `main`'s default. `main` never
   uses the alternate screen (`\x1b[?1049` is absent); it clear-repaints the
   primary screen per view. Alternate screen is an improvement to make.
7. A worktree-based check claimed `record`/`replay`/`measure` do not exist.
   They exist on `origin/main` (`cli.rs:1654/1719/1741`); the worktree was
   ~145 commits stale.

## 12. Where we should go next

**Immediate (decisions + #211):**

1. Decide the mode shape — recommended: raw default + line mode as a separate
   entry (login-time choice), shared grammar (§5–§7).
2. Reconcile #211 in three steps: keep-stack PR → line-mode PR → optional
   toggle. Fix the resize-drop and total-records-cap gaps while moving.
3. Housekeeping: fast-forward worktrees to `origin/main`; close or re-scope
   #230; add an input-mode task to the Phase 11 plan (it is in neither plan).

**Next (Phase 11 order):** F11.17 (view traps), F11.18 (pinned toolchain),
F11.14 (cheaper `Screen`), F11.5 (injected clock); then F11.10 (module split —
before F11.12, and after the open PRs land since they touch those files);
F11.12/F11.13 alongside. In parallel, the CI slice: #292 (fail fast / split
e2e), #300 (check list from `flake.nix`), #257 (disk) — these multiply the
speed of everything else.

**Later:** the optional in-session toggle; `--json` transcript output for
agents; documents as a protocol frame pair; the Roc partial-line surface; then
Phase 12's first slice **F12.4 (provider registry), F12.1 (world manifest),
F12.7 (ABI conformance)**.

## 13. Transport: SSH vs Iroh, and what wraps what

### Layer model

Three layers, and only the middle one is shared protocol:

1. **Transport envelope** — SSH channel bytes, Iroh QUIC stream frames
   (length-prefixed, `frames.rs:1-60`), WebSocket messages (`web/world_ws.rs`).
2. **Session protocol** — `ClientFrame`/`ServerFrame` JSON
   (`world_session.rs:107-362`). **Identical under every transport.** SSH
   synthesizes these frames in `world_ssh.rs:351-388`; the CLI's `WorldClient`
   (`world_net.rs:157-964`) speaks them over Iroh. Unwrapping the SSH and Iroh
   envelopes yields the same protocol.
3. **Guest ABI** — `plaza_update`/`plaza_view`/`plaza_layout` Wasm exports
   (`sandbox.rs`). A separate layer (server ↔ guest program), not part of the
   client protocol.

### How `id` connects today

- `id` is **already a real network client, not local-only**: `id world
join/stats/logs/record/replay` connect to remote nodes over **Iroh (QUIC)**
  with `--addr`/`--no-relay` overrides and relay/discovery (`iroh = "1"`,
  `iroh-mdns-address-lookup`, optional `iroh-mainline-address-lookup`;
  tests disable relays and loop back).
- `id serve` is the server. **Server-to-server transport already exists**:
  iroh-docs replication for records (`world_records.rs`), world sync, LogSync.
- SSH today is a **presentation** adapter (a human terminal), not a transport
  for the frame protocol. **"SSH as transport"** — running the JSON frame
  protocol over an SSH exec channel (`id world join --via ssh`) — is a
  different, future feature for constrained networks and jump hosts.

### SSH (TCP) vs Iroh (QUIC)

|                    | SSH over TCP                                                                | Iroh (QUIC + TLS 1.3)                      |
| ------------------ | --------------------------------------------------------------------------- | ------------------------------------------ |
| Time to first byte | ~3–5 RTT (TCP + version + kex + auth)                                       | 1-RTT handshake; 0-RTT resumption          |
| Multiplexing       | all channels share one TCP connection → cross-channel head-of-line blocking | independent streams, no HOL blocking       |
| Loss behavior      | one retransmit stalls everything                                            | per-stream recovery                        |
| NAT traversal      | none — needs a reachable host, jump host, or forward                        | hole punching + relay fallback             |
| Reachability       | universal (port 22, firewalls allow it)                                     | UDP may be blocked; relay covers that case |
| Client support     | every OS ships one                                                          | requires `id`/iroh                         |
| Identity           | host keys, known_hosts                                                      | peer IDs are public keys                   |

On a clean LAN both are fine — our frame-based JSON and per-event full
repaints dominate cost anyway. On lossy or high-RTT links QUIC wins. The
interactive latency story is mostly about **render batching** (§5), not the
transport.

### Tunnel combinations

Both directions are possible; they have different uses:

- **SSH over Iroh** — the useful one: gives third-party SSH clients
  reachability to NAT'd hosts (the "SSH over a tailnet" shape).
- **Iroh over SSH** — only for networks where UDP is blocked but SSH is
  allowed (outbound port 22 only).

Since `id` speaks both natively, tunneling is mostly for non-`id` tools.

### ssh3 status (checked 2026-10-18)

Research implementation (`francoismichel/ssh3`), an IETF draft
(`draft-michel-ssh3-00`), and papers — no OpenSSH adoption. Plan on classic
SSH/TCP being supported **indefinitely**. SSH servers in `id` stay rare and
opt-in (TCP reachability and performance costs).

## 14. Presentation capability tiers (ghostty-class default, graceful degradation)

Agreed target: **ghostty-class terminals as the default supported
presentation** (kitty keyboard protocol, graphics, synchronized output), with
a **degraded experience** when those are absent or the frame rate cannot be
met.

- **Detection is in-band**, so it works identically over SSH and Iroh:
  capability negotiation via escape sequences (`XTGETTCAP`, `CSI ? u`,
  `DECRQM`) from the terminal. The "client capability negotiation" trigger
  (§5) applies to presentation the same way it does to input.
- **Tier 1 — ghostty, kitty, wezterm, …:** kitty keyboard protocol →
  unambiguous Escape vs Alt, key events beyond the legacy byte stream;
  kitty graphics or sixel → images/sprites; synchronized output (DEC 2026)
  → flicker-free repaints; high fps feasible.
- **Tier 2 — xterm, GNOME Terminal, Windows Terminal, macOS Terminal:** no
  kitty keyboard → escape-timeout disambiguation (the guest's `Key.parse`
  heuristics today); cell grid only; **server-side redraw batching** keeps fps
  acceptable; an incremental/diff renderer helps most here (today every view
  is a full `\x1b[H\x1b[2J` repaint).
- **Tier 3 — dumb or slow:** the line mode (§5-§7) _is_ the degraded
  presentation — transcript output, no grid.

Every tier benefits from the batching work in §5; the kitty keyboard protocol
would also improve the **raw input** path, not just presentation.

## 15. Beyond the terminal: native clients and client-side capabilities (long-term)

The direction set for the platform after the terminal story settles:

- **Native client shells** — game engines (Bevy), Qt, C#/.NET, Unity,
  Electron, Windows desktop apps — implementing the same screen/input
  contract for things a terminal cannot do even with kitty protocol and high
  fps: 3D, audio, native widgets. The portable surface contract and the ABI
  conformance suite (F12.7) are what make a shell swappable.
- **Client-side capabilities** — some apps need the client's file system,
  GPU, and network beyond the p2p link. This extends the capability model
  (scopes/roles/ceilings; F12.4 provider registry) to the client side: named,
  granted, ceilinged providers for _local_ resources, keeping the Phase 12
  rule "no new authority path outside named ceilinged providers."
- **Multi-server, multi-client** — clients connect to several worlds/servers
  and to other clients at once; `id` as a client will often be a server
  simultaneously (configuration-dependent). **Client-only mode is always
  supported** — to keep web-app load cheap and to serve SSH-only clients.
- **SSH servers in `id`** — rare and opt-in (TCP reachability and performance
  costs; classic SSH supported indefinitely even if ssh3 ever matures).
- **Placement:** this is post-Phase-12 material. F12.7 (ABI conformance),
  F12.14 (portals) and F12.12 (many worlds per node) are the stepping stones;
  a "client shells + client capability providers" sketch belongs in a future
  Phase 13 plan.

## 16. Open questions for you

1. ~~Mode shape~~ **Decided:** raw default + separate line-mode entry, shared
   grammar. All variations (raw/buffered × primary/alternate, split UI, dual
   channel) land eventually, each for a different context (§13–§15).
2. **Line-mode chat UX** — keep PR #211's implicit plain-line-chat in line
   mode (recommended), or require explicit `/chat` everywhere for symmetry?
3. **Documents** — SSH-local blob read (fast) or a `documents` frame pair for
   web/CLI parity (cleaner)?
4. **In-session toggle** — first step or later?
5. **Priority of "SSH as transport"** (`id world join --via ssh` over an exec
   channel) vs Iroh-only for now? SSH-over-Iroh reachability (§13) — worth a
   Phase 12/13 task?
6. **Presentation tier detection** — do the kitty-protocol probe and redraw
   batching land with #211's reconciliation, or as a separate follow-up?
7. These doc files are uncommitted. Commit them to
   `platform-world-apps-migration` (or a fresh branch off `origin/main`)?

---

## References

- Companion transcription:
  [`doc/2026-10-18T00-00-00Z_reference_platform_state/`](../2026-10-18T00-00-00Z_reference_platform_state/2026-10-18T00-00-00Z_reference_platform_state.md)
- Plans: `.opencode/plans/phase-11-foundations.md`,
  `phase-12-platform-flexibility.md`, `phase-9-portable-platform.md`
  (committed on `origin/main`)
- Key code: `pkgs/id/src/world_ssh.rs` (SSH I/O, `SshIo`, `SshInputFrame`,
  explorer `Typing`), `pkgs/id/src/world_session.rs` (`ClientFrame`:107-258,
  `ServerFrame`:264-362, dispatch 1103-2468), `pkgs/id/src/world.rs`
  (events, `events_after`, journal), `pkgs/id/src/sandbox.rs` (`plaza_update`,
  UTF-8 input rule), `pkgs/id/src/world_net.rs` (`WorldClient`),
  `pkgs/id/src/directory_view.rs` (`parse_line`, `HELP`),
  `pkgs/id/src/commands/world.rs` (CLI verbs, `join` pump),
  `pkgs/id/src/world_presentation.rs` (`render_ansi`).
- Guest surface: `pkgs/id/examples/roc-world/` (`platform.roc`, `Screen.roc`,
  `Key.roc`, `RocJson.roc`, `RocGrid.roc`, `RocCap.roc`, `RocText.roc`),
  `pkgs/id/examples/territory/` etc.
- PR #211 branch `origin/platform-ssh-world` (merge-base `292d3c7a`).
- Transport: `pkgs/id/src/frames.rs` (length-prefixed JSON over Iroh),
  `pkgs/id/src/web/world_ws.rs` (WebSocket), `pkgs/id/Cargo.toml` (`iroh = "1"`,
  mdns/mainline address lookup). ssh3 status: `francoismichel/ssh3`,
  IETF `draft-michel-ssh3-00` (research stage, checked 2026-10-18).
- Decision records in `doc/`: `2026-10-18T04-00-00Z_fix_world_chat_visibility/`,
  `2026-10-17T16-30-00Z_fix_territory_view_fuel/`,
  `2026-10-17T15-30-00Z_feature_world_perf_budgets/`,
  `2026-10-17T15-00-00Z_architecture_next_stage_platform/`,
  `2026-10-16T13-00-00Z_feature_world_apps_structured_migration/`
- Live state checked 2026-10-18: `origin/main` `60f7a634`; branch
  `platform-world-apps-migration` `52c662f2` (stale vs `origin/main`);
  research notes `/tmp/opencode/report-notes-2026-10-18.md`.
