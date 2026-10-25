# The id shell contract, version 1

Reference for Phase 13 task F13.1 (issue #783): the frame protocol every `id`
shell speaks to a world host, written down as a versioned, checkable contract.
See the [Phase 13 plan](../../.opencode/plans/phase-13-client-shells.md).

**Status:** version 1, revision 1. The machine-readable contract is
[`pkgs/id/contract/shell-contract.v1.json`](../../pkgs/id/contract/shell-contract.v1.json).
That file is the source of truth; this document explains it and does not
repeat its tables. `id world contract` prints it.

## Why a contract

Before F13.1 the protocol existed only as the Rust `ClientFrame` and
`ServerFrame` enums (`pkgs/id/src/world_session/types.rs`). Each shell
followed them by hand: the SSH terminal (`pkgs/id/src/world_ssh.rs`), the
`id world join` pump (`pkgs/id/src/commands/world.rs`), and the browser
(`pkgs/id/web/src/world.ts`). Nothing caught a shell that drifted. The first
run of the conformance check found three shell bugs this way (see
[Findings](#findings-from-the-first-conformance-run)).

With the contract as data, a new shell (a native Bevy or Qt client, F13.6)
proves conformance by recording a session and running one command. It does
not have to read host code, and the host does not have to change.

## Where it sits

The platform has three layers (Phase 13 plan, "Architecture"):

1. **Transport envelope.** SSH channel bytes, iroh QUIC stream frames,
   WebSocket messages.
2. **Session protocol.** This contract. It is identical under every transport.
3. **Guest ABI.** The `plaza_*` Wasm exports. They are specified by **F12.7**,
   the layer beneath this contract. A guest never sees a frame, and a shell
   never sees the ABI.

Two Phase 12 items are stepping stones that the contract will absorb as
additive revisions:

- **F12.12** (many worlds per node): today's `world` routing field.
- **F12.14** (portals): cross-world messages, which would arrive as new server
  frames.

## Versioning

- `version` is the wire major. Version 1 is the protocol behind the
  `/id-world/1` ALPN (`pkgs/id/src/world_net.rs`), and a test pins the two
  together.
- `revision` counts additive changes within a version: a new optional field,
  or a new server frame that old shells may ignore.
- Removing or changing a frame or field needs version 2 and a new ALPN.
- **A shell sends only frames and fields the contract names.** It ignores
  server frame types and fields it does not know, because a later revision may
  add them. The conformance check is strict about what each side _sends_.
  Shells are tolerant about what they _read_.
- **New frame kinds are gated.** Any new frame kind is forward-only for older
  binaries (#227). The plan therefore requires F11.12 (journal format
  management) to land before the first Phase 13 frame type. F11.12 has not
  landed: only the `JOURNAL_VERSION` check exists, and the `check`/`backup`
  tooling is open. Version 1 therefore adds **no** frame kind. Everything it
  describes already existed on `main`.

## Transports

| Transport | Envelope                                  | One session is                  |
| --------- | ----------------------------------------- | ------------------------------- |
| iroh      | `u32` big-endian length prefix, then JSON | one bidirectional stream        |
| WebSocket | one text message per frame                | one connection                  |
| SSH exec  | `u32` big-endian length prefix, then JSON | one exec channel (`id-world/1`) |

Every frame is one UTF-8 JSON object of at most 1 MiB (`MAX_FRAME_BYTES`).
The SSH exec transport is F13.2. It reuses the iroh envelope byte for byte,
so unwrapping either one gives identical frames.

## Session lifecycle

A session moves through **phases**. Each client frame lists the phases in
which a shell may send it. Each frame a shell may open a session with names
the phase it `opens`:

- **opening.** The first frame, which may carry the `world` routing field.
  Query frames (`info`, `records`, `records_ticket`, `download_module`) open
  **query**. `join` opens **joined**. `install_begin` opens **install**. Every
  other opening frame (`invite`, the admin and policy frames, `caps`,
  `execution_profile_info`, `compile`) gets one reply, and then the host ends
  the session (**closed**).
- **query.** Further query frames, or `join`.
- **install.** `install_chunk` frames, then `install_end`.
- **joined.** The participant's live session: `chat`, `input`, `paste`,
  `resize`, `records`, `info`, `records_ticket`, `caps`, delegation and the
  admin capability and profile frames. Every frame keeps the session joined.

Before a join, an `error` reply ends the session. After a join, most errors
are answers, and the session continues.

## The journaled-vs-ephemeral boundary

Every client frame has a **class**:

| Class        | Meaning                                                                  | Frames                                        |
| ------------ | ------------------------------------------------------------------------ | --------------------------------------------- |
| `event`      | Becomes one committed world event: journaled, replayed, and projected.   | `chat`, `input`, `paste`                      |
| `ephemeral`  | Connection-local presentation context. Never journaled and never shared. | `resize`                                      |
| `query`      | Read-only. Never journaled.                                              | `info`, `records`, `caps`, ...                |
| `control`    | Session lifecycle. Never journaled itself.                               | `join`, `install_chunk`, `install_end`        |
| `admin`      | Authority-bearing configuration. May write a journal entry.              | `invite`, `update_caps`, `install_begin`, ... |
| `delegation` | Mints a narrower capability. Never widens authority.                     | `attenuate`, `delegate_to_friend`             |

Server frames are `durable` (`snapshot` and `event`, built from committed and
projected state), `ephemeral` (`view`, a presentation of the current state for
this connection's viewport) or `reply`.

This is the Phase 9 (P9.2) separation, now executable. Viewport and partial
typing never enter the journal. Committed input does. The test
`the_journaled_vs_ephemeral_boundary_is_what_the_contract_says` sends every
frame that is valid in the joined phase and checks the world's sequence after
each one. An `event` frame must commit exactly one event. Every other class
must leave the journal alone. Reclassifying a frame in the JSON changes what
the test demands.

Focus is not in version 1. A focus frame is reserved for the presentation work
(F13.4/F13.5), behind the F11.12 gate.

## The client projection

Every server frame is built from F11.11's single client projection. The
contract records the projection's one hard rule as data:
`projection.redacted_keys` is `["seed"]`, and any `seed` anywhere in a server
frame must be `0`. The conformance check enforces it, and the session tests
re-assert the F11.11 seed-leak test against the contract. Both a live install
and a reinstall with nonzero seeds produce transcripts that pass the check. A
transcript with a leaked seed fails it.

## Capability negotiation

Version 1 negotiates nothing in-band. It has one optional feature:

- **`surface`.** A `view` may carry a typed `surface` (the portable screen
  document). A shell that does not understand it renders `data`.

Presentation tiers (F13.4), the kitty keyboard protocol (F13.5) and client
capability providers (F13.7) are listed under `reserved`. Each needs a new
frame kind and waits for F11.12.

## Running the conformance check

A shell records each session as JSON lines, one frame per line:

```json
{"from": "client", "frame": {"type": "join", "capability": "..."}}
{"from": "server", "frame": {"type": "snapshot", "snapshot": {"...": "..."}}}
```

It then runs `id world contract --check session.jsonl` (`-` reads stdin). The
command prints `N frames conform to shell contract v1.1`, or lists each
violation as `frame <i> (<side>): <reason>` and exits nonzero. The check
reports:

- unknown frame types
- missing or unknown fields
- wrong JSON types or non-hex `data_hex`
- out-of-range integers, such as a `resize` wider than 500
- strings over their byte bound, and values outside an `enum`
- `uses` anywhere in a delegation
- a frame sent in the wrong phase, or after the host closed the session
- a leaked seed

The validator lives in `pkgs/id/src/shell_contract.rs`, with a TypeScript port
for the browser tests in `pkgs/id/web/src/shell-contract.ts`.

## How the contract stays true

The contract would be worthless if it drifted from the host. These tests in
`pkgs/id/src/shell_contract/tests.rs` pin it:

- **Variants.** The client frame names are read from serde's own
  unknown-variant error, and they must equal the contract's list. The server
  frame list is enforced by an exhaustive `match`, so a new `ServerFrame`
  variant does not compile until it is named, and then it must also be in the
  contract.
- **Required fields.** For every client frame, the smallest valid sample built
  from the contract must deserialize. Removing any required field must fail in
  serde. `null` must be accepted exactly where the contract says a field is
  nullable. This caught `compile.native`, a `serde(default)` field that may be
  absent but not `null`.
- **Limits.** The contract's numbers must match the host constants: frame
  size, ALPN, capability length, chunk size, module size and paste size.

These session tests in `pkgs/id/src/world_session/contract_tests.rs` run real
sessions through the check: a full joined session, query sessions,
single-reply opening sessions, an upload, and the journaling boundary.

These shell tests check the shells themselves:

- The terminal shells: `terminal_shell_frames_conform_to_the_shell_contract`
  (SSH raw mode and `cmd:` mode) and
  `the_pump_command_mode_maps_the_shared_grammar_to_contract_frames`
  (`id world join`).
- The web shell: `pkgs/id/web/src/world-contract.test.ts`. It drives the
  unmodified `world.ts` over a fake WebSocket and checks every frame the page
  sends.

## Findings from the first conformance run

The first run of the check found three bugs. Each is fixed in this change (#836):

1. **`id world join --command-mode` sent invalid frames.** `/stats`, `/caps`
   and `/records` were sent without their required `capability`.
   `/documents`, `/document`, `/logs` and `/help` were sent as frame types
   that do not exist. The host answered "invalid world frame". Now the pump
   fills in the session's capability and answers the local commands on
   stderr, so stdout stays a pure stream of frames.
2. **`/stats` in an SSH `cmd:` session always failed.** The shell sends
   `info` from the joined session, but the host answered "already joined",
   even though the SSH shell already had a renderer for the reply. The host
   now answers `info` in the joined phase, authorized by the session's own
   capability, the same way it answers `records`.
3. **A wide SSH terminal was refused a view.** A pty wider than 500 columns or
   taller than 200 rows produced a `resize` outside the contract, and the
   host answered with an error. The SSH shell now clamps to the contract's
   bounds, as the browser already did.

## References

- [Phase 13 plan](../../.opencode/plans/phase-13-client-shells.md), F13.1 and
  "Architecture"
- [Phase 12 plan](../../.opencode/plans/phase-12-platform-flexibility.md):
  F12.7 guest ABI, F12.12 many worlds, F12.14 portals
- [Phase 11 plan](../../.opencode/plans/phase-11-foundations.md): F11.11
  projection, F11.12 journal format
- Issues: #783 (F13.1), #227 (forward-only compatibility), #543 (F11.11)
- [SSH input modes decision](../2026-10-24T20-18-37Z_decision_ssh_input_modes/2026-10-24T20-18-37Z_decision_ssh_input_modes.md)
