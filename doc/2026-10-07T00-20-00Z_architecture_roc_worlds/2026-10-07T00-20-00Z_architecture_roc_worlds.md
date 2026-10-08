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

*(Superseded: the items below were completed in later sections. See
“Current status: not yet offered” at the end of this document.)*

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

---

## 2026-10-07T10-00-00Z Implementation: participant identity in the guest ABI, shared platform, tic-tac-toe

World programs now learn **who sent an event**: the guest `update` is
`model, U64, Str -> model` (Zig export `plaza_update(model, participant,
ptr, len)`), and the host passes the session's participant id. Programs that
only care about the event body ignore the argument; games can implement
turn-taking or per-player rules without the host having any game knowledge.
The id is clamped to `i32` before it reaches the guest, and the world's
participant limit keeps real ids far below that.

The example platform moved to `examples/roc-world/` (platform + Zig host
adapter + generated Roc ABI bindings) and is referenced by each app:
`examples/roc-counter/` and the new `examples/tic-tac-toe/`. Both compile to
import-free wasm modules with the same pinned Roc nightly; `build-host.sh`
rebuilds the host adapter, and each app's `build.sh` compiles and validates
its module.

Tic-tac-toe keeps the authoritative state in a 9-character board string plus
a ply count: the turn is the ply parity, so `update` remains a pure function
of `(model, event)` and replay is exact. Occupied cells, off-board cells,
non-digits and moves after a win are ignored inside the program; records
publish `{"board":..., "plays":..., "winner":...}`, so the same replication
and mirroring path works for a real game.

Verified: the sandbox test plays a complete game (X wins on the top row,
`records` shows `XXXOO---- / plays=5 / winner=X`), then checks that post-win
moves, occupied cells, out-of-range cells and garbage input change nothing;
the counter still runs unchanged under the new ABI.

---

## 2026-10-07T11-00-00Z Implementation: durable records on both sides

Records were already a projection (never serialized into the journal), but the
two stores that carry them needed durability checks on both ends:

- **Host.** A non-ephemeral world remembers its iroh-docs namespace in
  `.id-worlds/<world>/records.namespace`, and the serve process opens its docs
  database and blob store on disk. After a restart the same document is
  reattached — the test reads the namespace file before and after a restart
  and requires it to be byte-identical — so replicas that synced earlier are
  still on the same document and see updates, not a fresh one.
- **Replica.** `id world mirror --dir PATH` keeps a replica on disk: its own
  node identity (`node.key`), an fs blob store (`blobs/`) and a persistent
  docs database (`docs/`). Running it again reuses all three and only fetches
  what changed; without `--dir` the replica stays in memory. The process-level
  test runs the persistent mirror twice against a live host and checks the
  identity, the stores, and the replicated records.

Together with the host's journal + deterministic replay (world state) and the
content-addressed module directory (program bytes), a world's full public
state now survives a restart, and participants can keep their own durable
copy of the structured data.

### Sync completion: readiness is not arrival

`replicate` used to return as soon as the docs engine declared the document
*ready* (`PendingContentReady`). Readiness only means the initial sync round is
over — a peer can still be connecting, so a replica could read an empty
document even though the host had entries (seen as a rare flake under full
suite load). It now also waits for every announced entry's blob to be local,
with a short grace period for a document that is genuinely empty; only then
does it read the records. The same predicate is what a mirror needs before it
may print `{}`, so a quiet world prints promptly and a busy one never prints a
partial snapshot.

A second, rarer race surfaced under full-suite load: a blob can still be
mid-download when `has` reports it present, so the read itself fails with an
encoding error (`LeafHashMismatch`). `replicate` now retries the final read
until its deadline instead of surfacing that transient error, and the test
helper treats a transient read failure as "not there yet" rather than failing
the wait.

### One more end-to-end app

`test_tic_tac_toe_installed_over_iroh_and_played_by_two_players` uploads the
second example module into a running world (not `--world-module`), joins over
Iroh, plays a full game with `/input <hex digit>` moves, and reads the final
board from `id world records`. It proves the dynamic install path and the
records projection with a program that has no special support in the host.

