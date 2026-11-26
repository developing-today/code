# #14861: a terminal arcade game on a three-mode harness

Status: decided. Implementation follows in the slices below. Follow-on:
#14862 (checkers and chess) builds on the harness without changing its
core.

## What the issue asks for

- A reusable terminal harness with three display modes, chosen at run
  time: **alternate** (alternate screen buffer), **primary** (primary
  screen, cursor addressing, erase and rewrite in place) and **text**
  (low-write plain lines, only what changed).
- An arcade game in the falling-formation / tunnelling class, written in
  Roc on the id platform, playable start to finish in all three modes
  with keyboard input and a status display that stays terse in text
  mode.
- Remote verification only: builds and playability smoke tests run in a
  GitHub Actions job, and the built binaries are published as a workflow
  artifact.

## What already exists

- **Presenter tiers** (`crates/id-worlds/src/world_terminal.rs`):
  `TierPresenter` renders a `ScreenSurface` as a full ANSI render and
  then row diffs (Tier 2, and Tier 1 with synchronized output), or as a
  Tier 3 `TextDelta` that writes only the changed lines with no escape
  sequences. The tier comes from the startup probe (`probe_local_tty`,
  `ProbeOutcome::tier`).
- **Raw SSH sessions** (`crates/id-worlds/src/world_ssh.rs`): a PTY
  shell probes the terminal, enters the alternate screen
  (`ALT_SCREEN_ENTER`) before the first frame, sends each keystroke to
  the world as an `input` frame and leaves the alternate screen on every
  exit path. Command mode (`cmd:` login prefix) uses neither.
- **Roc platform** (`examples/roc-world`): pure, import-free guests with
  `init`, `update`, `view`, `records`, `wants`, `snapshot`, `restore`;
  `Screen.roc` builds the portable screen document; `Key.roc` parses
  raw key bytes into `KeyEvent`s; `wants` can subscribe to
  `time.tick:<ms>` (host events from participant 0), floored by the
  world's `min_tick_ms` (default 1000 ms). `build-native.sh` builds the
  static x64musl native worker the server can run instead of Wasm.
- **Shape of a game**: `examples/roc-burrow` is a turn-based tunnelling
  world (dig, walk, exit) with a semantic text grid.

What is missing is a way to pick the mode, a primary-screen mode at all,
a guest-side module that turns keys and ticks into a game loop with a
fixed layout, and a real-time game.

## Decisions

### D1. The harness has a guest half and a host half

Roc guests are pure and import-free, and the ABI lets only the host
write terminal bytes: the SSH presenter emits trusted cursor and colour
codes and nothing a guest supplies. So "a Roc harness with three display
modes" is built as two halves with one contract between them:

- **Guest half, `examples/roc-world/Harness.roc`:** a documented,
  game-agnostic module. It maps key events to actions, turns
  `time.tick` host events into steps, builds the `wants` subscription,
  owns the title / playing / paused / over phases, and lays out the
  screen by the layout contract (D5). A game supplies its state and a
  few functions; it never touches escape codes or modes.
- **Host half, the display mode:** the presenter writes the guest's
  screen in the mode the player chose (D2, D3). The same module bytes
  run in every mode, so a mode can never break a game, and a game
  cannot pretend to be a mode.

This keeps the security rule intact and makes the modes available to
every world, not only Roc ones.

### D2. Three modes on the existing presenter

| Mode        | Screen buffer   | Presenter                   | On exit                                                        |
| ----------- | --------------- | --------------------------- | -------------------------------------------------------------- |
| `alternate` | `?1049h` / `l`  | Tier 2 (Tier 1 when probed) | leave the alternate screen; the shell is as it was             |
| `primary`   | primary         | Tier 2 (Tier 1 when probed) | cursor below the last frame row; the final frame stays on show |
| `text`      | primary, no CUP | Tier 3 `TextDelta`          | nothing to restore                                             |
| `auto`      | as today        | as the probe decides        | as today                                                       |

- `alternate` and `primary` both need cursor addressing. Choosing one is
  the player's statement that the terminal has it, so the presenter
  uses at least Tier 2 even when the probe could not confirm it. The
  probe still decides the Tier 1 extras (synchronized output,
  graphics), which are only ever used on positive evidence.
- `primary` starts with home plus erase-display (`CSI H`, `CSI 2 J`) in
  the primary buffer, then uses the same row diffs. On exit it moves
  the cursor to the row after the frame and restores it, so the
  finished game stays in the scrollback.
- Both cursor modes hide the cursor (`CSI ? 25 l`) for the session and
  show it again on every exit path, next to the existing alternate
  screen and kitty-keyboard restores.
