# Roc checkers world

A checkers world program for `id`, sharing the platform in
`../../examples/roc-world` (slice 1 of #14862). The human plays red
against a deliberately weak AI playing black, under full
English-draughts rules: captures are forced, multi-jumps are single
moves, a man that reaches the far row is crowned and the turn ends,
and kings move and capture in both directions. A side with no pieces
or no legal move loses.

The AI is weak on purpose — that is the acceptance bar, not a bug.
It looks exactly one ply ahead and scores a move only by the material
it captures this turn, plus a small bonus for crowning and a whisper
for advancing a man. Uniform jitter of 0..40 rides on every score,
and one draw in four ignores the scores entirely and plays a
uniformly random legal move. It never checks whether the piece it
just moved can be recaptured, never looks at the opponent's threats,
and has no opening book. A beginner is genuinely hampered; anyone
good at checkers — plenty of 6-year-olds — beats it easily.

Everything is a pure function of the model and the event: keystrokes
are journaled like any other world input, the AI replies inside the
same `update` that completes the human's turn, and its randomness
comes from a xorshift PRNG carried in the model, so the journal
replays a game exactly. `update` routes host events by their JSON
`cap` field and everything else through `Key.parse`. The game is
turn-based and subscribes to nothing: `wants` is `{}`.

Controls: arrow keys or WASD move the cursor (brackets; green marks
the selected piece, yellow the cursor). Enter or Space selects a
piece and confirms a landing square; confirming on the selected
piece again deselects it. A piece with a jump must take it, and a
multi-jump must be finished — mid-chain the selection is locked to
the jumping piece. Enter (or R) starts a new game after a win or a
loss.

The board renders as text: `o`/`O` are red men/kings, `x`/`X` are
black men/kings, a dot is an empty dark square.

The frame is data: `view`/`layout` render a `Screen` document, and
the shell paints it in whichever presentation mode the session
negotiated — alternate, primary (cursor moves, erase/rewrite), or
the low-write text mode from the #14861 harness slices. The game
never changes when the mode does.

Records are published as JSON: `{"red": 11, "black": 12,
"phase": "playing", "plies": 7}`, so `id world records` and
`id world mirror` show the game state and can replicate it
peer-to-peer. Snapshots are the whole model as one JSON object;
`restore` reads it back with per-field defaults and clamps, and a
board that is not exactly 64 board bytes falls back to the opening
position instead of trapping.

Two players (#15111): each colour has a seat holding the participant
that plays it. The first participant to type takes red; a second
participant takes black by pressing J, which retires the AI (the
bottom line says so while black's seat is open). With black's seat
open the AI plays black exactly as in single-player. Turn enforcement
lives in the world program's `update`, not in the terminal: a key from
anyone who does not hold the seat of the side to move — the other
player out of turn, or a spectator — is dropped and the model is
unchanged, and the expects prove it. Once the game is over either
seated player can start a new one, and the seats carry over. Records
add `turn`, `red_seat` and `black_seat`; snapshots from before
seats restore with both seats open and red to move.

Later slices of #14862 build on this one: chess on the same harness,
id-network multiplayer (one binary hosts and the other joins, or a
centrally started server both players join), and the three-machine
demonstration (GitHub Actions serves, Blacksmith plays red, the
laptop plays black, one compiled artifact shared by all three).

## Build

```sh
cd program
ROC=/path/to/roc ZIG=zig ./build.sh
```

## Run

```sh
id serve --world --world-admin-token "$WORLD_ADMIN" --world-module program/checkers.wasm
id world invite HOST_NODE --admin-token "$WORLD_ADMIN" --name Ada
id world join HOST_NODE --capability TOKEN       # then: arrows/WASD, Enter to select and move
# a second player, invited the same way, joins and presses J to play black
```