---

## 2026-10-07T12-00-00Z Status: what is offered, and what is not

Offered today: a host runs one Roc world in the effect-free Wasm sandbox with
capability-gated sessions over Iroh and WebSocket; guests install/download
modules p2p; state is durable by fsynced journal + deterministic replay; the
program's structured records are mirrored into an iroh-docs document that any
participant can replicate with a read-only ticket, in memory or on disk, and
that survives host restarts on the same namespace; two example apps (counter,
tic-tac-toe) build with the shared platform and run end to end.

*(Superseded — see "Current status" at the end of this document. The worlds
hub, the capability runtime, the compile service and the native Roc tier
below were all built after this list was written.)*

---

## 2026-10-07T13-00-00Z Implementation: named worlds

The hard-coded `lobby` directory became `--world-name <NAME>` (default
`lobby`): a world's journal, module directory and records namespace live in
`.id-worlds/<name>/`, the world's in-memory id is that name, and a joining
client learns it from the snapshot. Names are validated as one path segment
(`a-z0-9-_`, 1..=64 characters, not `.`/`..`), so a caller can never choose a
directory.

Running several worlds on one machine therefore means several `serve`
processes, each with its own Iroh identity and store; the process boundary
isolates the worlds' sandboxes and journals from each other. The verified
path: a persistent server started with `--world-name tt` reports
`"world_id":"tt"` to a joining client and writes only `.id-worlds/tt/`,
leaving `.id-worlds/lobby` absent; `--world-name ../evil` is refused before
startup.

---

## 2026-10-07T12-00-00Z Implementation: journal checkpoints

Replay-from-genesis made restart time grow with a world's whole history. The
guest ABI now has an optional `plaza_snapshot(model) -> Str` /
`plaza_restore(Str) -> model` pair (the Roc platform requires `snapshot` and
`restore`; both example apps implement them). Both exports are optional in the
sandbox but must come as a pair.

- **Trigger.** After every `checkpoint_every` committed events (default 1000,
  `serve --world-checkpoint-every N`, `0` disables) and only for a healthy
  program on a durable world. Chat counts, since it also fills the journal.
- **Verified snapshots.** `WorldInstance::snapshot` does not hand the text out
  until a probe instance restored from it re-snapshots to byte-identical text
  and publishes identical records. An unfaithful snapshot is a silent state
  change on restart, so on any mismatch, trap or missing export the journal is
  simply left alone (and not retried until a new program is installed).
- **Atomic trim.** `FileJournal::compact` writes the replacement journal
  (header, every participant and revocation, one `checkpoint` entry carrying
  sequence, module hash, seed, snapshot and the retained recent events) to a
  sibling file, fsyncs it, renames over `journal.jsonl` and fsyncs the
  directory. A crash leaves the old or the new journal, never a mixture. If the
  directory sync fails the journal marks itself broken and refuses appends:
  otherwise a crash could resurrect the old file and drop acknowledged events.
- **Restore.** `WorldCore::restore` accepts a checkpoint only directly after
  the participant entries; the program is instantiated with its seed, restored
  from the snapshot, then the inputs after the checkpoint are replayed. The
  event sequence and the catch-up window continue across the checkpoint,
  capability digests (never secrets) and revocations survive it.
- **Found while testing.** `WorldCore::commit` (chat) did not share
  `commit_prepared`'s bookkeeping, so live chat was not counted toward the
  checkpoint threshold while restored chat was; both paths count now.

Verified: snapshot round-trips for both example apps (garbage text restores to
a defined state); a real counter world trimmed at 5/10/15 events restarts with
no inputs to replay, the same sequence, working capabilities, a surviving
revocation and a working catch-up; programs that cannot snapshot or whose
snapshot fails keep their full journal and keep accepting input; a checkpoint
after events is refused; and a process test plays tic-tac-toe across a trimmed
journal and a `serve` restart.

---

## 2026-10-07T13-00-00Z Design: worlds as a service

