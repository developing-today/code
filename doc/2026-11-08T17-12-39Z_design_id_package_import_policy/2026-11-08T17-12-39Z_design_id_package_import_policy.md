# Design: pure-package imports and package classes across capability tiers

- **Date:** 2026-11-08
- **Status:** proposed; the capability-tier vocabulary below follows the open #5958 design PR #7599 and must be revisited if that proposal changes.
- **Issue:** [#5994](https://github.com/developing-today/code/issues/5994), under the Roc library-import design [#5956](https://github.com/developing-today/code/issues/5956) and worlds program [#5955](https://github.com/developing-today/code/issues/5955).
- **Scope:** package classes, their declared platform requirements, a class-by-tier policy matrix, and a fixture specification. This record does not change the compiler, provider catalog, runtime grants, or Roc package syntax.

## Decision summary

1. **Classify dependencies by their transitive platform requirements, not by package name or a self-asserted `pure` label.** A package with an empty requirement set is pure for this policy. A deterministic PRNG algorithm with explicit state can be pure; a call to the current `random.u64` host request is effectful and standard-platform-dependent.
2. **A platform-dependent package must declare the platform surface it requires.** Its requirement set is distinct from the world author's requested ceiling and from the analyzer's `discovered_used` result. The exact metadata encoding and whether a package binds to the importing world's platform remain owned by #5956.
3. **The selected platform must satisfy the transitive package requirement set.** A platform module being present in the server catalog is not, by itself, a runtime grant. The import gate, the server/world ceiling, and per-world grants remain separate checks.
4. **Use #5958's proposed vocabulary without creating another tier scheme:** `standard`, `sandbox.fs`, `host.fs.read`, and `net.out`. As proposed in PR #7599, extension tiers are opt-in and off by default; the shipped isolated default does not change.
5. **Pure dependencies are allowed on every compatible profile.** They have no platform effects to reject. Therefore the fixture requirement is interpreted as a positive pure-package case plus compile-fail cases for effectful classes on profiles that lack their requirements. A compile-fail test for a genuinely pure package would contradict the policy.
6. **Use Roc's platform boundary as the premise:** the standard library supplies data structures and functions, not ambient I/O; platform APIs supply I/O, and a package that uses one must be compatible with the application's selected platform.

## 1. Current shape and boundary

`pkgs/id/crates/id-core/src/world_caps.rs` defines the current provider catalog. It distinguishes host requests, subscriptions, and Roc platform modules; the six current platform modules are `Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, and `RocText`. The catalog currently has no filesystem or network provider. Its `DefaultCeiling` field records whether a provider joins a default ceiling; it does not classify packages or implement a package-class policy. In particular, `loc.position` and `loc.watch` are catalogued but excluded from default ceilings.

The shipped Roc compile gate checks platform-module imports against the world's available platform surface. The example at `pkgs/id/examples/roc-cap-demo/` demonstrates a library using `RocCap`/`records.read`, a successful platform, a reduced platform that must reject the import, and a wrong-signature negative test. `roc-cap-demo-test.sh` pins the reason for each failure, and `pkgs/id/scripts/roc-test.sh` runs it as part of the `roc-test` check. That is a mechanism example, not a fixture for every package class.

PR #7599 proposes a separate model for world-requested tiers/capabilities, statically discovered use, and effective grants. This policy consumes that vocabulary; it does not redefine those sets. Its proposed `sandbox.fs`, `host.fs.read`, and `net.out` surfaces are not present in today's provider catalog, so their negative fixtures are specifications until those surfaces exist.

## 2. Terms and policy invariant

- **Package requirement set:** the platform modules and capability names a package's transitive dependency closure needs in order to typecheck and run. The package author declares the requirement; the package is not trusted merely because it calls itself pure. A package can require more than one capability or tier.
- **World requested set:** the tiers/capabilities requested by the world author. This is a ceiling request, not permission.
- **Discovered-used set:** the result of the static analysis proposed by #5958/#5997. It is derived from reachable code and emittable capability requests; it is not hand-written package metadata.
- **Effective grant:** what the server permits for a particular world instance after intersecting its grants, ceilings, and available catalog. This remains the runtime authority.
- **Capability profile:** the baseline `standard`, optionally combined with one or more opt-in tiers. The table below shows each single-tier minimum profile; combinations are the union of their requirements, not a new package class.

For a package to import successfully, all of these must hold:

```text
package requirements ⊆ selected platform surface
transitive package requirements ⊆ world requested set
discovered-used ⊆ world requested set
```

The first two govern whether the package/world can be compiled against the selected platform. The last is the #5958 manifest invariant. At runtime, existing grant and ceiling checks still decide whether an effect is allowed. Catalog presence or compile success must never be presented as user consent.

## 3. Package classes and tier policy

Classes are derived from requirements, not asserted by package names. `standard-dependent` means the package needs a provider or platform module in the current standard surface. `extension-dependent` packages are split below by the opt-in tier they need; a package requiring multiple extensions must declare their union.

| Package class                                                                                                                | Package author declares                                                                                         | `standard` only                                         | `standard` + `sandbox.fs`                                                                                                       | `standard` + `host.fs.read`                                                    | `standard` + `net.out`                                                        |
| ---------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| **Pure** — no platform requirements (for example, a parser, Unicode processing, or a deterministic PRNG with explicit state) | Empty requirement set; no `pure` assertion substitutes for analysis                                             | Allow                                                   | Allow                                                                                                                           | Allow                                                                          | Allow                                                                         |
| **Standard-dependent** — uses a current standard provider/module (for example, `RocCap` with `records.read`)                 | Exact standard provider/module requirements; do not declare a blanket capability wildcard                       | Allow when each requirement is on the selected platform | Same standard check; extension does not remove it                                                                               | Same standard check                                                            | Same standard check                                                           |
| **Scratch-filesystem-dependent**                                                                                             | `sandbox.fs` and the required `fs.read`/`fs.write` operations                                                   | Reject                                                  | Allow only when the platform exposes the declared operations and the host broker supports the requested scratch-directory scope | Reject unless the selected profile also includes `sandbox.fs`                  | Reject unless the selected profile also includes `sandbox.fs`                 |
| **Host-read-dependent**                                                                                                      | `host.fs.read` and `fs.host_read`, with the required admin-named roots represented by the platform requirement  | Reject                                                  | Reject unless the selected profile also includes `host.fs.read`                                                                 | Allow only when the server enables the tier and the named roots are authorized | Reject unless the selected profile also includes `host.fs.read`               |
| **Outbound-network-dependent**                                                                                               | `net.out` and `net.connect`, with the required admin-named destinations represented by the platform requirement | Reject                                                  | Reject unless the selected profile also includes `net.out`                                                                      | Reject unless the selected profile also includes `net.out`                     | Allow only when the server enables the tier and the destination is authorized |

Every `Allow` is conditional on the selected platform actually exposing the package's full transitive requirements and the world request covering them. These columns show minimum profiles: an extension-dependent package may run on a profile combining its required tier with other tiers. `standard` includes the existing standard surface; it does not mean every catalogued operation is automatically granted. For example, the current `loc.*` entries are excluded from default ceilings and remain subject to their existing consent and grant rules.

**Randomness clarification:** the issue's “random” example is pure only for an algorithm whose seed/state is explicit input. The current `random.u64` entry in `world_caps.rs` is a host request, so a package that uses it belongs in the standard-dependent class. This makes the class depend on effects rather than a library's topic or name.

## 4. Compile-fixture specification

The fixtures should pin policy cases, not just assert that a compiler exits nonzero. Each negative case must assert the missing module/capability or the policy diagnostic that caused rejection. A failure for a syntax error or unrelated type error is not a passing policy test.

| Class/case                  | Package and platform pair                                                                                                                              | Expected oracle                                                                            | Status / wiring                                                                                                                                                  |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pure positive control       | A small pure parser/Unicode package (and a deterministic PRNG with explicit seed if randomness is covered), imported by an app on the standard profile | `roc check` succeeds; the package declares no platform requirements                        | Add to the policy fixture set and run from `pkgs/id/examples/roc-import-policy/roc-import-policy-test.sh`                                                        |
| Standard-dependent negative | A `RocCap`/`records.read` package against the reduced platform without `RocCap`                                                                        | `roc check` fails and the output names missing `pf.RocCap`                                 | Mechanism already demonstrated and wired by `pkgs/id/examples/roc-cap-demo/roc-cap-demo-test.sh`; retain it as baseline, do not count it as proof of every class |
| Scratch filesystem negative | A package requiring `fs.read` or `fs.write` against standard-only platform                                                                             | Reject specifically because the required tier/provider is absent                           | Specify now; make executable when the provider/platform surface exists                                                                                           |
| Host read negative          | A package requiring `fs.host_read` against standard-only platform                                                                                      | Reject specifically because `host.fs.read` is absent; do not accept an unbounded host path | Specify now; make executable when the provider/platform surface exists                                                                                           |
| Outbound network negative   | A package requiring `net.connect` against standard-only platform                                                                                       | Reject specifically because `net.out` is absent; do not accept an unauthorized destination | Specify now; make executable when the provider/platform surface exists                                                                                           |

The literal phrase “one compile-fail fixture per package class” cannot apply to the pure class: a pure dependency must compile on a platform that satisfies Roc's base package surface. The test contract should therefore be **one positive pure fixture, plus a compile-fail fixture for each effectful class/profile mismatch**. If a future checker rejects a truly pure dependency on an otherwise compatible platform, that is a policy regression, not a passing negative fixture.

When the extension providers exist, put the fixture runner in the same normal path as the existing demo: the example-level shell script checks positive and negative cases, asserts expected diagnostics, and is invoked by `pkgs/id/scripts/roc-test.sh`, which is part of the `roc-test` flake check. Until then, the extension rows above are specifications; they must not be described as runnable or as enforced by the current `world_caps.rs` catalog.

## 5. Limits and open questions

1. **Requirement encoding and resolution:** #5956 owns whether a package pins the importing world's platform or declares a platform requirement checked as a subset. This design needs a requirement set at the package boundary but intentionally does not invent Roc header syntax or settle that open question.
2. **Tier vocabulary:** PR #7599 is open. Preserve its four names and its unchanged isolated default posture, but treat all tier-specific rows as provisional until #5958 lands.
3. **Extension I/O path:** no file/network provider exists in the current catalog. Any implementation must preserve the existing authorization, broker, and replay boundary; this document does not authorize direct ambient filesystem or socket I/O from Roc packages. The review of #7599 calls out this boundary for the capability design to settle.
4. **Static discovery:** packages that construct capability requests dynamically must follow #5958's fail-closed certification rule; a package declaration is not proof that its complete used set was discovered.
5. **Class overlap:** a package can require both standard and extension capabilities. Its declared requirement set is the union; do not force it into a single exclusive label or let a lower class erase a higher requirement.

## 6. Follow-on implementation shape

After #5956 settles platform-requirement representation and #5958 settles the tier model, implementation can add the package requirement record and subset check, then the fixture matrix above. The existing catalog and runtime grants remain authoritative for their existing jobs; adding a taxonomy must not silently change provider defaults, runtime permissions, or the `RocCap` broker. The compile-fail assertions should land with the checker behavior they pin, and the policy doc should be updated only if the final tier design changes these decisions.

## Sources

- [#5994 — pure-package import policy and compile-fail fixtures](https://github.com/developing-today/code/issues/5994)
- [#5956 — Roc library imports for worlds](https://github.com/developing-today/code/issues/5956)
- [#5958 — capability model](https://github.com/developing-today/code/issues/5958); [open design PR #7599](https://github.com/developing-today/code/pull/7599)
- [#5957 — capability-dependent demo](https://github.com/developing-today/code/issues/5957); [merged PR #6237](https://github.com/developing-today/code/pull/6237)
- Current implementation: [`world_caps.rs`](../../pkgs/id/crates/id-core/src/world_caps.rs), [`roc-cap-demo`](../../pkgs/id/examples/roc-cap-demo/README.md), [`roc-test.sh`](../../pkgs/id/scripts/roc-test.sh)
- Roc language reference: [Platforms](https://roc-lang.org/docs/main/langref/platforms/) (platforms provide I/O; the standard library does not); [Functions](https://www.roc-lang.org/docs/main/langref/functions/) (pure/effectful function distinction and effects originating at the platform).
