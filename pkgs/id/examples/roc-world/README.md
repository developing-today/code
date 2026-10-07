# Roc world platform

The platform (`platform.roc` + the freestanding Zig host adapter) that turns a
pure Roc program into a `id` world module. Every example app in this directory
references this one copy:

```
examples/roc-world/       platform + host adapter (this directory)
examples/roc-counter/     app: a big red button
examples/tic-tac-toe/     app: a two-player game
```

## The module ABI

A world program provides these functions:

```
init    : U64 -> model                    # seed from the host
update  : model, U64, Str -> model        # second argument is the participant id
view    : model, Str -> Str               # viewer id as JSON/string
records : model -> Str                    # optional JSON object projection
snapshot : model -> Str                   # state as text, for journal checkpoints
restore : Str -> model                    # inverse of snapshot
```

`update` and `view` are pure: they read the model and their arguments and
return a new model or a string. They cannot perform effects — the platform
requires no `Task`, and modules are compiled to an import-free Wasm module.
`records` is optional: when present it must return a JSON object of string
keys to JSON values, which the host validates and mirrors into the world's
iroh-docs document.

`snapshot`/`restore` let the host trim a world's journal. Without them a
world restarts by replaying every input it ever received; with them the host
periodically replaces the journal's history by a checkpoint holding
`snapshot(model)` and restarts from `restore(checkpoint)` plus the inputs that
followed. `restore(snapshot(m))` must behave exactly like `m`: before trimming,
the host restores the snapshot into a probe instance and refuses to trim unless
the probe re-snapshots to identical text and publishes identical records.
`restore` should be total (return a sensible model for text it cannot parse).
Both exports are optional at the Wasm level, so older modules keep working and
keep their full journals.

The host passes the id of the participant who sent an event as the second
argument of `update`, so programs can implement turn-taking or per-player
rules without the host knowing anything about the game.

## Regenerating the ABI bindings

```sh
ROC=/path/to/roc ZIG=zig ./build-host.sh      # rebuilds targets/wasm32/host.wasm
roc glue /path/to/roc/src/glue/src/ZigGlue.roc ./out platform.roc
cp ./out/roc_platform_abi.zig host/src/roc_platform_abi.zig
```

Keep the Roc compiler pinned to the nightly the bindings were generated with
(see the top of `host/src/roc_platform_abi.zig`); the ABI is pre-1.0 and
changes between nightlies.