Intent: one `serve` process hosts any number of worlds, many connections, and
programs that can ask the server for things (time, randomness, who is here,
speech) without giving up the properties the platform is built on. Programs
can be compiled from source on the server, and may run natively. Four pieces,
built in this order, each committed on its own: **hub**, **capabilities**,
**compile service**, **native tier**.

### 1. The hub: many worlds, many sessions

- A `WorldHub` owns the registry `name -> WorldService`. Every session's first
  frame may carry `"world": "<name>"`; absent means the default world
  (`--world-name`, `lobby`), so every existing client keeps working. A session
  is bound to one world for its lifetime; "change ad hoc" is opening another
  session (cheap: sessions are QUIC streams on one connection, or WebSockets).
- Capabilities are per world by construction (each world has its own table), so
  a token for world A is meaningless in world B. The admin token stays
  process-wide: it mints capabilities, installs programs and creates worlds.
- Worlds open lazily from their directory on first reference and are created
  on demand by an admin-authenticated frame (`invite`, `install_*`,
  `create_world`). Unauthenticated frames never create a world: an unknown name
  is "unknown world", indistinguishable from a bad capability.
- Hub frames: `list_worlds` (admin) and `create_world` (admin). Names are
  `[a-z0-9_-]{1,64}`, one path segment.
- Bounds: a ceiling on open worlds (`--world-max-open`) and on concurrent
  sessions (`--world-max-sessions`); a session holds a lease for its lifetime.
  Durable worlds with no sessions are evicted after an idle period and reopen
  from their journal on next use, so the number of *existing* worlds is
  limited by disk, not memory. Ephemeral worlds are never evicted.
- Each world keeps its own actor, journal, module directory, records document
  and (with the native tier) its own worker process: no state is shared across
  worlds except the blob store and docs engine, both content-addressed.

### 2. Capabilities: effects as data

The program is the same pure function it always was. What a program needs from
the server is expressed as **data it returns** and **events it receives**:

- `wants : model -> Str` (JSON, a pure projection like `records`) lists the
  subscriptions and requests the model currently has:
  `{"v":1,"subscribe":["time.tick:5000","players"],
    "requests":[{"id":"r1","cap":"time.now"}]}`.
- The host turns each new thing into an **event**, delivered through the
  existing `update(model, 0, json)` (participant 0 is the host; real
  participants start at 1). Every such event is **journaled** before it is
  delivered, so replay feeds the program exactly what it saw the first time
  and determinism survives.
  Events: `{"cap":"time.tick","now":<ms>}`,
  `{"cap":"players","event":"joined"|"left","participant":{...}}`,
  `{"cap":"result","id":"r1","ok":true,"value":...}` (or
  `"error":"cap_denied"|...`).
- Why not Wasm imports or Roc `hosted` effects: an effect that runs during
  `update` has a result the journal would have to capture anyway, and an
  in-flight call cannot be snapshotted or replayed; both tiers would also
  need their own effect plumbing. Data keeps one ABI for Wasm and native,
  keeps `update` pure, and keeps checkpoints exact.
- Catalog (the "server-side library"), each a provider behind one trait:
  `time.now`, `time.tick:<ms>` (>= 1000 ms, delivered only while someone is
  connected, so idle worlds stay idle), `random.u64`, `players` (join/leave
  events) and `players.list`, `chat.say` (a system chat line), `world.info`.
  New providers add names, never change existing ones: the interface is
  versioned by the `"v"` field and by cap names.
- **Authorization.** A world has a granted set. A request to an ungranted cap
  yields `cap_denied` (journaled, so the program sees it) and is counted in
  a **usage report** (`cap`, count, denied or not). Grants come from the
  server (`--world-cap`, repeatable, applied to the default world; admin
  `grant_caps` frame for any world; persisted in the journal) or, in
  `--world-cap-policy grant-on-use`, from observation: the first use of a
  known cap grants it. `id world caps` shows granted caps and the usage of
  the ungranted ones, which is the "derive what the program needs by
  looking at what it does" path. Installing a program also reports which caps
  its initial `wants` needs that the world has not granted.
