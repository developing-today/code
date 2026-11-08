# Design: adapter conformance harness & interop matrix CI

- Date: 2026-11-08
- Status: **partial decision record** — the harness architecture,
  certification criteria, matrix axes, CI gate placement, and the
  fixture/oracle model are decided now (§2–§6); the per-sibling
  fixture content and matrix rows (§7) are explicitly **blocked** on
  #5808–#5813, all verified OPEN on 2026-11-08 with no decision
  recorded. Issue #5814 therefore stays open for the §7 remainder,
  in the decided-now / blocked-remainder shape of
  [the #5866 record](../2026-11-08_design_id5866.md).
- Issue: [#5814](https://github.com/developing-today/code/issues/5814),
  follow-on of [#5674](https://github.com/developing-today/code/issues/5674)
  (postcard wire-format family and the transport-adapter model),
  programme epic #2056.
- Proof obligation source:
  [#5975](https://github.com/developing-today/code/issues/5975) — its
  record, [Self-hosting and interop stance](../2026-11-08T09-12-27Z_design_id-self-hosting-interop-stance/2026-11-08T09-12-27Z_design_id-self-hosting-interop-stance.md),
  assigns the conformance proof to this issue: every load-bearing
  element of the interop contract gets a harness check and a matrix
  row, so "does it actually interoperate" has a testable answer per
  element rather than a claim.
- Siblings consumed, not re-decided:
  [#5808](https://github.com/developing-today/code/issues/5808)
  (postcard wire schema v1),
  [#5809](https://github.com/developing-today/code/issues/5809)
  (versioning, negotiation & forward-compatibility policy),
  [#5810](https://github.com/developing-today/code/issues/5810)
  (framing & fragmentation per carrier class),
  [#5811](https://github.com/developing-today/code/issues/5811)
  (adapter: WebSocket/WSS + HTTPS fallback),
  [#5812](https://github.com/developing-today/code/issues/5812)
  (adapters: WebRTC, TCP/TLS, local IPC),
  [#5813](https://github.com/developing-today/code/issues/5813)
  (relay & proxy standard protocol).

## 1. What "certified" means here

An adapter is **certified** when the conformance harness (§2) passes
it against every certification criterion in §3, in every interop
matrix cell (§4) that applies to its carrier class, on the CI gate
in §5 — with the pass decided by the fixture/oracle model in §6,
never by inspection. Anything short of that is uncertified, however
much code it shares with a certified adapter: #5975 already fixes
that conformance is judged against the contract, never against a
parts list, and that library reuse alone is not conformance. This
record gives that position its enforcement mechanism.

## 2. Decision: one shared contract harness, adapter bindings only

1. **The harness is a single contract suite in `pkgs/id`, written
   once against the transport-adapter surface, in the shape the
   repo already uses:** the embedded-contract checker in
   [`pkgs/id/src/shell_contract.rs`](../../pkgs/id/src/shell_contract.rs)
   and its suite in
   [`pkgs/id/src/shell_contract/tests.rs`](../../pkgs/id/src/shell_contract/tests.rs),
   and the checked-peer driver in
   [`pkgs/id/src/world_session/contract_tests.rs`](../../pkgs/id/src/world_session/contract_tests.rs)
   — a scripted peer whose every frame, both ways, passes through
   the contract check. The conformance harness generalises that
   pattern from the shell contract to the adapter contract; it does
   not invent a second testing style.
2. **An adapter enters the harness through a binding only.** The
   binding constructs the adapter over an in-process carrier (the
   channel-backed `SessionIo` shape the contract tests already use)
   and declares the adapter's carrier class (§4). The binding
   contains no assertions; every assertion lives in the shared
   suite, so two adapters can never be certified against different
   readings of the contract.
3. **Both directions are checked.** The harness drives the adapter
   as client and as server (the checked peer says and listens), and
   checks frames the adapter emits as strictly as frames it accepts:
   an adapter that parses leniently but emits non-conforming frames
   does not interoperate and does not pass.
4. **The harness is implementation-agnostic.** It speaks to the
   adapter only in wire frames (postcard bytes; `postcard` is
   already a `pkgs/id` dependency) and carrier operations, so a
   from-scratch implementation in any language can be certified by
   the same suite through a binding — the #5975 requirement that
   conformance not be a Rust-library club.

## 3. Decision: certification criteria

An adapter is certified only when **all** of the following hold.
The criteria are pinned to the four load-bearing elements #5975
names; the concrete per-element assertions fill in per §7.

- **C1 — Wire format.** Every fixture frame the harness emits is
  accepted and decoded to the declared value; every frame the
  adapter emits decodes under the contract schema; malformed,
  truncated, and trailing-garbage frames are rejected with the
  contract's error behaviour, never partially applied.
- **C2 — Identity.** The adapter carries the identity binding
  (name-to-proof) unaltered across its carrier: a peer's proof
  verifies after transport exactly as it verifies in-process, and
  the adapter neither strips, rebinds, nor substitutes identity
  material.
- **C3 — Capability negotiation.** The adapter performs negotiation
  before relying on any capability, honours the negotiated ceiling
  (including attenuated capabilities), and fails closed — visibly,
  with the contract's error — when the peer does not offer a
  capability the session requires. Silent assumption is a failure.
- **C4 — Registry format.** Registry entries and references carried
  over the adapter resolve identically to the in-process reference
  behaviour: an entry registered through the adapter is resolvable
  by a peer that never used it.
- **C5 — Carrier honesty.** The adapter preserves the framing
  guarantees its carrier class declares (§4, #5810) — ordering,
  message boundaries or their declared absence, and fragmentation
  behaviour — and surfaces carrier failure (close, reset, timeout)
  as the contract's session failure, never as a hang or a
  half-frame.
- **C6 — No waivers.** There is no per-adapter skip list. A suite
  case that does not apply to a carrier class is excluded by the
  class declaration in the binding, recorded in the matrix; a case
  that applies and fails is a certification failure, not a known
  issue. Deliberately failing legs follow the repo's existing
  visible-failure precedent (`.github/workflows/mcpx-conformance.yml`:
  failures are read, not hidden) — visibility is for diagnosis,
  never a substitute for the §5 gate.

## 4. Decision: the interop matrix axes

The matrix is the Cartesian product of three axes, pruned only by
carrier-class applicability:

- **Adapter** (rows as the siblings land): the native/UDP baseline
  of #5674, WebSocket/WSS + HTTPS fallback (#5811), WebRTC,
  TCP/TLS, local IPC (#5812), and the relay & proxy protocol
  (#5813).
- **Carrier class** (from #5810): stream, datagram, and message —
  each adapter is tested in its declared class, and a multi-class
  adapter is tested once per class it claims.
- **Platform:** the platforms `pkgs/id` ships for — Linux,
  macOS, and Windows — exercised where the adapter's carrier exists
  on that platform.

Two dimensions ride on the adapter axis rather than standing alone:

- **Pairing.** Every cell runs twice over: the adapter against the
  reference in-process peer (the harness), and the adapter against
  *another certified adapter* over the real carrier. Self-pairing
  proves contract conformance; cross-pairing is the interop claim
  itself — an adapter that only talks to itself is not certified
  interoperable.
- **Direction.** Client-initiated and server-initiated sessions are
  separate legs inside each cell (§2.3), not separate cells.

A cell exists — and is mandatory — from the moment the sibling that
defines its adapter or carrier rule lands (§7). Before that, the
cell is listed as blocked in the matrix, not silently absent.

## 5. Decision: which CI gates certification

1. **PR gate.** The conformance suite is a `pkgs/id` flake check,
   run by the existing `id` workflow
   (`.github/workflows/id-check.yml`) in its per-check matrix for
   any PR touching `pkgs/id/**`. An adapter PR that fails the suite
   does not merge — certification is a merge gate, not a report.
2. **Full-matrix gate.** The complete matrix of §4 (every adapter ×
   carrier class × platform, both pairings) runs on the nightly
   cadence alongside `id-e2e-nightly.yml`; a nightly matrix failure
   on a previously certified cell revokes the cell's certified
   status until fixed, and is filed as a regression against the
   adapter, not absorbed as flake.
3. **No third gate.** Manual runs and dashboards may exist for
   diagnosis, but no certification claim may cite them: a cell is
   certified exactly when the PR gate's suite version and the
   nightly matrix agree it passes.

## 6. Decision: fixture/oracle model — a pass must be decidable

1. **Fixtures are canonical wire scripts, not recordings of one
   adapter.** A fixture is a versioned, in-repo sequence of steps —
   emit these postcard bytes, expect this verdict — expressed in
   contract terms, following the journal-replay shape of
   [the #5866 record](../2026-11-08_design_id5866.md) §3: fixed
   identities, timestamps, and ordering, so a run exercises the same
   verification paths as production traffic.
2. **The oracle is declared per step, in advance.** Each step names
   the expected outcome class — accept-and-decode-to-value,
   reject-with-error, negotiate-to-ceiling, resolve-to-entry,
   fail-session — and the expected emitted frame's *decoded value*,
   never its raw bytes alone. The harness compares the adapter's
   actual outcome to the declaration; equality is the pass. There is
   no human judgement inside a run.
3. **Negative fixtures are first-class.** At least one malformed,
   one over-ceiling, and one identity-mismatch fixture per
   load-bearing element; an adapter that accepts what the contract
   rejects cannot be certified by its positive results.
4. **Deterministic and hermetic.** Fixtures run with no network, no
   wall-clock dependence, and no production data; the same fixture
   pack against the same adapter yields the same verdict on every
   platform in the matrix, and a divergence is a defect in the
   adapter or the fixture — recorded either way, never re-rolled.
5. **Rejected alternatives:**
   - *Per-adapter bespoke tests* (each adapter's author writes its
     own suite): the suites would diverge in exactly the places the
     contract is ambiguous, certifying the author's reading instead
     of the contract. This is the failure the shared harness exists
     to prevent.
   - *Manual certification* (a reviewer runs the adapter once and
     attests): unrepeatable, unauditable, and it rots silently as
     the adapter changes — "certified, not hoped" requires a gate
     that re-runs on every change (§5).
   - *Snapshot/golden-file-only oracles* (compare emitted bytes to a
     stored golden file): they pin one implementation's accidental
     byte choices, bless regressions by re-recording, and cannot
     express rejection or negotiation outcomes. Golden bytes are
     legitimate as fixture *input*; as the oracle they decide
     nothing about conformance.
   - *Testing only against our own implementation's peer:* proves
     self-consistency, not interop; §4's cross-pairing requirement
     exists because the claim being certified is interoperability
     (#5975: a from-scratch implementation that satisfies the
     contract interoperates).

## Invariants implementation must hold

- **I1 — One suite.** Every certification assertion lives in the
  shared harness; bindings contain construction and class
  declaration only.
- **I2 — Contract-version pinning.** The harness declares the
  contract/schema version it certifies against; certifying an
  adapter against a different version than it declares fails
  loudly, in the shape of #5866's I1.
- **I3 — Decidable verdicts.** Every fixture step's expected outcome
  is declared before the run (§6.2); a step whose outcome cannot be
  machine-compared is not a certification step.
- **I4 — Both pairings, both directions.** No cell is certified on
  self-pairing alone, or on one session direction alone (§4).
- **I5 — No silent absence.** A matrix cell that cannot yet run is
  listed blocked with its blocking sibling; a cell that disappears
  from the matrix without a record amendment is a defect.
- **I6 — Policy is not conformance.** Permission, licensing, and
  moderation defaults are never harness assertions (#5975: they are
  server-admin choices); the harness must not fail an adapter for a
  policy difference.

## 7. BLOCKED: per-sibling fixture content and matrix rows

**Not decided by this record.** The concrete assertions under
C1–C5 depend on decisions that have not landed — verified
2026-11-08: #5808, #5809, #5810, #5811, #5812, and #5813 are all
OPEN with no decision recorded. Fixing their content here would
either guess a sibling's decision or silently constrain it; this
record does neither.

What fills in, mechanically, as each sibling lands:

- **#5808 (wire schema v1)** → the C1 fixture packs: the canonical
  frame set, the malformed-frame corpus, and the declared decode
  values.
- **#5809 (versioning & negotiation)** → the C3 verdict tables:
  each offered/required capability combination and its declared
  negotiate-or-fail outcome.
- **#5810 (framing per carrier class)** → the C5 cases and the
  carrier-class axis's concrete guarantees (boundaries, ordering,
  fragmentation limits) per class.
- **#5811 / #5812 / #5813 (the adapters and relay)** → their matrix
  rows become mandatory cells (§4) and their bindings enter the
  harness; the relay row additionally certifies that relaying does
  not alter C2/C4 material in transit.

**Resume condition:** each sibling's landing amends this record's
§7 with that sibling's pinned content and activates its cells; when
the last of #5808–#5813 has landed and its cells are pinned, the
remainder is exhausted and #5814 closes. Until then #5814 stays
open for exactly this remainder — no other part of #5814 remains
undecided.

## 8. Parent decisions: consumed, none changed

This record consumes #5674's framing (postcard wire-format family;
the transport-adapter model), #5975's interop contract (the four
load-bearing elements are §3's criteria verbatim), and the
decided-now / blocked-remainder shape of #5866. **No #5674 or #5975
decision is changed, narrowed, or pre-empted here.** One boundary
is worth stating explicitly because it is the only place this
record comes close to one: §5 makes certification a *merge gate
for adapter PRs in this repo*. That is a process choice about our
own repository, not a protocol requirement — #5975 fixes that a
self-hoster's conformance is a property of their implementation
against the contract, and nothing here makes our CI a requirement
of interoperability for anyone else. If a future amendment finds a
§7 fill-in cannot be applied without changing a #5674 or #5975
decision, that conflict is recorded as an explicit proposed change
to the parent decision, never folded in silently.

## 9. Scope exclusions

No harness code (implementation is built against this record later,
on its own tickets), no re-decision of the wire schema,
negotiation, framing, or any adapter/relay design (#5808–#5813
keep their own records), and no changes to the shell contract or
world-session suites — they are cited as precedent, not modified.
