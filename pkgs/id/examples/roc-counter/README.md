# Roc counter world

A tiny pure Roc world program used by `id`'s multiplayer service. The host
owns the authoritative state; guests send `inc`, and every participant sees
the returned `count=N` presentation over either Iroh or WebSocket. It also
demonstrates the `records` export (structured data mirroring).

## Build

Requires Zig 0.16 and the Roc nightly that generated `host/src/roc_platform_abi.zig`
(currently nightly-2026-10-04-130536d). From this directory:

```sh
ROC=/path/to/roc ZIG=zig ./build.sh
```

The checked-in `host/src/roc_platform_abi.zig` is generated Roc ABI code; keep
the compiler build pinned when regenerating. `build.sh` builds the freestanding
Zig host, compiles `main.roc` using the `platform.roc` wasm32 target, then
validates the result with `wasm-tools`.

## Run a hosted world

```sh
id serve --world --world-admin-token "$WORLD_ADMIN" --world-module counter.wasm
```

Or leave out `--world-module` and install a compiled module while the world is
running:

```sh
id world install HOST_NODE counter.wasm --admin-token "$WORLD_ADMIN" --addr HOST_IP:PORT
```

`id world invite HOST_NODE --admin-token "$WORLD_ADMIN" --name Ada` prints a
guest capability. Join with `id world join HOST_NODE --capability TOKEN`; type
`/input 696e63` to increment and ordinary lines to chat. The browser view at
`http://localhost:PORT/world` uses the same session protocol.

The program also publishes structured records (`records : Model -> Str`):

```sh
id world records HOST_NODE --capability TOKEN      # {"count": N}
id world mirror HOST_NODE --capability TOKEN --follow   # live iroh-docs replica
```

`records` reads the host's current set; `mirror` replicates it peer-to-peer
over iroh-docs, so a replica keeps receiving changes. Records are bounded
(<= 4096 keys, keys <= 256 bytes without control characters, values <= 8 KiB,
1 MiB total) and are recomputed from the program, never stored as a second
source of truth.

The module has no Wasm imports. The host applies fuel, memory, module-size and
message-size limits. A trap fails closed: that world program is marked
unhealthy and must be reinstalled. Module and live state currently reset when
the host process exits.