- Bounds: <= 64 entries per `wants`, <= 8 KiB args, <= 64 KiB results, <= 64
  requests in flight. A request is executed when its id appears in `wants`
  after being absent from the previous `wants`; a program removes it when the
  result arrives. After a restart in-flight requests run again (at least
  once).

### 3. Compile service

- Admin frame `compile {admin_token, files, target, seed}` carries up to 8
  `.roc` files (<= 256 KiB total). The server writes them to a private job
  directory beside a copy of the world platform, runs the configured `roc`
  binary (`--roc-bin`, `$ROC`, or `PATH`) with a cleared environment,
  wall-clock and CPU/memory/file-size limits and bounded output, validates the
  result with the same import-free policy as an upload, then installs it by
  the normal journaled path. Diagnostics are returned to the caller.
- Source is stored with the module (`sources/<hash>/`), so a world can say
  what it runs. Compilation is admin-only: it executes a compiler on
  attacker-chosen text, and the only defense is limits, not trust.
- The platform (host adapter + prebuilt linker inputs) lives in a directory
  (`--roc-platform`), produced by `examples/roc-world/build-host.sh`; it is not
  embedded in the binary.

### 4. The native tier

- Roc builds to a static x86-64 musl executable (verified with the pinned
  nightly: the platform's `x64musl` target links a Zig host `main`, with the
  musl runtime pieces produced by the local Zig toolchain). The executable is a
  **worker**: it serves the same entry points over stdin/stdout frames.
- A world's native program runs in its own child process, launched by the
  server with: `no_new_privs`, rlimits (address space, CPU, files, processes,
  core), a **seccomp allowlist** (no `open*`, sockets, `clone`, `exec` after
  start, `ptrace`, no executable `mmap`/`mprotect`), a per-call wall-clock
  deadline, and kill on drop. Crash, timeout or protocol violation poisons the
  program exactly like a Wasm trap.
- Native is **admin-installed and opt-in** (`--world-native`): unlike Wasm,
  the guarantee is the OS sandbox, not an import-free format. Native modules
  are not offered for download to participants.
- It implements the same `WorldProgram` trait as Wasm, so snapshots,
  checkpoints, records and capabilities work unchanged.

### Order of work

