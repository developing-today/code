# Phase 13: client shells and capability providers

See the [platform world-state report](../../doc/2026-10-18T18-40-00Z_report_platform_world_state/2026-10-18T18-40-00Z_report_platform_world_state.md) (§13–§15) and the [architecture record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md). This phase builds on [Phase 12](phase-12-platform-flexibility.md); the stepping stones it names are F12.7 (ABI conformance), F12.14 (portals) and F12.12 (many worlds per node). Dependencies are named per task.

## Goal

Phase 12 made the platform something others can build on, on the *host* side. Phase 13 makes it something others can build on, on the *client* side: one versioned shell contract that every client — terminal, web, and native — implements, more than one way to reach a world (iroh, SSH, and SSH-over-iroh), a terminal experience that degrades gracefully across terminal classes instead of assuming one, and capability providers that run on the *client* under the same named, granted, ceilinged rule Phase 12 imposes on the host. A client-only mode stays cheap and always available, so a shell never has to run a server.

## Scope and non-goals

**In scope:** the client↔server frame contract as a written, versioned specification; SSH as a *transport* for that contract (not just a presentation adapter); SSH-over-iroh reachability for third-party SSH clients; presentation capability tiers with in-band detection; the kitty keyboard protocol on the raw input path; a native-shell reference client on the contract; client-side capability providers under the ceiling model; and multi-server/multi-client topology with a documented client-only mode.

**Out of scope:** new *host* capabilities (kv, blob, http.fetch, notify — those are F12.5); the guest ABI itself (F12.7); portals (F12.14); world-to-world messaging semantics; journal replication (F12.13); and fair scheduling (F12.10). Phase 13 consumes those; it does not redo them. Native shells are a reference *client* for the contract, not a new server runtime: the host stays the one authority.

## What "best in class" means here

Exit tests for the whole phase, each measurable.

| Criterion | Target |
| --- | --- |
| The shell contract is written, versioned, and checkable | A spec document plus a conformance check a new shell runs against; two independent shells pass it unchanged |
| SSH carries the frame protocol | `id world join --via ssh` replays the same session as the iroh path, byte-for-byte frames, over an exec channel |
| A third-party SSH client reaches a NAT'd host | SSH-over-iroh connects an unmodified `ssh` client to an `id` world behind NAT |
| One terminal experience, tiered | Ghostty/kitty/wezterm get kitty keyboard + graphics; xterm-class gets a diff-rendered cell grid; dumb terminals get line mode — detected in-band, identically over SSH and iroh |
| A native shell is swappable | A non-terminal shell renders the same world and sends the same input events, with no host change |
| A client capability respects the ceiling | A named, granted client provider (local file read) is refused when over its ceiling, with an audit entry, and no authority path exists outside named ceilinged providers |
| Client-only stays cheap | A client connects to two worlds at once without becoming a server; `id` documents the client-only path |

## Architecture

### The three layers (unchanged, restated once)

1. **Transport envelope** — SSH channel bytes, iroh QUIC stream frames, WebSocket messages.
2. **Session protocol** — `ClientFrame` / `ServerFrame` JSON (the `ClientFrame` and `ServerFrame` enums in `pkgs/id/src/world_session/types.rs`). Identical under every transport.
3. **Guest ABI** — the `plaza_*` Wasm exports (`sandbox.rs:1-8`), a server↔guest layer separate from the client protocol.

Phase 13 works almost entirely in layers 1 and 2, and names layer 3 only as the F12.7 dependency beneath the contract.

### Client-shell variants

A *shell* is anything that speaks layer 2 and renders a world. Four variants share the contract:

- **Terminal shell** — `world_ssh.rs` today (presentation adapter) and `id world join`'s pump. Becomes a first-class transport (F13.2) with tiered presentation (F13.4, F13.5).
- **Web shell** — `web/src/world.ts` over the `/ws/world` WebSocket bridge (`src/web/world_ws.rs:51`). The reference for "a client is cheap and stateless".
- **Native shells** — Bevy, Qt, .NET, Unity, Electron — implementing the same contract for what a terminal cannot do (3D, audio, native widgets). The contract and the F12.7 conformance suite are what make a shell swappable (F13.6).
- **Client-only / headless** — a client that holds sessions to several worlds without serving. `id world join/stats/logs/record/replay` already act as remote iroh clients via `WorldClient` (`world_net.rs:157`); F13.8 makes multi-world client sessions and client-only mode explicit and documented.

### Capability-provider model

Host capability providers are the hard-coded `CATALOG` (the `CATALOG` constant in `pkgs/id/src/world_caps.rs`) executed by a string `match`; F12.4 turns them into manifested, registered components. Phase 13 extends the *same* model to the client:

- A client capability is a **named provider** with a manifest (name, version, argument schema, bounds, required scope, default ceiling), exactly like an F12.4 host provider.
- It is **granted** by the world/admin and **ceilinged** per world, and each use carries an **audit entry** — the Phase 12 rule "no new authority path except through a named, ceilinged provider" applies unchanged to local resources (files, GPU, network beyond the p2p link).
- Client providers **never widen the guest ABI**: the guest still sees no imports (`sandbox.rs:1-8`). A client provider is a *shell-side* effect the guest requests through the same capability protocol, resolved on the client where the resource lives.
- The distinction to keep is **who holds the resource**: host providers (time, schedule, world state) stay on the server; client providers (a local file, a GPU buffer, a native notification) execute on the client that granted them, and their result travels back as a bounded capability reply — never as a new host-authority path.

