# D1-045 — In-session command toggle (prefix key + visible mode indicator)

Research package for tracker item D1-045 in
[#535](https://github.com/developing-today/code/issues/535). Source: REF
§106-112, 135-149 (shape 3, triggers); RPT §7 Step 3.1; RPT §16 Q4.
Prerequisite [#546](https://github.com/developing-today/code/issues/546)
(D1-041: one line grammar shared by SSH command mode, the `id world join`
pump, and CLI verbs) is closed, so the grammar this design reuses exists.

**Headline: the demand gate is not met — see the verdict at the end. This
package is written so that the day the gate is met, the implementation
issue needs no further research.**

## Question

An SSH world session is in exactly one mode, chosen at login. Raw mode
forwards keystrokes to the world program; command mode parses lines
locally into session frames (`/records`, `/stats`, `/quit`, …). Can a user
in a raw session drop into a command prompt and return, without
reconnecting — and should they be able to?

## Current state (verified on main, 2026-11-01)

- Mode is a login property: prefixing the SSH username with `cmd:` sets
  `command_mode` (`COMMAND_PREFIX`, `pkgs/id/src/world_ssh.rs`), and the
  session's `ShellMode` is fixed from it for the connection's life.
- In raw mode, `SshIo::recv` forwards input frames unconditionally — there
  is no prefix handling, no escape state, nowhere a toggle could hide
  today.
- Command mode parses each line through the shared line grammar from #546
  (the same grammar drives the `id world join` pump in
  `pkgs/id/src/commands/world.rs`), and `command_frame` turns parsed
  commands into session frames. Anything the toggle offers is therefore
  already built — only the door is missing.

## The trigger

REF shape 3 names two candidate triggers; the analysis:

- **Ctrl-P.** One byte, no line-start condition needed. But Ctrl-P is
  "previous" in readline-style editors and is a plausible in-world key;
  raw mode's whole promise is that bytes belong to the program. Stealing
  one control byte from every program, always, is a real tax.
- **`~` at line start, then Enter** (the OpenSSH escape convention). Costs
  nothing except a literal tilde starting a line followed by Enter —
  escapable by doubling (`~~` sends one literal `~`), the convention
  every SSH user already knows from `~.` — and only arms at line start,
  so mid-line tildes are untouched.
- **Recommendation:** `~` + Enter as the trigger, OpenSSH semantics:
  the escape is recognised only immediately after a newline, `~~` yields
  a literal `~`, and the escape line itself is consumed by the shell,
  never forwarded. The state machine needs line-start tracking in the raw
  input path and one byte of lookahead (the byte after `~` decides);
  both are small and testable. Ctrl-P is the fallback if a future client
  cannot send `~` reliably — not a second trigger in v1 (two doors,
  double the collision surface).

## The mode indicator

While toggled, the user must be able to *see* they are talking to the
shell, not the world. Options:

- A pinned bottom row belongs to D1-046 (split UI), which does not exist
  yet and is itself gated behind this item — building the indicator on it
  would invert the dependency.
- A terminal-title (OSC) indicator is invisible in many clients.
- **Recommendation for v1: an in-band prompt line.** On entry the server
  writes a distinct prompt (e.g. `\r\n[command] world> `); each command
  line is answered as in command mode today; the prompt is the indicator.
  Entry also stops forwarding keystrokes to the guest (output from the
  guest may continue — pausing it would freeze live worlds; interleaving
  is accepted, as in OpenSSH's escape, which also only intercepts input).

## The return path

Three exits from the toggled prompt, all parsed by the shared grammar:

- the prefix again (`~` + Enter) — toggles back;
- `/return` — an explicit verb, for discoverability;
- `/quit` — still quits the *session*, as it does in command mode today;
  it must not silently degrade into "return".

On return the guest needs a repaint: raw-mode guests render with a full
clear-repaint (D1-044's incremental renderer is a separate, still-open
prerequisite of D1-052), so the shell requests a redraw the way a resize
already forces one — the `Resize` frame is the existing precedent for
"presentation state changed, repaint". The repaint is the main
implementation risk and the first thing the implementation issue should
prototype: a guest that does not repaint on resize will come back stale
after every toggle round-trip.

## Risks and open questions

- Guests that read tilde-led lines (text adventures, REPLs) lose exactly
  the `~`+Enter sequence unless they double the tilde; documented in the
  prompt help text, same as OpenSSH.
- Paste bursts containing `\n~` followed by Enter will trip the escape;
  OpenSSH has the same behaviour and the doubled-tilde escape covers
  deliberate input.
- Browser/CLI clients have no raw keystroke stream of this shape; the
  toggle is an SSH-shell feature and the other clients keep choosing
  their mode per connection. Parity is explicitly not claimed.
- The indicator line interleaves with guest output (no regions until
  D1-046). Accepted for v1; it is how the escape behaves in OpenSSH too.

## Demand-gate assessment — VERDICT: NOT MET

The tracker conditions this item: "**Only if users ask for it.**" That
condition is the design's own gate (RPT §16 Q4 left it an open question),
so this package assessed it against the record on 2026-11-01:

- Repository issue search for "command toggle" and "prefix key" returns
  only #535 itself — no issue, from any user, requests this feature.
- #535's comment history is agent triage notes; no user request for an
  in-session toggle appears there either.
- The only origin of the idea is the internal design report (RPT §7 Step
  3.1), which proposed it conditionally — a proposal is not a request.

**Verdict: the demand gate is not met.** Per the task acceptance for
#535's D1-045, no implementation issue is filed and the D1-045 checkbox
stays unticked. The gate is met the day a request from an actual user of
the SSH client is on the record (an issue, or Drew noting a request on
#535); at that point the implementation issue files directly from this
package — trigger, indicator, return path, and the repaint prototype are
already decided above.

## Links

- Tracker: #535 item D1-045. Prerequisite: #546 (closed).
- Mode selection and raw path: `COMMAND_PREFIX`, `SshIo::recv` in
  `pkgs/id/src/world_ssh.rs`; shared line grammar: `pkgs/id/src/
  commands/world.rs` (pump) and the `WorldLine` parsing in `world_ssh.rs`.
- Successor that builds on this (and is gated behind it): D1-046 in #535.
