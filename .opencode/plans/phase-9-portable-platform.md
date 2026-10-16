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
- **P9.3a complete in this worktree:** server-wide source-only install policy, stacked on #214. Source compilation remains enabled and raw artifacts are refused. Compiler provenance, per-world profiles, and broader allowed-function ceilings remain open.
- **P9.2a complete in this worktree:** focusable browser surfaces encode common keys with terminal-compatible bytes and send them through the existing durable input event. SSH already forwards PTY bytes through that same event.
- **P9.2b implementation in progress:** bounded paste is an explicit input event; browser ResizeObserver and SSH PTY/window-change events set connection-local viewport context. Structured screens are cropped per connection without entering the journal. Roc apps still render at their declared dimensions; a future guest ABI may expose dimensions for app-level reflow.
- **P9.4/P9.5 remain planned:** Roc JSON/data helpers and Ratatui-shaped adapters/examples.

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

## P9.3a acceptance

- Source-only is an explicit server option, off by default, and requires an enabled world plus a configured server-side Roc compiler/platform.
- Raw upload is rejected before consuming chunks and raw installation is also refused through the service interface.
- Source compilation still validates and installs through the established module validator, persistence, and activation path.
- Native execution stays separately gated; trusted compiler provenance is not claimed until compiler identity is recorded.
