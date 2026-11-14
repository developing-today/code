# Design: used-capability derivation tool — static call-graph scan producing the registry used-set (#5998)

Status: spec, ready to implement. This document specifies the tool; it
contains no implementation. Parent design: #5958 (capability model —
declaration vs discovery, registry schema), recorded in the #5958 design
record (issue #5958, note 6720327099). Programme: #5955. Triage of
record for this issue: note 6700990165 on #5998. The programming-model
design #5963 constrains one rule below (§5.1) and is cited where it does.

## 0. Premises, verified at scan-writing time

Verified against origin/main `e648d70e8e9b9c2de86d854d98ba4eaa557893bd`
(2026-11-11; unchanged in these paths at `b8c733e53e`):

- The capability namespace is one static catalog:
  `pkgs/id/crates/id-core/src/world_caps.rs` builds `PROVIDER_REGISTRY`
  with the `define_provider_registry!` macro and exposes
  `catalog_names() -> BTreeSet<String>` and `is_known(name)`. At the
  verified commit the production catalog has 20 entries: 14 host
  providers (`time.now`, `time.tick`, `random.u64`, `players`,
  `players.list`, `chat.say`, `chat.history`, `records.read`,
  `schedule.at`, `schedule.cancel`, `world.info`, `device.info`,
  `loc.position`, `loc.watch`) and 6 `ProviderKind::PlatformModule`
  entries (`Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, `RocText`).
  **The tool never hardcodes this list** (§2.2); the list here is a
  worked-example snapshot only. (One further entry exists behind the
  `test-provider` feature and is not part of the production catalog.)
- The declared set already exists in the same representation:
  `Wants` / `parse_wants` (`WANTS_VERSION = 1`, `MAX_WANTS_ENTRIES =
64`) hold `BTreeSet<String>` over catalog names, and `CapLedger`
  keeps `granted` / `allowed` as `BTreeSet<String>` plus per-name
  `CapUsage { allowed, denied }` counts, with unattributable use
  aggregated under `UNKNOWN_CAP_USAGE_KEY = "<unknown>"`.
- Worlds are Roc apps on the plaza platform
  (`pkgs/id/examples/roc-world/platform.roc`): the platform
  `exposes [Screen, Key, RocJson, RocGrid, RocCap, RocText]` — exactly
  the six PlatformModule catalog names — and a world uses a module by
  importing it (`import pf.RocCap`).
- Host providers are invoked by name at runtime, not by Wasm import:
  the guest module is import-free (plaza ABI v1,
  `pkgs/id/examples/roc-world/ABI.md`). A world builds request
  documents whose `"cap"` field is the catalog name
  (`RocJson.String("chat.say")` inside `RocCap.chat_request`, etc.)
  and names subscriptions the same way in its `wants` document.
  Capability references in source are therefore exactly two shapes
  (§3), and a binary/Wasm scan can see none of them (§8).
- No derivation tool exists yet: nothing in `pkgs/id` performs a
  call-graph or used-capability scan (the only `call_graph` hits are
  the vendored pavex compiler, unrelated).

## 1. Purpose and non-goals

The tool statically derives, for one world build, the set of catalog
capabilities its code can reach — `used_static` in the #5958 registry
schema — so the registry can present the S16 line ("it asks for
{requested}, its code can reach {used_static}, and it has been seen
using {used_observed}") and the narrowing delta
`requested − used_static`.

In scope: inputs, the reference model, the reachability algorithm,
the over-approximation rules, the dynamic-dispatch limitation and its
marking, and the output contract.

Out of scope (owned elsewhere, cited not re-decided):

- Implementing the tool (this spec is the deliverable of #5998).
- The permission-UX surfacing of over-declaration (sibling under
  #5955); this spec only emits the sets that UX diffs.
- The dynamic-loading exception _policy_ — which uncertified worlds
  may run, and under what ceiling (follow-up under #5955). This spec
  defines only how the tool _marks_ uncertified output (§6).
- `used_observed`: produced by the existing `CapLedger` / `CapUsage`
  instrumentation at runtime, not by this tool. The two producers
  emit the same set type and are reconciled by the registry per the
  #5958 record §3, never merged by this tool.

## 2. Inputs

### 2.1 Source closure (required)

The world's Roc source closure: the entry app file plus every module
reachable from it through `import` statements, including the platform
modules it imports from the world's platform (`pf.*`) and any local
library modules. The closure is computed by the tool from the entry
file (§4, step 1); the caller supplies:

- `--entry <path>`: the world's app file (e.g. `main.roc`).
- `--platform <path>`: the platform interface file the app targets
  (e.g. `platform.roc`), used to resolve `pf.*` imports to the
  platform's own module files, whose contents join the closure.
- `--world-id <string>` and `--build-hash <string>`: copied verbatim
  into the output envelope (§7). The build hash is the caller's
  content hash of the exact closure scanned; the tool does not define
  the hashing scheme (the registry entry is per (world, build), #5958
  record §5).

A file that is imported but missing, or an import cycle that cannot
be resolved to files, is an input error (§7.3) — never a partial scan
presented as complete.

### 2.2 Catalog snapshot (required)

`--catalog <path>`: a JSON array of catalog names — the serialization
of `catalog_names()` (`BTreeSet<String>`) for the catalog the world
will run against, produced by the host build from `world_caps.rs` at
a known commit. The tool also takes `--catalog-commit <sha>`, copied
into the output envelope.

The tool must not parse `world_caps.rs` source and must not embed a
name list: the registry is macro-generated, so source parsing is
brittle, and a hardcoded list silently goes stale the day a provider
is added (the catalog grew `device.info` between the #5958 record and
this spec). Every name the tool emits is validated against this
snapshot (§4, step 5).

### 2.3 Requested set (optional)

`--wants <path>`: the world's declared `wants` document (the
`parse_wants` format, `WANTS_VERSION = 1`). When supplied, the tool
additionally emits the diff fields of §7.2. The scan itself never
depends on it: derivation is from code and catalog only.

## 3. Reference model — what counts as using a capability

A _reference_ is an occurrence, inside a reachable definition (§4),
of one of the following two shapes. These are the only two channels
by which Roc world code can name a capability (§0).

### 3.1 Platform-module references (PlatformModule entries)

An `import pf.<Module>` statement in any file of the closure, where
`<Module>` is a PlatformModule catalog name (`Key`, `Screen`,
`RocJson`, `RocGrid`, `RocCap`, `RocText` at the verified commit),
counts as a use of that catalog entry. Collection of these
imports is closure-wide: an import in any file of the source
closure (§4 step 1) counts, whether or not any definition of the
importing file is reachable (§4 step 4) — this matches the compile
gate's `check_imports` (`world_compile.rs`), which checks imports
in every submitted source file without filtering by definition
reachability. (Definition-level reachability still governs the
host-provider references of §3.2.) Granularity is the module:
importing the module counts the entry whether one function or all
functions are used, because the catalog itself is module-grained
(the compile gate checks platform modules as units, per
`world_caps.rs`). The platform's own internal imports do **not**
count: `platform.roc` imports all six modules to expose them, so
references are collected from the world's files and library modules,
not from the platform interface file or the platform's
implementation files behind it. A platform module's own references
to host providers (e.g. `RocCap.chat_request` embedding
`"chat.say"`) are collected only when that module is itself imported
by the world — it then belongs to the closure as a library module,
and its definitions are subject to the same reachability rules as
any other library code (§4).

### 3.2 Host-provider references (Request / Subscription entries)

A string literal exactly equal to a host-provider catalog name is a
reference when it appears in a reachable definition in either of
these positions:

1. as the `"cap"` field value of a request document — in practice the
   `RocJson.String("<name>")` argument paired with `{ key: "cap", … }`
   in a document constructor, whether in `RocCap`'s constructors
   (`chat_request`, `now_request`, `players_request`,
   `world_request`, `history_request`, `records_request`, …) or in
   world code building a document itself; or
2. as the capability-name argument of a request-issuing call
   (`RocCap.issue` and any wrapper over it), or as an entry of a
   `wants` document's subscribe/requests name lists constructed in
   code.

Recognition rule (deliberate simplification, and it is sound for
this codebase): within a reachable definition, **any** string literal
exactly equal to a catalog name counts as a reference to that name,
whatever its syntactic position. Roc world code has no other
stringly-typed capability channel, catalog names are dotted and
distinctive (`records.read`, `schedule.at`), and the failure mode of
the wider rule is a rare false positive (a chat message containing
the literal text `"time.now"` in reachable code over-reports
`time.now`) — the safe direction for a used-set that exists to bound
authority. Position-aware narrowing may come later; v1 implements
the wider rule so the implementer makes no per-position judgements.
Literals equal to no catalog name are ignored. Literals in
unreachable definitions are ignored (§4).

Two recognition refinements override the plain exact-equality rule
above; both exist because real worlds in this codebase already use
these shapes, and omitting them would leave the result incomplete
yet certified — the failure §6 exists to prevent.

**Serialized wants documents.** A world may return its wants
document as one serialized string literal (the territory example
returns `"{\"v\":1,\"subscribe\":[\"players\"],\"requests\":[]}"`
as a single literal from `wants`). Such a literal equals no catalog
name, so exact matching alone misses every name inside it. Rule:
when a string literal in a §3.2 position parses as a JSON wants
document (an object carrying `subscribe` and/or `requests`
arrays), the entries of those arrays are capability positions,
recognised per the runtime's own schema (`parse_wants` in
`world_caps.rs`): `subscribe` entries are strings — subscription
specifications, subject to the normalisation below — while
`requests` entries are _objects_, and each object's `cap` string
field is the capability position (a §3.2 position-1 reference,
matched exactly against the catalog). A parsed document whose
entries deviate from that schema (a non-object request entry, a
non-string `cap`) is a document the runtime itself rejects; the
scan does not guess at it — that shape triggers the §6 uncertainty
marking. A
wants-shaped string that does _not_ statically parse — built by
interpolation or concatenation, so its entries may be produced at
runtime — is not partially guessed at: it triggers the §6
uncertainty marking instead.

**Subscription specifications are normalised before catalog
matching.** A `subscribe` entry is a subscription _specification_,
not a catalog name: the runtime parses it with
`Subscription::parse` (`world_caps.rs`) and grants the capability
`Subscription::cap` returns. At the verified commit the mapping is:
`players` → `players`; `time.tick:<ms>` (a positive u64 interval)
→ `time.tick`; any other spec is `Unknown` and maps to itself.
Rule: normalise every subscribe entry through exactly these
runtime semantics, then apply catalog matching to the _result_ —
so `time.tick:5000` (already used by the lounge example) counts as
a use of `time.tick` even when no literal `time.tick` appears
anywhere, and an `Unknown` spec counts only when it is exactly a
catalog name. The tool replicates the runtime mapping for the
catalog snapshot's commit; if the runtime's subscription grammar
grows, this rule grows with it — the snapshot's `catalog_commit`
(§7.1) pins which grammar was applied.

## 4. Algorithm

1. **Closure.** Parse the entry file's imports; resolve each import
   to a file (world-local module, `pf.*` → the platform's module of
   that name per §2.1, package imports → their source files).
   Repeat transitively. The result is the file set; a missing file is
   an input error (§7.3).
2. **Definition graph.** Within the closure, enumerate named
   definitions (top-level values and functions, including lambdas
   bound to names). Build edges `A → B` when `B`'s name is referenced
   in `A`'s body (direct call, partial application, or value
   reference — the distinction does not matter, §5.1). References to
   definitions outside the closure (Roc builtins, platform-provided
   primitives) are leaves.
3. **Seeds.** The entry points are the definitions the platform
   invokes: the fields of the app's `program` record (`init`,
   `update`, `view`, `layout`, `records`, `wants`, `snapshot`,
   `restore` — those present), plus any definition exported by the
   entry module. Everything reachable from a seed is _reachable_;
   reachability is a fixpoint over the definition graph.
4. **Collection.** Collect references per §3 in two passes with
   different reachability treatment. (a) _Platform-module imports_
   (§3.1) are collected from **every file in the closure**,
   independent of definition reachability: a library that the entry
   imports but whose definitions are never referenced still belongs
   to the closure, and its platform imports are uses — the compile
   gate (`check_imports` in `world_compile.rs`) refuses an
   ungranted platform import in any submitted file, so the grant is
   required to compile the unchanged sources whether or not a
   definition is reached. (b) _Host-provider references_ (§3.2) are
   collected per reachable definition: catalog-name literals in its
   body, serialized-wants extraction, and subscription
   normalisation, all per §3.2. Union both passes into the
   candidate used-set.
   Additionally apply §5 and §6: value-passed definitions contribute
   their references at the passing site (already true under step 2's
   value edges — stated as a rule in §5.1 so no implementer
   "optimises" it away), and non-literal capability names trigger
   the uncertainty marking of §6.
5. **Validation and emission.** Intersect the candidate set with the
   catalog snapshot: any candidate name not in the snapshot is
   dropped and reported on stderr as a stale-catalog warning (it
   cannot be emitted — the output invariant is `used_static ⊆
catalog`). Emit per §7, names sorted (BTreeSet order: byte-wise
   lexicographic, so `RocCap` sorts before `chat.say`).

Properties the implementation must hold (the implementer's test
oracle): **soundness over reachable code** — every capability a
reachable definition can name by literal or module import appears in
the output; **monotonicity** — adding a reference to reachable code
never shrinks the output; **determinism** — identical inputs produce
byte-identical output (all sets sorted, no timestamps or host data
inside the payload); **dead-code exclusion** — references in
definitions unreachable from the seeds do not appear (this is what
makes the set a per-build property rather than a per-repository
one). Reachability is definition-level and branch-insensitive: a
reference in a reachable but never-taken branch **does** count,
matching the #5958 record §3 ("over-approximates along dead or
never-taken paths" — paths, not definitions).

## 5. Over-approximation rules

The used-set is an upper bound by design; these are the only rules
that widen it, and each is stated so an implementer never has to
guess.

### 5.1 First-class platform functions (the #5963-constrained rule)

A platform function — a `RocCap` request constructor, an issuing
helper, or any definition whose body carries capability references —
may be passed as a value: stored in the model, given to `List.map`,
handed to a callback, returned from a function. Rule: **a platform
function passed as a value counts as used, wholesale** — every
capability referenced by that definition enters the used-set at the
point the definition is referenced as a value, exactly as if it had
been called there. Mechanically this is why §4 step 2 builds value
edges as well as call edges. This is the conservative rule the #5998
triage adopted (note 6700990165): while #5963's programming model
leaves value-passing untracked, the scan treats passing as use. If
#5963 later makes value-passing trackable (e.g. capabilities as
typed values whose flow the compiler follows), this rule may be
narrowed by a successor record; until then it stands, and an
implementation must not narrow it unilaterally.

### 5.2 Module granularity

Importing a platform module counts the whole module entry (§3.1),
never a per-function subset — the catalog has no finer names for
module contents.

### 5.3 Aliasing and re-export

A definition that aliases or re-exports another (a wrapper that only
forwards, a module that re-exposes a helper) carries the original's
references: edges follow through aliases to the defining body, so
wrapping a constructor never hides its capability, and never
double-counts (the output is a set).

### 5.4 No path sensitivity, no constant folding beyond literals

Branches, guards, and early returns do not narrow the set.
Arithmetic or concatenation that _produces_ a catalog name is not
folded — it is not a literal, so §6 applies instead.

## 6. Dynamic dispatch — the explicit limitation, and its marking

Exactness ends where names stop being literals. The following, in
reachable code, defeat static derivation for the affected requests:

- a capability name computed at runtime (`Str.concat`, a variable,
  a value decoded from data or from the model) in any §3.2 position;
- a request document whose `"cap"` field is set from a non-literal;
- a wants document in serialized form that does not statically
  parse (§3.2) — interpolated or concatenated, so its
  subscribe/requests entries may never appear as literals;
- code loaded or generated outside the scanned closure (the
  dynamic-loading case whose _policy_ belongs to the #5955
  follow-up).

Marking rule (normative): when the scan encounters a non-literal in
a §3.2 capability position in reachable code, it

1. adds the string `"<unknown>"` (`UNKNOWN_CAP_USAGE_KEY`, the same
   bucket the runtime ledger uses for unattributable use) to the
   output's `uncertain` list — **never** to `used_static` itself,
   which contains catalog names only;
2. sets `certified: false` in the output envelope; and
3. still emits every literally-derived name it found.

A scan with no such encounter emits `uncertain: []` and
`certified: true`. Certification is a property of the scan, stated
in the envelope, so the registry and the #5955 policy can act on it
without re-deriving anything. The tool itself takes no policy
action: it neither widens the used-set to the whole catalog nor
suppresses the partial result — presenting the partial set honestly,
flagged, is the behaviour the #5958 record §4 requires of derived
data generally.

## 7. Output contract

### 7.1 Envelope

One JSON object per scan, written to stdout or to `--out <path>`:

```json
{
  "world_id": "roc-chat",
  "build_hash": "<caller-supplied>",
  "vocab_version": 1,
  "wants_version": 1,
  "catalog_commit": "<sha the catalog snapshot was generated at>",
  "certified": true,
  "uncertain": [],
  "used_static": ["Key", "RocCap", "RocJson", "chat.history", "chat.say"]
}
```

`used_static` is exactly the `used.static` array of the #5958
registry schema (record §5): the same names, the same sorted order,
dropped into the registry entry unchanged — the registry
representation _is_ a `BTreeSet<String>` of catalog names serialized
as a sorted JSON array, so used vs declared is a plain set diff with
no translation layer. `vocab_version` and `wants_version` are the
constants of the same names in `world_caps.rs` at the catalog
commit. This envelope is the tool's whole output; the registry
assembles the full #5958 entry (requested set, observed set, delta)
from it and its other producers.

### 7.2 Diff fields (only when `--wants` is supplied)

```json
{
  "requested_unused": ["records.read"],
  "static_not_in_requested": []
}
```

- `requested_unused` = requested − used_static: the narrowing delta
  of the #5958 record §4, the "you could drop these" set.
- `static_not_in_requested` = used_static − requested: the static
  overflow. Per #5958 record §5 this is the flagged-defect case (an
  entry is clean only when `used_static ⊆ requested`); the tool
  reports it, stores nothing, and never resolves it by widening the
  requested set.

### 7.3 Exit codes and errors

- `0` — scan complete, `certified: true`.
- `2` — scan complete, output emitted, `certified: false` (§6).
  Callers that gate on certification check the code or the flag.
- `1` — input error (missing import target, unreadable entry /
  platform / catalog file, malformed `--wants`): no output object is
  emitted; the diagnostic goes to stderr. A partial scan is never
  presented as a result.

### 7.4 Worked example

`pkgs/id/examples/roc-chat` at the verified commit: `main.roc`
imports `pf.Key`, `pf.RocCap`, `pf.RocJson` (§3.1 → `Key`, `RocCap`,
`RocJson`) and calls `RocCap.history_request` and
`RocCap.chat_request` from reachable definitions; those constructors
embed `RocJson.String("chat.history")` and
`RocJson.String("chat.say")` (§3.2 → `chat.history`, `chat.say`). No
non-literal capability names occur. Output `used_static`, in
BTreeSet order, is exactly the array in §7.1:
`["Key", "RocCap", "RocJson", "chat.history", "chat.say"]`,
`certified: true`. The implementer's first fixture test is this
example with this expected set; the other in-repo examples
(`roc-world`, `lounge`) are the next fixtures, with expected sets
derived by hand from their imports and constructor calls before the
tool is run on them.

## 8. Alternatives considered and rejected

- **Scan the compiled Wasm.** Rejected on a verified fact: plaza
  guests are import-free Wasm32 modules (ABI v1) — capabilities are
  named in runtime request documents, not imports, so a binary scan
  sees nothing. The information exists only in source.
- **Runtime profiling only.** Already exists (`CapUsage`) and is the
  `used_observed` producer; it under-approximates paths no run has
  taken and cannot answer for a build that has never run. The #5958
  record §3 settles "static, instrumentation, or both": both.
- **Substring / text search over source.** Matches comments,
  unreachable code, and unrelated strings; misses nothing the literal
  rule catches but adds noise the reachability pass exists to
  remove, and it cannot apply §5.1 or §6 at definition granularity.
- **Type-directed derivation (Koka-style).** Exact where available;
  Roc's platform boundary here is stringly-typed by design (the
  `"cap"` document field), so there is no effect row in the type
  system for this tool to read. Noted in #5998's own scoping.
- **Hardcoding the catalog in the tool.** Rejected in §2.2: the
  catalog is a living macro-generated registry.

## 9. What implementation looks like (boundary of this spec)

An implementer needs no further design decisions: the inputs and
their formats (§2), the two reference shapes (§3), the five
algorithm steps and four test-oracle properties (§4), the widening
rules (§5), the uncertainty marking (§6), and the byte-level output
contract with exit codes (§7) are all fixed above. Remaining choices
that are _not_ design decisions — implementation language, Roc
parser/library selection, internal graph representation, CLI
framework — are left to the implementation PR, which should land
with the §7.4 fixtures as tests and the §4 properties as property
tests. Any change to §§3–7 after this record lands is a design
change and goes in a successor dated record, not in the
implementation PR's fine print.
