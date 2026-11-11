# Used-capability derivation tool — spec

Issue: #5998 (design). Parent topic: #5958 (capability model);
program: #5955 (worlds). Triage of record: the 2026-11-07 triage
note on #5998 (comment 6700990165), which verified every premise
below against main and fixed the one open rule (value-passed
capability-naming functions) conservatively, pending #5963.

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
over the world's Roc source closure, plus extraction of catalog
names from the capability positions of reachable definitions.
Capabilities are not Roc-callable functions: a world *requests* a
capability by naming it as data — it constructs request and
subscription documents (usually through platform helper modules
such as `RocCap`) whose `cap` field carries a catalog name,
supplied as a string literal at the issuing call
(`RocCap.issue(session, "chat.say")` in
`examples/roc-chat/main.roc`) and embedded again in the helper
builder that constructs the document (`RocCap.chat_request` builds
the object whose `cap` is `"chat.say"`, in
`examples/roc-world/RocCap.roc`). The scan is exact for that
protocol except for the named over-approximation sources in §5 —
unlike binary/API static analysis (noisy, needs an API→capability
mapping the platform does not otherwise have) or type inference
(exact, unavailable in Roc).

The output pairs with the declared set in the registry format #5958
defines (its requested-vs-discovered pairing): both sides are the
same set type over the same names, so over-declaration and
under-declaration are plain set differences (§6).

## 2. Definitions

- **Catalog.** The provider catalog in
  `pkgs/id/crates/id-core/src/world_caps.rs`: the const
  `PROVIDER_REGISTRY` (`ProviderRegistry` over `CATALOG_ENTRIES`),
  with `catalog_names() -> BTreeSet<String>` and `is_known(name)`.
  Catalog names are dotted capability names (`chat.say`,
  `records.read`, `schedule.at`, `loc.position`, …). The catalog is
  the *only* capability namespace the tool knows.
- **Capability name.** A catalog name carried as data in a request
  or subscription document — the `cap` field the runtime dispatches
  on. A world names a capability in exactly two places: as a string
  literal at an issuing call in its own code (above), and inside
  the platform helper builder that constructs the document. Both
  are use sites (§4 step 4).
- **Platform helper.** A definition in a platform module (`RocCap`,
  `RocJson`, …) that constructs or issues request/subscription
  documents. Helpers are ordinary closure definitions: their
  bodies are scanned, and calling one is a call-graph edge like
  any other. A helper is not a catalog name and is never checked
  against the catalog.
- **Source closure.** The transitive set of Roc modules a world (or
  library) compiles from: its entry module plus everything it
  imports, recursively, within the package being scanned and the
  platform tree it imports from. Platform modules are part of the
  closure like any other module: helper bodies carry capability
  literals (§2), so they are scanned, never treated as leaves.
- **Entry points.** The definitions the platform can invoke on the
  world: the world's exported effect handlers / main definitions,
  as declared by the world's platform interface. The scan is seeded
  exactly at the entry points; a scan request names them (§3).
- **Used-set.** The set of catalog names the derivation proves the
  world can name in a capability position (§4 step 4), under the
  rules of §5.
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
   is an input error. Entry points are also a certification input:
   a result is certified *for the whole world* only when the
   resolved entry-point set equals the platform interface's
   complete declared export set for the world kind being scanned.
   A caller override that resolves to a proper subset (one valid
   handler out of several the platform can invoke) yields a
   partial-scope record (§6.2): the omitted handlers' capabilities
   are outside the scan, the record names the entry points it was
   seeded with, and it is never certified — §6.3 must not read a
   partial record as evidence about the whole world.
3. **Catalog snapshot** — the catalog as `BTreeSet<String>` from
   `PROVIDER_REGISTRY.catalog_names()` in the `id-core` build the
   tool is compiled against. The catalog is compiled in, not read
   from the scanned package: the platform the world will run
   against is the authority, and a string literal in a capability
   position (§4 step 4) that is not a catalog name is an error
   naming the literal (it indicates version skew between the world
   and the platform, which must surface loudly, not degrade the
   scan). References to platform helpers resolve inside the closure
   (§2), never against the catalog: a helper name is not a
   capability name, and no helper reference is a skew error.

No other configuration exists. In particular there is no
allowlist/denylist input: exceptions are expressed by the rules of
§5 and the `<unknown>` marker, not by per-world configuration.

