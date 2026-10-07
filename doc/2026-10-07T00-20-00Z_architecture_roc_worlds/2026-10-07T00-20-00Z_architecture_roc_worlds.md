# Roc-Defined Multiplayer Worlds

This document records the initial architecture for running user-authored Roc
programs as shared, interactive worlds served by `id`. The immediate target is
a small end-to-end multiplayer demo, not a general-purpose game engine.

## Intent

Turn an `id` node into a place where people can join the same live experience
from a browser or terminal, talk and play together, and optionally upload a
Roc program that defines the experience. Uploaded programs are untrusted.
They must not receive ambient filesystem, network, process, or host access.

## Current baseline

- `pkgs/id` provides Iroh blobs, metadata, peer access policy, CLI/REPL, and an
  optional browser UI.
- `pkgs/roc` contains experiments and examples for several Roc hosts, but no
  shared-world runtime.
- The repository currently has no `pkgs/plaza` package and no Wasmtime
  dependency wired into `pkgs/id`.
- `/tmp/proto` contains an untracked prototype: a Roc `init/update/view`
  platform compiled to wasm32-freestanding, a Zig 0.16 host, and a smoke-test
  wasm module. It demonstrates ABI feasibility only; it is not yet checked in,
  reviewed as a security boundary, or proven safe for hostile input.

## Proposed model

### World authority

One `id` node hosts a world and is authoritative for its state. Clients send
bounded input events; the host serializes them, checks permissions and limits,
and asks the world module to process accepted events. It then publishes the
resulting versioned state/event to all clients. This avoids client consensus
and makes the first demo tractable. It is not a claim of trustless execution:
participants trust the host to apply rules honestly.

The world contract should be a pure deterministic state transition:

- `init(seed) -> state`
- `update(state, event) -> state`
- `view(state, viewer) -> presentation`

The view result is presentation-neutral structured data, not HTML or terminal
escape sequences. Web and terminal clients render the same state using their
own adapters. Chat is an ordinary world event/state stream in the demo, so the
same transport and reconnect behavior is exercised by social and game events.

### Wasm execution boundary

Compile Roc modules to a narrowly defined wasm32 target and execute them in
Wasmtime. The guest receives no WASI, filesystem, socket, clock, random, or
process imports. Host capabilities are explicit, typed, and mediated outside
the guest. For the first demo, module execution needs no host imports beyond
the ABI/runtime allocator.

The runtime must enforce, before accepting an uploaded module:

- Wasm validation and an explicit import/export allowlist.
- Per-call fuel/instruction budget, memory maximum, input/output size limits,
  and bounded execution time at the host boundary.
- Deterministic inputs: seed, event, and state are supplied by the host; no
  guest clock or ambient randomness.
- Transactional state update: malformed output, trap, fuel exhaustion, or
  invalid state leaves the last committed world state unchanged.
- A recoverable world error and operator-visible diagnostics without leaking
  server secrets to participants.

Wasmtime limits reduce resource-exhaustion risk but do not alone prove complete
sandboxing. The implementation must stay on supported Wasmtime defaults,
minimize imports, keep the engine current, and test adversarial modules.

### Capabilities and permissions

Separate the authority to join a world from authority to change its rules.
An opaque, scoped capability token should name a world and permitted actions
(for example `join`, `chat`, `input`, `moderate`, `replace-module`), with
expiry/revocation policy decided before deployment. The server validates a
capability on every operation; clients never receive the host's Iroh secret
key or ambient filesystem authority. Module capabilities are distinct from
participant capabilities and default to none.

The initial vertical slice may use a world-local invitation/join token, but it
must not silently treat an Iroh node ID as authorization. Existing `id`
peer-write policy remains independent of world membership.

### Transports and presentations

- Reuse `id`/Iroh for node identity, world artifacts, and peer connectivity.
- Add a world session protocol for join, snapshot, event submission, and
  versioned broadcast. It must have message-size limits and reconnect/snapshot
  recovery; transport selection (Iroh stream vs WebSocket bridge) is an
  implementation seam, not part of the Roc guest API.
- Browser and terminal clients are presentations of the same session model.
  The first terminal target can be the existing CLI/REPL; SSH hosting and
  native GUI are later milestones.
- A host may offer multiple worlds, each with isolated state, module instance,
  capability namespace, quotas, and lifecycle.

## Initial vertical slice

