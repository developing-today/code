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
The game
is turn-based and subscribes only to `players`: `wants` is
`{"v":1,"subscribe":["players"],"requests":[]}`.

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
"phase": "playing", "plies": 7, "turn": "black", "white_player": "ann",
"black_player": "AI"}`, so `id world records` and
`id world mirror` show the game state and can replicate it
peer-to-peer. Snapshots are the whole model as one JSON object —
board, castling rights, and en-passant target included — and
`restore` reads it back with per-field defaults and clamps; a board
that is not exactly 64 piece bytes falls back to the opening
position instead of trapping.

## Two players

The world holds two seats, white and black. A participant's first
key takes the first open seat, white before black; with both seats
held, a third participant only watches. Turn order is enforced by the
world program, not the screen: keys from the side not to move, from a
watcher, or from the host change nothing, and a key that ends a turn
ends that player's input, so the rest of one event cannot move for the
opponent. An open seat is played by the weak AI, so one player alone
gets the single-player game above unchanged. When a seated player
leaves (a `players` presence event from the host), their seat opens,
the board says so, and the AI moves for that side at once if it was
their turn. The bottom row names who plays each side (`W:ann  B:AI`).

Two separate id processes play a whole game over the id network
with `pkgs/id/scripts/board-games-two-player.py` (#15111). It runs
both topologies, a host whose own player joins from the host's data
directory and a central server that two players join, and plays
checkers and chess to the end. After every move both players must
read the same records (seats, turn, board, plies). It also checks
that out-of-turn and watcher keys change nothing, that a player who
leaves hands the seat to the AI, and that a restarted host restores
the same records from its journal:

```sh
cd pkgs/id && python3 scripts/board-games-two-player.py --id-bin target/debug/id \
  --checkers-wasm apps/checkers/program/checkers.wasm \
  --chess-wasm apps/chess/program/chess.wasm
```

A later slice of #14862 adds the three-machine demonstration.

### Reuse a compiled board-game artifact

The `id roc game` workflow publishes both games in one
`board-games-<commit>` artifact. To verify an existing artifact again
without rebuilding either game (or the arcade game), manually dispatch
that workflow with **both** `source_run_id` and `source_artifact_id` from
its original successful build. Leave both blank for a fresh build.
For example, using IDs from a successful producer run:

```sh
gh workflow run id-roc-game.yml --repo developing-today/code --ref main \
  -f source_run_id=RUN_ID -f source_artifact_id=ARTIFACT_ID
```

The reuse run skips both compiler jobs, downloads that exact artifact
from the selected run, and records its source and wasm hashes in the
consumer summary. An expired/deleted artifact or a mismatched run/ID
fails the download; there is no fallback rebuild. This is only an
artifact-reuse check, not a live game or a server restart. The artifact
still contains wasm bundles, not an `id` executable; runtime provisioning
and the Actions/Blacksmith/laptop game remain outstanding under #15112.

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
