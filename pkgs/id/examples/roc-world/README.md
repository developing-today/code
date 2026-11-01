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

The host/guest boundary is specified in [ABI v1](ABI.md), including the
`plaza_*` exports, buffer ownership, and capability protocol.

A world program provides these functions:

```
init    : U64 -> model                    # seed from the host
update  : model, U64, Str -> model        # second argument is the participant id
view    : model -> Str                    # legacy text or a portable screen document
records : model -> Str                    # optional JSON object projection
wants : model -> Str                      # optional: subscriptions and requests
snapshot : model -> Str                   # state as text, for journal checkpoints
restore : Str -> model                    # inverse of snapshot
```

`update` and `view` are pure: they read the model and their arguments and
return a new model or a string. They cannot perform effects — the platform
requires no `Task`, and modules are compiled to an import-free Wasm module.
`records` is optional: when present it must return a JSON object of string
keys to JSON values, which the host validates and mirrors into the world's
iroh-docs document.

`view` may return ordinary UTF-8 text, or a versioned portable screen JSON
document produced by `Screen.render_json`. The host validates the grid (at
most 120×50 cells), Unicode display widths, palette indices, and style ranges.
The SSH presenter emits only trusted cursor/colour escape codes, while the web
presenter creates text-only DOM nodes with fixed palette styles. Never put
terminal escape sequences, HTML, or CSS in a guest view. `Screen.render_plain`
remains useful for older clients and text-only output.

In the browser, focus the world screen and use the keyboard to send input. The
web adapter encodes common keys as terminal bytes (UTF-8 text, control keys,
and standard navigation sequences), matching SSH PTY input and the existing
`layout` hook. Apps define `layout : model, U64, U64 -> Str` next to `view`;
sessions with a viewport render through it (no journal, no sequence change),
and cropping remains the fallback for programs that return `view(model)`.
`Screen.fill` and `Screen.rule` cover rect fills and separators.

`Key.parse` helper. Keyboard events are journaled exactly like other world
input. Resize remains presentation-local and is not sent to shared game state.
The host crops a structured screen to each participant's connection-local
viewport; Roc apps still render at their declared dimensions and do not receive
viewport metadata.

`wants` is the capability protocol: a JSON document of the subscriptions the
program holds and the one-shot requests it has outstanding
(`{"v":1,"subscribe":["players","time.tick:5000"],"requests":[{"id":"r1","cap":"time.now"}]}`).
The host answers each new request with a host event (participant 0) carrying
`{"cap":"result","id":"r1","ok":true,"value":...}` or `"ok":false` with an
error code, and delivers subscription events (`time.tick`, `players`) as host
events too. Every host event is journaled once the program accepts it, so
replay is exact. A request to a capability the world has not granted is
answered with `cap_denied` — never silently. See the design doc for the catalog
(`time.now`, `time.tick`, `random.u64`, `players`, `players.list`,
`chat.say`, `chat.history`, `records.read`, `schedule.at`, `schedule.cancel`,
`world.info`, and the platform modules `Key`, `Screen`, `RocJson`, `RocGrid`,
`RocCap`, `RocText`).

`schedule.at` asks for one fire of a named schedule at a unix-millisecond time
(a time already past fires on the next wakeup; one more than 30 days ahead is refused).
In a durable world, pending schedules survive a restart, and they fire even when
nobody is connected. A fire arrives as `{"cap":"schedule","event":"fired","id":...}`, with
`"late":true` when the world reopened after the time had passed.
`schedule.cancel` drops a pending schedule and reports whether one was dropped.

The platform also exposes the dependency-free `RocJson.roc` module to apps as
`pf.RocJson`. Its `decode`/`encode`, object `get`, and type-specific `as_*` accessors let
apps handle host events and produce request/record documents without string
splicing. JSON number lexemes are preserved as strings to avoid implicit
floating-point conversion. Decode accepts at most 65,536 UTF-8 bytes and 64
nested arrays/objects; exceeding either bound is an explicit error. Encoding
also validates number lexemes and returns an error rather than emitting
malformed JSON. Duplicate object keys are preserved, and `get` returns the
first matching key.

The platform also exposes `RocGrid.roc` as `pf.RocGrid` for row-major byte
grids. `new`, `in_bounds`, `index`, `get`, `set`, `count`, `neighbors4`, and
`row` are bounds-checked and never trap: a read outside the grid returns a
caller-supplied fallback, and a write outside it returns the grid unchanged.
Boards, canvases, maps, and inventories build on this instead of re-deriving
index arithmetic per app. `examples/roc-grid` is the checked-in example.

`RocCap.roc` is exposed as `pf.RocCap` for the request side of the capability
protocol. `issue` reserves `r0`, `r1`, … ids and records each one's capability;
`attach` stores the request document so `requests` can re-announce it in
`wants` after replay; `chat_request`, `now_request`, `players_request`,
`world_request`, `history_request`, `records_request`, `schedule_at_request`, and `schedule_cancel_request` build the documents; `answered` routes a
result back to the capability that asked; `done` retires it; and `from_pending`
rebuilds a session from a snapshot. `history_lines` reads the rows of a
`chat.history` result. Apps stop re-deriving id counters and pending lists.

`Screen.put_grid` draws a `RocGrid` at a position through a glyph function with
one style for the whole grid, so maps and single-colour boards need no manual
row assembly. `Screen.fill` and `Screen.rule` cover rect fills and separators.

`RocText.roc` is exposed as `pf.RocText`: total, byte-level text helpers
(`trim`, `lowercase`, `starts_with`, `drop_prefix`, `contains`, `lines`,
`join_lines`) for the operations the pinned Roc nightly builds reliably.

One nightly caveat: with
nightly-2026-10-04-130536d's wasm backend, `List.get` past the second
element of a string list produced by `Str.split_on` returned truncated
elements in some module compositions (observed while building
`examples/lounge`; recorded in the design doc). The examples avoid
positional access beyond index 1; prefer `after`/`between`-style extraction
for now and re-check with a newer nightly before relying on it.

A second one, found while making `Screen` cheaper: using a `Str` again after
passing it to `Str.to_utf8` corrupts memory in that backend (`roc test` does
not show it; the wasm build returns rows made of other allocations). Read the
bytes once and rebuild the `Str` from them when you need both, as
`Screen.json_quote` does.

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

## Source-only server policy

For a server that accepts guest programs only as Roc source, enable
`id serve --world --world-admin-token <secret> --world-source-only` and configure
the trusted compiler/platform with `--roc-bin` and `--roc-platform` (or their
`ID_ROC_BIN` / `ID_ROC_PLATFORM` environment variables). The server refuses
startup if source-only mode has no compiler/platform, and the flag cannot be
combined with a raw `--world-module` bootstrap. It rejects raw Wasm/native
uploads before receiving upload chunks; authorized source compilation still
installs through the normal Wasm validation and execution bounds. The default
remains backward-compatible and accepts admin-authenticated raw modules.

Source-only mode is not a compiler sandbox or compiler provenance guarantee:
the server operator must trust the configured compiler and platform. The
compiler version/digest is not yet recorded with each module; see the [source
policy design record](../../../../doc/2026-10-14T20-00-05Z_feature_source_only_world_installs/2026-10-14T20-00-05Z_feature_source_only_world_installs.md)
for tracked follow-ups. Native execution remains a separate opt-in and retains
OS sandboxing.

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
