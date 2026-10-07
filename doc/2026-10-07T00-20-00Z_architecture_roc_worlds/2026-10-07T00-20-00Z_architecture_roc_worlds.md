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

---

## 2026-10-07T06-37-59Z Implementation: Roc guest executes in the world actor

The trap was caused by the host passing pointer `0` for a zero-length viewer
string. Zig can represent an empty slice that way; Roc's `Str` bridge cannot.
The runner now allocates a one-byte guest buffer for an empty input but passes
logical length zero, then frees the one-byte allocation. The checked-in Roc
counter test now verifies `count=0`, two `inc` updates, and `count=2` under the
default 10M fuel / 16 MiB memory limits.

`WorldProgram` is the execution seam. `WorldHandle::spawn_with_program` owns a
single mutable program inside the serialized actor. An input is validated and
assigned a candidate sequence, passed to the program, and only committed and
broadcast on success. A trap or rejected update does not enter the event log;
the actor marks that program unhealthy and refuses further game inputs/views
until an admin installs a replacement. Chat remains host-owned and continues
as an ordinary sequenced event. `WorldInstance` implements this trait: input
bytes must be UTF-8 for Roc `Str`, and the per-participant viewer JSON is passed
to `view`.

### Roc authority safety versus Wasm resource safety

Roc itself can be capability-safe server-side: the world platform exposes only
`init/update/view` and a pure `Str`/model API. No file, socket, clock, process,
environment, FFI, or ambient random effect is provided. A deliberately small
platform standard library keeps that property; the language does not need Wasm
to avoid host authority it was never given.

That does **not** make arbitrary code resource-safe. A pure program can loop
forever or allocate until the process is exhausted. Current deployment choices
are therefore:

- **Uploaded/peer-supplied programs:** Wasmtime only, no imports/WASI, with
  module/input/output limits, a per-call fuel budget, a linear-memory cap, and
  poison-on-trap. This is the implemented path.
- **Operator-authored server programs:** the same restricted Roc platform is
  capability-safe by construction, but running native Roc in the server
  process is not yet implemented or claimed safe. A native tier needs a
  separate worker/process with CPU, memory, and restart supervision before it
  is enabled. Wasm can still be used as a resource guard even for trusted code.
- **Browser:** Wasm can be used for local preview/prediction; its output is
  never authoritative. The server's actor state is always the source of truth.

### Module install and presentations

- `id serve --world --world-admin-token T` creates the in-memory lobby. `--web`
  is optional; Iroh p2p works without an HTTP listener.
- `--world-module PATH` loads a precompiled, import-free module at startup.
- `id world install NODE module.wasm --admin-token T` uploads a module in
  bounded chunks over `/id-world/1`. The server validates and instantiates it,
  pins its bytes in the local Iroh blob store, then atomically switches the
  actor and sequences a `program_installed` event. The command prints the
  module's BLAKE3 hash.
- The browser page `/world` can issue invites (separate admin bearer), upload a
  `.wasm` module, join, chat, send input, and render host-produced view text.
  Browser, CLI, and Iroh clients all drive the same `world_session` protocol.
- The hosted world state and module pin are currently volatile: restarting
  `serve` resets the actor and releases its temp-tag pin. Durable module
  selection, event-log replay/state restoration, and fetching module bytes from
  other nodes by hash are planned separately; an already installed module is
  not silently persisted as live state.

---

## 2026-10-07T06-45-00Z Verification: Roc guest, Iroh, browser and upload path

The checked-in counter guest now passes both its direct sandbox test and its
`WorldProgram` actor test under the default resource limits. The bug was in
the Zig adapter: it passed a null pointer for an empty viewer string, which
Roc's `Str` bridge rejects. The runner now allocates one guest byte while
keeping the logical empty-string length zero.

`examples/roc-counter/build.sh` reproduces the guest with the pinned Roc
nightly plus Zig 0.16, and `wasm-tools validate` verifies the result. The
Zig host source and generated Roc ABI adapter are checked in; `host.wasm` is
generated under `targets/` (the duplicate stale copy was removed).

