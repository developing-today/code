# Design: first apps and the ecosystem

This chapter is design. It describes the first applications built on ID and the program ecosystem that distributes them, as designed in 26 of the 57 records the archive survey homes in the combined knowledge-layer and first-apps theme; the other 31 records are in [Design: the knowledge layer](knowledge-layer.md). The chapter runs in the order the programme ran: the first app (sticky notes and the did-it tracker, #2031), the first application families (chat, boards, match play), the shipped Roc guest apps that are the platform's working first apps, and then the ecosystem ladder #6278 — share and fork (#6291), gallery and updates (#6293), the public gallery (#6294), and creator tooling (#6308). The final section states what exists today.

Related chapters: [Design: the knowledge layer](knowledge-layer.md), [Design: worlds and the guest runtime](worlds-runtime.md), [Gallery, match-audit and walk](../reference/cli-gallery-walk.md).

## The first app: sticky notes and the did-it tracker (#2031)

`2026-11-01_design_id2031.md` (2026-11-01) is the design for the first ID app. Its acceptance bar is stated in the record: identities A and B exchange notes and did-it events; A's view excludes C's claims but leaves them retrievable on demand; and no permanent feature is built on an unreviewed proof of concept. The recommendation: build the proof of concept on reusable event and projection behaviour, not on an app-only event model — first make arbitrary committed event bodies durable and transferable, then land the did-it schema and evaluator and the writer-set projection as independently testable library slices, then compose those with existing document and blob events in an example. The record names a prerequisite the original follow-up issues missed: generic application bodies were validated at ingest but not persisted for later retrieval or range-synced, so a body-persistence slice must land before the app slices (#3788 did-it model and evaluator, #3789 writer-set projection, #3790 example proof of concept).

The did-it data model has its own investigation: `2026-10-29T13-04-50Z_investigation_didit_tracker_data_model` (2026-10-29). The sticky-notes half of the first app is a knowledge object; its note model is the one designed in [Design: the knowledge layer](knowledge-layer.md).

## The first application families

- Chat. `2026-11-01_design_id2051.md` designs ID chat v1. `2026-10-30T19-40-00Z_investigation_irc_first_chat_on_id` (2026-10-30) is the IRC-first investigation beneath it: channels, graph-linked objects, presence. `2026-11-09T01-27-00Z_design_chat_anchor_data_model` designs the chat anchor data model — typed links and durable part selectors, so a chat message can anchor to a part of another object and the anchor survives edits.
- Boards and posts. `2026-11-08T13-45-00Z_design_id5857_curated_board_submissions` designs curated-board submissions — intake queues, intake roles, notifications. `2026-11-08_design_id5856.md` decides the post-to-board flow and post semantics. `2026-11-08_design_id5859.md` decides live-object embeds in playable posts. The boards surface design itself (signed boards, portable themes, playable posts) is `2026-11-01_design_id2054.md`, homed with the shell records.
- Match play. `2026-11-01_design_id2039.md` designs match attestation and audit v1 — one of the two sub-issues of epic #2056 still open at the investigation charter's writing. `2026-10-29T10-54-33Z_investigation_match_audit_netcode_replay` investigates the match audit netcode: log format, replay validation, cheating signals. `2026-10-30T19-20-40Z_investigation_match_result_attestation` investigates match-result attestation: mutual concession, witness adjudication, offline play.
- Early surfaces. `2026-03-20T05-30-00Z_feature_collaborative_cursor_enhancements` (2026-03-20) and `2026-03-23T00:00:00Z_feature_file_list_ui` (2026-03-23) are the earliest records in this theme — collaborative cursors and the file list UI of the pre-worlds editor generation. `2026-04-01T00-00-00Z_feature_multi_instance` covers multi-instance support for `id`. `2026-10-17T09-00-00Z_feature_world_fixtures` designs session recording to replayable fixtures, the testing base the app designs rely on.
- Games and assets. `2026-11-08T09-14-46Z_research_id5988_prior_art_bot_games_programmable_worlds` is the prior-art research on scripted-bot battle games and user-programmable worlds. `2026-11-06T00-50-21Z_design_dynamic_world_tick_cost` designs the dynamic-world per-tick cost model and the bot-battle benchmark. `2026-11-08T00-57-08Z_design_shader_asset_format` designs the shader asset format v1. `2026-10-16T15-00-00Z_feature_roc_grid` is the RocGrid record — a bounds-checked grid library for world programs, the library the grid guest app demonstrates.

## The shipped first apps: the 9 Roc guest apps

The platform's working first apps are the 9 deployable Roc guest apps in `pkgs/id/examples/`, plus the one packaged app. They run on the shared Plaza platform (`examples/roc-world/`), whose guest shape is fixed: every app provides `init`, `update`, `view`, `layout`, `records`, `wants`, `snapshot` and `restore`; guests are compiled to import-free Wasm modules with no WASI, no filesystem, network, clock or randomness imports; all effects go through the capability protocol, with requests answered as participant-0 events and refusals returned as events with stable error codes (`cap_denied`, `unknown_cap`, `bad_args`, `failed`, `actor_denied`, `outside_ceiling`, `result_too_large`). The full platform treatment is in [Design: worlds and the guest runtime](worlds-runtime.md).

| App | Location | What it is |
|---|---|---|
| tic-tac-toe | `apps/tic-tac-toe/` | The sole packaged app (`app.toml` plus `program/main.roc`): two-player tic-tac-toe, digit `0`–`8` moves, ply parity picks X or O, records publish board, plays and winner. |
| lounge | `examples/lounge/` | Capability-system demo: subscribes to `players` and `time.tick:5000`, greets arrivals through `chat.say` requests, counts refusals, polls `time.now`. |
| roc-counter | `examples/roc-counter/` | Tiny pure counter world: guests send `inc`, all see the count, demonstrates records mirroring and the Screen presentation. |
| roc-text-counter | `examples/roc-text-counter/` | The no-Screen legacy-text twin of roc-counter; keeps the plain-text presentation path covered (#3181). |
| roc-chat | `examples/roc-chat/` | Minimal chat client on `RocCap` with `chat.say` and `chat.history`; typing `hist` shows the newest 8 lines. |
| roc-grid | `examples/roc-grid/` | RocGrid demo: a 4×3 byte grid, `set` claims the centre cell, bounds-checked reads with fallback. |
| roc-screen | `examples/roc-screen/` | Screen and Key demo: a bordered-box counter screen document; presenters render the structured grid. |
| territory | `examples/territory/` | Multiplayer territory game: at most 8 seated players on a 16×8 grid, adjacency-only claiming, seats persist across leave, rejoin and restart. |
| roc-cap-demo | `examples/roc-cap-demo/` | Library-import worked example (#5957): a capability-dependent Roc library over `records.read`, with two compile-must-fail negative tests. |

Two neighbouring directories are not apps: `examples/roc-world/` is the shared platform package, and `examples/roc-import-policy/` is a compile-fixture matrix for the import gate. The Rust twins (`examples/rust-counter/`, `examples/rust-world/`) are the F12.8 guest SDK and its counter example, not Roc apps; `examples/rust-sketch/` is the sketch/redo-as-new-stroke guest (#13608), whose domain model and literal-vector tests live in the crate and whose host load/play fixture is the `tests/sketch_host_play.rs` integration test (#13656). `examples/first-app-poc/` is a README only: the sticky-notes and did-it proof of concept of #2031, whose executable artifact is the Rust integration test `tests/first_app_poc.rs`, by design.

## The ecosystem ladder (#6278)

The program ecosystem core topic is #6278. Its first-pass record, `2026-11-10T21-24-00Z_design_id_program_ecosystem` (2026-11-10), settles the vocabulary and the load-bearing decisions the ladder steps share (decisions D1–D6, invariants 1–7), and states the rule of the ladder: a step that needs a different answer amends the core record first; no step silently forks a decision. Its principles are the #1240 principles restated as constraints: local-first always, with a hosted version intended; anything built must be completely usable by others hosting a small or large server themselves; peer-to-peer-hosted worlds and data behave the same as on a large hosted public server, with differences only in backend scale elements and access controls, never in protocol behaviour; the hosted instance is a public backstop, not a requirement.

### v1 — share and fork (#6291)

`2026-11-11T05-01-15Z_design_id_program_ecosystem_v1` (2026-11-11) scopes v1 to four operations: export a program as one signed bundle file; import a bundle into a world (sideload, fully offline); fork a held bundle with structural attribution; export a world as a template-world package file and import it as a new world. Out of scope for v1, by name: the gallery and registry, channel and update-policy user experience, the hosted public gallery, and federation. V1 builds beside the F12.2 world fork, template and provenance implementation and reuses it.

### v2 — gallery and updates (#6293)

`2026-11-11T05-24-00Z_design_id_program_ecosystem_v2` (2026-11-11) refines D6 and D4 for its slice: the gallery as a registry in depth — the registry record, the publish, unpublish, list and fetch operations, the three publishing paths, and the line between validity and admission; template packages as registry records; and update channels with the update flow into worlds that installed earlier versions.

### v3 — the public gallery (#6294)

`2026-11-14T13-33-00Z_design_id_program_ecosystem_v3` (2026-11-14) designs the operated instance: the hosted public gallery on the main public endpoint — categories, search, featured collections, author pages keyed by public key — plus the curation posture, abuse handling and scale elements v2 deferred to it. Out of scope, by name: federation and lineage-aware discovery at scale (v4, #6295), payments, and browsing accounts or logins — identity is keys. All operator policy in v3 is layered on the D6 registry design, never in the protocol.

### Creator tooling (#6308) and network defaults

`2026-11-18_design_id6308_creator_owner_basics_v1.md` (2026-11-18) designs creator tooling and insight — owner basics v1 for #6308, a child of #6278. `2026-11-14T19-05-00Z_design_id_build_defaults` designs the build-time defaults file for ID network defaults, and `2026-11-09T04-43-05Z_design_id5788_build_variants.md` is the design record for feature-leveled build variants — the distribution mechanics the ecosystem's packages are built under.

An example with invented data, following the v1 design: a creator exports the program `counter-example` as one signed bundle file, sends the file to a friend by any channel, and the friend imports it into their own world offline. The import verifies the bundle's signature and manifest; a fork of the imported bundle carries structural attribution back to the original. No gallery, account or network service participates in any step of that flow.

## What exists today

Grounding, from the library survey for this book (checkout `1ccb4a19457`, 2026-11-23):

- Shipped: the 9 Roc guest apps and the packaged tic-tac-toe app listed in the table above, with checked-in Wasm artifacts beside most apps so host conformance suites run without a Roc toolchain. These are the first apps in the literal sense: working guest programs on the shipped Plaza platform.
- Shipped: the program-side machinery the ecosystem designs build on — the program registry and gallery surface named in the CLI (`gallery`, `publish`, `unpublish`, `publications` command families exist in the shipped CLI table), and the bundle record path the #10662 listing design distinguishes itself from: author-signed bundle records with author, name, version and payload hash, fetched by content hash and verified on the bundle path.
- Shipped as a proof of concept only: the first app of #2031 — sticky notes plus did-it — exists as the integration test `tests/first_app_poc.rs` and a README in `examples/first-app-poc/`. It is not a packaged application.
- Still design: the ecosystem ladder as designed — signed bundle export/import/fork as the v1 user flow (#6291 is a design record; the record states no implementation code is in it), the gallery as a registry with update channels (#6293, design only), the hosted public gallery as an operated instance (#6294, design only), and creator tooling (#6308, design only). The chat v1, boards and match-attestation designs are likewise design records; the shipped chat and boards functionality in the `id` program is the earlier application surface documented in the reference chapters, not an implementation of these v1 designs — the records themselves make no completion claim, and this chapter makes none for them.