## 4. Algorithm

1. **Close.** Parse the entry module; walk imports recursively to
   the full source closure. Collect every definition (function or
   value) in the closure, keyed by module-qualified name.
2. **Graph.** Build the call graph over closure definitions: an
   edge A → B for each direct call of B in A's body, and a
   dependency edge A → V for each *value* definition V that A
   references — a module-level aggregate, record, or alias such as
   `ops = { read: records.read }` reached through `ops.read`, or
   `read = records.read` used unapplied. A value definition's
   initializer is analysed like a body: when V is reachable, the
   platform references in its initializer are collected (§5 rule
   R5), and function values stored in it take the value-flow edges
   of rule R3 from V. There are no sink nodes: platform helper
   definitions are closure definitions, and catalog names enter
   the result only at Collect (§4 step 4).
3. **Reach.** Mark definitions reachable from the entry points,
   following call edges to a fixpoint (cycles are expected;
   reachability is a visited-set computation, not a recursion).
   Value-flow edges from §5 rule R3 are added during this pass and
   participate in the same fixpoint.
4. **Collect.** The used-set is the set of catalog names that
   appear in a *capability position* in a reachable definition. A
   capability position is:
   (a) the capability argument of a request-issuing call (the
   `RocCap.issue`-shaped position — a string literal supplied
   where a request is issued);
   (b) the value of a `cap` field in a request or subscription
   document a definition constructs — including inside platform
   helper builders (`RocCap.chat_request`'s `"cap": "chat.say"`),
   which is why helper bodies are closure code, not leaves;
   (c) an element of a subscription list, normalized by the
   runtime's own subscription parsing — the equivalent of
   `Subscription::parse(spec).cap()` in `world_caps.rs`, never by
   stripping an arbitrary `:<args>` suffix. The parser recognizes
   exactly two parameterized/plain forms: `players` (capability
   `players`) and `time.tick:<ms>` with `<ms>` a positive `u64`
   (capability `time.tick`). Every other spec — `players:extra`,
   `time.tick:0`, `time.tick:soon` — is an unknown subscription
   whose capability string is the *whole* spec, so in a capability
   position it takes the §3 version-skew error like any other
   non-catalog name; it must never be suffix-stripped into a known
   name and certified. This is how `parse_wants` itself reads
   subscription entries; and
   (d) a *serialized* document a definition returns as a single
   string (a whole wants document carried as escaped JSON):
   extraction parses the embedded document with the same field
   rules as (b) and (c) — `cap` fields of its `requests` entries,
   and its `subscribe` entries, which are bare capability strings
   with no `cap` field (the territory shape:
   `{"v":1,"subscribe":["players"],"requests":[]}`, read exactly
   this way by `parse_wants`) — rather than treating the string as
   opaque.
   String literals in any other position are not uses. References
   inside string interpolations are ordinary references:
   extraction strips literal text, never the interpolated
   expressions, whose targets stay in the graph. Each collected
   name is checked with `is_known`; a capability-position literal
   that is not a catalog name is the §3 version-skew error. Names
   a reachable definition can cause to be named through a function
   value it passes on are collected under §5 rule R2.
5. **Certify.** If any reachable definition contains a §5 rule R4
   construct, add the unknown marker and mark the result
   uncertified. Certification is whole-scan: one dynamic site
   anywhere reachable uncertifies the scan, because the site can
   in principle reach any capability. A scan whose resolved
   entry-point set is a proper subset of the platform's declared
   exports (§3) is likewise never certified, whatever else it saw:
   it is emitted as a partial-scope record (§6.2), not a
   whole-world one.
6. **Emit** the output document of §6, deterministically: names
   sorted (the `BTreeSet` iteration order), no timestamps or
   environment data in the document, so identical inputs produce
   byte-identical output.

## 5. Over-approximation rules

The scan is exact for direct calls in a statically closed world.
Exactly four rules widen it, and one rule marks its limit. No
other widening is permitted: an implementation that approximates
anywhere else is not implementing this spec.

- **R1 — direct reference.** A catalog name that appears in a
  capability position (§4 step 4) in a reachable definition is
  used — at an issuing call in the world's own code, or inside a
  reachable platform helper builder. This is the base rule and
  covers the overwhelming majority of real code.
- **R2 — capability-naming function passed as a value.** A
  function — platform helper or closure-local — whose body names
  capabilities, taken as a value in a reachable definition in any
  non-applied position (passed as an argument, returned, stored
  in a structure, captured by a closure), contributes *every*
  catalog name its own body can name (its §4 step 4 literal set),
  whether or not the scan can prove the value is ever applied.
  Rationale (the triage's conservative ruling, pending #5963):
  once such a function becomes a value, tracking every application
  site is a dataflow problem the programming model does not yet
  make trackable; counting its full named set is the sound
  direction (it can only overstate, never understate). If #5963's
  model later makes value-passing trackable, this rule narrows —
  by amending this spec, not by implementation choice.
- **R3 — local function passed as a value.** A closure-local
  function taken as a value in reachable code is treated as
  reachable itself (a value-flow edge from the taking definition
  to it), because it may be applied anywhere the value flows.
  This is what makes R2 compose: a helper that names
  `records.read` in a capability position and is passed as a
  callback contributes `records.read`. Every closure definition —
  app code or platform helper — gets value-flow edges the same
  way.
- **R4 — dynamic dispatch / runtime loading.** A reachable use of
  any mechanism that selects code or capabilities at run time —
  dynamic loading, runtime module resolution, a capability
  position filled by a *computed* (non-literal) name, or a
  platform escape hatch of the same shape — defeats enumeration.
  A *literal* string name is R1, never R4: string-named requests
  are the protocol's normal case (§2), and treating them as
  dynamic would decertify every ordinary world. The scan adds the literal name
  `<unknown>` (the existing `UNKNOWN_CAP_USAGE_KEY` in
  world_caps.rs, the same bucket the runtime ledger uses for
  unattributable use) to the used-set and marks the result
  uncertified (§2). It does **not** add the whole catalog: the
  used-set stays a truthful record of what was *seen*, and the
  certification flag, not set inflation, carries the uncertainty.
- **R5 — value-definition dependencies.** A platform reference
  that sits in the initializer of a module-level value definition
  — an aggregate member (`ops = { read: records.read }`), an alias
  (`read = records.read`), or a stored structure — counts as used
  when the value definition is referenced from reachable code,
  exactly as if the reference appeared in the referencing
  definition's body. Without this rule a reachable access like
  `ops.read` would contribute nothing: neither a direct call from
  the accessor's body nor an R3 function-value edge names the
  value definition, so a genuinely used capability could be
  omitted from a certified result and §6.3 would report it as
  over-declared. Value definitions referenced only from
  unreachable code stay dead (§5's dead-code rule applies to them
  unchanged).

Dead code is not an approximation source in either direction:
capability names referenced only from unreachable definitions
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
    /// The entry-point definitions the scan was seeded with
    /// (module-qualified names), as resolved in §3.
    pub entry_points: BTreeSet<String>,
    /// True iff `entry_points` is a proper subset of the platform
    /// interface's declared exports for the world kind: the
    /// record's scope is partial (§3, §6.2) and `certified` is
    /// false.
    pub partial: bool,
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
diffed the same way. Representation is not pairing semantics,
though: §6.3 defines what may serve as the *declared* side of the
diff, and a live `Wants::caps()` snapshot may not — it is a
demand snapshot, not a declaration (§6.3).

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
- `entry_points` — the sorted array of entry-point definitions
  the scan was seeded with (module-qualified names).
- `scope` — `"whole-world"` when `entry_points` equals the
  platform interface's complete declared export set for the world
  kind, otherwise `"partial"`. A partial record always has
  `certified: false`; §6.3 treats it like an uncertified record
  (over-declared side suppressed) and names its restricted scope.
- Optional diagnostic field `unreachable_refs` (sorted array of
  catalog names referenced only from unreachable code) may be
  emitted for tooling display; consumers MUST NOT diff against it,
  and the registry pairing below ignores it.

Duplicate keys in a serialized record are a parse error for any
consumer (the #6520 rule the codebase already applies to persisted
JSON: a document is validated as written, never as a last-wins
collapse).

### 6.3 Pairing and differences

Given a `UsedCaps` record for a world and `declared` — the
world's **stable declared capability surface**: the declaration of
record for that world and build (the set the world declares it
may request, as the registry stores it), *not* a live
`Wants::caps()` snapshot. `Wants::caps()` collects only the
current requests and subscriptions (`world_caps.rs`): a world
like `examples/roc-chat` starts from `RocCap.empty` and its
`wants` emits only pending requests, so `chat.say` is absent
until a send is in flight and disappears when it completes.
Diffing whole-program static use against that momentary snapshot
would report deferred-but-legitimate effects as under-declared
while a request is unissued, and vanished ones as over-declared
after it completes. A pairing against a `wants` snapshot may be
displayed only labelled demand-vs-use, never declared-vs-used;
the differences below are defined on the stable declaration
only:

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
| Capability-position literal with no catalog name | error naming the literal (§3.3) |
| R4 construct reachable | **not an error** — `<unknown>` + uncertified (§5) |
| Entry-point override resolving to a proper subset of the platform's declared exports | **not an error** — partial-scope record (§6.2), never certified |
| Platform reference only in dead code | not an error, not in the used-set (§5) |

## 8. Implementation shape (for the follow-on)

- A library function in `id-core` beside `world_caps.rs`
  (`derive_used_caps(closure, entry_points) -> Result<UsedCaps>`),
  with a thin CLI front-end in the `id` binary for registry
  production; the catalog and the unknown key are already in
  `id-core`, so no new crate is needed.
- The Roc front-end the implementation uses must expose, per
  definition: direct call targets, value-position references to
  named functions, string literals in capability positions
  (§4 step 4 — including `cap` fields inside serialized JSON
  documents, and with references inside string interpolations
  retained as graph references), and the R4 constructs. If the available Roc
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

1. **Direct use, transitive:** entry → helper, the helper naming
   `records.read` in a capability position; expect exactly
   `{records.read}`, certified.
2. **Subscription-style name:** a reachable `schedule.at` use;
   expect it present (the catalog namespace is uniform — no
   special-casing by provider family).
3. **Value-passed request builder:** a helper naming `chat.say`
   in a capability position, passed as a callback and never applied
   in the fixture; expect `chat.say` present (R2), certified.
4. **Value-passed local helper:** helper naming `loc.position`
   in a capability position, passed as a value; expect
   `loc.position` present (R3).
5. **Dead code:** a `time.now`-class capability named only from
   an unreachable definition; expect it absent, and
   present in `unreachable_refs` if emitted.
6. **Dynamic loading:** a reachable runtime-loading call; expect
   `<unknown>` present and `certified: false`, with the statically
   visible uses still listed (R4).
7. **Version skew:** a platform reference absent from the catalog;
   expect the §7 error naming it.
8. **Diff pairing:** a fixture stable declaration + fixture record
   exercising §6.3 both ways, including the uncertified suppression
   of the over-declared side.
9. **Value-definition aggregate:** `ops = { read: records.read }`
   with `ops.read` accessed from a reachable definition and no
   direct `records.read` call anywhere; expect `records.read`
   present (R5), certified.
10. **Partial entry points:** a scan seeded with only `init` on a
    world whose `update` handler uses `chat.say`; expect a
    partial-scope record with `chat.say` absent from `used`,
    `certified: false`, the seeded entry points named, and the
    §6.3 over-declared side suppressed.
11. **Request protocol (roc-chat shape):** the entry calls
    `RocCap.issue(session, "chat.say")` and the platform helper
    builder `RocCap.chat_request` embeds `"chat.say"` as the
    document's `cap`; expect `chat.say` present, certified. A
    variant with the issuing definition unreachable expects
    `chat.say` absent.
12. **Serialized wants document:** a reachable definition returns
    its whole wants document as one escaped JSON string of the
    territory shape, `{"v":1,"subscribe":["players"],"requests":[]}`;
    expect `players` present, certified (§4 step 4d — subscription
    entries are bare strings, extracted per (c)). A variant with
    `"subscribe":["time.tick:soon"]` expects the §3 version-skew
    error naming the whole spec, never a certified `time.tick`
    (§4 step 4c).

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
  against): `examples/roc-chat/main.roc` (`RocCap.issue` naming
  capabilities as literals) and `examples/roc-world/RocCap.roc`
  (request builders embedding `cap` names); `world_caps.rs` —
  `PROVIDER_REGISTRY`,
  `catalog_names()`, `is_known`, `Wants::caps()`, `parse_wants`
  (`WANTS_VERSION`), `CapLedger` / `CapUsage`,
  `UNKNOWN_CAP_USAGE_KEY`.