The actor owns the program instance. For an input, it computes the next event
sequence, calls the program, and only commits/broadcasts the event on success.
A trap or oversized presentation poisons that program and refuses further
game inputs/views until replacement. The CLI integration test drives the real
counter through Iroh (`count=0`, `inc`, `count=1`) and checks the browser page
is served from the same `serve --world --web` node. A WebSocket test separately
uploads chunks, verifies the host pins the BLAKE3 artifact, and observes its
view change after game input.

Browser TypeScript checking, all 343 browser unit tests, and the web bundle
build pass. Full Rust tests pass: 655 library, 97 CLI integration, 5 remote
write/QUIC, 19 doctests. Nix `id-lib` builds with 479 unit tests and its
hermetic integration/doctest checks.

**Boundary reminder:** pure Roc with a deliberately effect-free platform is
capability-safe by construction; Wasm is not required to deny ambient file or
network access that the platform never grants. Wasmtime is used for the
untrusted upload tier because it additionally bounds CPU, memory, stack, and
module size. The operator-authored native Roc tier is a future, separate
worker with OS-level CPU/memory limits; native in-process Roc execution is not
enabled or claimed resource-safe.

This means live state is volatile and cannot yet be restored from a snapshot
or replayed after process restart. Persistent worlds require a separately
designed guest serialization/versioning contract; do not serialize raw Roc
pointers or Wasm linear memory and call that durable state. The later protocol
must define versioned guest save/load exports or reconstruct state by replaying
bounded events from a trusted seed.

---

## 2026-10-07T00-52-09Z Plan: authoritative world core and join capabilities

The next vertical slice is transport-independent and server-authoritative. It
adds an in-memory world core that serializes all mutations, issues scoped
join capabilities, assigns monotonic event sequence numbers, and returns a
snapshot plus events after a caller's cursor. It does not add network routes or
pretend to provide persistence yet.

Initial event kinds are bounded chat text and opaque game input. The core
validates membership, capability scope, input size, chat length, and event-log
retention before sequencing. The game input remains an event for a later Roc
runner integration; no guest code executes in this milestone. A snapshot
contains world ID, current sequence, bounded recent events, and participants
without exposing capability secrets. Capability material is generated with
the OS cryptographic RNG, stored only as a digest, compared in constant time,
and can be revoked. A join capability grants only `join/chat/input` for one
world; moderation and module replacement remain future scopes.

The Rust API is the seam for later Axum WebSocket and Iroh transports. A
single actor owns the core so concurrent inputs are ordered deterministically.
Tests must cover sequence ordering, unauthorized/expired/revoked capabilities,
bounded input/log behavior, and reconnect snapshots. Persistence, distributed
consensus, browser UI, TUI, and the Roc trap diagnosed above are outside this
milestone.

---

## 2026-10-07T01-05-34Z Plan: WebSocket world session bridge

Expose the actor through `/ws/world` as a presentation-neutral JSON protocol.
The first client frame presents an out-of-band join capability; the server
returns a bounded snapshot. Subsequent `chat` and opaque `input` messages are
validated by `WorldHandle`, sequenced by its actor, and broadcast to all
authorized sessions. Event sequence numbers are the reconnect cursor. A
client whose cursor has fallen behind retention receives a resnapshot-required
response instead of an incomplete replay.

The existing `WebSecurity` layer continues to enforce Host/Origin and optional
web-token checks for the WebSocket handshake. The world capability is a second,
independent authorization layer. This milestone does not add a public
capability-issuance endpoint or embed a shared world token in the web UI; the
join capability must be obtained out-of-band until an explicit invite flow is
implemented. The server must not log or echo capability secrets.

## 2026-10-07T02:00:00Z Implementation: WorldWebState and bridge tests

