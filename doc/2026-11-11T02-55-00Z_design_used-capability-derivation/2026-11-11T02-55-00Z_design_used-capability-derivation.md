# Used-capability derivation tool — spec

Issue: #5998 (design). Parent topic: #5958 (capability model);
program: #5955 (worlds). Triage of record: the 2026-11-07 triage
note on #5998 (comment 6700990165), which verified every premise
below against main and fixed the one open rule (value-passed
platform functions) conservatively, pending #5963.

This document is the tool spec the issue's "done looks like" asks
for: inputs, algorithm, over-approximation rules, and output schema,
sufficient to implement without further design decisions. It
specifies the tool only — implementing it is a follow-on, and the
permission-UX surfacing of over-declaration is a sibling design
point under #5955, out of scope here.

## 1. Purpose

A world declares the capabilities it wants (its `Wants` document).
Nothing today derives the capabilities a world's code *actually
uses*, so declarations can only be checked by running the world and
watching the ledger — and only for the paths a run happens to take.
This tool derives the used-set statically: call-graph reachability
over the world's Roc source closure, against the platform-function
namespace the capability catalog defines. In Roc all effects are
named platform functions, so the scan is exact except for the named
over-approximation sources in §5 — unlike binary/API static
analysis (noisy, needs an API→capability mapping the platform does
not otherwise have) or type inference (exact, unavailable in Roc).

The output pairs with the declared set in the registry format #5958
defines (its requested-vs-discovered pairing): both sides are the
same set type over the same names, so over-declaration and
under-declaration are plain set differences (§6).

## 2. Definitions

- **Catalog.** The provider catalog in
  `pkgs/id/crates/id-core/src/world_caps.rs`: the const
  `PROVIDER_REGISTRY` (`ProviderRegistry` over `CATALOG_ENTRIES`),
  with `catalog_names() -> BTreeSet<String>` and `is_known(name)`.
  Catalog names are dotted platform-function names (`chat.say`,
  `records.read`, `schedule.at`, `loc.position`, …). The catalog is
  the *only* capability namespace the tool knows.
- **Platform function.** A Roc-callable function the platform
  provides, identified by exactly one catalog name.
- **Source closure.** The transitive set of Roc modules a world (or
  library) compiles from: its entry module plus everything it
  imports, recursively, within the package being scanned. Platform
  modules themselves are leaves — their *names* are catalog names,
  their bodies are not scanned.
- **Entry points.** The definitions the platform can invoke on the
  world: the world's exported effect handlers / main definitions,
  as declared by the world's platform interface. The scan is seeded
  exactly at the entry points; a scan request names them (§3).
- **Used-set.** The set of catalog names the derivation proves the
  world can reach a call to, under the rules of §5.
- **Certified.** A used-set is *certified* when the scan saw no
  construct that defeats static enumeration (§5 rule R4). Only a
  certified used-set supports the over-declaration difference
  (§6); an uncertified one is a sound lower bound plus the unknown
  marker, never evidence that a declaration is unused.

## 3. Inputs

The tool takes exactly three inputs:

1. **Source closure root** — a path to the world's (or library's)
   entry Roc module, plus the package roots needed to resolve its
   imports. The tool computes the closure itself (§4 step 1); an
   import it cannot resolve inside the package is an input error,
   never a silent skip (a partial closure would understate the
   used-set while looking exact).
2. **Entry points** — the list of entry-point definitions, by
   module-qualified name. Default: the platform interface's
   declared exports for the world kind being scanned, when the
   caller does not override. A scan with zero resolved entry points
   is an input error.
3. **Catalog snapshot** — the catalog as `BTreeSet<String>` from
   `PROVIDER_REGISTRY.catalog_names()` in the `id-core` build the
   tool is compiled against. The catalog is compiled in, not read
   from the scanned package: the platform the world will run
   against is the authority, and a platform function reference that
   resolves to no catalog name is an error naming the reference
   (it indicates version skew between the world and the platform,
   which must surface loudly, not degrade the scan).

No other configuration exists. In particular there is no
allowlist/denylist input: exceptions are expressed by the rules of
§5 and the `<unknown>` marker, not by per-world configuration.

## 4. Algorithm

1. **Close.** Parse the entry module; walk imports recursively to
   the full source closure. Collect every definition (function or
   value) in the closure, keyed by module-qualified name.
2. **Graph.** Build the call graph over closure definitions: an
   edge A → B for each direct call of B in A's body. Platform
   functions are sink nodes labelled by catalog name.