- `text` forces Tier 3 even on a capable terminal: no escape sequence
  is ever written, and after the first view only changed lines are.
- `auto` keeps today's behaviour exactly (alternate screen, probed
  tier), so nothing changes for anyone who does not ask.

### D3. Choosing the mode at run time

- **SSH:** the client sends `ID_DISPLAY=alternate|primary|text|auto`
  as an SSH environment request (`ssh -o SetEnv=ID_DISPLAY=text …`;
  `id world ssh-tunnel` passes it through untouched). The server reads
  it before the shell opens. An unknown value refuses the join with a
  one-line message naming the four values, rather than guessing. No
  login prefix is added, so the mode composes with `explore:`.
- Command mode (`cmd:`) is unchanged and ignores `ID_DISPLAY`.
- **`id world join`:** its pump reads whole stdin lines, which cannot
  drive an arcade game, so it gets no `--display` flag in this issue.
  SSH (directly or through `id world ssh-tunnel`) is the way to play.

### D4. Time and input

- The harness subscribes to one `time.tick` interval, the game's step
  period, and never asks for less than the world's floor. The game is
  designed for the default 1000 ms floor: the player's own moves apply
  on the keystroke, and only the formation advances on a tick, so it
  plays well at one step a second. Lowering the floor for a world is an
  operator setting and not part of this issue.
- Keys, through `Key.parse_events`: arrows, `h j k l` and `w a s d`
  move; space acts (fire, dig); `p` pauses; `r` restarts after the end;
  `q` returns to the title. Unknown keys are ignored. Host events other
  than the tick are ignored, as in every example.
- Every participant shares one game (a shared world); any player's keys
  drive it. This needs no seats and suits a first version.

### D5. The layout contract

The guest does not know the mode, so the layout must read well in all
three:

- The board is at most 24 columns by 12 rows plus a two-column row
  gutter, and the whole screen fits 40 by 16, well inside 80 by 24.
- Each board row starts with a fixed label (`a` to `l`), so a changed
  line in text mode says where it belongs. The board games in #14862
  need rank labels anyway.
- One status line, always the same row and shape:
  `score 120  lives 2  wave 1`. In text mode it is written only when
  it changes.
- One message line under it for the phase (`press space to start`,
  `paused`, `cleared! r to play again`, `game over, r to retry`).
- Glyphs are plain ASCII, so text mode on any terminal shows the same
  board.

### D6. The game: Sporestorm

An original game in the falling-formation class: a formation of spores
drifts across the field and sinks one row each time it reaches an edge,
and the player's lantern on the bottom row moves left and right and
fires a beam straight up.

- Field 20 by 12. Formation 3 rows by 6 spores. One beam on the field at
  a time. Spores drop seeds from the lowest spore of a column chosen by
  a splitmix generator seeded from `init`, so a run is deterministic
  for a given seed and replays exactly from the journal.
- Score 10 per spore; three lives; a seed reaching the lantern costs a
  life.
- Clear the formation to win; lose when lives reach zero or the
  formation reaches the lantern's row. Then `r` starts again.
- `records`: phase, score, lives, wave, spores left, outcome.
  `snapshot` and `restore` cover the whole state, and `restore` rejects
  any snapshot that breaks an invariant.
- Glyphs: `A` lantern, `o` spore, `|` beam, `.` seed, space for empty.

## Slices

1. **This record.**
2. **Host display modes:** a `DisplayMode` in `world_terminal`, the
   `ID_DISPLAY` environment request and the D2 behaviour in
   `world_ssh`, with tests for each mode's byte stream (alternate
   brackets with `?1049`, primary never does and leaves the frame,
   text never writes an escape byte) and the unknown-value refusal.
   Documented in the book's world SSH page.
3. **`Harness.roc`:** the guest half (D1, D4, D5) with its README
   section and a small example in the module docs, checked with Roc's
   `expect` tests.
4. **Sporestorm:** `examples/roc-sporestorm` (game, build script,
   README), plus an ignored id-worlds host test that runs the built
   module with a test clock through a full win and a full loss.
5. **Remote verification and artifact:** a `workflow_dispatch` job on a
   GitHub-hosted runner that builds the module with the pinned Roc
   toolchain, builds the native x64musl worker, runs the host test and
   a scripted raw SSH session in each of the three modes, and uploads
   `sporestorm.wasm`, the native worker and the three session
   transcripts as the workflow artifact.

#14862 then adds its boards on top of slices 2 and 3; it needs no tick,
and the harness makes the tick optional.

## Not in scope

- A `--display` flag or raw-key pump for `id world join`.
- Seats, turns or per-player games.
- Lowering the tick floor, sound, colour themes or graphics.
- Any change to the guest ABI or the screen document format.
