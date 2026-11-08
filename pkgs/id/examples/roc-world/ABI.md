# Plaza guest ABI — version 1

This document specifies the byte-level boundary between an `id` world host and
a Roc world compiled for Wasm32. Version 1 is the interface implemented by
[`host/src/host.zig`](host/src/host.zig), [`platform.roc`](platform.roc), and
the import-free guest runner in [`../../src/sandbox.rs`](../../src/sandbox.rs).
The Roc platform's module-level functions are described in the
[world-platform README](README.md).

`plaza_abi_version() -> i32` returns `1` for this specification. A semantic
change to an existing export or to its wire contract needs an explicit ABI
version decision; adding optional exports does not change the meaning of the
existing ones. The current runner accepts modules without this getter for
backward compatibility and allowlists it when present; it does not yet use the
returned value to negotiate versions.

## Module and value conventions

- The guest is an import-free Wasm32 module. It exports its linear memory as
  `memory`; the runner provides no WASI, filesystem, network, clock, or random
  imports. Each world has a persistent instance and its model pointer remains
  in that instance's linear memory.
- Wasm `i32` values carry pointers, byte lengths, and participant IDs; Wasm
  `i64` values carry seeds and dimensions. In the Roc/Zig source these are
  represented as `usize` or `u64`. Pointers are byte offsets into this
  instance's memory, not host addresses.
- Strings and event/snapshot payloads are UTF-8 byte sequences. Lengths are
  byte counts, not character counts. Pointer/length ranges must be valid in
  the exported memory. The runner bounds-checks copied output and enforces its
  configured message and memory limits.
- A model pointer is opaque to the host. `init`, `update`, and `restore`
  produce the model pointer used by later calls; projection calls must not
  consume or replace it.
- Unless noted otherwise, optional exports may be absent. `memory`,
  `plaza_alloc`, `plaza_free`, `plaza_init`, `plaza_update`, `plaza_view`, and
  `plaza_out_len` are required. `plaza_snapshot` and `plaza_restore` must either
  both be present or both be absent. The diagnostic getters are independently
  optional in the runner, but the Roc host exports them as a pair.

## Exports

Signatures below are Wasm signatures; `i32` pointers and lengths refer to the
guest's linear memory.

| Export | Signature | Contract |
| --- | --- | --- |
| `plaza_abi_version` | `() -> i32` | Returns `1` for this ABI specification. New v1 host builds export it; older guest modules may omit it. |
| `plaza_alloc` | `(i32 byte_len) -> i32 ptr` | Allocates guest-owned memory for host-provided input bytes. The host writes the input before calling the guest operation and releases the allocation with `plaza_free`. |
| `plaza_free` | `(i32 ptr, i32 allocation_len) -> ()` | Releases a matching allocation made by `plaza_alloc`. The length is the allocation length; for an empty logical input the runner allocates and later frees one byte while passing logical length zero to the operation. |
| `plaza_init` | `(i64 seed) -> i32 model` | Creates the initial model from the host-provided `u64` seed. Called once for a newly instantiated world. |
| `plaza_update` | `(i32 model, i32 participant, i32 event_ptr, i32 event_len) -> i32 model` | Applies one UTF-8 serialized world event and returns the next model. `participant` is the sender's ID; host-generated events use participant `0`. The runner commits the returned model only when the call succeeds. |
| `plaza_view` | `(i32 model) -> i32 output_ptr` | Returns UTF-8 text or a portable screen JSON document for the current model. |
| `plaza_layout` | `(i32 model, i64 width, i64 height) -> i32 output_ptr` | Optional viewport-specific rendering. Width and height are the connection's viewport dimensions. When absent, the host falls back to `plaza_view`. |
| `plaza_records` | `(i32 model) -> i32 output_ptr` | Optional structured projection. The returned bytes are a JSON object of string keys to JSON values; the host validates and publishes it. |
| `plaza_wants` | `(i32 model) -> i32 output_ptr` | Optional capability request/subscription document, as specified below. |
| `plaza_snapshot` | `(i32 model) -> i32 output_ptr` | Optional UTF-8 serialization of the model for a journal checkpoint. Must be paired with `plaza_restore`. |
| `plaza_restore` | `(i32 snapshot_ptr, i32 snapshot_len) -> i32 model` | Reconstructs a model from bytes produced by `plaza_snapshot`. The input is allocated by the host and released after the call. A Roc program should return a sensible model for invalid snapshot text rather than trap. |
| `plaza_out_len` | `() -> i32 byte_len` | Returns the byte length of the most recent output from `view`, `layout`, `records`, `wants`, or `snapshot`. It is one shared output slot. |
| `plaza_error_ptr` | `() -> i32 ptr` | Optional diagnostic pointer exposed after a guest trap. In the Roc host it points to the captured Roc crash message in guest memory. |
| `plaza_error_len` | `() -> i32 byte_len` | Optional diagnostic length paired with `plaza_error_ptr`; the Roc host captures at most 4096 bytes. The bytes are not NUL-terminated and are diagnostic only, not a result/status channel. |

