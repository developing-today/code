# Input event model for relayed apps (touch, mouse, hover)

**Request:** issue [#1831](https://github.com/developing-today/code/issues/1831) (Drew, 2026-10-28), slice S1 of the split recorded by triage task 7753 and re-verified by triage task 11974. This design is the gate slice: it defines the input event model only. Extending the accepted-interface definitions is S2, the Android proof-of-concept app is S3, and the dynamic-Roc stretch is S4; all three stay on issue #1831 and are sequenced after this design lands. Recorded before implementation, as required by [`DOCUMENTATION_PROTOCOL.md`](../DOCUMENTATION_PROTOCOL.md).

## Intent

Apps running in worlds today receive keyboard input only: a journaled `Input(Vec<u8>)` event, with browser keys encoded to terminal bytes (see `doc/2026-10-14T20-10-00Z_feature_world_keyboard_parity/`). The accepted interfaces cannot express pointer input at all — `Interfaces` in `pkgs/id/src/app_manifest.rs` is `{ cli, web }` with `deny_unknown_fields`, and `pkgs/id/contract/shell-contract.v1.json` names no pointer, mouse, touch, or scroll event. Issue #1831 asks for a proof-of-concept Android app that relays the phone's touch input into a live app, and for the accepted interfaces to grow the events that relay needs. The relayed interface is deliberately limited to the subset that can be supported end to end; this design names that subset, the per-environment extras beyond it, and — as the issue's acceptance requires — what is refused, explicitly, instead of silently dropped.

## Environments

Four producer environments are in scope. Every rule in this design is stated per environment; there is no environment-free event.

| Environment | Producer | Pointing source |
|---|---|---|
| `native` | Desktop/native shell | Mouse (touchscreen laptops report through the same pointer path) |
| `web` | Browser/Wasm shell, including the #1826 phone peer page | Mouse, touch, or stylus, via the browser |
| `tui` | Terminal shell | Terminal mouse reporting: SGR encoding (DECSET 1006) with button-event tracking (DECSET 1002), which reports press, release, and motion while a button is held. A terminal offering only press-only reporting (X10 / DECSET 9 / 1000) cannot carry `release` or `drag`: the producer refuses those events for that session with reason `no_tracking_mode` (see the refusal semantics below) instead of approximating them. |
| `android` | The S3 native Android app | Multi-touch touchscreen |

## The common core

The core is the set every environment must be able to produce, because the relay subset — minimally click, ideally drag and scroll — is built from it, with one named exception: `long_press` is core only where the producer has a timing source, and the `tui` environment is that exception (refused `no_timing_source`, per the refusal table) — an app must not assume `long_press` on `tui`. Coordinates are in the target surface's cell grid (the world screen is a cell grid; producers map device pixels to cells at the edge, so the model never carries pixels). Every event carries a `pointer_id`; v1 guarantees only the primary pointer (see Decisions). A `pointer_id` is unique only within its producing connection: when journaling, the shell namespaces it with the connection's identity, and press/release pairing during replay is always scoped to that (connection, `pointer_id`) pair, so two connections can never cross-pair each other's events.

| Event | Meaning |
|---|---|
| `press` | Pointer contact/button down at a cell (`button: primary` in the core) |
| `release` | Pointer contact/button up at a cell; carries which button was released (`button: primary` or `secondary`, matching its `press`), the hold duration `held_duration_ms` (milliseconds between press and release), and whether it satisfied the producer's click threshold (`is_click: bool`). A producer that cannot identify the released button refuses `secondary_press` with reason `button_identity_unavailable` rather than emitting an ambiguous `release`. |
| `cancel` | The pointer stream for an open `press` was interrupted (browser `pointercancel`, producer disconnect, OS gesture takeover). Journaled like any activation event; it ends the gesture, is never a click, and triggers host cleanup of the pressed state. When a producer disconnects with a press still open, the host synthesizes `cancel` for that pointer so no pressed state can leak into replay. |
| `drag` | Delivered on the wire as `press`, a sequence of incremental `drag_move` events (one per cell change, in order), and `release` — the gesture is the sequence, reconstructed in order, so an app can render live feedback during the gesture instead of only at release. Each `drag_move` is its own shell-contract frame, so the contract's 1 MiB frame limit bounds a single move, never the whole path; a producer whose move buffer overflows coalesces consecutive moves (keeping the latest cell) rather than buffering an unbounded path. |
| `scroll` | Wheel or pan delta at a cell (`dx`, `dy` in cell units) |
| `long_press` | A `press` held in place past the long-press threshold (500 ms default, the Android platform default), synthesized by the producer and delivered as its own event |

A click is not a separate wire event: it is the semantic an app derives from `press` + `release` on the same cell within the producer's click threshold (default 250 ms — a project default, not a web standard: Pointer Events leaves click timing to the implementation, device, and platform, and a browser may accept a longer hold as a click where this model does not). The classification bands are exhaustive: a stationary release with `held_duration_ms` at or above the click threshold but below the long-press threshold is a plain `release` with `is_click: false` — a deliberate slow press, not a click and not a `long_press`; an app that wants tap semantics consumes `release` itself and treats `is_click` as a convenience derivation, never the only signal. To preserve deterministic replay without relying on wall-clock timing between journaled events, `release` explicitly carries the producer-measured hold duration (`held_duration_ms`) and the resolved click classification (`is_click`). Every replay consumer therefore observes identical timing and derives the same click semantic deterministically.

## Per-environment extras

Extras are events an environment can produce that the core does not require. An app opts into extras per environment in its interface declaration (the S2 manifest work); an extra the app did not declare is refused like any unsupported event.

| Extra | `native` | `web` | `tui` | `android` |
|---|---|---|---|---|
| `hover_enter` / `hover_leave` (with `hover_move`) | yes | yes, with a fine pointing device | yes, terminal mouse reporting without buttons | no — refused |
| `secondary_press` (right click) | yes | conditional — see Decision 1 | yes, where the terminal reports button 3 and the OS/terminal does not reserve it | no — `long_press` is the equivalent, see Decision 2 |
| Multi-touch contacts beyond the primary pointer | n/a | representable, not in v1 subset | n/a | representable, not in v1 subset |

## Decisions

### Decision 1 — web right click: refused by default; app opt-in may take over the context menu

The question issue #1831 poses — can a Wasm build take over the context menu for right click, or is web right-click refused? — is answered: **both, in that order.** By default, web `secondary_press` is **refused**: the browser's context menu belongs to the user, and a relayed app silently swallowing it would break browser and accessibility conventions. A Wasm build **may** opt in per app: the app's web interface declaration (S2) states `context_menu: app`, and the web shell then suppresses the context menu (`contextmenu` prevented) **only over the app's own surface**, not the surrounding page, and delivers `secondary_press`. Where a browser or embedded webview reserves the gesture anyway, the shell reports the event refused with reason `reserved_by_browser`. So web right click exists in the model only as a declared, conditional extra — never as a core assumption.

### Decision 2 — Android gestures map onto the model, as decisions

| Android gesture | Model event |
|---|---|
| Tap | `press` + `release` (the app derives click) |
| Long press (≥ 500 ms, no move) | `long_press` — the right-click equivalent; this is how Android apps receive secondary actions, since the platform has no right click |
| Touch drag | `drag` |
| Pan / fling | `scroll` (fling carries its velocity as continued `dy`/`dx` deltas) |
| Pinch | refused in v1, reason `gesture_not_in_subset` |
| Stylus hover | refused in v1, reason `hover_unsupported` |

These are the S3 app's contract, not assumptions it may re-decide: if the PoC finds one unworkable (e.g. the host needs fling distinguished from pan), that is a deviation recorded against this design, per the documentation protocol.

### Decision 3 — activation events are journaled; hover is ephemeral

`press`, `release`, `cancel`, `drag` (as its `press` / `drag_move` / `release` sequence), `scroll`, and `long_press` are shell-contract class `event`: journaled and replayed like keyboard `Input`, in sequence order. `long_press` is synthesized by the producer precisely so replay never needs a wall clock; similarly, `release` records `held_duration_ms` and `is_click` so click derivation during replay is deterministic without wall-clock dependence. `hover_enter`, `hover_leave`, and `hover_move` are class `ephemeral`: connection-local presentation context, never journaled and never seen by other sessions — the same boundary the keyboard-parity design drew for resize and focus (one participant's pointer position must not rewrite shared world state).

### Decision 4 — v1 is single-pointer; the model is not

Every event carries `pointer_id`, so multi-touch is representable from the start, but the v1 relay subset guarantees only the primary pointer. A second simultaneous contact is refused with reason `multi_touch_deferred`. This keeps S2/S3 honest about what is proven (the issue's "extend from there once the subset is proven") without painting the model into a single-touch corner.

## Refusal table and refusal semantics

An event an environment cannot produce, or an app did not declare, is **refused explicitly**: the shell answers with a refusal naming the event and a reason from this table, and the producer surfaces it (the S3 app shows it; the #1826 page logs it). Silently dropping an input event is a protocol violation — it is how a user ends up tapping a phone and watching nothing happen with no explanation anywhere.

| Event | `native` | `web` | `tui` | `android` |
|---|---|---|---|---|
| `press` / `release` | supported | supported | supported | supported |
| `cancel` | supported (producer- or host-synthesized) | supported (producer- or host-synthesized) | supported (producer- or host-synthesized) | supported (producer- or host-synthesized) |
| `drag` | supported | supported | supported | supported |
| `scroll` | supported | supported | supported (buttons 4/5) | supported (pan/fling) |
| `long_press` | supported (producer-synthesized) | supported (producer-synthesized) | refused `no_timing_source` | supported |
| `hover_*` | supported | supported with fine pointer; refused `hover_unsupported` on touch-only devices | supported | refused `hover_unsupported` |
| `secondary_press` | supported | refused `context_menu_not_claimed` unless Decision 1 opt-in; then refused `reserved_by_browser` where the browser reserves it | supported where reported; refused `not_reported_by_terminal` otherwise | refused `no_secondary_button` — use `long_press` |
| Contacts beyond primary | n/a | refused `multi_touch_deferred` | n/a | refused `multi_touch_deferred` |
| Pinch / zoom gestures | n/a | refused `gesture_not_in_subset` | n/a | refused `gesture_not_in_subset` |

A second refusal class applies in every environment: an event the target app did not declare in its interface (S2) is refused `not_declared_by_app`, whatever the environment could produce.

Refusals are per-event answers, so they exist only for gestures the producer could observe. A gesture the environment consumes without reporting it at all (a terminal that swallows a right-click for its own menu and reports nothing) produces no event and can produce no refusal — there is nothing to answer. That unobservability is disclosed up front instead: the environment's declared capabilities (the S2 interface work) state which events it cannot report, so an app learns before any gesture that, e.g., `secondary_press` on a given terminal is unobservable, rather than expecting a per-event `not_reported_by_terminal` that can only be returned when the terminal reported enough to identify the gesture.

## How issue #1826 consumes this model

Issue #1826 (browser/Wasm peer page for phone runs) does not get a phone-specific event set. The peer page is a `web`-environment producer of exactly this model: it speaks the same shell-contract frames, follows the web column of every table above (tap on the phone's browser is `press`/`release` via Pointer Events; hover is refused there as on any touch-only web device; right click follows Decision 1). Touch-gesture ownership on the peer page is explicit: the page sets `touch-action: none` on the app's surface, so the browser does not take over pan/zoom gestures that belong to the app; where the browser or OS still interrupts the stream (scroll chaining at a surface edge, OS-level gestures), the producer emits `cancel` and the gesture ends explicitly — the stream is never silently truncated. The native Android app (S3) and the #1826 page therefore relay the same events into the same apps and differ only in their environment column — one model, two phone peers, no duplicated hover/web semantics to drift apart.

## S2 handoff (what the follow-on implements)

1. `Interfaces` gains an input declaration: per environment, which core groups and extras the app accepts. `deny_unknown_fields` stays; a manifest declaring no input accepts keyboard only — existing behavior is unchanged.
2. `shell-contract` names the core/extras frames with the classes in Decision 3, the refusal reasons in the refusal table, and the cell-coordinate rule.
3. Nothing in S2 changes existing `cli`/`web` behavior; S3 (Android PoC) and S4 remain separate, later slices of issue #1831.

## Limits and gaps (recorded for issue #1831)

- No pinch/zoom, no stylus pressure or tilt, no multi-touch beyond the primary pointer in v1 (Decision 4).
- TUI pointer support is bounded by the terminal: no mouse reporting, no events; terminals that reserve right-click for their own menus refuse it.
- Web hover requires a fine pointing device; touch laptops in touch mode refuse hover, per the table.
- Latency of the relay path (phone → host → rendered response) is an S3 measurement, not a number this design asserts.

## Acceptance mapping (issue #1831, criterion 1)

- Common core defined: press/click (as `press`/`release` + derived click), drag, scroll, long press — "The common core" above.
- Per-environment extras: hover enter/leave on web and TUI, right click where the platform allows it — "Per-environment extras".
- Web/Wasm context-menu question answered explicitly — Decision 1 (refused by default; per-app opt-in takes over the menu on the app surface only).
- Android gesture mapping stated as design decisions — Decision 2.
- Refusal table naming refused events per environment — "Refusal table and refusal semantics".
- Issue #1826 consumption stated — "How issue #1826 consumes this model".