3. **Reach.** Mark definitions reachable from the entry points,
   following call edges to a fixpoint (cycles are expected;
   reachability is a visited-set computation, not a recursion).
   Value-flow edges from §5 rule R3 are added during this pass and
   participate in the same fixpoint.
4. **Collect.** The used-set is the set of catalog names labelling
   platform-function nodes that are (a) called from a reachable
   definition, or (b) taken as a value in a reachable definition
   (§5 rule R2). Each collected name is checked with `is_known`;
   an unknown name here is the §3 version-skew error.
5. **Certify.** If any reachable definition contains a §5 rule R4
   construct, add the unknown marker and mark the result
   uncertified. Certification is whole-scan: one dynamic site
   anywhere reachable uncertifies the scan, because the site can
   in principle reach any platform function.
6. **Emit** the output document of §6, deterministically: names
   sorted (the `BTreeSet` iteration order), no timestamps or
   environment data in the document, so identical inputs produce
   byte-identical output.

## 5. Over-approximation rules

The scan is exact for direct calls in a statically closed world.
Exactly three rules widen it, and one rule marks its limit. No
other widening is permitted: an implementation that approximates
anywhere else is not implementing this spec.

- **R1 — direct reference.** A platform function called (applied)
  in a reachable definition is used. This is the base rule and
  covers the overwhelming majority of real code.