1. Hub (this section's first part) with tests: isolation, concurrency, limits,
   eviction, ad hoc creation, CLI and web selection.
2. Capability runtime and a demo app that uses it.
3. Compile service.
4. Native tier.

Each step appends its verification log below.

### 2026-10-07T14-00-00Z Verification: the hub

Built as designed (`src/world_hub.rs`; the session driver now takes a hub).
Details worth recording:

- `serve` holds one `WorldHub` shared by the Iroh protocol, the WebSocket
  bridge and the HTTP invite endpoint. `ServeWorlds` is the storage policy
  (directories under `.id-worlds/<name>/`, one records document per world,
  `--world-module` for the default world only). Flags:
  `--world-max-open` (256), `--world-max-sessions` (1024),
  `--world-idle-secs` (600; `0` keeps worlds open).
- `world` is a routing field stripped from a session's first frame, so no
  frame type changed. `WorldClient::open_session(world)` opens another stream
  on the same connection; the per-connection session bound is 16.
- CLI: `--world NAME` (or `$ID_WORLD`) on every client subcommand, plus
  `id world list` and `id world create NAME`. The browser page has a World
  field (`/world?world=arena`).
- The existing single-world API keeps working: a `WorldService` converts into
  a one-world hub, which cannot create or evict.

Verified: name validation; admin-only creation and listing; outsiders cannot
create, probe or open worlds by naming them (nothing is opened on their
behalf); capabilities do not cross worlds; 16 concurrent first sessions share
one open; session and open-world ceilings refuse and recover; idle durable
worlds make room and reopen on demand; ephemeral worlds never evict; 64
sessions across 8 worlds see only their own world's events; one real Iroh
connection hops across worlds with six clients at once; HTTP invite and
WebSocket join route by world name; and a process test runs one `serve` with
two different programs, creates worlds ad hoc, proves isolation, evicts an
idle world, reopens it with its state, and restarts with every world intact.

### 2026-10-07T14-30-00Z Verification: capabilities

Built as designed: `src/world_caps.rs` (schema, bounds, catalog, ledger) and
the actor runtime (host events, wants diffing, tick timers, presence,
providers). The actor closure became a `WorldActor` struct — the capability
runtime needed shared state across commands, ticks and follow-ups.

Details worth recording:

- The `wants` export is optional at the Wasm level (like `records`), so
  existing modules keep running without it.
- **Subscriptions are capabilities too**: `players` and `time.tick` must be
  granted; a refused subscription notifies the program once
  (`{"cap":"subscription",...,"error":"cap_denied"}`) and never fires. This
  was found by the process test: presence events silently never fired until
  `players` was granted.
- Host events (`WorldEventKind::System { event }`) carry sequence 0: they are
  journaled and replayed, but never sequenced, never counted as world
  events, and never broadcast to sessions. Sessions learn about the view
  change through a views watch and re-send the view — participant events
  still deliver event-then-view in order, so client behavior is unchanged.
- Presence is per session, refcounted; a session's drop guard tells the world
  on every exit path. Ticks fire only while someone is present; the timer
  lives in the actor's `select!` loop.
- **Found while testing (nightly bug):** with
  nightly-2026-10-04-130536d's wasm backend, `List.get` past the second
  element of a string list built by `Str.split_on` returned
  truncated-at-the-first-space elements in the lounge's module (same source
  built in another directory was correct; both LLVM and dev backends;
  recorded for an upstream report). The lounge now stores each pending
  request as a complete JSON object and matches results by id, so it never
  indexes past position 1.

Verified: the ledger (deny, wildcard, grant-on-use), schema bounds and
refusals; the actor (refused request counted, granted request answered with
a real timestamp, `chat.say` commits a host chat line sessions can see,
grant-on-use, ticks only while present, host events and grants journaled,
restore of grants and system events, checkpoint keeps the granted set); the
sandbox (lounge: wants at rest, request on arrival, sanitized name, denial
retired, grant path, snapshot round trip); and a process test (report shows
granted/wanted/missing, the denial is visible in the view, a grant makes the
greeting appear as a real chat event, and grants survive a restart).

### 2026-10-07T15-00-00Z Verification: the compile service

Built as designed (`src/world_compile.rs`; frame `compile`, reply
`compiled`, CLI `id world compile MAIN.ROC [-f extra.roc] [--seed N]`,
serve `--roc-bin`/`--roc-platform` with `$ID_ROC_BIN`/`$ID_ROC_PLATFORM`
defaults). Notes worth keeping:

- The job stages the platform (platform.roc + targets/, 1.3 MiB) into a
  per-request temp directory and rewrites the app's `pf: platform "…"` to
  the staged copy, so any relative platform path in the source works.
- The compiler child runs with a cleared environment (its cache lives in
  the job directory, so concurrent jobs never collide), CPU/address-space/
  file-size rlimits, bounded captured output, and a wall-clock deadline
  with kill. `RLIMIT_NPROC` is deliberately *not* set: it counts the user's
  whole process table (threads included) and would starve the compiler's
  worker threads for reasons unrelated to the job.
- The result goes through the same import-free validation as an upload
  before installation.
- **Found while testing:** `roc` on `PATH` was nightly-2026-08-10 while the
  platform's ABI is nightly-2026-10-04; the old nightly links a module
  whose linear memory is declared 64 MiB (the pinned one: 8.3 MiB), which
  the sandbox refuses. The same source and platform built with both
  nightlies produced different modules. Operators should point
  `--roc-bin` at the nightly the platform was built with; the test
  discovers it the same way.

Verified: spec bounds and platform-path rewriting; the counter's source
compiles, validates and runs (count=1 after one input); and a process test
serves with `--roc-bin`/`--roc-platform`, compiles the counter source via
`id world compile`, plays one input over Iroh, and refuses a wrong admin
token.

