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

The Phase tracker panel's per-phase progress counts F-items against live GitHub state, and this table is the mapping it counts from — maintained as plan data here, never copied into the aggregator. Researched and verified against GitHub on 2026-11-14 (task 16681): each ref is the item's dedicated tracking issue, except F11.6 and F11.14, which were delivered by PR with no dedicated issue (per the 2026-10-20 Phase 11 status record). Phase 12 is not in this table: its uniform source is the ordered step list in #1235, which the panel already counts. Phases 0–10 have no rows because their plans carry no F-item numbering at all (Phase 9's plan included) — the panel states that gap rather than inventing a percentage for them.

| Phase | Item   | Ref     | Title                                                 |
| ----- | ------ | ------- | ----------------------------------------------------- |
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
