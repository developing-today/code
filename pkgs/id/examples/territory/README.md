# Territory

A multiplayer world for up to eight players on a 16x8 grid. Each player who
acts gets a seat (a mark A–H and a cursor). Move with arrows or `hjkl`, claim
the cell under the cursor with enter or space. The first claim may go anywhere
empty; after that a player may only claim empty cells next to their own
territory.

Seats persist. A seat keeps its cells when its player leaves, and a player who
rejoins under the same name takes the seat back. The journal replays the grid
and seats after a restart; everyone starts offline.

## Build

```bash
./build.sh
```

Requires `roc` (or `ROC=...`) and `wasm-tools` (or `WASM_TOOLS=...`). The build
passes `--wasm-memory=8454144`: Roc's default initial memory is 64 MiB, above
the sandbox's 16 MiB limit, and the module traps on instantiation without it.

`territory.wasm` is checked in. Rebuild it after changing `main.roc`.

## Run

The module path must be absolute, because `serve` changes into its data
directory.

```bash
id serve --data-dir ./data --world --world-admin-token "$ADMIN" \
  --world-module "$PWD/examples/territory/territory.wasm" \
  --world-cap players --world-ssh-port 2223 --web --port 8787
```

`--world-cap players` grants the presence subscription the world asks for, so
joins and leaves name their seats. The server prints its node ID and its
`local_addr=0.0.0.0:<port>`.

Compiling this source on a host (`id world compile`) also needs `Key` granted:
`id world caps NODE --admin-token "$ADMIN" --grant Key`.

Mint an invite and join from another machine:

```bash
id world invite NODE --admin-token "$ADMIN" --name ann
ID_WORLD_CAPABILITY=1.<token> id world join NODE
```

On one machine, point the client at the server's port and skip relays:

```bash
ID_WORLD_CAPABILITY=... id world join NODE --addr 127.0.0.1:<port> --no-relay
```

The CLI join prints server frames as JSON lines. Send input as hex: `/input 6c`
moves right (`l`), `/input 20` claims (space).

## Presentations

- **CLI:** `id world join` as above.
- **SSH:** `ssh -p 2223 lobby@HOST`. The password is the guest capability.
  The screen is the plain-text view.
- **Web:** `http://HOST:8787/world`, paste the capability.
- **Records:** `id world records NODE --capability ...` prints the JSON
  projection (grid, seats, online flags). `id world mirror` replicates it over
  Iroh with an iroh-docs replica.
