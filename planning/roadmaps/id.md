# Roadmap: id

`id` is the peer-to-peer CLI built on Iroh. Its current growth area is the worlds platform: sandboxed Wasm programs (written in Roc) that run in shared, journaled worlds and render to browsers and SSH terminals.

Sources: [Phase 11](../../.opencode/plans/phase-11-foundations.md), [Phase 12](../../.opencode/plans/phase-12-platform-flexibility.md), [Phase 13](../../.opencode/plans/phase-13-client-shells.md), [next-stage record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md), [`pkgs/id/TODO.md`](../../pkgs/id/TODO.md). How to read this file: [planning README](../README.md).

**Order of work (set by Drew, 2026-10-26): Phase 12 is the goal, and the work needed to get to Phase 12 comes first.** Every item below has its own GitHub issue — including the smallest, furthest-out idea — and priority labels encode the order: the path to Phase 12 is `now` + `priority/high`, Phase 12 is `next`, Phase 13 and long-term ideas are `priority/low`. The dashboard mirrors this roadmap (org Project "Developing Today", Area `id-train`; Grist Program roadmap, Area `id`). An item that turns out blocked, wrong, or superseded gets parked or closed with a note — dropping an issue is always allowed; silently stalling is not. Completed work is recorded in GitHub (closed issues, merged PRs), not in this file.

## Now — the path to Phase 12