### Data and sync touchpoints with the existing platform

- **Events.** `Input`, `Resize`, `Paste` are client frames (the `ClientFrame` enum in `pkgs/id/src/world_session/types.rs`); guest events (`Input` bytes, `System`) are the durable journaled kind (`sandbox.rs:677-685`). The contract must keep the two kinds distinct: ephemeral per-connection context (viewport, focus, partial-line typing) never enters the journal; committed program events do. This is the same separation Phase 9 (P9.2) established and F11.11's projection enforces.
- **Worlds.** Many worlds already live in one process through `WorldHub` (lazy open, single-flight restore, idle eviction, bounds — `world_hub.rs`). Multi-server/multi-client (F13.8) is a *client* affordance on top of this, not a new server model: a client names a world per session, and each session holds a lease.
- **Providers.** Phase 13 client providers depend on the F12.4 registry for the manifest shape, and on F12.6 audit for the audit-entry contract. Without those two, client providers would reinvent manifesting and auditing on the client; with them, the client reuses the same types.
- **Frames vs journal.** Frames are the ephemeral transport; the journal is the durable record. F13.2 (SSH transport) must transport frames without touching the journal; F13.4/5 (presentation) may only change how a *view* is rendered, never what is recorded.

## Security and trust

- **No new guest imports.** Nothing in Phase 13 adds a Wasm import; the guest remains import-free (`sandbox.rs`).
- **Client providers are ceilings, not capabilities.** A client provider widens what a *shell* may do on its own machine, not what a guest may cause elsewhere. The ceiling and audit rules from F12.5/F12.6 apply; a client provider ships off by default, behind a ceiling, with a refusal test.
- **SSH trust boundaries.** SSH-over-iroh (F13.3) tunnels an *unmodified* third-party SSH client into a NAT'd `id` host. The trust decision stays where it is today: host keys/known_hosts on the SSH side, peer IDs (public keys) on the iroh side. The tunnel must not become a bypass for identity — a peer that reaches the SSH server through iroh still authenticates to `id` the normal way.
- **The projection holds.** No new frame may leak a seed or host event past F11.11's single client projection. The shell contract is written against the projection, and its conformance check re-asserts the seed-leak test.
- **Client-only means no ambient authority.** A client-only process must not silently become a server or an authority. F13.8 documents the exact flag boundary.

## Workstreams

Tasks are numbered F13.n. Each names an exit criterion a test or CI job can check.

### A. The contract and transports

- **F13.1 The shell contract.** Write the `ClientFrame`/`ServerFrame` protocol down as a versioned "shell contract": the screen document, typed input (key/paste/focus/resize), the session lifecycle (join, snapshot, event, view, records, module install), capability negotiation (presentation tiers), and the journaled-vs-ephemeral boundary. Names F12.7 as the ABI layer beneath it and F12.14/F12.12 as stepping stones. Exit: a spec document plus a conformance check; the terminal and web shells both pass it unchanged.
- **F13.2 SSH as a transport for the frame protocol (`id world join --via ssh`).** Run the frame protocol over an SSH exec channel for constrained networks and jump hosts. Today exec requests are refused (`world_ssh.rs:252`) and `WorldClient` speaks iroh only (`world_net.rs:157`). Exit: `id world join --via ssh` and the iroh path produce identical frames for a scripted session, with a test that unwraps both envelopes to the same bytes.
- **F13.3 SSH-over-iroh reachability (and iroh-over-SSH fallback).** Tunnel SSH over iroh so a third-party SSH client reaches a NAT'd `id` host (tailnet-like); iroh-over-SSH only where UDP is blocked but port 22 is open. Exit: an unmodified `ssh` client connects to a world behind NAT through the tunnel; a test proves the fallback path negotiates when UDP is denied.

### B. Presentation tiers

- **F13.4 Presentation capability tiers with in-band terminal detection.** Target ghostty/kitty/wezterm as tier 1 (kitty keyboard protocol, kitty graphics or sixel, synchronized output, high fps), xterm-class as tier 2 (escape-timeout keys, cell grid, batching and diff rendering), and line mode as tier 3. Detection is in-band (XTGETTCAP, `CSI ? u`, DECRQM) so it works identically over SSH and iroh. Exit: the three tiers are detected and rendered correctly in one test harness, and every tier benefits from the F11.9-baselined batching.
- **F13.5 Kitty keyboard protocol on the raw input path.** Negotiate `CSI > u` where supported so Escape vs Alt is unambiguous and key events go beyond the legacy byte stream; extend `Key.parse` and the browser encoder to match. Exit: a kitty-protocol terminal and the web encoder produce the same key event for the same keystroke, proven by a parity test.

### C. Native shells