The world endpoints (`/ws/world`, `POST /api/world/invite`) now take their own
narrow `WorldWebState { world, admin_token }` via `world_ws::world_routes()`,
merged into the main router with `.merge(...with_state(...))`. This keeps the
bridge independent of `AppState` so it can be served and tested standalone.

Verified by in-crate tests over real loopback sockets (`tokio-tungstenite`
dev-dependency, already in the lockfile via axum) and `tower::oneshot`:

- invite: 401 for missing/wrong/unprefixed bearer, 200 with the correct admin
  secret, 404 when the admin secret or world is not configured;
- join -> snapshot, chat broadcast to every joined client, reconnect with
  `after` replays only missed events without a snapshot;
- bad first frame, forged capability, and oversized capability are rejected
  and the connection is closed;
- a revoked participant is disconnected.

Known limit: revocation is enforced when the next world event is delivered to
that socket (each delivery re-validates the capability), not instantly. An idle
revoked socket stays open until the next event or until it closes. Pushing a
close on revoke would need a revoke notification channel from the actor.

## 2026-10-07T03:00:00Z Decision: execution tiers and the p2p world protocol

### Where Wasm is used (and where it is not)

Roc has no ambient authority: a Roc program can only do what its platform's
effects allow. A platform whose entire host surface is the byte-oriented
`init/update/view` ABI therefore cannot reach the filesystem, network, or
processes, and native Roc is a legitimate server-side tier **if that platform
stays minimal**. What native Roc does not give us is resource bounding. A Roc
program can still loop forever, allocate without limit, or overflow the stack,
and its memory safety rests on the Roc compiler and runtime being correct.
Wasmtime gives fuel, memory caps, and a second isolation layer for free.

So the rule is **provenance decides the tier, not the language**:

| Tier | What runs | Who may supply it | Bounds |
| ---- | --------- | ----------------- | ------ |
| N: native Roc | Roc compiled with the `id` world platform (Rust host), no effects beyond the program ABI | The operator only, built from source on the host | Authority: none by construction. CPU/memory: not bounded in-process, so run in a dedicated worker with rlimits and a watchdog (not yet built) |
| W: Wasm in Wasmtime | The same program compiled to wasm32, import-free | Anyone: uploaded, or fetched from a peer by blob hash | Fuel per call, memory cap, size limits, poisoned on trap (implemented in `sandbox.rs`) |
| B: Wasm in the browser | The same wasm module, for view/prediction only | The host serves the module the world pins | Browser sandbox; never authoritative, server state wins |

All three tiers implement one `WorldProgram` seam (`update(participant, bytes)`
and `view(viewer) -> bytes`), so a world does not know which tier runs it.
An operator can pin trusted module hashes for tier N; an unpinned module is
always tier W. Peer-supplied code is never promoted to tier N.

### P2P transport: `/id-world/1`

Worlds are reachable over Iroh as well as the WebSocket bridge. The WebSocket
and Iroh transports drive the **same** session logic (`world_session.rs`)
through a small `SessionIo` trait, so protocol behavior cannot diverge.

- ALPN `/id-world/1`, registered on the `serve` router when `--world` is set.
  `--world` no longer requires `--web`; web is only needed for `/ws/world`.
- One bidirectional QUIC stream is one session. A connection may carry a few
  sessions (bounded). Frames are a big-endian `u32` length followed by one
  UTF-8 JSON object, at most 16 KiB. The JSON is exactly the WebSocket
  protocol, so presentation adapters are transport-agnostic.
- The first frame is `join {capability, after?}` or `invite {admin_token,
  display_name}`; anything else is an error and the stream ends. A peer that
  sends no first frame within 10 s is dropped.
- Iroh authenticates the **node**, not the participant. The remote node ID is
  logged but never grants world authority; the capability does. This matches
  the earlier rule that node identity must not silently act as authorization.
- Within `/id-world/1`, new frame fields and variants may be added; removing
  or changing one needs `/id-world/2`.

