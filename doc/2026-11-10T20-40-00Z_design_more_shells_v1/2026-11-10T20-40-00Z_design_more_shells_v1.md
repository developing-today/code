# More shells v1 — first-iteration design (mobile, native, hardware)

Issue: [#6296](https://github.com/developing-today/code/issues/6296)
(v1 — first-iteration design)
Core topic: [#6279](https://github.com/developing-today/code/issues/6279)
(Phase 14: More shells — research + design)
Filed from: [#1240](https://github.com/developing-today/code/issues/1240)
(next-horizons; Drew's Phase 14 principles, 2026-11-05)
Builds on: the F13.1 shell contract
([#783](https://github.com/developing-today/code/issues/783);
`doc/2026-10-25T05-54-40Z_reference_shell_contract`,
`pkgs/id/contract/shell-contract.v1.json`), F13.6 native shells
([#788](https://github.com/developing-today/code/issues/788), open),
F13.7 capability providers
([#789](https://github.com/developing-today/code/issues/789), closed).
Status: design only — the v1 design record for the More shells
topic. No implementation is landed or claimed by this document.

## Principles (from #6279, Drew 2026-11-05 — constraints, not flavor)

1. **Local-first is always the goal, AND a hosted version is
   intended.** Central coordination supports a large graph of
   broadly accessible objects; it is an offloadable backstop, never
   a requirement. The app must not technically need us to function.
2. **Self-hosting parity.** P2P-hosted worlds and data behave the
   same as on a large hosted public server; differences live only
   in backend scale elements and access controls.
3. **Direct where possible.** Users connect to each other and
   transfer/interact directly; all comms through iroh on the
   backend; every shell in this design is a first-class client.
4. **Publishing paths are plural:** share to a big server, publish
   on pkarr, or self-host. No shell may assume exactly one.

## The one gate

Phase 13 made shells a contract; v1 spends it. **A shell is done
when it passes the F13.1 conformance check, not when it demos.**
The check already exists: a shell records a session against
shell-contract v1 (`pkgs/id/contract/shell-contract.v1.json`,
printed by `id world contract`) and runs the conformance command;
the contract is strict about what a shell *sends* and tolerant
about what it *reads*. Every shell below — phone, native,
ESP32-class — is specified as a **capability statement against
that contract**: the set of frames it sends, the server frames it
presents, and the capability providers it declares. A shell that
cannot state its capabilities in those terms is not a v1 shell.

Shell-contract v1 adds no frame kinds (the F11.12 journal-format
gate named in the contract record still stands), so v1 shells work
within the existing frame set. Anything a shell needs that the
contract does not name is a contract-revision proposal, filed
separately — never a private extension.

## v1 shells

### 1. Mobile web / PWA — the browser shell, made honest offline

The browser shell exists (`pkgs/id/web/src/world.ts`). v1 is not a
rewrite; it is the offline and notification story the core topic
asks for:

- **Offline read model.** A journaled world is its journal. The
  shell keeps a local snapshot (last applied journal sequence +
  rendered state) sufficient to *read* the world fully offline.
  Offline writes are not queued as intent: the journal is the
  authority, and a phone that cannot reach the host (directly or
  via iroh) is a reader, not a writer. The capability statement
  says so.
- **Reconnect and catch-up.** On reconnect the shell presents its
  last applied sequence; the host streams the missed journal
  suffix; the shell applies it in order and resumes live frames.
  Divergence (host journal rewritten past the shell's sequence)
  resolves by re-snapshot, never by merge — the journal's
  total order is the design's answer to conflict.
- **Push/notify** arrives through the F12.5 provider mechanism as
  a capability provider (below): a notification is a hint to
  re-sync, carrying no world content of its own.
- **Installable PWA shape**, same origin and session code as the
  desktop browser shell; the frame layer is shared verbatim, the
  presentation layer is what shrinks.

### 2. Native — one target, chosen to prove the frame layer splits

F13.6 (#788, open) is the native-shell line; v1 names its first
target and its proof obligation:

- **Shared frame layer, native presentation.** The Rust
  `ClientFrame`/`ServerFrame` types are the frame layer and are
  reused as-is; each native shell writes only presentation and
  platform glue. The v1 design question — what #1072's research
  left open, per #6279 — is answered by construction: if a second
  native target needs frame-layer changes, the split is wrong and
  the contract record gets the finding.
- **First target: a desktop native shell** on the platform the
  fleet develops on (Linux), because its conformance run is
  cheapest to wire into the existing check. Mobile-native
  (Android/iOS) is a v2 question (#6297's ladder rung), not a v1
  omission: the PWA covers phones in v1.
- **Done = conformance**, recorded session and all, exactly as
  the contract record prescribes for "a native Bevy or Qt client".

### 3. Hardware — ESP32-class, an interaction subset stated plainly

The hardware shell is where "capability statement" stops being a
metaphor:

- **What survives at this size:** presence (who/what is in the
  world), journal-head state in compressed form, and a small set
  of deliberate inputs (confirm/act on a presented choice). Rich
  rendering, free text entry, and world hosting do not survive,
  and the capability statement excludes them by name.
- **Transport** is iroh like every other shell (Principle 3);
  where the device class cannot hold a full iroh node, it speaks
  to a paired full node it owns — the pairing is a deployment
  fact, not a protocol fork, and the frames on the wire are the
  contract's.
- **Power/attention posture:** the device polls or is woken on
  journal-head change; it never holds a live session open to
  look alive. A hardware shell that is off is absent, and the
  contract already tolerates absent shells.
- **Done = conformance** on the frames it does send, plus a
  recorded statement of the subset above. A hardware shell
  claiming frames it cannot present is the failure mode this
  section exists to prevent.

## Capability providers — the F13.7 extension point

Shell-specific powers (camera, files, sensors, notifications)
enter only through **F13.7 capability providers** (#789, closed:
client-side capability providers under the ceiling model). Rules
for v1:

- A power a shell has is a declared provider with a manifest, in
  the F12.4 provider-registry shape (#1061, closed/landed) — never
  an ambient platform API the shell happens to reach.
- The host learns a shell's powers from its declarations at
  session start; undeclared powers do not exist for that session.
- The ceiling model binds: a provider can only narrow what the
  shell's session is allowed, never widen it past the world
  host's own grants.

## Sequencing (recorded per the core topic's instruction)

- **F12.4 provider registry (#1061): LANDED (closed).** The
  registry/manifest shape this design relies on exists.
- **F12.10 fair scheduling and per-account quotas (#1067): STILL
  OPEN.** Named explicitly, per the topic's sequencing note:
  quotas bind the *hosted* side (many accounts on one host), and
  every v1 shell above is local-first — none needs a quota
  decision to be designed or to pass conformance against a
  self-hosted world. The dependency lands where it belongs: the
  hosted-deployment follow-on, not this record. If a v1
  implementation slice reaches a hosted surface before #1067
  lands, it stops and names #1067 rather than inventing quota
  behavior.
- **F13.6 native shells (#788): open** — the native section above
  is written to be its v1 input.
- **F13.7 capability providers (#789): closed** — relied on as
  landed.

## Follow-on slices (implementation issues to file from this record)

1. PWA offline read model + catch-up in the browser shell
   (snapshot store, sequence resume, re-snapshot on divergence).
2. First native shell (Linux desktop) against the shared frame
   layer, with its conformance run wired into the check.
3. ESP32-class hardware shell spike: presence + journal-head +
   confirm/act subset, capability statement recorded.
4. Notification capability provider (F12.5 mechanism) for the
   mobile shell.
5. Conformance-suite packaging: one command runs every v1
   shell's recorded session (the suite is the product, per
   #6279).

## What this record does not decide

- v2–v4 ladder scope (#6297, #6298, #6299) — each gets its own
  record when its rung starts.
- Mobile-native (app-store) shells — deferred above, with the
  reason stated.
- Any new frame kind — gated on F11.12 and a contract revision,
  per the contract record.
- Hosted quota/scheduling behavior — #1067's design, named above.
