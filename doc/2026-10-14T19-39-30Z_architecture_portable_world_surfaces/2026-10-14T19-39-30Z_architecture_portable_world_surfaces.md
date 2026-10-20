# Portable world surfaces and controlled execution profiles

See [the working plan](../../.opencode/plans/phase-9-portable-platform.md).

## Request and intent

The request is to continue platform work so most Ratatui-shaped SSH applications can be built with the platform, and TUI/SSH interfaces can also be presented in web through direct translation or multiple presentation interfaces. It also asks for customizable execution bounds, administrator-controlled platform functions, source-only compilation for untrusted programs, and a general-purpose data/game/communication platform.

The current world runtime already has a useful portable core: a deterministic update loop, a capability broker, bounded Wasm, an OS-confined native worker, and a shared WebSocket/SSH session driver. But a program view is only UTF-8 text, SSH strips control codes, and the browser inserts that text into a `<pre>`. The Roc `Screen` module already builds styled cells but only serializes them by hand as plain or ANSI text. There is no widget/event-loop API or Ratatui dependency.

The chosen architecture is to make a **validated presentation value** the seam between the deterministic world program and its transports. The guest supplies structured data, not terminal escape sequences, HTML, or CSS. Each transport has a presenter for that same value. Text remains a first-class legacy presentation. A screen is a bounded row-major cell grid compatible with the output of a Ratatui `Buffer` adapter and the existing Roc `Screen` module.

## P9.1 interface

The versioned guest payload uses a reserved discriminator and a compact row/span representation: `{"kind":"screen","version":1,"width":80,"height":24,"rows":[...],"styles":[{"x":0,"y":0,"length":4,"fg":2,"bg":0,"bold":true}]}`. Rows are Unicode text; style spans are non-overlapping terminal-column ranges. Foreground/background are indices in the fixed xterm-256 palette (`0` means terminal/browser default), and `bold` is the only initial modifier. The host derives a normalized row-major cell buffer so both presenters consume the same validated model. Host parsing validates dimensions, total cells, UTF-8, grapheme/display width, control characters, style spans, and the existing world presentation byte cap before a transport sees it. Unknown versions, malformed shapes, invalid style codes, or out-of-bound buffers are errors, not partially rendered content. Any output that is not the reserved schema remains legacy text.

The world session keeps its existing `data` string for older clients and adds an optional typed `surface` field. SSH and web prefer `surface` when present, otherwise they retain their old text behavior. The SSH presenter emits only its own cursor-home/clear and allowlisted SGR codes; every guest glyph is validated and terminal controls are impossible. The web presenter creates text-only cell nodes and maps the same numeric palette/style bits to fixed CSS classes; guest content never becomes executable HTML/CSS.

Roc `Screen` gets a JSON view encoder that emits this schema with correct string escaping and coalesced style spans. Existing `render_plain` and `render_ansi` functions stay available for apps that intentionally target a text-only consumer. The P9.1 format does not add new host authority and does not change the world journal.

## Compatibility and limits

- Existing clients ignore the optional `surface` and continue to display `data`.
- Existing programs that return plain text continue to use the old presentation path.
- A maximum grid size and per-glyph byte bound prevent oversized, pathological layouts; the current overall presentation size limit still applies.
- The protocol is not a drop-in runtime for every Ratatui application. Ratatui widgets can be rendered into a buffer and adapted; their application state, event loop, timers, and backend-specific side effects must use the world update/capability model.
- P9.1 does not yet deliver viewport dimensions or typed key events to the guest. P9.2 owns that change so it can define ephemeral-vs-journaled input carefully.

## Execution security decisions

Existing server-wide world fuel, memory, payload, and native worker limits are configurable; the current per-world platform capability ledger is the authorization ceiling and imports are checked during source compilation. The native tier is opt-in but remains isolated by Landlock/seccomp/rlimits; Wasm remains import-free. The follow-up work should make these settings more explicit and per-world where required, and add source-only install policy plus compiler provenance.

“Direct control” is not interpreted as unconfined host access. An administrator may explicitly enable named platform effects under a per-world ceiling and tune limits inside validated bounds. Unrestricted filesystem/network/process access is not added by a configuration toggle; any future direct host integration needs a separately audited adapter with least-privilege OS controls, explicit operator opt-in, and independent tests.

Untrusted programs should be accepted only as source submitted to the configured trusted Roc compiler when source-only policy is enabled; raw Wasm remains an explicit trusted-artifact path unless an administrator disables it. The compiler executable is operator-configured today but its identity/version is not pinned in stored module provenance. P9.3 must decide a reproducible compiler identity format before claiming provenance.

## Follow-up work

- P9.2: viewport size, resize, key/modifier, paste, and focus event schema; per-session handling and web/SSH parity.
- P9.3: source-only mode, compiler identity/provenance, per-world execution profile, capability ceiling configuration, and bound validation. Do not silently permit guest-selected native code.
- P9.4: a Roc JSON library and structured data helpers; migrate lounge/game examples from substring parsing and hand-built JSON.
- P9.5: Ratatui buffer adapter, app packaging, multiple presentation variants, and data/game/communication example catalog.
- General replicated storage remains a separate design: records are a read-only projection, not a general write database or alternate document authority.

## References

- [Working plan](../../.opencode/plans/phase-9-portable-platform.md)
- [Roc worlds architecture](../2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md)
- [Attenuated capabilities design](../2026-10-09T09-59-07Z_design_attenuated_capabilities/2026-10-09T09-59-07Z_design_attenuated_capabilities.md)
- `pkgs/id/src/world.rs`, `pkgs/id/src/world_session.rs`, `pkgs/id/src/world_ssh.rs`, `pkgs/id/src/sandbox.rs`, `pkgs/id/examples/roc-world/Screen.roc`