---

## 2026-10-07T16-00-00Z Implementation: the native Roc tier

A Roc app now also builds to a **static x64musl executable** (the platform's
`x64musl` target: `crt1.o` + the Zig worker host + musl pieces, all produced
by the local zig toolchain — nothing is downloaded). The executable is a
**worker** (`host/src/native_worker.zig`) that serves the same entry points
as a Wasm module over stdin/stdout frames: init, update, view, records,
wants, snapshot, restore.

- **Model lifecycle by snapshot.** On x64musl the app releases its model
  arguments with refcount conventions this host cannot safely incref (the
  refcount slot is not where the wasm layout keeps it), so the worker never
  keeps a model pointer between calls: after `init` and after every `update`
  it takes the app's own `snapshot`, and before every call it `restore`s
  from that snapshot. Returned strings are leaked (bounded per call) instead
  of released with a mismatched refcount.
- **Spawn hardening** (in the post-fork hook): `no_new_privs`, dumpable off,
  rlimits (address space, CPU, file size, processes, core), Landlock — the
  ruleset denies *every* path except an EXECUTE+READ grant on the worker's
  own file (opened O_PATH by the parent before the fork; pre_exec hooks must
  not touch the filesystem) — and a seccomp allowlist that permits the
  worker's syscalls (including the one pending `execve`, which Landlock
  makes unable to load any other binary) and kills the process on anything
  else, with `mmap`/`mprotect` denied the executable bit via argument
  checks.
- **Wiring.** `install_wasm` detects ELF magic: native requires
  `--world-native` (admin opt-in — the guarantee is the OS sandbox, not an
  import-free format), is stored in the module directory with `0700`
  permissions, is never pinned for peer download, and restores after a
  restart from its stored file (`world_store::restore_program` dispatches on
  the magic). `NativeProgram` implements the same `WorldProgram` trait as a
  Wasm guest, so snapshots, checkpoints, records and capabilities work
  unchanged.

**Nightly caveats recorded for upstream reports:** with
nightly-2026-10-04-130536d's wasm backend, `List.get` past the second
element of a string list from `Str.split_on` returned truncated elements in
some module compositions; on x64musl, `Str`-suffix interpolation
(`"count=${…}"` with nothing after the interpolation) produced an empty
suffix — the examples build views with `Str.concat` instead.

Verified: the native counter plays through `NativeProgram` (init, update,
view, records, wants, snapshot/restore round trip, a dead worker poisons);
the allowlist kills an "outlaw" binary that opens a file and a socket while
the real worker runs under the same filter; and a process test serves with
`--world-native`, installs the worker over Iroh, plays an input, restarts
(state restored from the stored worker + journal) and refuses the same
install without `--world-native`.

---

## 2026-10-07T17-00-00Z Current status

Offered today, per process: a `serve` hosts any number of named worlds (the
worlds hub) with capability-gated sessions over Iroh and WebSocket;
untrusted Roc programs run as import-free Wasm in the sandbox, or — admin
opt-in — as native workers under Landlock+seccomp+rlimits; sources can be
compiled on the server (`id world compile`) with the pinned nightly; the
program's structured records mirror into an iroh-docs document and replicate
peer-to-peer (`id world records` / `id world mirror [--dir] [--follow]`);
world state is durable by fsynced journal + deterministic replay, trimmed
behind verified program checkpoints; capabilities (time, ticks, randomness,
players, `chat.say`, `world.info`) let programs ask the server for effects
as data; three example apps (counter, tic-tac-toe, lounge) build with the
shared platform and run end to end.

Not offered yet:

- **Replicating records in the browser.** The `/world` page reads records
  through the session; iroh-docs replication is the CLI's job.
- **Versioned module upgrades.** Upgrades are admin installs by hash; there
  is no channel or rollback beyond installing a previous hash.
- **Native compilation on the fly.** `id world compile` produces Wasm; a
  native `--target=x64musl` compile path (staging the musl link pieces) is a
  natural follow-up.
