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

A world program provides four functions:

```
init    : U64 -> model                    # seed from the host
update  : model, U64, Str -> model        # second argument is the participant id
view    : model, Str -> Str               # viewer id as JSON/string
records : model -> Str                    # optional JSON object projection
```

`update` and `view` are pure: they read the model and their arguments and
return a new model or a string. They cannot perform effects — the platform
requires no `Task`, and modules are compiled to an import-free Wasm module.
`records` is optional: when present it must return a JSON object of string
keys to JSON values, which the host validates and mirrors into the world's
iroh-docs document.

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
