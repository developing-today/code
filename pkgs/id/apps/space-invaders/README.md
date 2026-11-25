# Roc space-invaders world

A single-player Space Invaders world program for `id`, sharing the
platform in `../../examples/roc-world` (slice 4 of #14861, #14938).

A 4x8 alien formation marches across a 40x20 field, drops a row at
each edge, and drops bombs from its bottom row. The player ship moves
along the bottom, fires one bullet at a time, and has three lives.
Clearing the formation starts the next wave one row lower; losing
every life — or letting the formation reach the ground row — ends
the game.

The game advances on the host's `time.tick:1000` subscription (the
fastest tick the default world limits arm). Everything is a pure
function of the model and the event: keystrokes and ticks are
journaled like any other world input, and the bomb pattern comes
from a xorshift PRNG carried in the model, so the journal replays a
game exactly. `update` routes host events by their JSON `cap` field
and everything else through `Key.parse`.

Controls: Left/Right arrows or A/D move, Space or Up fires, Enter
(or R) starts the next wave after a clear and retries after a loss.

The frame is data: `view`/`layout` render a `Screen` document, and
the shell paints it in whichever presentation mode the session
negotiated — alternate screen, primary, or low-write text
(#14936/#14937). The game never changes when the mode does.

Records are published as JSON: `{"score": 120, "lives": 3,
"wave": 1, "phase": "playing", "aliens_left": 17, "ticks": 42}`,
so `id world records` and `id world mirror` show the game state
and can replicate it peer-to-peer. Snapshots are the same model as
one JSON object; `restore` reads it back with per-field defaults
and clamps, so a checkpoint it cannot parse restarts sensibly
instead of trapping.

## Build

```sh
cd program
ROC=/path/to/roc ZIG=zig ./build.sh
```

## Run

```sh
id serve --world --world-admin-token "$WORLD_ADMIN" --world-module program/space-invaders.wasm
id world invite HOST_NODE --admin-token "$WORLD_ADMIN" --name Ada
id world join HOST_NODE --capability TOKEN       # then: arrow keys / A/D, Space to fire
```

## Verification + artifact

The `id roc game` workflow (`.github/workflows/id-roc-game.yml`,
#14939) builds the game with the pinned toolchain, runs the Roc
module tests and this program's expects — including the scripted
playthrough at the bottom of `program/main.roc` — and drives the
built module through the plaza ABI with the same script
(`smoke/drive.mjs`; run it after a build with `smoke/smoke.sh`).
Each green run publishes this directory — including
`program/space-invaders.wasm` — as the `space-invaders-<sha>`
workflow artifact, so other machines pull the binary instead of
rebuilding.