### Buffer lifetime and errors

For an input call, the host allocates in guest memory, writes the bytes, calls
the guest with the pointer and logical byte length, and frees that allocation
after the call. The guest must not retain the input pointer. The returned model
pointer is retained by the host; it is not a `plaza_alloc` buffer.

Snapshot support is an all-or-nothing pair. Before trimming a journal, the
host restores the snapshot into a probe instance and refuses the trim unless
the probe produces byte-identical snapshot text and identical published
records. A module without the pair keeps its full journal. The host replaces
the probe's initial model when it applies `plaza_restore`; modules should make
restore total for malformed or unsupported snapshot text.

Every output-producing export returns a pointer and leaves its byte length in
`plaza_out_len`. The host reads and copies that range immediately. The Roc
adapter reuses one output slot: the next output-producing call releases the
previous `RocStr`, so consumers must not retain an output pointer across such a
call. The host validates the pointer/length range before reading it.

Guest traps and fuel exhaustion are failures, not successful ABI return
values. The Roc host's `roc_crashed` callback captures up to 4096 bytes and
traps; after a trap the runner may read the optional diagnostic pointer/length
pair. A missing or unusable diagnostic is still a guest failure. The runner
does not use the diagnostic exports for normal return values.

## Capability protocol

When exported, `plaza_wants` returns a UTF-8 JSON document of the current
subscriptions and outstanding one-shot requests. Version 1 uses this shape:

```json
{
  "v": 1,
  "subscribe": ["players", "time.tick:5000"],
  "requests": [{"id": "r1", "cap": "time.now"}]
}
```

The `v` field is optional for compatibility; when present it must be `1`.
`subscribe` and `requests` may be omitted when empty. The complete document is
limited to 64 KiB, with at most 64 subscriptions and 64 requests. Subscription
specs and capability names are 1–64 bytes. A request has a unique
program-chosen `id` (1–64 bytes, no control characters), a `cap` name, and
optional `args` (at most 8 KiB of serialized JSON; absent means `null`). At
most four distinct tick intervals may be requested. A malformed or over-limit
document is a guest-program failure. The host answers a request with a
participant-0 event such as
`{"cap":"result","id":"r1","ok":true,"value":{"now":123}}`;
failures use `"ok":false` and a stable `"error"` code. Refusal codes include
`cap_denied`, `unknown_cap`, `bad_args`, `failed`, `actor_denied`,
`outside_ceiling`, and `result_too_large`. Subscription events include
`{"cap":"time.tick","interval":5000,"now":123}` and player joined/left
events; refused subscriptions use a `cap: "subscription"` event with `spec`,
`ok: false`, and an error code. Host events are journaled when the program
accepts them, so replay feeds the same events back through `plaza_update`.

The runtime provider catalog is:

| Capability | Behavior |
| --- | --- |
| `time.now` | One-shot current Unix time in milliseconds (`{"now": ...}`). |
| `time.tick` | Subscribe as `time.tick:<milliseconds>` with a positive interval; periodic host events while a participant is present, subject to the world's configured minimum interval and the four-interval limit. |
| `random.u64` | One-shot random unsigned 64-bit value (`{"value": ...}`). |
| `players` | Subscribe to participant join/leave events. |
| `players.list` | One-shot participant list, including ID, display name, and presence. |
| `chat.say` | Commit a chat line from `args.text`; result contains its sequence. |
| `chat.history` | Read recent chat lines; `args.count` defaults to 16 and is capped at 64. |
| `records.read` | Read a byte-bounded, key-ordered page of published records. `args.prefix` filters keys and `args.after` is an exclusive cursor; continue with `next_after`. |
| `schedule.at` | Schedule one event using printable-ASCII `args.id` (1–64 bytes) and Unix-millisecond `args.at`; at most 64 schedules may be pending, past times fire on the next wakeup, and times more than 30 days ahead are refused. |
| `schedule.cancel` | Cancel a pending schedule by printable-ASCII `args.id`; result reports `{"cancelled": bool}`. |
| `world.info` | Read the world ID and current sequence. |

The `Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, and `RocText` names are
platform modules exposed to Roc source, not runtime request providers. The
server's grant/ceiling policy still applies to runtime capabilities; a known
but ungranted request is returned as a refusal event, not silently executed.
`wants` is a pure projection of the model: the program stores pending request
state in its model (and, when needed, in snapshots) so it can re-announce
outstanding requests after replay.

## Source references

- Export implementation and memory/output ownership: [`host/src/host.zig`](host/src/host.zig).
- Wasm export list and Roc-to-host declarations: [`platform.roc`](platform.roc).
- Required/optional export checks, memory marshalling, and result handling:
  [`../../src/sandbox.rs`](../../src/sandbox.rs).
- Capability schema, limits, and catalog:
  [`../../crates/id-core/src/world_caps.rs`](../../crates/id-core/src/world_caps.rs).
- Capability execution and result values:
  [`../../src/world/actor.rs`](../../src/world/actor.rs).