- **F13.6 Native client shells on the shell contract.** A reference native shell (Bevy or Qt) that implements the shell contract for what a terminal cannot do (3D, audio, native widgets), plus a decision record on the long-term interface (WIT/component model from F12.8). Depends on F13.1 and F12.7. Exit: the native shell renders the same world and emits the same input events as the terminal shell, and passes the F13.1 conformance check.

### D. Client capabilities

- **F13.7 Client-side capability providers under the ceiling model.** Named, granted, ceilinged providers that run on the client for local resources (a bounded local file read, a GPU buffer, a native notification), manifesting with the F12.4 shape and auditing with the F12.6 contract. The guest requests them through the existing capability protocol; the reply is a bounded capability reply, never a new host-authority path. Depends on F12.4 and F12.6. Exit: a client provider is added without editing the host's `CATALOG`, appears under a ceiling, is refused when over it, and leaves an audit entry.

### E. Topology

- **F13.8 Multi-server, multi-client topology; client-only mode always supported.** Clients connect to several worlds and servers and to other clients at once, and `id` is often client and server simultaneously. A client-only mode must always exist (cheap web load, SSH-only clients); SSH servers in `id` stay rare and opt-in. Depends on F13.1. Exit: one client holds live sessions to two worlds at once, the client-only flag boundary is documented and tested to add no authority, and a world's journal is unchanged by a second client joining it.

## Sequencing

```text
F12.7 (guest ABI spec)          -> F13.1 -> F13.2, F13.6, F13.8
F13.1 (shell contract)          -> F13.4 -> F13.5
F13.2 -> F13.3                  (SSH transport first, then tunnelling)
F12.4 + F12.6 (registry, audit) -> F13.7
F13.4/F13.5 after the F11.9 batching baseline exists
```

F13.1 is the highest-return task, for the same reason F12.1 and F12.4 were in Phase 12: it turns a client-side change into data (a conformance check) instead of core-code edits. F13.7 is the other early slice, but it cannot start until F12.4 and F12.6 land, so its design is written now and its implementation is gated.

## Acceptance criteria

Each F13.x above carries an exit criterion. The consolidated checkpoints are:

- [ ] `.opencode/plans/phase-13-*.md` committed (this file) — closes #564
- [ ] The plan holds D1-050..D1-056: SSH frame transport (F13.2), SSH-over-iroh (F13.3), presentation tiers (F13.4), kitty keyboard (F13.5), native shells (F13.6), client capabilities (F13.7), multi-server topology (F13.8)
- [ ] It names F12.7, F12.14 and F12.12 as stepping stones
- [ ] One implementation issue per F13.x item references this plan and #564

## Risks

- **Ordering debt.** F13.7 (client capabilities) and F13.4/5 (presentation) rest on Phase 12 work that is not yet implemented. The plan designs them now and gates their implementation on F12.4/F12.6 and F11.9 respectively, so a worker cannot build on an unbuilt foundation.
- **Authority creep on the client.** Client providers widen what a *shell* may do on its own machine. Each ships off by default, behind a ceiling, with an audit entry and a refusal test; none adds a Wasm import.
- **Tunnelling as an identity bypass.** SSH-over-iroh must not let a peer reach the SSH server through iroh and skip `id` authentication. The trust boundary stays at host keys + peer IDs, unchanged.
- **Forward-only compatibility.** Any new frame kind is forward-only for older binaries (#227). F11.12 must land before the first Phase 13 frame type; the shell contract is versioned for the same reason.
- **Scope.** Eight tasks, several XL. F13.1, F13.2, and the F13.7 design are the first slice; the rest is decided after their exit tests.

## Open questions

1. **Which native shell first?** Bevy (Rust-native, same language as the server) vs Qt/.NET/Unity (broader app surface). The decision record in F13.6 answers it; recommend Bevy for the reference because it shares the toolchain.
2. **`--via ssh` priority.** Is SSH-as-transport worth shipping before SSH-over-iroh, or is the tunnel the one users actually need? RPT §16 Q5 leaves this open; F13.2 before F13.3 is the conservative order, but the two may swap.
3. **Client-provider scope.** Which local resources (file read, GPU buffer, notification) are in the first client-provider slice, and does the world/admin grant them or the client's own user? F13.7's design must answer "who grants".
4. **WIT / component model.** F12.8 records the decision; F13.6 consumes it. If F12.8 chooses WIT, the shell contract may need a second, WIT-shaped edition — flag it, do not block on it.

## References

- [Phase 12 plan](phase-12-platform-flexibility.md) (F12.4 registry, F12.6 audit, F12.7 ABI, F12.12 many worlds, F12.14 portals)
- [Phase 11 plan](phase-11-foundations.md) (F11.9 baseline, F11.11 projection, F11.12 journal format)
- [Phase 9 and 10 plan](phase-9-portable-platform.md) (P9.2 input/viewport, the ephemeral-vs-journaled boundary)
- [Platform world-state report](../../doc/2026-10-18T18-40-00Z_report_platform_world_state/2026-10-18T18-40-00Z_report_platform_world_state.md) (§13 transport, §14 presentation tiers, §15 native clients and client capabilities)
- [Architecture record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md)