# Roc chess world

A chess world program for `id`, sharing the platform in
`../../examples/roc-world` (slice 2 of #14862). The human plays white
against a deliberately weak AI playing black, under full chess
rules: legal move generation includes castling, en passant, and
promotion, and pseudo-legal moves are filtered by king safety, so
pins, discovered checks, and moving into check are all handled by
the one filter. Checkmate and stalemate end the game; a side to move
with no legal move loses if it is in check and draws if it is not.

The AI is weak on purpose — that is the acceptance bar, not a bug.
It looks exactly one ply ahead and scores a move only by the
material it captures this turn, plus the promotion bonus and a
whisper for advancing a pawn. Uniform jitter of 0..40 rides on
every score, and one draw in four ignores the scores entirely and
plays a uniformly random legal move. It never checks whether the
piece it just moved can be recaptured, never looks at the
opponent's threats, and has no opening book. A beginner is
genuinely hampered; anyone good at chess — plenty of children —
beats it easily.

Everything is a pure function of the model and the event:
keystrokes are journaled like any other world input, the AI replies
inside the same `update` that completes the human's turn, and its
randomness comes from a xorshift PRNG carried in the model, so the
journal replays a game exactly. `update` routes host events by
their JSON `cap` field and everything else through `Key.parse`.
The game is turn-based and subscribes to nothing: `wants` is `{}`.

Controls: arrow keys or WASD move the cursor (brackets; green marks
the selected piece, yellow the cursor). Enter or Space selects a
piece and confirms a landing square; confirming on the selected
piece again deselects it. Confirming a pawn move onto the last rank
opens the promotion choice: Left/Right (or A/D) cycle queen, rook,
bishop, knight, and Enter confirms the promotion. Enter (or R)
starts a new game after a win, a loss, or a draw.

The board renders as text: uppercase letters are white pieces,
lowercase are black, a dot is an empty dark square.

The frame is data: `view`/`layout` render a `Screen` document, and
the shell paints it in whichever presentation mode the session
negotiated — alternate, primary (cursor moves, erase/rewrite), or
the low-write text mode from the #14861 harness slices. The game
never changes when the mode does.

Records are published as JSON: `{"white": 16, "black": 15,
"phase": "playing", "plies": 7}`, so `id world records` and
`id world mirror` show the game state and can replicate it
peer-to-peer. Snapshots are the whole model as one JSON object —
board, castling rights, and en-passant target included — and
`restore` reads it back with per-field defaults and clamps; a board
that is not exactly 64 piece bytes falls back to the opening
position instead of trapping.

Later slices of #14862 build on this one: id-network multiplayer
(one binary hosts and the other joins, or a centrally started
server both players join), and the three-machine demonstration
(GitHub Actions serves, Blacksmith plays white, the laptop plays
black, one compiled artifact shared by all three).

## Build

```sh
cd program
ROC=/path/to/roc ZIG=zig ./build.sh
```

## Run

```sh
id serve --world --world-admin-token "$WORLD_ADMIN" --world-module program/chess.wasm
id world invite HOST_NODE --admin-token "$WORLD_ADMIN" --name Ada
id world join HOST_NODE --capability TOKEN       # then: arrows/WASD, Enter to select and move
```