- **R2 — platform function passed as a value.** A platform
  function that appears in a reachable definition in any
  non-applied position — passed as an argument, returned, stored
  in a structure, captured by a closure — counts as used,
  wholesale, whether or not the scan can prove the value is ever
  applied. Rationale (the triage's conservative ruling, pending
  #5963): once a platform function becomes a value, tracking every
  application site is a dataflow problem the programming model
  does not yet make trackable; counting it as used is the sound
  direction (it can only overstate, never understate). If #5963's
  model later makes value-passing trackable, this rule narrows —
  by amending this spec, not by implementation choice.
- **R3 — local function passed as a value.** A closure-local
  function taken as a value in reachable code is treated as
  reachable itself (a value-flow edge from the taking definition
  to it), because it may be applied anywhere the value flows.
  This is what makes R2 compose: a local helper that calls
  `records.read` and is passed as a callback contributes
  `records.read`. Platform functions are not re-analysed this way
  (they are sinks); only closure definitions get value-flow edges.
- **R4 — dynamic dispatch / runtime loading.** A reachable use of
  any mechanism that selects code or capabilities at run time —
  dynamic loading, runtime module resolution, string-named
  invocation, or a platform escape hatch of the same shape —
  defeats enumeration. The scan adds the literal name
  `<unknown>` (the existing `UNKNOWN_CAP_USAGE_KEY` in
  world_caps.rs, the same bucket the runtime ledger uses for
  unattributable use) to the used-set and marks the result
  uncertified (§2). It does **not** add the whole catalog: the
  used-set stays a truthful record of what was *seen*, and the
  certification flag, not set inflation, carries the uncertainty.

Dead code is not an approximation source in either direction:
platform functions referenced only from unreachable definitions
are **not** in the used-set. (An implementation may report them in
the optional diagnostic field of §6, but they never enter the
diffed set.)

## 6. Output schema

### 6.1 In-memory representation

```rust
pub struct UsedCaps {
    /// Catalog names, plus "<unknown>" iff `certified` is false
    /// and rule R4 fired.
    pub used: BTreeSet<String>,
    /// False iff a rule R4 construct was reachable.
    pub certified: bool,
}
```

Invariants, enforced by the constructor the implementation
provides (not by caller discipline):

- every element of `used` is either a catalog name
  (`is_known(name)`) or the literal `<unknown>`;
- `<unknown>` is present only when `certified` is false;
- `used.len() <= catalog_names().len() + 1`.

This is deliberately the same representation as the declared side:
`Wants::caps() -> BTreeSet<String>` and `CapLedger`'s `granted` /
`allowed` sets are `BTreeSet<String>` over the same catalog names,
and the runtime instrumentation path (`CapLedger.usage` keys, with
`<unknown>` for unattributable use) produces the same set type.
Static and runtime discovery are two producers of one type; #5958's
question "static, instrumentation, or both" is answered: both,
diffed the same way.

### 6.2 Serialized form

The registry record is a JSON document:

```json
{
  "v": 1,
  "subject": "worlds/<world-name>",
  "used": ["chat.say", "records.read"],
  "certified": true
}
```

- `v` — schema version, `1` (mirroring `WANTS_VERSION`).
- `subject` — the scanned package's registry name.
- `used` — the used-set as a sorted JSON array of strings (the
  `BTreeSet` serialization); `<unknown>` appears here exactly when
  `certified` is false and R4 fired.
- `certified` — the §2 certification flag.
- Optional diagnostic field `unreachable_refs` (sorted array of
  catalog names referenced only from unreachable code) may be
  emitted for tooling display; consumers MUST NOT diff against it,
  and the registry pairing below ignores it.

Duplicate keys in a serialized record are a parse error for any
consumer (the #6520 rule the codebase already applies to persisted
JSON: a document is validated as written, never as a last-wins
collapse).

### 6.3 Pairing and differences

Given `declared = wants.caps()` and a `UsedCaps` record for the
same world:

- **Under-declared** = `used − declared`. Always reportable, for
  certified and uncertified records alike: the scan only ever
  *proves* uses (R2/R3 overstate within what was seen; they never
  invent a name), so every element is a real use the declaration
  misses. (The `<unknown>` element, when present, is excluded from
  this difference and reported as its own line.)
- **Over-declared** = `declared − used`. Reportable **only** for
  certified records: an uncertified scan cannot prove a negative,
  and surfacing its difference as over-declaration would punish
  exactly the worlds the exception policy under #5955 marks
  uncertified. For uncertified records the registry shows the
  pairing with the over-declared side suppressed and the
  uncertified state named.

## 7. Errors

All errors name the offending artifact; none degrade silently.

| Condition | Result |
|---|---|
| Entry module unparseable | error naming the file |
| Import unresolvable inside the package | error naming the import (§3.1) |
| Zero entry points resolved | input error (§3.2) |
| Platform reference with no catalog name | error naming the reference (§3.3) |
| R4 construct reachable | **not an error** — `<unknown>` + uncertified (§5) |
| Platform reference only in dead code | not an error, not in the used-set (§5) |

## 8. Implementation shape (for the follow-on)

- A library function in `id-core` beside `world_caps.rs`
  (`derive_used_caps(closure, entry_points) -> Result<UsedCaps>`),
  with a thin CLI front-end in the `id` binary for registry
  production; the catalog and the unknown key are already in
  `id-core`, so no new crate is needed.
- The Roc front-end the implementation uses must expose, per
  definition: direct call targets, value-position references to
  named functions, and the R4 constructs. If the available Roc
  AST access cannot distinguish applied from value positions, R2
  cannot be implemented as specified and the implementation task
  must say so on #5998 rather than silently widening or narrowing
  the rule.
- Determinism is a hard requirement (§4 step 6): the registry
  diffs records across runs, and a nondeterministic record is
  indistinguishable from a capability change.

## 9. Test plan (oracle fixtures for the implementation)

Each fixture is a small Roc package with a hand-computed expected
record — the oracle is the fixture author's reading of §5, stated
in the fixture, never the tool's own output:

1. **Direct use, transitive:** entry → helper → `records.read`;
   expect exactly `{records.read}`, certified.
2. **Subscription-style name:** a reachable `schedule.at` use;
   expect it present (the catalog namespace is uniform — no
   special-casing by provider family).
3. **Value-passed platform function:** `chat.say` passed as a
   callback, never applied in the fixture; expect `chat.say`
   present (R2), certified.
4. **Value-passed local helper:** helper calling `loc.position`
   passed as a value; expect `loc.position` present (R3).
5. **Dead code:** `time.now`-class platform function referenced
   only from an unreachable definition; expect it absent, and
   present in `unreachable_refs` if emitted.
6. **Dynamic loading:** a reachable runtime-loading call; expect
   `<unknown>` present and `certified: false`, with the statically
   visible uses still listed (R4).
7. **Version skew:** a platform reference absent from the catalog;
   expect the §7 error naming it.
8. **Diff pairing:** a fixture `Wants` + fixture record exercising
   §6.3 both ways, including the uncertified suppression of the
   over-declared side.

## 10. References

- #5998 — this design task; #5958 — capability model (the pairing
  this output feeds); #5963 — programming model (may narrow R2);
  #5955 — worlds program (dynamic-loading exception policy owns
  how uncertified records are presented).
- Triage of record on #5998, comment 6700990165 (premises,
  representation, and the R2 ruling).
- Prior-art research: `worlds-capabilities-brief-2026-11-04`
  (Lane B item 8) — the Android/Koka contrast in §1.
- Code anchors (verified at the main this spec was written
  against): `world_caps.rs` — `PROVIDER_REGISTRY`,
  `catalog_names()`, `is_known`, `Wants::caps()`, `parse_wants`
  (`WANTS_VERSION`), `CapLedger` / `CapUsage`,
  `UNKNOWN_CAP_USAGE_KEY`.
