# Phase 9: portable world surfaces and controlled execution profiles

## Goal

Build the missing platform foundations for interactive data, game, and communication apps: a transport-neutral screen contract that can be rendered by SSH terminals and the browser; structured Roc libraries for the protocol; and explicit per-world execution policy that preserves sandboxing while letting an administrator grant narrowly named capabilities and tune resource bounds.

The immediate slice (P9.1) is a backwards-compatible, validated cell-grid presentation contract and safe terminal/web presenters. It must not require ANSI in guest output, execute arbitrary markup, or change the existing world event/journal ABI.

## Sequence

1. **P9.1 Portable screen view.** Accept legacy plain-text view strings or a versioned structured screen document; validate bounds and cell contents once; render the same model as DOM cells in web and allowlisted ANSI cells in SSH. Add a Roc `Screen` JSON encoder and example/host tests.
2. **P9.2 Input and viewport.** Add typed key, paste, focus, and resize input events with a compatibility path for existing text/hex inputs. Keep transport input decoding separate from durable program events; specify which events are ephemeral and which are journaled.
3. **P9.3 Execution profiles.** Make source-only mode and compiler provenance explicit; support server/per-world capability ceilings and configurable compile/runtime resource bounds. Direct control means explicitly granted platform effects, not unconfined process access. Retain OS isolation for native workers and import-free Wasm unless a separately audited sandbox adapter is designed.
4. **P9.4 Standard libraries.** Add tested Roc JSON parse/encode primitives and migrate an app/example to structured host events, records, and presentation. Grow data, game, and communication helpers on stable capabilities rather than host-specific shortcuts.
5. **P9.5 Adapters/examples.** Provide a Rust/Ratatui buffer-to-screen adapter or documented capture interface, plus equivalent terminal and browser examples. Ratatui widgets alone are not an app runtime; applications still supply update state and event-loop logic.

## Status and current worktree scope

