# Feature-leveled build variants — design record

Design record for issue #5788 ("id: design: feature-leveled build
variants on the crate split — client-only, server, headless, embedded
(from #3447)"). Written per `doc/DOCUMENTATION_PROTOCOL.md` (dated
record in `doc/`). Origin issue: #5788, parent epic #2056. Depends on
the crate split #3447 and its seam issues #4403–#4411; related:
#3846 (F13.8 client-only boundary, closed — the boundary this record
builds products on), #1066 (F12.9 program upgrade semantics),
#5685 (per-interface benchmark matrix — variants join it), #788
(reference shell). Consumers already named: the Android client, the
ESP32/provider line, Bevy incorporation, the terminal shell.

Status: v1 design, settled. No implementation is authorized by this
record alone; §9 names the ordering and the implementation slices as
next actions, gated on the #3447 seams.

Verified at origin/main `129df4af83` (2026-11-09): the workspace at
`pkgs/id/` contains only the root `id` package plus the two extracted
crates `crates/id-core` and `crates/id-native-shell` (the latter
depends on `id` with default features off, `world` on). All
capability selection still lives as cargo features on the root `id`
package: `default = [sandbox, world]`, `sandbox` (wasmtime),
`world`, `ssh`, `web`, `dht`, and the off-by-default currency-PoC
harness binding. There are no variant composition crates, no
capability matrix, and no cross-variant conformance contract in the
tree. #3447 and seams #4403, #4405, #4406, #4408, #4409, #4411 are
open; #4407 is open/blocked; #4404 is closed and #4410 merged
(neither is a blocker for this record).

---

## 1. Problem

The crate split exists to make variants possible, but nothing
decides what the variants *are*. Without a settled level set, each
consumer invents its own feature combination on the root package,
and "client-only" becomes whatever the Android build happened to
link this month. Feature selection on one monolith package also
cannot express the two properties variants actually need: a
*negative* guarantee (a client-only build must not contain the
server stack, not merely not call it) and a *conformance* guarantee
(the same journal produces the same state in every variant that
carries the world).

## 2. Decisions in one paragraph

There are exactly four levels — **L0 embedded/provider**,
**L1 client-only**, **L2 headless service**, **L3 full node** (§3,
with the capability matrix that defines them). A variant is a
**composition crate**: a thin final crate that depends on seam
crates with an explicit feature set and contains no capability code
of its own (§4). Cargo features remain the mechanism *inside* a
seam crate for optional dependencies; they are never the variant
definition, and no variant is expressed as `cfg` inside the root
package's remains. Conformance is per variant (§6): the shell and
receipt contracts each level must honour, checked by replaying the
determinism fixtures as cross-variant oracles. Upgrade behaviour
per variant follows #1066 with an explicit degrade contract (§7).

## 3. The level set and capability matrix

Levels are cumulative in capability but not in linkage: each level
is its own composition, not a superset binary with flags.

| Capability | L0 embedded/provider | L1 client-only | L2 headless service | L3 full node |
|---|---|---|---|---|
| Journal + event store (core) | ✓ | ✓ | ✓ | ✓ |
| World state / replay | ✓ | ✓ | ✓ | ✓ |
| Sync (peer) | provider link only | ✓ | ✓ | ✓ |
| Sandbox / program execution (wasmtime) | — | optional module | ✓ | ✓ |
| Providers (host services) | ✓ (its purpose) | — | ✓ | ✓ |
| Serve (accept peers/clients) | — | — | ✓ | ✓ |
| Presenters: terminal/SSH | — | ✓ | — | ✓ |
| Presenters: web | — | optional module | — | ✓ |
| DHT address lookup | — | ✓ | ✓ | ✓ |
| Currency-PoC harness binding | — | — | — | — |

Rules the matrix is read under:

- **L0 is a library, not a binary.** It is the embeddable core a
  provider-class consumer (ESP32 line, Bevy incorporation) links.
  It never serves, never presents, and executes no sandboxed
  programs; its sync is the single provider link it exists to
  offer.
- **L1 must not link the server stack.** No serve path, no provider
  host, no wasmtime unless the optional sandbox module is selected.
  This is a `cargo tree` property of the composition crate,
  testable in CI — not a code-review promise.
- **L2 is L3 minus presenters.** "Headless service" is the server
  for machines nobody sits at: full serve/sync/sandbox/provider
  set, no terminal/SSH/web presenter code linked.
- **L3 is the full set** — the current `id` binary's capability
  profile, re-expressed as a composition.
- The currency-PoC binding is a harness artefact, not a capability
  of any level; it stays off-by-default on the seam crate that owns
  it and appears in no composition.

## 4. Expression: composition crates, features inside seams