**Update 2026-11-21 (proposed in the [results-first plan](../plans/2026-11-21_results-first.md)):** the ordered path to Phase 12 closed on 2026-11-08 (#1235), and 14 of the 18 Phase 12 items are closed. The four that remain are the Now work, together with one end-to-end slice of the #9106 substrate. The phase is finished when its nine exit criteria are each recorded as met by a command, and the first `id` release is tagged. Priority labels and queue bands follow at that plan's step 0.

| Title                                               | Status      | Intent                                                                                                                                                                                                                   | Links        |
| --------------------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------ |
| Many worlds per node                                | in-progress | 1,000 idle / 100 active worlds with a per-world memory budget. Memory budget and eviction are done; the latency target is not: p99 measured 5,650 ms against 50 ms (#557, 2026-11-17). The profile comes before any fix. | F12.12 #557  |
| Delta and binary frames                             | planned     | A typical redraw costs under 10 percent of a full frame. Waits on four protocol decisions, which are asked with defaults.                                                                                                | F12.11 #805  |
| One end-to-end slice of the signed-record substrate | proposed    | Two nodes: a signed record written at an `id://` address, synced, a conflicting write reported, and a CLI command that shows it. The library modules are merged; nothing reaches them from the CLI yet.                  | #9106, #9112 |

## Next — Phase 12: platform flexibility

Make the platform something other people can build on without changing its core: new capabilities added as components, worlds described and reproduced as data, programs written in several languages and upgraded without losing state, and many worlds run on one node and across nodes — all with the same sandbox and authority rules. Full plan: [Phase 12](../../.opencode/plans/phase-12-platform-flexibility.md). Sequencing inside the phase is in the plan; the F12.4 gate bug (#838: the default ceiling is the whole catalog) is fixed before the registry lands.

| Title                                    | Status   | Intent                                                                                                                                      | Links |
| ---------------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| Sync protocol + tags/documents hardening | proposed | Check the Keybase server side, old-style invites and a real accept before P6.3. The tag index (#183) and the removal rules (#195) are done. | #194  |

## Later — Phase 13 and long-term ideas (priority/low)

Phase 13 plan: [phase-13-client-shells.md](../../.opencode/plans/phase-13-client-shells.md). F13.1 (shell contract, #783) and F13.2 (SSH frame transport, #784) are already done; the rest waits behind Phase 12 foundations (F13.7 needs F12.4 + F12.6; F13.4/F13.5 need the F11.9 baseline).

| Title                         | Status  | Intent                                                                               | Links      |
| ----------------------------- | ------- | ------------------------------------------------------------------------------------ | ---------- |
| Presentation capability tiers | planned | Terminal classes detected in-band; tiered rendering instead of one assumed terminal. | F13.4 #786 |
| Kitty keyboard protocol       | planned | Disambiguated keys on the raw input path, so worlds can tell key combinations apart. | F13.5 #787 |
| Native TUI client             | idea    | A ratatui TUI on the shell contract, with a custom efficient presentation protocol.  | #1071      |
| Native GUI client             | idea    | A full GUI app (Tauri/egui/Bevy family) doing what the TUI does, faster and better.  | #1072      |

## Later — Phase 14: hosted, federated, local-first (plan of record #6312)

Decided by Drew on 2026-11-05 in the next-horizons discussion #1240: **all seven themes are planned**, each as a research-and-design core topic with a v1 first-iteration design issue and v2/v3/v4 follow-up outline issues. First implementation items: hosted worlds + discovery; agents-in-worlds is the differentiating spike. Principles: local-first always, a hosted tier as an explicit goal, everything hostable by anyone at any scale, p2p behavior identical to hosted behavior, our hosted instance is the public backstop and never a requirement.

| Topic                                  | Core  | Ladder (v1 design → v4)    |
| -------------------------------------- | ----- | -------------------------- |
| Hosted worlds & always-on hosting      | #6276 | #6283, #6284, #6285, #6286 |
| Discovery, registry & federation       | #6277 | #6287, #6288, #6289, #6290 |
| More shells (mobile, native, hardware) | #6279 | #6296, #6297, #6298, #6299 |
| Agents in worlds (MCP bridge)          | #6280 | #6300, #6301, #6302, #6303 |
| Scale & performance hardening          | #6281 | #6304, #6305, #6306, #6307 |
| Creator tooling & insight              | #6282 | #6308, #6309, #6310, #6311 |

Design order: F12.4 (#1061) and F12.10 (#1067) designs first, then Phase 14 v1 designs in the order hosted worlds → discovery → agents spike → ecosystem → shells → scale → creators. Implementation follows Phase 13.

## Phase tracker — F-item mapping (source of record for #9923)

The Phase tracker panel's per-phase progress counts F-items against live GitHub state, and this table is the mapping it counts from — maintained as plan data here, never copied into the aggregator. Researched and verified against GitHub on 2026-11-14 (task 16681): each ref is the item's dedicated tracking issue, except F11.6 and F11.14, which were delivered by PR with no dedicated issue (per the 2026-10-20 Phase 11 status record). Phase 12 is not in this table: its uniform source is the ordered step list in #1235, which the panel already counts. Phases 0–10 rows were added for #13944 (2026-11-25): their plans number steps, not F-items (P-steps in `doc/2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md` for Phases 0–8, the P9/P10 lists in `.opencode/plans/phase-9-portable-platform.md` for Phases 9–10), so the F-numbering here mirrors those step lists in order (Phase 3's P3.1a–c sub-steps take their own F-numbers F3.2–F3.4, shifting P3.2–P3.8 to F3.5–F3.11; Phase 6's P6.7a–c take F6.7–F6.9, shifting P6.8 to F6.10). Each Phase 0–10 item's ref is the validation audit that walked every step of its phase and closed only when each step was verified or separately tracked — #1075 for Phases 0–8, #1076 for Phase 9, #1077 for Phase 10 — so an item counts done exactly when its phase's audit is closed in live GitHub state.

| Phase | Item   | Ref     | Title                                                 |
| ----- | ------ | ------- | ----------------------------------------------------- |
| 0 | F0.1 | #1075 | Baseline: fix or quarantine the three known failures |
| 0 | F0.2 | #1075 | Toolchain 1.99.0 in rust-toolchain.toml |
| 0 | F0.3 | #1075 | Dependencies, one group per commit |
| 0 | F0.4 | #1075 | Pull pages capped by bytes, not item count |
| 0 | F0.5 | #1075 | rust-version set to 1.96, the true minimum |
| 0 | F0.6 | #1075 | Dial deadlines in the remaining modules |
| 0 | F0.7 | #1075 | mDNS on serve, on by default |
| 0 | F0.8 | #1075 | store.rs FsStore comment corrected to redb |
| 0 | F0.9 | #1075 | Tuple encoding property tests and fuzz target |
| 0 | F0.10 | #1075 | Test harness: regression runner and multi-node helper |
| 0 | F0.11 | #1075 | verify_strict regressions at every call site |
| 0 | F0.12 | #1075 | Nix check uses the pinned toolchain |
| 1 | F1.1 | #1075 | Storage decision record: redb everywhere |
| 1 | F1.2 | #1075 | ChainExt struct, validation, builder and reader |
| 1 | F1.3 | #1075 | Event store on redb with fork freeze and proofs |
| 1 | F1.4 | #1075 | Crash-safety gate for redb |
| 1 | F1.5 | #1075 | Waiting caps: 256 per author, 7 days |
| 1 | F1.6 | #1075 | Store traits over redb |
| 1 | F1.7 | #1075 | Canonical CBOR golden vectors |
| 1 | F1.8 | #1075 | Ingest pipeline: signature verify and fork freeze |
| 1 | F1.9 | #1075 | Identity store moves from libsql to redb |
| 1 | F1.10 | #1075 | Crash gate for the identity store |
| 2 | F2.1 | #1075 | Auth-frontier mapping |
| 2 | F2.2 | #1075 | Admission gate |
| 2 | F2.3 | #1075 | Voiding: authority-based dependents |
| 2 | F2.4 | #1075 | Friendship with its cap, and the dissolve projection |
| 2 | F2.5 | #1075 | Delegation: edges, attenuation, chain search, veto |
| 2 | F2.6 | #1075 | World capabilities without use-count bounds |
| 2 | F2.7 | #1075 | Cross-world reconciliation |
| 2 | F2.8 | #1075 | Property tests over random operation orders |
| 2 | F2.9 | #1075 | Server config: creation limits, provisioning, aliases |
| 3 | F3.1 | #1075 | Range sync on the redb store (P3.1) |
| 3 | F3.2 | #1075 | Fork in a batch isolates and continues (P3.1a) |
| 3 | F3.3 | #1075 | T=0 validation rejects non-termination (P3.1b) |
| 3 | F3.4 | #1075 | Flights chunked under the frame cap (P3.1c) |
| 3 | F3.5 | #1075 | LogSync through p2panda-sync on our traits (P3.2) |
| 3 | F3.6 | #1075 | Method selection: LogSync vs range (P3.3) |
| 3 | F3.7 | #1075 | Blob gating and observe (P3.4) |
| 3 | F3.8 | #1075 | Relay defaults: n0, home-relay deadline (P3.5) |
| 3 | F3.9 | #1075 | Relay map order: QAD-capable relays first (P3.6) |
| 3 | F3.10 | #1075 | Real-network holepunch and QAD tests (P3.7) |
| 3 | F3.11 | #1075 | Relay operations: systemd unit and metrics (P3.8) |
| 4 | F4.1 | #1075 | DataScheme adapter around the crate |
| 4 | F4.2 | #1075 | DGM and the from_join membership fix |
| 4 | F4.3 | #1075 | from_join: immediate update and withholding rule |
| 4 | F4.4 | #1075 | Scheme v1: serde_bytes encoding |
| 4 | F4.5 | #1075 | recent:N and full history modes |
| 4 | F4.6 | #1075 | Lazy full history via member epoch secrets |
| 4 | F4.7 | #1075 | Heads rule: cap 32 |
| 4 | F4.8 | #1075 | Rotation policy and PCS recovery procedure |
| 4 | F4.9 | #1075 | Warnings at 96, 128 and 1,000 members |
| 4 | F4.10 | #1075 | scheme: none notice in UI and metadata |
| 4 | F4.11 | #1075 | DataScheme update cost measurement |
| 4 | F4.12 | #1075 | Welcome size budget: chunking above the frame cap |
| 4 | F4.13 | #1075 | Membership source is the group fold |
| 5 | F5.1 | #1075 | Observed-remove tags in the store |
| 5 | F5.2 | #1075 | Tag index on the fixed tuple encoding |
| 5 | F5.3 | #1075 | Compaction: snapshot at a causal cut |
| 5 | F5.4 | #1075 | Documents: content events and snapshots |
| 5 | F5.5 | #1075 | Search over the tag index |
| 5 | F5.6 | #1075 | Content on removal, kept |
| 5 | F5.7 | #1075 | Aliases as tags |
| 6 | F6.1 | #1075 | Seitan V2 invites: tokens, derivations, proofs |
| 6 | F6.2 | #1075 | Freshness window, default 5 minutes |
| 6 | F6.3 | #1075 | Tickets, option E: standard ticket links |
| 6 | F6.4 | #1075 | Invite events and links |
| 6 | F6.5 | #1075 | Pull policy: open pull with authorized option |
| 6 | F6.6 | #1075 | Fork recovery: remove and re-enrol |
| 6 | F6.7 | #1075 | CLI: groups (P6.7a) |
| 6 | F6.8 | #1075 | CLI: delegation (P6.7b) |
| 6 | F6.9 | #1075 | CLI: documents and tags (P6.7c) |
| 6 | F6.10 | #1075 | Rejection reasons everywhere (P6.8) |
| 7 | F7.1 | #1075 | Relay kit for Cloudflare Tunnel and VPS testing |
| 7 | F7.2 | #1075 | Cloudflare Workers pkarr option (design) |
| 7 | F7.3 | #1075 | DHT opt-in per world |
| 7 | F7.4 | #1075 | Access control: none now, revisit later |
| 7 | F7.5 | #1075 | Deployments and real-network tests |
| 8 | F8.1 | #1075 | Time and witnesses: causal-validity-only scope |
| 8 | F8.2 | #1075 | Retention and pruning |
| 8 | F8.3 | #1075 | Multi-device |
| 8 | F8.4 | #1075 | Views: explorer and web |
| 8 | F8.5 | #1075 | Cross-world rules |
| 8 | F8.6 | #1075 | Warnings UI |
| 8 | F8.7 | #1075 | BeeKEM reconsideration, conditional |
| 8 | F8.8 | #1075 | Clippy and other lints, deferred |
| 9 | F9.1 | #1076 | Portable screen view (P9.1) |
| 9 | F9.2 | #1076 | Input and viewport (P9.2) |
| 9 | F9.3 | #1076 | Execution profiles (P9.3) |
| 9 | F9.4 | #1076 | Standard libraries: Roc JSON and helpers (P9.4) |
| 9 | F9.5 | #1076 | Adapters and examples (P9.5) |
| 9 | F9.6 | #1076 | Standard-library consolidation (P9.6) |
| 9 | F9.7 | #1076 | Server-observed chat history (P9.7) |
| 9 | F9.8 | #1076 | Guest-aware reflow (P9.8) |
| 9 | F9.9 | #1076 | Presence and records readback (P9.9) |
| 9 | F9.10 | #1076 | App scaffolding and inspection CLI (P9.10) |
| 9 | F9.11 | #1076 | Library growth: text and widgets (P9.11) |
| 10 | F10.1 | #1077 | Role-scoped invites and world directory |
| 10 | F10.2 | #1077 | Session recording to replayable fixtures |
| 10 | F10.3 | #1077 | Scheduled events |
| 11    | F11.1  | #238    | Relay kit location                                    |
| 11    | F11.2  | #229    | Firefox E2E isolation                                 |
| 11    | F11.3  | #241    | Explain the LogSync flakes                            |
| 11    | F11.4  | #551    | A fast lane                                           |
| 11    | F11.5  | #552    | Injected time                                         |
| 11    | F11.6  | PR #247 | Per-call fuel metrics and budgets                     |
| 11    | F11.7  | #553    | Host metrics and tracing                              |
| 11    | F11.8  | #539    | Benchmarks                                            |
| 11    | F11.9  | #542    | Bytes per frame                                       |
| 11    | F11.10 | #540    | Split the large modules, with no behavior change      |
| 11    | F11.11 | #543    | One client projection                                 |
| 11    | F11.12 | #227    | Journal format management                             |
| 11    | F11.13 | #554    | Error taxonomy                                        |
| 11    | F11.14 | PR #248 | Cheaper Screen                                        |
| 11    | F11.15 | #555    | Smaller modules                                       |
| 11    | F11.16 | #544    | Replay self-check                                     |
| 11    | F11.17 | #547    | A view trap must not poison the world                 |
| 11    | F11.18 | #541    | Pinned toolchain for checked-in artifacts             |
| 13    | F13.1  | #783    | The shell contract                                    |
| 13    | F13.2  | #784    | SSH as a transport for the frame protocol             |
| 13    | F13.3  | #785    | SSH-over-iroh reachability                            |
| 13    | F13.4  | #786    | Presentation capability tiers                         |
| 13    | F13.5  | #787    | Kitty keyboard protocol on the raw input path         |
| 13    | F13.6  | #788    | Native client shells                                  |
| 13    | F13.7  | #789    | Client-side capability providers                      |
| 13    | F13.8  | #790    | Multi-server, multi-client topology; client-only mode |
