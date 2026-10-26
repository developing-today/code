# Roadmap: id

`id` is the peer-to-peer CLI built on Iroh. Its current growth area is the worlds platform: sandboxed Wasm programs (written in Roc) that run in shared, journaled worlds and render to browsers and SSH terminals.

Sources: [Phase 9 and 10](../../.opencode/plans/phase-9-portable-platform.md), [Phase 11](../../.opencode/plans/phase-11-foundations.md), [Phase 12](../../.opencode/plans/phase-12-platform-flexibility.md), [next-stage record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md), [`pkgs/id/TODO.md`](../../pkgs/id/TODO.md). How to read this file: [planning README](../README.md).

The next-stage record sets the order: foundations first (a CI signal that can be believed, visible costs, code that is easy to change), then flexibility. Phase 12 does not start in earnest until Phase 11's CI and visibility tasks are done.

## Now

| Title                                   | Status      | Intent                                                                                                                  | Links                                                                                                       |
| --------------------------------------- | ----------- | ----------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| A CI signal that can be trusted         | in-progress | The Nix check had no recorded success in 392 runs. Fix the flakes and the disk failures so a red check means something. | F11.1 to F11.4, #229, #241, #255, #257, #176, #300                                                          |
| One place decides what a client may see | in-progress | Seed, chat history and host events follow the same rule through a single projection, with a test per row.               | F11.11 (#250, #256), #242, [chat visibility fix](../../doc/2026-10-18T04-00-00Z_fix_world_chat_visibility/) |
| Journal format management               | planned     | A version header, a check command and a backup command, so a rollback to an older binary is safe.                       | F11.12, #227                                                                                                |
| Cost visibility                         | in-progress | Per-call fuel is measured and guarded by budgets. Host metrics, benchmarks and bytes per frame follow.                  | F11.6 (merged in #247), F11.7, F11.8, F11.9                                                                 |

## Next

| Title                                   | Status   | Intent                                                                                                                        | Links                              |
| --------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------- | ---------------------------------- |
| Split the large modules                 | planned  | `world.rs` is 6,120 lines and `cmd_world` has a 730 KB stack frame. Split with no behavior change.                            | F11.10                             |
| Injected clock                          | planned  | Schedule and tick tests stop sleeping.                                                                                        | F11.5                              |
| Typed error codes                       | planned  | Every capability error carries a documented code, including the reason `schedule.at` drops today.                             | F11.13                             |
| Replay self-check                       | planned  | `id world verify` replays a journal twice and reports any divergence.                                                         | F11.16                             |
| A view trap does not poison the world   | planned  | A guest `view` that traps shows an error screen and recovers on the next healthy event.                                       | F11.17                             |
| Pinned toolchain for checked-in modules | planned  | Every example is built with the compiler pinned in `nix/roc.nix`, and CI checks it.                                           | F11.18                             |
| Cheaper guests                          | planned  | A cheaper `Screen` and smaller Wasm modules (each at or under 400 KB).                                                        | F11.14, F11.15                     |
| Sync protocol hardening                 | proposed | Bounds on frame buffering and batch bytes, a stall fix for joiners with missing control messages, held messages with a limit. | #178, #179, #180, #186, #187, #189 |
| Tags and documents follow-ups           | proposed | Tag event bodies travel with the event, a persisted incremental tag index, who may clear a claim, document removal rules.     | #182, #183, #185, #195             |
| Capability ceilings, written down       | proposed | Define the server and per-world ceilings. The code exists, the definition does not.                                           | #230                               |
| Seitan V2 check                         | proposed | Check the Keybase server side and old-style invites before building on them.                                                  | #194                               |

## Later

These are the Phase 12 tasks. The order inside the phase is in the plan.

| Title                                   | Status  | Intent                                                                                                                          | Links  |
| --------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------- | ------ |
| Worlds as data                          | planned | A `world.toml` that `id world apply` reconciles, with a dry run and a drift report.                                             | F12.1  |
| Forking and templates                   | planned | A world starts from another world's module and snapshot, with provenance.                                                       | F12.2  |
| Builder and moderator roles             | planned | Journaled grants to install modules or revoke participants, with an audit record for each use.                                  | F12.3  |
| Provider registry                       | planned | A new host capability is a manifest and a provider, with no edit to `world.rs`.                                                 | F12.4  |
| First new providers                     | planned | A bounded key-value store, content-addressed blobs, an allowlisted `http.fetch` and rate-limited notifications.                 | F12.5  |
| Audit log and telemetry                 | planned | A structured audit of admin actions and provider use, with OpenTelemetry export.                                                | F12.6  |
| ABI specification and conformance suite | planned | The `plaza_*` exports and capability protocol written down and versioned.                                                       | F12.7  |
| More guest languages                    | planned | A Rust SDK and one of AssemblyScript or Zig, each passing the conformance suite. The component model decision is recorded here. | F12.8  |
| Program upgrade with state migration    | planned | A failed upgrade rolls back, shown by a test.                                                                                   | F12.9  |
| Fair scheduling and quotas              | planned | One spinning world cannot raise another world's p99 latency by more than 10 percent.                                            | F12.10 |
| Delta and binary frames                 | planned | A typical redraw costs under 10 percent of a full frame.                                                                        | F12.11 |

## Ideas

| Title                                    | Status   | Intent                                                                                               | Links                                                                           |
| ---------------------------------------- | -------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| Worlds as places                         | idea     | A directory of worlds a node serves, with presence and open or closed state.                         | [Phase 10 sketch](../../.opencode/plans/phase-9-portable-platform.md)           |
| Outbound notifications                   | idea     | A visible capability to post a bounded summary to a room or device when a world reaches a milestone. | Phase 10 sketch, F12.5                                                          |
| SSH chat and document viewing for worlds | proposed | Chat, input, paginated records and read-only document viewing in SSH shells. The PR has conflicts.   | #211                                                                            |
| Worlds on Cloudflare Workers             | idea     | A pkarr option for running on Workers. The design is merged.                                         | [P7.2 design](../../doc/2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md) |
| Group encryption and authorization       | idea     | Long-term item in `pkgs/id/TODO.md`.                                                                 | [TODO](../../pkgs/id/TODO.md)                                                   |
| Datastar for the web UI                  | idea     | A plan to move from HTMX exists. Its state is not checked.                                           | [plan](../../.opencode/plans/htmx-to-datastar-migration.md)                     |

## Done

Completed work is recorded in GitHub — see closed issues and merged PRs.