1. Check in a minimal Roc wasm app and reproducible build recipe, then validate
   the module format and ABI in a small host-side smoke test.
2. Implement a Wasmtime runner behind a deep Rust interface that accepts a
   module, state bytes, event bytes, and explicit limits, returning new state
   bytes or a bounded error. Keep Wasmtime details inside the runner module.
3. Add an in-memory authoritative world actor with event validation, state
   versioning, join/chat/input messages, and snapshot-on-reconnect.
4. Demonstrate two clients sharing chat plus one tiny game (tic-tac-toe) with
   the same Roc module and world state.
5. Add the browser and terminal presentation adapters; verify a move made in
   one appears in the other.
6. Only then add uploaded module installation, persistent worlds, and richer
   capability delegation.

## Decisions and non-goals

- Start server-authoritative; do not implement peer consensus, rollback
  netcode, arbitrary guest I/O, or a universal game engine in the first slice.
- Treat the Roc/Wasm ABI as versioned protocol. Pin compiler/platform metadata
  with each module; reject incompatible modules rather than guessing.
- Keep world state separate from the `id` file/tag namespace. Store module
  artifacts content-addressably, but give live sessions their own lifecycle.
- Keep HTTP/WebSocket origin and host protections from the existing web UI;
  world membership checks are additional, not replacements.
- Do not promise hostile-code safety based solely on "it is WebAssembly".
  The import surface, Wasmtime configuration, limits, and adversarial tests
  are part of the security design.

## Open questions before implementation

- Which exact Roc compiler build and wasm ABI are support targets? The local
  2026-10-04 nightly prototype is evidence, not yet a compatibility promise.
- Should the web UI bridge world events over WebSocket while native clients
  use Iroh streams, or should all clients initially use one transport?
- What token issuance/revocation UX is acceptable for public worlds, and how
  does it compose with existing node allowlists?
- Which Wasmtime release/features are available in the pinned Nix toolchain,
  and can the engine be packaged reproducibly for supported platforms?
- What restart semantics should apply to live state: volatile reset, snapshot
  persistence, or event-log replay?

## References

- [`pkgs/id/ARCHITECTURE.md`](../../pkgs/id/ARCHITECTURE.md)
- [`pkgs/id/src/access.rs`](../../pkgs/id/src/access.rs)
- [`pkgs/id/src/web/collab.rs`](../../pkgs/id/src/web/collab.rs)
- Roc host prototype at `/tmp/proto` (local scratch; not part of this repo)

---

## 2026-10-07T00-35-00Z Deviation: keep Roc model state in a per-world guest instance

The first runner sketch described serialized state/event bytes passed through
stateless `init/update/view` calls. Inspection of the working Roc + Zig
prototype showed that this does not match the actual generated platform ABI:
`plaza_init(u64) -> usize`, `plaza_update(model_ptr, str_ptr, str_len) ->
model_ptr`, and `plaza_view(model_ptr, viewer_ptr, viewer_len) -> output_ptr`,
with `plaza_out_len() -> usize`. Roc's model is an opaque Roc allocation, not a
stable serializable byte format.

The intended executable seam will therefore use one Wasmtime `Store` + `Instance`
per world, created from the reviewed module. The Rust world actor owns that
instance and the opaque model pointer never leaves it. Each call replenishes
fuel; the Store retains its memory limit. Inputs and outputs remain bounded
byte slices copied through exported linear memory. Guest imports remain empty.
The Roc/Zig adapter is responsible for Roc string representation and allocator
correctness. The runner validates export names and signatures before creating a
world.

The first checked-in Wasmtime runner validates an import-free module and
executes a small test ABI. The checked-in Roc/Zig counter compiles and
initializes, but traps on its first `view` call. The prototype relies on
guest-defined runtime symbols (`roc_alloc`, `roc_dealloc`, `roc_realloc`, and
diagnostics); those are not host imports and their implementations currently
live inside the module. Do not upload or expose this runner as a world service
until the platform glue is debugged against Wasmtime and hostile-module tests
cover the ABI. Keep import denial strict; do not work around the trap by adding
WASI or ambient host capabilities.

This means live state is volatile and cannot yet be restored from a snapshot
or replayed after process restart. Persistent worlds require a separately
designed guest serialization/versioning contract; do not serialize raw Roc
pointers or Wasm linear memory and call that durable state. The later protocol
must define versioned guest save/load exports or reconstruct state by replaying
bounded events from a trusted seed.
