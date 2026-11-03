# World presentation, rendering, and fuel: the current model

Status record, 2026-10-31. How world programs present, who renders, and what
fuel meters — written down after the roc-counter Screen migration (#565)
surfaced the gaps in the pkgs/id test suite. Implementation facts are from
`src/world_presentation.rs`, `src/world_caps.rs`, and `src/sandbox.rs`;
planned work is cited by issue.

## The three layers

1. **Guest.** A Roc program compiled to WASM. It runs only on the host —
   inside the wasmtime sandbox, or as a hardened native worker process.
   Clients never receive the module and never execute it.
2. **Host.** Validates every presentation, keeps the journal, enforces
   capability ceilings, and transports frames to sessions.
3. **Adapters.** Trusted client-side code renders what the guest emitted:
   a terminal presenter for SSH, a web presenter for the browser. Rendering
   is client-side; computation — including composing the screen — is not.

## Presentation is data

A guest's calls split into data and presentation:

- Data: `Init`, `Update`, `Records`, `Wants`, `Snapshot`, `Restore`.
- Presentation: `View`, and `Layout` — the size-aware call (the guest
  reflows to the dimensions it is given). `View` is the fallback when a
  guest exports no `plaza_layout`.

A presentation is one of two things:

- **Legacy UTF-8 text.** Always valid, permanently supported.
- **A portable screen:** the versioned `kind: "screen"` JSON document — a
  normalized cell buffer (width, height, row-major cells, style spans),
  capped at 120x50. Guests never supply terminal escapes, HTML, or CSS;
  adapters render the buffer with their own trusted code.

Sessions carry the result as frames: a snapshot or event frame holds either
the text or a `surface` screen object.

## Fuel

- Wasmtime fuel meters **every** guest call kind, not just rendering —
  any untrusted module needs a CPU bound. At zero the call traps, in
  production exactly as in tests; the test budgets are the tripwire, not
  test-only trivia.
- Budgets live per (example, call kind) in `src/sandbox.rs` and are
  defined as the measured baseline + 25%, rounded up to the next 100,000.
  They are regenerated with the shipped `print_fuel_baseline` helper;
  re-baselining is deliberate and says why in the commit.
- Screen composition is the expensive part (roc-screen baselines: View
  123,675, Layout 378,384) next to state updates (~1k-14k). That is why
  screen examples carry 200k/500k budgets while text programs sit at the
  100k floor.
- Fuel is only meaningful where the WASM runs. Client-side execution
  would make it unenforceable, which is one reason no client-side
  execution exists (below).

## Capabilities

The catalog (`src/world_caps.rs`): `time.now`, `time.tick`, `random.u64`,
`players`, `players.list`, `chat.say`, `chat.history`, `records.read`,
`schedule.at`, `schedule.cancel`, `world.info`, plus the platform modules
`Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, `RocText`.

A source's imports are checked against the world's capability ceiling at
compile time; at run time a program asks the server for effects as data.
A world may exclude `Screen` from its ceiling — text programs are
unaffected.

## Text-only today; tiers planned

- Text-only works today at the protocol level: a program that never
  imports `Screen` returns plain text (roc-counter worked this way until
  #565). Nobody has to have a world that includes a screen.
- There is **no first-class line/teletype surface yet**: `WorldSurface`
  has exactly one variant (`Screen`), and text is the untyped legacy
  path. The nearest append-a-line primitive is `chat.say`/`chat.history`.
- Planned: presentation capability tiers with in-band terminal detection
  (#786, F13.4) — tier 1 full terminal (kitty graphics, synchronized
  output), tier 2 xterm-class cell grid with diff rendering, tier 3 line
  mode — identical over SSH and iroh. SSH input/presentation designs:
  #535. Delta/binary frames (screen patches instead of full-buffer
  updates): #805 (F12.11); a ScreenPatch validation bug is tracked as
  #2910.

## Browser and DOM

- The screen document is designed to suit "an SSH terminal or web DOM":
  the browser receives the cell buffer over the world websocket and a
  trusted adapter draws it.
- **No DOM/HTML capability exists, by design.** An untrusted guest
  emitting markup into a page is an injection hole; cells are inert.
  Web-native specialization remains an open design question (#2146,
  how a program appears as a website). The shipped web UI's world view
  is still lobby-level; a populated host view is #2144.
- **No client-side WASM execution exists or is filed.** The only
  "run elsewhere" track is hosted worlds (a Cloudflare Durable Object
  as the server), which changes which server runs the guest, not the
  layering above.

## Testing state

Covered: per-example fuel budgets including the screen-heavy examples;
the `world measure` report; screen validation and frame encoding at the
session layer; capability import checks; two browser-websocket
end-to-end tests (upload a module; compile from source), both using the
Screen counter.

Gap: no end-to-end **text-only** world over the browser socket — the
text path is covered at unit level only. No DOM presenter exists to
test. Tier behaviour (#786) is untested because it is unbuilt.
