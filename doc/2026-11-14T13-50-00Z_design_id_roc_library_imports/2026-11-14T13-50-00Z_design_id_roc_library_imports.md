# Design: Roc library imports for worlds — platform-requirement representation, subset checking, provenance, and import errors

- **Date:** 2026-11-14
- **Status:** proposed; every use of the capability-tier vocabulary below is provisional on the open capability-model design (#5958, PR #7599) and must be revisited if that proposal changes.
- **Issue:** [#5956](https://github.com/developing-today/code/issues/5956), under the worlds design program [#5955](https://github.com/developing-today/code/issues/5955).
- **Scope:** the platform-dependent half of Roc library imports for worlds — how a library declares the platform functions it uses, how that declaration is checked against the compiling world's platform at import time, where imported libraries come from, and what happens when an import's needs exceed the platform. This record settles the two questions the landed pure-package policy (#5994) explicitly left to this issue: the requirement metadata encoding, and whether a package binds to the importing world's platform. It does not change the compiler, the provider catalog, runtime grants, or Roc package syntax.

## Decision summary

1. **A library declares a platform-requirement set in its manifest, by catalog name.** The declaration lives in the `platform` block of the library manifest defined by #5969 (a KDL profile, per #5753). Each entry names one catalog entry — a platform module (`RocCap`) or a capability (`records.read`, `time.now`) — optionally pinned to the catalog version of that entry. An absent or empty `platform` block is a pure library. There are no wildcards and no self-asserted `pure` label: purity is the empty set, exactly as #5994 decided.
2. **A library does not bind to a platform — neither its own nor the importing world's.** It declares requirements; at import time the checker verifies the library's transitive requirement set is a subset of the importing world's selected platform surface. This answers the issue's open question in the subset direction #5994's checker column already pointed to, and it is what the shipped mechanism demonstrates (the `roc-cap-demo` library compiles against a full platform and is rejected by a reduced one).
3. **Import checking is three subset checks at compile time, and nothing at runtime changes.** Transitive requirements ⊆ selected platform surface; transitive requirements ⊆ the world's requested set; discovered-used ⊆ the world's requested set (the #5958 manifest invariant, consumed here, not redefined). Runtime grants, ceilings, and consent seams still decide effects; a successful import is never a grant.
4. **Provenance is recorded for every import, and identity is always a hash.** Registry imports resolve to a manifest hash; uploads enter through the registry per #5969; URL imports follow #5969 §6 — identity is (URL, pinned content hash, version), bytes are fetched and verified against the pin before any checking, and there is no "track latest" mode.
5. **An unmet import is a compile error that names the excess.** The diagnostic names the library (name + manifest hash), each requirement that failed, which check it failed, and the remedy. It is never a runtime surprise, a silent downgrade, or a bare "incompatible platform".

## 1. Requirement representation (the encoding #5994 §5.1 left open)

The library manifest is the one #5969 §2 defines. This design owns the semantics of its `platform` block — the seam #5969 explicitly left ("requirements, semantics owned by the capability-model child"):

```kdl
library "greeter" version="1.0.0" {
    kind "source-library"
    build-mode "compile-in"
    hash "blake3:…"
    license "…"
    platform {
        requires "RocCap"                  // a platform module, by catalog name
        requires "records.read"            // a capability the library issues through it
        requires "time.now" version=1      // optionally pinned to the catalog version
    }
}
```

Rules:

- **Names are catalog names.** A `requires` value must be the `name` of an entry in the provider catalog — `pkgs/id/crates/id-caps/src/world_caps.rs` (`PRODUCTION_CATALOG_ENTRIES` / `CATALOG`; each `ProviderManifest` carries `name`, `version`, and `kind`). Platform-module requirements use the module names the compile gate already checks (`Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, `RocText`); capability requirements use the capability names the runtime catalog uses (`records.read`, `chat.say`, `time.now`, `random.u64`, …). One namespace, no parallel vocabulary. (Citation fix: this issue's body and the #5994 record cite the catalog at `id-core/src/world_caps.rs`; since step 1 of the crate split (#4403) it lives in the `id-caps` crate — `id-core/src/lib.rs` documents the move. Follow-on work updates those citations; the names did not change in the move.)
- **Exact names, no wildcards.** A `*` requirement, a prefix requirement, or a tier-only declaration in place of names is invalid. A library may require many entries; its set is the union it declares, never a single class label (per #5994 §5.5, class overlap is normal).
- **Version pins are optional and exact.** `version=` pins the `ProviderManifest.version` the library was written against. An unpinned requirement accepts any catalog version of that name; a pinned one fails if the compiling catalog carries a different version, with the mismatch named (§5). Pins exist so a catalog contract change cannot silently reinterpret an existing library.
- **Unknown names fail closed.** A `requires` naming no catalog entry is refused at registry/upload time and at import time, the same posture as `WorldManifest::parse` refusing fields that name an unknown capability. This has a deliberate consequence for the extension tiers: `sandbox.fs`, `host.fs.read`, and `net.out` have no providers in today's catalog, so a requirement on `fs.read`, `fs.host_read`, or `net.connect` is currently unresolvable and fails as an unknown requirement. Those rows of #5994's fixture matrix stay specifications until the providers land; this design does not invent names for surfaces that do not exist.
- **The declaration is checked, not trusted.** The checker derives the library's discovered-used set from its source and platform binding (today's Roc mechanism: a package binds to a platform — the demo library's `lib/main.roc` binds to `../roc-world/platform.roc` — and the shipped compile gate checks platform-module imports against the platform's surface). The declared set must cover the discovered set for the library's own code: `discovered-used ⊆ declared`. An undeclared use is a declaration mismatch error (§5), not a quiet widening. This is #5958's fail-closed certification rule applied at the package boundary.
- **Tier and class labels are derived, never declared.** The package class and tier profile #5994's matrix uses (pure / standard-dependent / scratch-filesystem-dependent / …) are computed from the requirement set by the checker and the registry display. An author who declares a lower class than their requirements imply has declared nothing — the set is the record.

A library's **requirement set** in everything below is its transitive set: its own declared set union the declared sets of its dependencies, recursively, resolved by manifest hash (§4). Cycles in the dependency graph are refused at resolution time, naming the cycle.

## 2. Resolution target: requirements checked against the importing world (the binding question)

Two shapes were open: platform-dependent libraries resolve against the world's platform, or they carry their own platform requirement that must be a subset of the world's. This design takes the second as the mechanism and the first as its effect — stated precisely:

- A library carries **a requirement set, not a platform**. It never names a platform implementation, a platform package, or a platform version as its resolution target.
- At import time the requirement set is checked against **the importing world's selected platform surface**: the set of catalog entries that world's platform exposes (its platform modules plus the providers reachable through them).
- A library that genuinely needs one specific surface shape expresses it with exact names and version pins; name + version is the identity the catalog itself uses, so no separate platform identity is needed.

Why not bind the library to a platform: a bound library is usable by exactly the worlds on that platform, so every platform variant multiplies library variants; the demo (#5957, PR #6237) already shows the subset shape working — one library, accepted by the full platform, rejected by the reduced platform with the missing module named. Binding would also contradict #5994 decision 2's invariant form, which is stated as set inclusion, not platform identity.

## 3. Subset checking at import time

Checking happens at three moments, with distinct jobs; conflating them is how catalog presence gets mistaken for consent.

**At registry / upload time** (per #5969): every declared name exists in the catalog (and any version pin matches a published catalog version); the manifest is internally consistent (kind/build-mode agreement, hash present). No world is involved yet; nothing here says who may import the library.

**At import / compile time** — the import gate, extending the shipped platform-module compile gate. All three must hold, and each failure is its own diagnostic (§5):

```text
transitive requirements ⊆ selected platform surface
transitive requirements ⊆ world requested set
discovered-used (whole import closure) ⊆ world requested set
```

- The first check is satisfiability: can this platform serve this library at all?
- The second is the world author's ceiling request applied to what is being brought in: a world that did not request a tier does not acquire it transitively through a library. This is the check that keeps "worlds may import whatever dependencies they want" (this issue's open scope question) honest — the answer this design gives is: **any dependency whose transitive requirements the world's selected platform and requested set cover**; there is no separate allowlist of libraries.
- The third is #5958's manifest invariant over the closure, consumed unchanged.
- Sensor capabilities keep their existing rules through all of this: a requirement naming a `loc.*` capability can be satisfied at compile time and still be refused per use at runtime; the import gate never mints the explicit ceiling entry or grant `world_caps.rs` requires for them.

**At runtime:** unchanged. Grants, ceilings, consent seams, and the refusal codes in `world_caps.rs` (`cap_denied`, `outside_ceiling`, …) decide effects. Compile success is evidence the import was satisfiable, and is never presented — in UI, logs, or records — as user consent or a runtime grant.

The checker reads the catalog from `id-caps` at the version the compiling world is built against; the checked-against surface (platform identity + the catalog version) is recorded in the resolution record (§4) so a verdict can be re-derived later.

## 4. Import resolution and provenance

An import names a library in one of the three ways #5969's taxonomy provides, and every resolution produces a **resolution record**: importing world/app, library name + manifest hash, source kind, the library's transitive requirement set as resolved, the platform surface checked against, and the outcome (accepted, or refused with the §5 diagnostic). The record is what makes a build reproducible and a refusal auditable.

- **Registry:** the import names a library by name + version (or subtype scope, per #5969 §4); resolution pins the manifest hash it resolved to. Re-resolution that would change the hash is a new build event, never a silent drift.
- **Upload:** an uploaded library becomes a registry record first (kind, manifest, hash per #5969); importing it is then the registry case. There is no direct-from-upload import path that skips the record.
- **URL:** identity is the triple (URL, pinned content hash, version) per #5969 §6. The importer fetches the URL and verifies the bytes against the pin **before** requirement checking, subtype checking, or build. A mismatch means the fetched bytes are not the named library: the import fails as that library, full stop. Until a URL library's bytes are fetched and verified, its declared requirements participate as _declared-not-checked_ and are displayed that way (the same declared/checked distinction #5969 §4 sets for subtype conformance); a world whose policy requires checked requirements refuses declared-only imports, and that refusal names the state, not a capability.
- **Interaction with typing:** the fetched/registered manifest is the one #5969 defines; this design adds no second manifest. Subtype conformance, attachment eligibility, and build mode are read from the same manifest the requirement set is read from, so one hash settles every "which library was this" question.

## 5. Failure mode and error quality

An import that fails any §3 check is a **compile-time refusal of the import** — the world does not build with a degraded library, a stubbed capability, or a warning that scrolls past. Following the demo's precedent (its negative test pins that the failure names the missing `pf.RocCap`, not merely that compilation failed), every import diagnostic carries:

- the library: name + manifest hash;
- the diagnostic code: `import_requirement_unmet`, with a subreason per check — `not_on_platform` (requirement absent from the selected platform surface), `not_requested` (absent from the world's requested set), `unknown_requirement` (names no catalog entry), `version_mismatch` (pin vs compiling catalog), `declaration_mismatch` (discovered-used ⊄ declared), `unverified_provenance` (URL bytes not yet fetched/verified, or hash mismatch);
- the **excess set, exhaustively**: every requirement that failed that check, by name — never just the first, never a count;
- what it was checked against: the selected platform surface's identity and the world's requested set;
- the remedy, in the same terms: choose a platform exposing the named entries, extend the world's requested set (naming the tier, where the missing entries belong to one — tier names provisional per #5958), declare the missing uses in the library's manifest, or pin a different library version.

```
import refused: library "greeter" (manifest blake3:9f2…) into world "chess-room"
  import_requirement_unmet / not_on_platform:
    RocCap            — not exposed by the selected platform (roc-world/platform-no-caps)
    records.read      — not exposed by the selected platform (roc-world/platform-no-caps)
  remedy: select a platform exposing RocCap + records.read, or use a library
          version whose requirements the selected platform covers.
```

The negative-test contract follows #5994 §4: each fixture pins the diagnostic — the code, the subreason, and the exact excess set — as its oracle; a failure for an unrelated reason (syntax, ordinary type error) is not a passing policy test. The demo task's negative tests are the first two fixtures of this contract (missing-module and wrong-signature), and #5994's matrix supplies the rest as the extension providers land.

## 6. What this design deliberately does not do

- **No new Roc syntax.** The requirement encoding is manifest metadata (KDL, per #5753 and #5969), not a Roc header invention; the Roc-side platform binding stays the mechanism the compiler sees, and §1's agreement check keeps the two honest.
- **No re-decision of landed siblings.** #5994's class × tier policy table and fixture contract stand; #5969's kinds, manifest, attachment, subtype, and URL-identity rules stand. Where this record and #5994's §6 follow-on shape disagreed in emphasis, the resolution is: the requirement record and subset check are built exactly as §1/§3 here specify, then #5994's fixture matrix lands against them.
- **No runtime change.** Provider defaults, the `RocCap` broker, grant and ceiling behavior, and the sensor-cap rules are untouched; adding this taxonomy must not silently widen any world's authority.
- **No tier invention.** Tier names appear only as derived labels and remedies, marked provisional on #5958; if #5958 lands different vocabulary, the labels change and the name-level requirement sets — the actual record — do not.

## 7. Follow-on implementation shape

1. Manifest `platform`-block parser + validator against the `id-caps` catalog (unknown name, duplicate, and version-pin checks), wired into the registry/upload path #5969 defines.
2. Transitive requirement resolver over registry manifests (by manifest hash, with cycle refusal) producing the resolution record of §4.
3. The import gate in the compile path: the three §3 subset checks with §5 diagnostics, generalizing the shipped platform-module gate and the `roc-cap-demo` negative tests.
4. #5994 §4's fixture matrix wired through `pkgs/id/examples/roc-import-policy/` and `pkgs/id/scripts/roc-test.sh`, each negative case asserting its diagnostic.
5. Citation updates: this issue's body and the #5994 record point at `id-caps` for `world_caps.rs`.

Acceptance mapping for this issue's "done looks like": import resolution — §4; platform-requirement representation for libraries — §1; subset checking against the platform catalog — §2/§3; error behavior — §5. A reader implementing the demo task needs no further design decisions from this record.

## Sources

- [#5956 — Roc library imports for worlds](https://github.com/developing-today/code/issues/5956) (this issue); program [#5955](https://github.com/developing-today/code/issues/5955)
- [#5994 — pure-package import policy](https://github.com/developing-today/code/issues/5994), record: [`doc/2026-11-08T17-12-39Z_design_id_package_import_policy/`](../2026-11-08T17-12-39Z_design_id_package_import_policy/2026-11-08T17-12-39Z_design_id_package_import_policy.md) — its §5.1 and §6 name the two questions §1/§2 here settle
- [#5969 — library typing & upload taxonomy](https://github.com/developing-today/code/issues/5969), record: [`doc/2026-11-08_design_id5969.md`](../2026-11-08_design_id5969.md) — manifest, kinds, URL identity (§6 there)
- [#5958 — capability model](https://github.com/developing-today/code/issues/5958); [open design PR #7599](https://github.com/developing-today/code/pull/7599) — tier vocabulary and the manifest invariant, consumed provisionally
- [#5957 — capability-dependent demo](https://github.com/developing-today/code/issues/5957); [merged PR #6237](https://github.com/developing-today/code/pull/6237) — `pkgs/id/examples/roc-cap-demo/`
- Current implementation: [`world_caps.rs`](../../pkgs/id/crates/id-caps/src/world_caps.rs) (catalog, refusal codes, sensor-cap rules), [`world_manifest.rs`](../../pkgs/id/crates/id-caps/src/world_manifest.rs) (fail-closed manifest parsing precedent), [`roc-test.sh`](../../pkgs/id/scripts/roc-test.sh)
