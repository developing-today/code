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
- **P9.2b complete in PR #219:** bounded paste is an explicit input event; browser ResizeObserver and SSH PTY/window-change events set connection-local viewport context. Structured screens are cropped per connection without entering the journal. Roc apps still render at their declared dimensions; guest-aware reflow remains a future ABI follow-up.
- **P9.4 first slice complete in this worktree:** pure-Roc `RocJson.roc` decode/encode and typed accessors are exported by the Roc world platform; lounge now decodes host events and encodes requests/records. Broader data/game/communication helpers remain planned.
- **P9.5 implementation in progress:** `ScreenSurface::from_cells` provides a validated Rust buffer-capture interface; Ratatui stays optional and app-owned. Existing Roc/SSH/browser examples provide producer and presenter paths.

“Direct control” is limited to explicit named capabilities under an admin-controlled ceiling. This plan does not enable unrestricted filesystem, network, or process access; native execution remains OS-sandboxed.

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

## P9.3a acceptance

- Source-only is an explicit server option, off by default, and requires an enabled world plus a configured server-side Roc compiler/platform.
- Raw upload is rejected before consuming chunks and raw installation is also refused through the service interface.
- Source compilation still validates and installs through the established module validator, persistence, and activation path.
- Native execution stays separately gated; trusted compiler provenance is not claimed until compiler identity is recorded.