- **P9.1 complete:** portable surface contract and SSH/web presenters are committed on `platform-portable-ui` (PR #214, stacked on #209).
- **P9.3a complete in PR #215:** server-wide source-only install policy. Source compilation remains enabled and raw artifacts can be refused.
- **P9.3b in PR #225:** source builds attach server-observed compiler/platform/source digests, target, and stable arguments to the install event; raw artifact installs are labeled as artifacts. Metadata survives restart/checkpoint and is available through the additive module-info field. This is not a trust attestation. The latest full Nix check failed in Firefox E2E, tracked in issue #229; Rust build/lint checks passed. Human-readable compiler version remains open; server/per-world capability ceilings are tracked in issue #230.
- **P9.3c complete:** define optional per-world resource requests that inherit server limits when absent and clamp to the stricter server hard ceiling when present. Capability ceilings remain separate.
- **P9.3d in PR #228:** persist profiles per durable world and apply them only when a new instance is installed or restored. Runtime changes are not applied to an existing guest; the admin API communicates that boundary. Focused Rust tests and the PR run check passed; rollback compatibility is tracked in issue #227.
- **P9.3e in PR #231:** server and per-world capability ceilings bound which granted host capabilities authority reaches. The server ceiling is `serve --world-allowed-caps` (runtime configuration); the world ceiling is durable and applies immediately, and it never rewrites grants. Record: [world capability ceilings](../../doc/2026-10-16T11-00-00Z_feature_world_capability_ceilings/2026-10-16T11-00-00Z_feature_world_capability_ceilings.md).
- **P9.3f complete in this worktree:** operator CLI commands for capability ceilings (`id world caps --ceiling`) and execution profiles (`id world profile`). Record: [world CLI controls](../../doc/2026-10-16T12-00-00Z_feature_world_cli_controls/2026-10-16T12-00-00Z_feature_world_cli_controls.md).
- **P9.2a complete in this worktree:** focusable browser surfaces encode common keys with terminal-compatible bytes and send them through the existing durable input event. SSH already forwards PTY bytes through that same event.
- **P9.2b complete in PR #219:** bounded paste is an explicit input event; browser ResizeObserver and SSH PTY/window-change events set connection-local viewport context. Structured screens are cropped per connection without entering the journal. Guest-aware reflow is complete in this worktree (P9.8).
- **P9.4 first slice complete in this worktree:** pure-Roc `RocJson.roc` decode/encode and typed accessors are exported by the Roc world platform; lounge now decodes host events and encodes requests/records. Broader data/game/communication helpers remain planned.
- **P9.4b complete in this worktree:** migrated `territory` to `RocJson`, updated `roc-screen` with checked-in artifacts, fixed `lounge` participant decoding, added sandbox screen integration test. Record: [structured apps migration](../../doc/2026-10-16T13-00-00Z_feature_world_apps_structured_migration/2026-10-16T13-00-00Z_feature_world_apps_structured_migration.md).
- **P9.4c in this worktree:** the platform exposes `pf.RocGrid`, a bounds-checked row-major byte grid (`new`, `get`, `set`, `count`, `neighbors4`, `row`) with a checked-in `examples/roc-grid` example and a sandbox test. Record: [RocGrid](../../doc/2026-10-16T15-00-00Z_feature_roc_grid/2026-10-16T15-00-00Z_feature_roc_grid.md).
- **P9.4d in this worktree:** the platform exposes `pf.RocCap`, shared request bookkeeping for the capability protocol (`issue`, `attach`, `requests`, `from_pending`, the request builders, `answered`, `done`), verified by a compile-and-run test. Record: [RocCap](../../doc/2026-10-16T16-00-00Z_feature_roc_cap/2026-10-16T16-00-00Z_feature_roc_cap.md).
- **P9.8 complete in this worktree:** guest-aware reflow. The platform requires `layout : model, U64, U64 -> Str`; sessions with a viewport render through `plaza_layout`, without journaling or sequence changes, and cropping remains the fallback. `roc-screen` widens on viewports of at least 30 columns. Record: [guest-aware reflow](../../doc/2026-10-16T19-00-00Z_feature_guest_reflow/2026-10-16T19-00-00Z_feature_guest_reflow.md).
- **P9.11 complete in this worktree:** `Screen.fill`/`Screen.rule` cover rect fills and separators, and `pf.RocText` ships proven text helpers (`trim`, `lowercase`, `starts_with`, `drop_prefix`, `contains`, `lines`, `join_lines`), both verified by compile-and-run tests. `RocText` is a grantable module name. Record: [screen widgets and text](../../doc/2026-10-16T19-30-00Z_feature_screen_widgets/2026-10-16T19-30-00Z_feature_screen_widgets.md).
- **P9.9 complete in this worktree:** `records.read` returns byte-bounded, prefix-scoped pages of the world's published records with an exclusive `after` cursor, requires only the `join` scope, and is exposed as `RocCap.records_request`/`records_page`/`next_after`. Record: [records readback](../../doc/2026-10-16T18-00-00Z_feature_records_readback/2026-10-16T18-00-00Z_feature_records_readback.md).
- **P9.10 complete in this worktree:** `id world new` scaffolds a buildable Roc app (main.roc, executable build.sh, README), `id world stats` prints module info, capability report, and execution profile as one JSON object, and `id world logs` tails recent event frames for a bounded deadline. Record: [authoring CLI](../../doc/2026-10-16T18-30-00Z_feature_world_authoring_cli/2026-10-16T18-30-00Z_feature_world_authoring_cli.md).
- **P9.7 complete in this worktree:** `chat.history` returns the newest bounded chat lines (default 16, clamp 64, oldest-first) from the retained-events buffer, requires the CHAT scope, and is exposed to apps as `RocCap.history_request`/`history_lines`. The platform modules `RocJson`, `RocGrid`, and `RocCap` are now grantable catalog names. `examples/roc-chat` is a checked-in chat client built on it, with a sandbox end-to-end test. Record: [chat history](../../doc/2026-10-16T17-00-00Z_feature_chat_history/2026-10-16T17-00-00Z_feature_chat_history.md).
- **P9.6 complete in this worktree:** library consolidation. `territory` keeps its board in `pf.RocGrid` (no app-local index arithmetic), `lounge` drives its request lifecycle through `pf.RocCap`, `Screen.put_grid` draws a grid through a glyph function, and the grid example round-trips through `RocGrid.bytes`/`from_cells`. Record: [RocGrid](../../doc/2026-10-16T15-00-00Z_feature_roc_grid/2026-10-16T15-00-00Z_feature_roc_grid.md), [RocCap](../../doc/2026-10-16T16-00-00Z_feature_roc_cap/2026-10-16T16-00-00Z_feature_roc_cap.md).
- **P9.5 in progress in this worktree:** `territory` renders a portable styled screen (per-seat colours, inverted cursor, scoreboard) at the default fuel budget; `Screen.put_row` batches row draws; the shared host allocator clamps non power-of-two alignments instead of trapping. Record: [screen rendering and alignment](../../doc/2026-10-16T14-00-00Z_feature_screen_render_alignment/2026-10-16T14-00-00Z_feature_screen_render_alignment.md). `ScreenSurface::from_cells` remains the validated Rust buffer-capture interface; Ratatui stays optional and app-owned.

“Direct control” is limited to explicit named capabilities under an admin-controlled ceiling. This plan does not enable unrestricted filesystem, network, or process access; native execution remains OS-sandboxed.

## Proposed next steps (added 2026-10-16)

These are new slices beyond the original P9.1–P9.5 sequence. They are ordered so
each one is useful alone and does not depend on a later one.

- **P9.6 Standard-library consolidation.** Finish migrating the checked-in apps
  onto the shared libraries: `territory` keeps its board in `pf.RocGrid` and
  `Screen` gains a `put_grid` draw that takes one, so no app re-derives index
  arithmetic; `lounge` moves its id counter and pending list to `pf.RocCap`.
  Acceptance: no example carries its own `cell_index`, id counter, or pending
  matcher; all checked-in tests keep passing.
- **P9.7 Server-observed chat history.** (Done — see status above.) Room chat
  exists but a world program cannot read it, so communication apps cannot show
  logs, search, or reply in context. Add a `chat.history` capability request
  (bounded count, oldest-first window, maybe a `since` cursor) served from the
  journal, with grant checks and replay-safe results. Acceptance: a bounded,
  granted, journal-derived window with tests that an ungranted request is
  refused and counts against usage.
- **P9.8 Guest-aware reflow.** (Done — see status above.) The platform now
  requires a `layout : model, U64, U64 -> Str` entry point; sessions render
  through it with their viewport and cropping remains the fallback.
- **P9.9 Presence and records readback.** (Done — see status above.) `players.list`
  exists, but apps could not read records published by other participants, so
  multi-terminal and shared-canvas apps were impossible. Add a bounded
  `records.read` capability request (prefix-scoped, paged) with the same grant
  and usage accounting as other capabilities.
- **P9.10 App scaffolding and inspection.** (Done — see status above.)
  `id world new` scaffolds a Roc app; `id world stats` reports module, caps,
  and profile state; `id world logs` tails recent events for a deadline.
- **P9.11 Library growth: text and widgets.** (Done — see status above:
  `Screen.fill`/`rule` and `pf.RocText`.) List/table/meter widgets remain a
  follow-up once two apps need the same one.

## Phase 10 progress

- **P10.1 started in this worktree:** role-scoped invites (`spectator`, `player`, `guest`, `delegator`) over the existing scope machinery, and `id world list --json` for directory feeds. Builder/moderator roles with their own authority, and a cross-node directory feed, remain follow-ups. Record: [roles and directory](../../doc/2026-10-17T08-00-00Z_feature_world_roles_directory/2026-10-17T08-00-00Z_feature_world_roles_directory.md).

- **P10.2 started in this worktree:** session recording to replayable fixtures. `id world record` writes a world's retained history and records to a JSON Lines fixture through a join capability, and `id world replay` runs that fixture offline in the sandbox under the operator's seed and compares the records. Host events (presence, capability results, ticks) never enter the event stream, so a fixture replays faithfully only for programs that do not consume them; the records check reports the difference. Reinstall windows and partial histories need follow-ups. Record: [session recording](../../doc/2026-10-17T09-00-00Z_feature_world_fixtures/2026-10-17T09-00-00Z_feature_world_fixtures.md).

## Next stage

Phase 11 ([foundations](phase-11-foundations.md)) and Phase 12 ([platform flexibility](phase-12-platform-flexibility.md)) continue from here. Phase 10 items still open: builder and moderator roles (F12.3), world forking and templates (F12.2), outbound notifications (F12.5), and the directory feed.

## Phase 10 sketch: worlds as places

Not committed beyond the slice above, recorded so the direction is visible.

- **World directory feeds and portals.** A browsable directory of worlds a
  node serves and knows (names, presence counts, open/closed), with the web UI
  and `id world list` sharing one serializer.
- **Roles beyond admin/guest.** Builder (may install modules), moderator (may
  kick), spectator (read-only presence), with the existing scope machinery as
  the substrate.
- **World forking and templates.** Clone a world's journal prefix into a new
  world for tournaments, save-states, and demos, with provenance showing the
  parent.
- **Scheduled events.** A first-class `schedule` capability for tournaments and
  reminders, durable in the journal, delivered as ticks.
- **Outbound notifications.** An explicit, user-visible capability to post a
  bounded summary to a room or device when a world reaches a milestone.
- **Session recording.** Export a world's journal window to a replayable
  fixture so app authors can test against real sessions, and so bugs can be
  reproduced exactly.

## Acceptance for P9.1

- Old plain-text programs render exactly as before.
- A bounded, versioned screen DTO has deterministic parsing/serialization and rejects dimensions, cell counts, glyphs, or style values outside policy.
- SSH never relays guest-supplied control sequences; only the trusted presenter emits known cursor and SGR codes.
- Browser output uses text nodes/CSS only, never guest-supplied HTML or CSS.
- The same screen DTO has parity tests for web and terminal presenters.
- Roc `Screen` can produce the DTO without asking app authors to hand-concatenate JSON.

See [the design record](../../doc/2026-10-14T19-39-30Z_architecture_portable_world_surfaces/2026-10-14T19-39-30Z_architecture_portable_world_surfaces.md).

P9.3a record: [source-only world installs](../../doc/2026-10-14T20-00-05Z_feature_source_only_world_installs/2026-10-14T20-00-05Z_feature_source_only_world_installs.md).

P9.2a record: [browser and SSH keyboard parity](../../doc/2026-10-14T20-10-00Z_feature_world_keyboard_parity/2026-10-14T20-10-00Z_feature_world_keyboard_parity.md).

P9.2b record: [per-session viewport and paste policy](../../doc/2026-10-15T16-00-00Z_feature_world_viewport_and_paste/2026-10-15T16-00-00Z_feature_world_viewport_and_paste.md).

P9.4 first-slice record: [pure-Roc JSON module](../../doc/2026-10-15T09-27-28Z_feature_roc_json/2026-10-15T09-27-28Z_feature_roc_json.md).
P9.5 record: [terminal buffer capture interface](../../doc/2026-10-15T16-30-00Z_feature_terminal_buffer_capture/2026-10-15T16-30-00Z_feature_terminal_buffer_capture.md).

P9.3b record: [observed world module provenance](../../doc/2026-10-16T09-00-00Z_feature_world_module_provenance/2026-10-16T09-00-00Z_feature_world_module_provenance.md).

P9.3c record: [per-world resource profile contract](../../doc/2026-10-16T10-00-00Z_feature_world_resource_profile/2026-10-16T10-00-00Z_feature_world_resource_profile.md).

P9.3d record: [durable per-world resource profiles](../../doc/2026-10-16T10-30-00Z_feature_durable_world_resource_profiles/2026-10-16T10-30-00Z_feature_durable_world_resource_profiles.md).

P9.3e record: [world capability ceilings](../../doc/2026-10-16T11-00-00Z_feature_world_capability_ceilings/2026-10-16T11-00-00Z_feature_world_capability_ceilings.md).

P9.3f record: [world operator CLI controls](../../doc/2026-10-16T12-00-00Z_feature_world_cli_controls/2026-10-16T12-00-00Z_feature_world_cli_controls.md).

P9.4b record: [structured apps migration](../../doc/2026-10-16T13-00-00Z_feature_world_apps_structured_migration/2026-10-16T13-00-00Z_feature_world_apps_structured_migration.md).

P9.5b record: [screen rendering and alignment](../../doc/2026-10-16T14-00-00Z_feature_screen_render_alignment/2026-10-16T14-00-00Z_feature_screen_render_alignment.md).

## P9.3a acceptance

- Source-only is an explicit server option, off by default, and requires an enabled world plus a configured server-side Roc compiler/platform.
- Raw upload is rejected before consuming chunks and raw installation is also refused through the service interface.
- Source compilation still validates and installs through the established module validator, persistence, and activation path.
- Native execution stays separately gated; trusted compiler provenance is not claimed until compiler identity is recorded.