Clients: `id world invite <NODE> --admin-token T --name N` and
`id world join <NODE> --capability C [--after SEQ]` (stdin lines are chat;
`/input <hex>` sends opaque input; server frames print as JSON lines).

### Not yet done (explicit)

- `WorldProgram` wiring into the actor, then the Roc wasm demo. The checked-in
  Roc module still traps on its first `view`; that must be diagnosed first.
- Tier N worker process, module distribution by blob hash, persistence.

---

## 2026-10-07T08-00-00Z Implementation: durable worlds by event sourcing

Program state is never serialized. `serve --world` (without `--ephemeral`)
keeps the lobby in `.id-worlds/lobby/`:

- `journal.jsonl`: one `JournalEntry` per line (`created`, `issued`,
  `revoked`, `committed`), `fsync`ed before a mutation is acknowledged or
  broadcast. Capability **digests** are journaled, never the bearer secrets.
- `modules/<blake3>.wasm`: every installed module, written before the
  journal may reference it (temp file + rename).

On start, `WorldCore::restore` rebuilds participants, revocations and the
sequence; the last `program_installed {module_hash, seed}` is re-instantiated
and the inputs after it are replayed. Pure programs make this exact. Rules:

- A torn final journal line (crash mid-append) is truncated: it was never
  acknowledged. Corruption before the last line, a journal for another world,
  or an unknown version refuses to start rather than dropping history.
- If a journal append fails, the world turns read-only (revocation still
  applies in memory, since denying is the safe side).
- If the module is missing or replay fails, chat and membership keep working
  and the program is marked unavailable until an admin installs one.
- `--world-module` goes through the same journaled install, and is skipped
  when that module is already active (re-installing would reset the program).

Verified: unit tests for restore, torn tails, mid-file corruption, wrong-world
journals, tampered modules, storage failure, and Roc replay; a CLI test that
restarts `serve` and checks the old capability still joins and `count=2`
survives, then advances to `count=3`.

Known limit: the journal grows without bound. Compaction needs a guest
snapshot/restore contract (or periodic checkpoint + truncated replay), which
is not designed yet.

---

## 2026-10-07T09-00-00Z Implementation: structured records in iroh-docs

A world can publish **structured records**: a JSON object mapping record keys
to JSON values, produced by a new optional `records : model -> Str` function
in the Roc platform (exported as `plaza_records(i32) -> i32`, sharing the
`plaza_out_len` output slot with `view`). Records are validated by the host:
keys 1..=256 bytes without control characters, values <= 8 KiB, at most 4096
records; the set is capped at 1 MiB. Invalid records reject the input (or the
install) before anything is committed, and poison the program like any other
failure.

Storage and replication use **iroh-docs**, one document per world
(`records.namespace` in the world directory remembers its ID so tickets stay
valid across restarts):

- The host mirrors the current records into the doc (single writer), writing
  changed values and deleting removed keys. Deletes are exact: keys are stored
  as `key\0` and record keys cannot contain control characters. Unchanged
  records are not rewritten (content-hash comparison), so replicas only sync
  real changes.
- Any participant can ask a session for a **read-only ticket**
  (`records_ticket`), or page the records directly (`records {prefix, after}`,
  key-ordered, ~48 KiB pages with the world sequence attached).
- `id world mirror NODE --capability C [--follow]` starts an in-memory
  iroh-docs replica, imports the ticket and keeps syncing (also after the
  initial sync) — replication is peer-to-peer over iroh-docs, not through the
  world session.
- The doc is a projection of authoritative state, never a second source of
  truth: records are recomputed from the program, and the world journal +
  module replay remain what defines state. A replica can forge nothing: the
  ticket carries the namespace public key and only the host's author writes.

Verified: unit tests for reconcile (write/delete/no-op), schema bounds, actor
publish/reject semantics, real-guest export; an end-to-end replication test
where a second node syncs a ticket and receives live updates; session paging
tests; and a process test that restarts `serve`, queries records over the
world protocol, and mirrors them from a separate process over iroh-docs.
