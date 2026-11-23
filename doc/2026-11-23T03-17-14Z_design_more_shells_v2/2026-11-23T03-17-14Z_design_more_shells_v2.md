# More shells v2 — outline: native & TUI completion

Issue: [#6297](https://github.com/developing-today/code/issues/6297)
(v2 — outline: native & TUI completion)
Core topic: [#6279](https://github.com/developing-today/code/issues/6279)
(Phase 14: More shells — research + design)
Previous rung: v1 — first-iteration design
([#6296](https://github.com/developing-today/code/issues/6296), closed;
record `doc/2026-11-10T20-40-00Z_design_more_shells_v1/`, PR #8243)
Sibling rungs: v3 — hardware shell
([#6298](https://github.com/developing-today/code/issues/6298)),
v4 — third-party shells
([#6299](https://github.com/developing-today/code/issues/6299))
Scope sources: native GUI client
([#1072](https://github.com/developing-today/code/issues/1072), open),
native TUI client
([#1071](https://github.com/developing-today/code/issues/1071), open),
F13.6 native shells
([#788](https://github.com/developing-today/code/issues/788), open)
Status: outline only — the v2 design outline for the More shells
topic. No implementation is landed or claimed by this document.
The v1 slice ladder (#10927–#10931) is still the programme's
active front; this outline takes its turn behind it, per the
2026-11-17 triage of record on #6297.

## Principles (unchanged from v1, Drew 2026-11-05)

Local-first always, with a hosted version as an offloadable
backstop, never a requirement; self-hosting parity between p2p
and hosted behaviour; direct connections over iroh, every shell
a first-class client; plural publishing paths (big server,
pkarr, self-host). The v1 record states them in full; they bind
this rung identically.

## What v1 settled that v2 builds on

- A shell is a **capability statement against the F13.1 shell
  contract** (frames sent, server frames presented, capability
  providers declared), and **done = conformance plus behavioral
  completion checks** for the shell's required workflow — never
  a demo, never conformance on a thin transcript alone.
- The frame layer is the shared Rust `ClientFrame`/`ServerFrame`
  types; shells write presentation and platform glue only. If a
  second native target needs frame-layer changes, the split is
  wrong and the contract record gets the finding.
- Provider declaration in v1 rides the F12.4 registry/manifest
  out of band; in-band negotiation waits for a contract revision.
- Reconnect requires a history-identity check; divergence
  resolves by re-snapshot, never by merge.

## v2 scope

### 1. Native desktop — the general-availability path (from #1072)

v1 named the first native target (Linux desktop, slice #10928)
to prove the frame layer splits. v2 is the path from that proof
to a shell a user can simply install:

- **Second native target.** One more desktop platform, chosen
  under Open questions below. Its obligation is the v1 proof
  obligation at GA stakes: presentation and glue only, or the
  frame-layer finding goes to the contract record.
- **Packaging and distribution.** An installable artifact per
  target platform, built and released through the fleet's normal
  channels — no hand-built binaries, no shell that exists only
  on a developer's machine.
- **Release gate.** GA means the conformance suite and the
  behavioral completion checks (offline reading, reconnect
  recovery, presentation of the claimed subset) run in the
  release path and block it — the v1 "one gate" applied to a
  shipped artifact, not a spike.

### 2. TUI — feature completion (from #1071)

The ratatui TUI client is a shell in the contract's terms, not
a debug view:

- **Claimed subset, stated plainly** in the hardware shell's
  style: the frames it sends, the server frames it presents,
  the capability providers it declares — and what it excludes
  by name (rich rendering and media are the expected exclusions;
  the capability statement settles the list).
- **Feature completion** means the TUI covers the full
  text-native workflow of a world session — join, read, act,
  reconnect with the v1 history-identity rule — at parity with
  the browser shell's text surface, not a read-only tail of the
  journal.
- **Done = the same gate:** conformance on its recorded session
  plus behavioral completion checks for that workflow.

### 3. The mobile-native question — settled here, one way or the other

v1 explicitly deferred mobile-native (app-store) shells to this
rung: the PWA covered phones in v1, and v2 is where the ladder
said the question gets answered. This iteration must record a
decision — adopt app-store shells as a v2 slice, defer them
again with the reason and the rung that owns them next, or drop
them with the reason. Silence is the one outcome this outline
forbids; the decision itself is Drew's (Open questions).

### 4. One conformance suite across every shell

v1 slice #10931 packages the suite for the v1 shells. v2
extends the same suite — one command, every shell's recorded
session and behavioral checks — to the native GA targets and
the TUI, so "passes the suite" means the same thing on every
rung of the ladder, including v3's hardware subset.

## Non-goals

- Hardware shell expansion — that is v3 (#6298); v2 touches
  hardware only through the shared suite (§4).
- Third-party shells and any shell SDK for outside authors —
  v4 (#6299).
- Any new frame kind — still gated on F11.12 and a contract
  revision, per the contract record; v2 works within the
  existing frame set.
- Hosted quota/scheduling behaviour — #1067's design; as in v1,
  a v2 slice that reaches a hosted surface before #1067 lands
  stops and names it rather than inventing quota behaviour.
- In-band provider negotiation — still gated on the contract
  revision that names it; v2 shells declare out of band via
  F12.4 exactly as v1 shells do, unless that revision lands
  first (Decisions, D4).

## Key decisions this iteration must settle

- **D1 — Second native target platform** (needs Drew's ruling;
  Open questions Q1).
- **D2 — Mobile-native:** adopt, defer with an owner rung, or
  drop (needs Drew's ruling; Q2).
- **D3 — TUI claimed subset:** the exact frame/presentation
  subset the TUI claims, recorded as its capability statement
  before implementation starts.
- **D4 — Contract revision timing:** whether the in-band
  provider-declaration revision is pulled into v2's scope or
  stays a separate contract-track item v2 merely consumes if
  it lands.
- **D5 — GA support bar:** which platforms/versions the native
  GA path commits to keeping green in the suite, and the
  deprecation rule for dropping one.

## Open questions needing Drew's ruling (explicit, per the acceptance criteria)

- **Q1 (D1):** Which second desktop platform proves the
  frame-layer split at GA — macOS or Windows? (Fleet development
  is Linux-first; either choice is a real second toolchain in
  the release path.)
- **Q2 (D2):** Mobile-native app-store shells: adopt in v2,
  defer to a named later rung, or drop? The PWA remains the
  phone story until this is answered "adopt".
- **Q3 (D5):** Is the GA bar "the suite is green on every
  supported platform before any release ships", or per-platform
  releases that can ship independently when their own suite
  passes?

Decisions a lane or the design mill may settle without Drew:
D3 (recorded as the TUI's capability statement and reviewed
against the contract) and D4 (a sequencing call, unless it
grows into a frame-kind change, which returns it to the
F11.12 gate).

## Dependencies

- v1 slices #10927–#10931 (all open at this writing) — in
  particular #10928 (first native target; blocks the second
  target and the GA packaging) and #10931 (conformance-suite
  packaging; blocks §4's extension).
- F13.6 native shells (#788, open) — the native sections of v1
  and v2 are its inputs.
- F12.10 quotas (#1067, open) — hosted surfaces only, as above.
- The shell-contract record — any contract revision (D4) lands
  there first, never as a shell-private extension.

## Follow-on slices (filed as linked issues from this outline)

1. Second native target on the shared frame layer (blocked by
   #10928; platform per Q1's ruling).
2. TUI client feature completion on the shell contract
   (capability statement first, per D3).
3. Native GA packaging, distribution, and the release gate
   (blocked by slices 1 and #10928).
4. Mobile-native decision record (Q2/D2 — the decision and its
   recorded reasoning; implementation, if adopted, is filed
   from that record, not from this outline).
5. Conformance suite extension to the v2 shells (blocked by
   #10931 and slices 1–2).

## What this outline does not decide

- v3 (#6298) and v4 (#6299) scope — each gets its own record
  when its rung starts, as v1's record said of this one.
- The Q1–Q3 rulings themselves — they are Drew's, listed above;
  this outline frames them and proceeds on everything that
  does not wait for them.
- Any frame kind, quota behaviour, or contract change — the
  gates named above own those.
