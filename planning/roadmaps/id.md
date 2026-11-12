# Roadmap: id

`id` is the peer-to-peer CLI built on Iroh. Its current growth area is the worlds platform: sandboxed Wasm programs (written in Roc) that run in shared, journaled worlds and render to browsers and SSH terminals.

Sources: [Phase 11](../../.opencode/plans/phase-11-foundations.md), [Phase 12](../../.opencode/plans/phase-12-platform-flexibility.md), [Phase 13](../../.opencode/plans/phase-13-client-shells.md), [next-stage record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md), [`pkgs/id/TODO.md`](../../pkgs/id/TODO.md). How to read this file: [planning README](../README.md).

**Order of work (set by Drew, 2026-10-26): Phase 12 is the goal, and the work needed to get to Phase 12 comes first.** Every item below has its own GitHub issue — including the smallest, furthest-out idea — and priority labels encode the order: the path to Phase 12 is `now` + `priority/high`, Phase 12 is `next`, Phase 13 and long-term ideas are `priority/low`. The dashboard mirrors this roadmap (org Project "Developing Today", Area `id-train`; Grist Program roadmap, Area `id`). An item that turns out blocked, wrong, or superseded gets parked or closed with a note — dropping an issue is always allowed; silently stalling is not. Completed work is recorded in GitHub (closed issues, merged PRs), not in this file.

## Now — the path to Phase 12

| Title                                   | Status      | Intent                                                                                                                                                                                                                                                                                        | Links                                                                               |
| --------------------------------------- | ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Validate the delivered work             | in-progress | Phases 9 and 10 were delivered faster than they were verified, and several Phase 11 items are claimed done on contradictory records. Dedicated validation tasks audit each phase against its plan, run the full test layers, add missing tests, and file a tracked issue for every gap found. | #1075 (Phases 0–8 + platform completion), #1078 (delivered Phase 11), umbrella #796 |

## Next — Phase 12: platform flexibility

Make the platform something other people can build on without changing its core: new capabilities added as components, worlds described and reproduced as data, programs written in several languages and upgraded without losing state, and many worlds run on one node and across nodes — all with the same sandbox and authority rules. Full plan: [Phase 12](../../.opencode/plans/phase-12-platform-flexibility.md). Sequencing inside the phase is in the plan; the F12.4 gate bug (#838: the default ceiling is the whole catalog) is fixed before the registry lands.

| Title                                    | Status      | Intent                                                                                                                                                                                                          | Links                  |
| ---------------------------------------- | ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| First new providers                      | planned     | A bounded key-value store, content-addressed blobs, an allowlisted `http.fetch` (off by default) and rate-limited notifications — the notifications provider also covers the old "outbound notifications" idea. | F12.5 #1062            |
| Audit log and telemetry                  | planned     | A structured audit of admin actions and provider use, with OpenTelemetry export.                                                                                                                                | F12.6 #1063            |
| Delta and binary frames                  | planned     | A typical redraw costs under 10 percent of a full frame.                                                                                                                                                        | F12.11 #805            |
| Many worlds per node                     | planned     | 1,000 idle / 100 active worlds with a per-world memory budget.                                                                                                                                                  | F12.12 #557            |
| Journal replication                      | planned     | Read replicas over a signed stream; a world keeps a single writer.                                                                                                                                              | F12.13 #806            |
| Portals                                  | planned     | Cross-world messaging through a capability both worlds' admins have granted.                                                                                                                                    | F12.14 #1068           |
| Authoring tools                          | planned     | `id world dev`: watch, rebuild, hot-swap, fixture replay, fuzz harness. (`id world doctor`, F12.16, is done.)                                                                                                   | F12.15 #558            |
| Denial-of-service test suite             | planned     | Fuel, memory, journal, frame, session, schedule and provider bounds all refused with documented codes, in CI.                                                                                                   | F12.18 #560            |
| Sync protocol + tags/documents hardening | proposed    | Bounds on frame buffering and batch bytes, held-message limits, joiner stall fixes; tag index and claim rules. Older phase leftovers, kept tracked.                                                             | #183, #194, #195       |

## Later — Phase 13 and long-term ideas (priority/low)

Phase 13 plan: [phase-13-client-shells.md](../../.opencode/plans/phase-13-client-shells.md). F13.1 (shell contract, #783) and F13.2 (SSH frame transport, #784) are already done; the rest waits behind Phase 12 foundations (F13.7 needs F12.4 + F12.6; F13.4/F13.5 need the F11.9 baseline).

| Title                               | Status  | Intent                                                                                                         | Links      |
| ----------------------------------- | ------- | -------------------------------------------------------------------------------------------------------------- | ---------- |
| Presentation capability tiers       | planned | Terminal classes detected in-band; tiered rendering instead of one assumed terminal.                           | F13.4 #786 |
| Native client shells                | planned | A reference native shell on the shell contract. Umbrella: #538. Prerequisite: session identity on input, #840. | F13.6 #788 |
| Native TUI client                   | idea    | A ratatui TUI on the shell contract, with a custom efficient presentation protocol.                            | #1071      |
| Native GUI client                   | idea    | A full GUI app (Tauri/egui/Bevy family) doing what the TUI does, faster and better.                            | #1072      |

## Later — Phase 14: hosted, federated, local-first (plan of record #6312)

Decided by Drew on 2026-11-05 in the next-horizons discussion #1240: **all seven themes are planned**, each as a research-and-design core topic with a v1 first-iteration design issue and v2/v3/v4 follow-up outline issues. First implementation items: hosted worlds + discovery; agents-in-worlds is the differentiating spike. Principles: local-first always, a hosted tier as an explicit goal, everything hostable by anyone at any scale, p2p behavior identical to hosted behavior, our hosted instance is the public backstop and never a requirement.

| Topic                                  | Core  | Ladder (v1 design → v4)    |
| -------------------------------------- | ----- | -------------------------- |
| Hosted worlds & always-on hosting      | #6276 | #6283, #6284, #6285, #6286 |
| Discovery, registry & federation       | #6277 | #6287, #6288, #6289, #6290 |
| Program ecosystem (share/fork/gallery) | #6278 | #6291, #6293, #6294, #6295 |
| More shells (mobile, native, hardware) | #6279 | #6296, #6297, #6298, #6299 |
| Agents in worlds (MCP bridge)          | #6280 | #6300, #6301, #6302, #6303 |
| Scale & performance hardening          | #6281 | #6304, #6305, #6306, #6307 |
| Creator tooling & insight              | #6282 | #6308, #6309, #6310, #6311 |

Design order: F12.4 (#1061) and F12.10 (#1067) designs first, then Phase 14 v1 designs in the order hosted worlds → discovery → agents spike → ecosystem → shells → scale → creators. Implementation follows Phase 13.