- One composition crate per level (`id-embedded`, `id-client`,
  `id-headless`, `id-node` as working names; final names land with
  the seam crates they compose). A composition crate holds: its
  manifest (seam-crate dependencies with explicit
  `default-features = false` plus the level's feature set), its
  binary or library entry point, and its conformance test target.
  Nothing else.
- Features inside a seam crate are for optional *dependencies*
  (the existing pattern: `sandbox` pulls wasmtime, `web` pulls the
  HTTP stack, `dht` pulls the lookup client). A seam feature must
  never gate a *product* decision; if a capability appears in the
  §3 matrix, its in/out status is decided by composition, not by a
  feature flag a consumer can flip.
- The root `id` package's feature set is frozen as the legacy L3
  profile. New capabilities land in seam crates; the root package
  shrinks as seams extract (#3447 ordering) and is not where new
  variant behaviour is added.
- A variant may not reach sideways: L1 selecting the web presenter
  module composes the presenter seam crate; it does not enable a
  flag that drags the serve stack in behind it. If a presenter's
  seam crate depends on serve internals, that is a seam defect to
  fix in #3447's line, not a reason to widen L1.

## 5. Size and performance budgets

#3447's order applies per variant: shipped-binary runtime
performance first, build time second.

- **Linkage budgets are the size budget.** The enforceable form of
  "small" at this design level is the negative guarantee of §3:
  L0 links no presenter, serve, or sandbox code; L1 links no serve
  or provider-host code. The ≤2 MiB-class discipline from the fleet
  work is the reference point for L0's eventual binary-size gate;
  a numeric per-level size gate is set when the first composition
  crate exists to measure, and is recorded in the #5685 benchmark
  matrix rather than guessed here.
- Every level joins the #5685 per-interface benchmark matrix as a
  row set: binary size, cold start, replay throughput on the
  shared fixtures, idle memory. A level that regresses runtime
  performance against the matrix does not ship by widening its
  composition to borrow another level's faster path — the seam is
  fixed instead.
- Build time is measured per composition crate; a level whose
  build pulls another level's dependency tree has failed §4, which
  is caught by the `cargo tree` assertions before build-time
  numbers are even interesting.

## 6. Conformance

- **Replay oracle (all levels carrying world state — L0–L3).**
  The determinism replay fixtures are the cross-variant oracle:
  the same journal replayed by every level must produce the same
  state hash. This runs per composition crate in CI; a divergence
  is a stop-ship defect in the seam crate, never papered over in a
  composition.
- **Shell contract (L1, L3; L2 where it exposes a control
  surface).** Levels with presenters honour the #788 shell
  contract identically for the presenters they carry; a level is
  not "mostly conformant" — the contract tests for its carried
  presenters either pass or the level does not ship that
  presenter.
- **Receipt contract (L0–L3).** Every level issues and verifies
  the same receipt shapes for the operations it performs. L0's
  provider receipts and L3's serving receipts are the same
  contract at different levels, so a client of any level can
  verify any other level's receipts.
- **Composition checks (CI, per level).** `cargo tree` assertions
  for the §3 negative guarantees, plus a feature-powerset spot
  check on each seam crate a level composes, so a seam feature
  change cannot silently widen a level.

## 7. Upgrade and migration interplay (#1066)

- A level's program-version support is part of its composition
  record: the composition crate states the program versions it can
  execute (L0/L2/L3 with sandbox) or carry (L1).
- **Client-only meeting a newer program:** L1 never partially
  executes a program newer than it carries. It degrades to
  read-only rendering of the world's journal state and issues an
  explicit unsupported-version receipt (§6), so the user and the
  server both see the same fact. Silent skip and best-effort
  execution are both prohibited.
- **Embedded pinning:** L0 pins program versions at composition
  time; upgrading an embedded deployment is a new composition, not
  a runtime fetch.
- **Negotiation:** sync handshake exchanges supported program
  versions before journal exchange, so a version gap is known
  before state moves, per #1066's semantics. L2/L3 serving a
  world to an older client serve the journal and the receipts;
  execution stays with the side that carries the version.

## 8. The issue's open questions, answered

- **Release artefacts or build-time choices?** Both, at different
  levels. Composition crates are the build-time choice every
  embedder uses. Published release binaries are produced for L1
  and L3 (the levels with end users) once their compositions
  exist; L0 is library-only and L2 is built by its operators until
  a named consumer needs a published artefact.
- **Does the server ever lose features to a level?** No "lite
  server" level exists. L2 is the answer for a server without a
  human surface; below that, a deployment composes L0/L1. L3 is
  always the full set — a server binary that quietly lacks a
  capability is a support trap, and §3 gives it no matrix row.
- **Who owns variant drift?** The matrix in §3 is the registry. A
  fifth level, or a capability move between levels, is a change to
  this record (amendment PR) plus the §6 conformance checks —
  never a consumer-local feature set. Consumers select levels;
  they do not mint them. Drift is detectable mechanically: the
  composition crates' manifests either match the matrix or CI's
  `cargo tree` assertions fail.

## 9. Ordering and next actions

This record settles the design against the *planned* seams; where
a seam crate has not landed yet, the matrix names the capability,
and the composition crates are created as their seams land, in
#3447's priority order. Implementation slices, each its own issue
when its seams exist: (a) L3 composition re-expressing the current
binary (first, proves the mechanism against a known profile);
(b) L1 composition with the `cargo tree` negative-guarantee tests;
(c) replay-oracle conformance wiring per §6; (d) L2 composition;
(e) L0 composition when the ESP32/provider consumer is ready to
link it. No slice is filed by this record; each is filed when its
seam prerequisites close, so the queue never holds unworkable
implementation tasks against open seams.

## 10. Alternatives rejected

- **One binary, runtime flags.** Fails the negative guarantee —
  the server stack is linked and reachable — and makes
  conformance a property of invocation, not of the artefact.
- **Feature-only, on the root package (status quo grown up).**
  This is what exists today; it cannot express L1's linkage
  guarantee, and every consumer's feature list becomes a private
  variant (§8, drift).
- **`cfg` thickets in the monolith's remains.** Explicitly ruled
  out by the issue's sketch and by §4: a variant is a composition
  of crates, never a thicket of `cfg`.
- **Per-consumer variants (Android build, ESP32 build, …).**
  Consumers are why levels exist; naming levels after consumers
  guarantees a new "variant" per consumer and no shared
  conformance story.
