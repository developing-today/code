# Design: the capability model — requested vs discovered-used sets, registry schema, custom-compiled platforms

- **Date:** 2026-11-08
- **Status:** proposed design for review; no implementation is included here.
- **Issue:** [#5958](https://github.com/developing-today/code/issues/5958), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955) (statements S8–S16 of the 2026-11-04 brief); capability-provider parent [#5686](https://github.com/developing-today/code/issues/5686).
- **Ruling that binds this note:** Drew's C2 capability-posture ruling, 2026-11-04 23:57 CST, recorded on [#5958](https://github.com/developing-today/code/issues/5958#issuecomment-6650954322): the shipped default posture (completely isolated seccomp, no fs open, no sockets) **does not move**; file/network capability is an **opt-in tier, off by default on servers**; publishing with it is **first-party only for now**; the architecture must remain capable of later general enablement.
- **Inputs:** the S9 inventory and vocabulary work on #5958; the current `pkgs/id` code at this checkout (commit `5ab0c06928`); the sibling designs [#5997](https://github.com/developing-today/code/issues/5997) (`doc/2026-11-05_design_id5997.md`), [#5965](https://github.com/developing-today/code/issues/5965) (`doc/2026-11-05_design_id5965.md`), and the open [#6000](https://github.com/developing-today/code/issues/6000) policy PR [#7025](https://github.com/developing-today/code/pull/7025), which explicitly leaves the canonical registry schema here.
- **Method:** every "current state" claim below was read from the tree at this checkout and is cited by file and line. Statements about what the design decides are marked as decisions; anything left open names the issue that owns it.

## Decision summary

| Question | Decision | Where |
| --- | --- | --- |
| Do we have more than one platform? | **No — one platform tree per server, one static provider catalog.** Six `PlatformModule` entries, two execution tiers (Wasm guest, native worker). Per-world platform selection does not exist today and is what this design adds. | §1 |
| Capability vocabulary v1 | Four tiers over one flat namespace: `standard` (today's catalog, default), `sandbox.fs` (sandbox file read+write, opt-in), `host.fs.read` (arbitrary file read, opt-in, admin-hard), `net.out` (arbitrary network out, opt-in, admin-hard). Expanded namespaces below; the four required names map one-to-one onto the tiers. | §2 |
| Declaration vs discovery | The **requested** set is declared by the world (a manifest); the **discovered-used** set is derived, never hand-written (#5997 §1/§3); the **effective grant** is what the server permits. Invariant: `discovered-used ⊆ requested` and `discovered-used ⊆ granted ∩ ceiling`. | §3 |
| Registry schema | One registry record per world release identity carrying `requested`, `discovered_used` (with a certification state and provenance) and the effective grant as separate facts; absent/uncertified usage is **unknown**, never empty. | §4 |
| Custom-platform compile path | A **requirement-set platform**: the compile tooling synthesises the minimal platform tree that satisfies the requested set, signed and content-addressed; paid in compile credits by the requesting author; shared across identical requirement sets by content hash. | §5 |
| Expansive-platform case (S15) | A world on a wider platform is displayed and enforced against its **discovered-used** set, not its requested set; the requested set is the ceiling, the used set is the honest claim. | §6 |
| Interaction with writer-set authority (#5545) | **Separate axis, intersecting only at admission.** A capability grant is not a writer-set grant: it bounds what the code may call, not whose writes count. Split cleanly; the one intersection rule is stated. | §7 |

## 1. S9 — the platform inventory, answered at this checkout

**Answer: one platform, one catalog, six platform modules, two execution tiers. There is no per-world compiled-platform selection today.**

### 1.1 One static catalog

`pkgs/id/crates/id-core/src/world_caps.rs` holds a single `CATALOG_ENTRIES: &[ProviderManifest]` (`world_caps.rs:233`) and a compatibility `CATALOG` view derived from it (`:246`). Every entry carries `name`, `version`, `kind`, `argument_schema`, `bounds`, `required_scope`, `default_ceiling` (`:202-217`). There are 19 entries: 11 host requests/subscriptions (`time.now`, `time.tick`, `random.u64`, `players`, `players.list`, `chat.say`, `chat.history`, `records.read`, `schedule.at`, `schedule.cancel`, `world.info`), two sensor caps excluded from default ceilings (`loc.position`, `loc.watch`, `:438-453`), and six Roc platform modules.

### 1.2 Six platform modules

`ProviderKind::PlatformModule` (`world_caps.rs:136`) has exactly six catalog entries: `Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, `RocText` (`:454-501`), all `DefaultCeiling::Included`. The compile gate uses them as the closed import namespace (`world_compile.rs:111-146`: a world may import only a platform module its effective grant names). They are checked at compile time, not called as host requests (`ArgumentSchema::NotApplicable`).

### 1.3 One platform tree per server

Compilation takes a `platform_dir` holding `platform.roc` and `targets/` (`world_compile.rs:49-70`); `platform_modules()` treats every other `.roc` file in that directory as a module (`:93-107`); the compiler stages a copy of that tree into the job directory (`:229`, `copy_platform` at `:559`). The tree is chosen per server (`ID_ROC_PLATFORM`, default `./examples/roc-world` — `cli.rs:556`), not per world. The repository's own platform is `pkgs/id/examples/roc-world/platform.roc`, which `exposes [Screen, Key, RocJson, RocGrid, RocCap, RocText]`.

One reduced fixture exists — `pkgs/id/examples/roc-world/platform-no-caps.roc`, `platform.roc` with `RocCap` removed — but it is explicitly a negative-test fixture ("Do not build real worlds on this platform"), not a second supported platform.

### 1.4 Two execution tiers, not two platforms

Per the F10 architecture recorded on #5958: the **Wasm guest** tier (import-free Wasm, deterministic wasmtime fuel — `pkgs/id/src/sandbox.rs`) and the **admin-installed native** tier (`world_native.rs:1-15`, seccomp allowlist with no `open*` and no sockets, `no_new_privs`, rlimits, never peer-downloaded). Both run the same platform ABI over the same frames.

**Consequence for this design.** Nothing today can grant a world filesystem or network authority, at any tier. The vocabulary in §2 is therefore new surface, and the mechanism that gives a world *a different platform* (S14) does not exist — it is specified in §5.

## 2. Capability vocabulary v1 (S12)

One flat, dotted, never-reused namespace (the existing naming rules: ASCII, ≤64 bytes, meaning never changes). The four names the brief requires map onto four **tiers**, and the tier — not the individual capability — is what the default posture and the server admin approve.

| Tier | Names in v1 | Default | Grant authority | What it means |
| --- | --- | --- | --- | --- |
| `standard` | today's catalog, including `loc.*` sensor caps and the six platform modules | **on** (unchanged) | world ceiling + grant, as today | Server data, presentation, and sensor caps under existing consent rules. No filesystem, no network. |
| `sandbox.fs` | `fs.read`, `fs.write` | **off** | server admin enables the tier; world ceiling + grant narrow it | Read/write inside the world's own scratch directory only. Path is server-chosen; no `..`, no symlink traversal, no absolute paths. |
| `host.fs.read` | `fs.host_read` | **off** | server admin enables the tier *and* names each readable root | Arbitrary file read on the host, bounded to admin-named roots. This is the dangerous tier: it is a host-disclosure capability, not a convenience. |
| `net.out` | `net.connect` | **off** | server admin enables the tier *and* names allowed destinations | Arbitrary outbound network. v1 records "arbitrary"; a destination allowlist is the minimal safe form and is required at the admin surface. |

Rationale for tiers rather than four independent grants:

1. **The brief's four names are not peers.** "Standard" is the existing posture; the other three are expansions of it, in ascending danger. Tiers let an admin say "this server allows sandbox files, but never host reads or network" in one decision, and let the registry display the posture as one sentence.
2. **The C2 default is a tier default, not a per-name default.** Keeping it per-name invites drift (one new name quietly default-on); keeping it per-tier makes "default does not move" a property of the tier table, testable as one assertion.
3. **Future granularity fits underneath.** A later `net.connect:api.example.com` or `fs.read:<root>` is a narrowing *within* a tier, not a new tier; the vocabulary stays additive.

Deliberately **not** in v1: process/exec, raw device access, camera/microphone (these are the `media.*`/`form.*` surface of the capability taxonomy investigation, a separate track), and write access outside the sandbox. Naming them here as out of scope keeps "arbitrary file read" from being read as "arbitrary file write".

Every new name is a **host-request provider** in the existing shape (`{id, cap, args}` → `{"cap":"result", ...}`), so the wire protocol needs no version bump (`WANTS_VERSION = 1` already admits new names). The refusal codes are the existing six; the advisory reasons are the existing per-capability vocabulary plus `outside_tier` (the tier is not enabled on this server) and `path_outside_root` / `destination_not_allowed` for the two admin-bounded tiers.

## 3. Declaration vs discovery (S11, S13, S16)

Three sets, three different authorities, never conflated:

| Set | Who produces it | What it means | Trust |
| --- | --- | --- | --- |
| **requested** | the world's manifest (declared by the author) | "this world asks for these capabilities" | a request, not a grant; may over-ask |
| **discovered-used** | the derivation tool (#5998), static call-graph reachability over the closed platform namespace, per #5997 §1 | "these are the exact platform functions this release can call" | authoritative only when certification succeeded; never hand-written |
| **effective grant** | the server: `granted ∩ server ceiling ∩ world ceiling ∩ catalog`, existing `CapLedger::effective()` (`world_caps.rs:1358-1377`) | "this world may use these now" | authority |

**The two invariants** (both fail closed):

```
discovered-used ⊆ requested
discovered-used ⊆ effective grant
```

The first is checked at admission: a derived set wider than the declaration is a **manifest defect** (the author under-declared), refused with a diagnostic naming the extra names. The second is checked at admission and at run: a use outside the effective grant is the existing `cap_denied`, unchanged.

**Why the used set is derived and not declared.** #5997 §3 already settled this: an in-type signature cannot drift from the code, while a hand-written manifest can, and Roc has no effect rows to carry it. The mitigation is structural — humans declare the *ceiling* (requested), machines compute *usage* (discovered). This design adopts that split verbatim and does not re-open it.

### 3.1 Discovery mechanism: static first, profiling as corroboration

Answering the issue's first open question directly:

- **Static analysis is the certification mechanism.** Call-graph reachability over the closed platform-function namespace produces the discovered-used set. It is path-independent by construction: it enumerates every platform function reachable from the entry points, which is exactly the "same for every run" question the issue asks. Higher-order boundaries take the union of possible targets; where the target set cannot be enumerated, certification fails rather than guessing (#5997 §1, and #6000 §1's five certification-voiding features).
- **Data-encoded capabilities are inside the analysis surface.** Not every capability is exercised by *calling* a platform function. The ABI also accepts arbitrary strings from `program.wants` (`pkgs/id/examples/roc-world/platform.roc:56-57`), and existing worlds use that channel as data, not as calls: `pkgs/id/examples/territory/main.roc:114-115` subscribes to `players` by returning literal JSON without invoking any capability-specific platform function, and `RocCap` helpers likewise construct JSON rather than call host functions. A call-graph-only derivation would miss exactly these capabilities and either under-report the used set or leave ordinary worlds uncertifiable. The derivation must therefore analyze, conservatively, every value reachable code can emit through `program.wants` and RocCap-style JSON construction: any capability named by an emittable request/subscription value counts in `discovered_used`, exactly as if the corresponding platform function had been called. Where the emittable values are not a closed, statically enumerable set of literals (arbitrary string construction, concatenation from runtime input), certification **fails** with that reason — it never certifies a call-graph-only subset as the used set. (Restricting the ABI so requests are typed values rather than arbitrary strings would remove this failure mode; that is an ABI change, out of scope for this design, and is recorded here as the known cost of the current string channel.)
- **Profiling is corroboration, not certification.** A profiling run (platform-call instrumentation during execution) cannot prove absence — it only sees paths taken. It is retained for three jobs: (a) detecting **drift**, a runtime use of a name outside the certified set, which is a hard fault (§3.2); (b) **ranking** the used set by hotness for the observability surface (#6017), which is presentational only; (c) **test evidence** that a derived set is not vacuous.
- **Neither mechanism is alone sufficient**, so the design names both and their roles. A registry that showed only the static set would hide real-path usage; one that showed only the profile would present a path-dependent sample as if it were the whole truth.

### 3.2 Reconciliation when static and profiling disagree

| Case | Meaning | Action |
| --- | --- | --- |
| Profile ⊆ static | Normal. The profile exercised a subset of reachable calls. | None; the static set stays authoritative. |
| Profile uses a name **outside** the certified static set | The certification premise is broken (dynamic loading, reflection, an analyzer bug, or a stale artifact) | **Hard fault.** The world is quarantined: the use is refused (`cap_denied`, reason `outside_certified_set`), the world is marked **not certified** in the registry with the offending name and the run evidence, and re-certification is required before the world runs again. Never silently widen the set at runtime — that is exactly the OCaml-style runtime-discovery failure #5997 §4 rejects. |
| Static names the profile never called | Normal over-approximation (a branch not taken, a dead-but-reachable path). | None; the registry shows the static set and #6017 may annotate "not observed in the last N runs". The annotation never reduces the set. |

The asymmetry is deliberate: over-approximation is safe (the world looks more capable than it is), under-approximation is not (a capability was reached that nobody certified). Only the second is a defect.

### 3.3 Path-dependent used sets, presented honestly

The used set is a property of the **code** — the set of platform functions that *can* be called — not of a run. A world whose `init` reads a file only when a flag is set still *uses* file read; the fact that a particular run did not take that path is an observation, not a capability fact.

Presentation rules (binding on #6017):

- The registry's `discovered_used` is the static, path-independent set. A profile observation is a **separate field** (`observed_subset` or equivalent) and is never substituted for it.
- When the two differ, the surface says so in one sentence: "can use A, B, C; observed using A, B in the last N runs". It never says "uses A, B".
- When certification failed, the surface says **"used set unknown — not certified (reason)"**. It never renders an absent set as "uses none" (#6000 §2 makes this a hard rule, adopted here).

## 4. Registry schema for requested and used sets (S13, S16)

The canonical schema lives here; #6000's semantics and #6017's presentation consume it. Field names are settled enough to implement; the wire encoding is JSON, versioned, and additive.

### 4.1 One record per release identity

A registry entry describes an immutable **world release** (the artifact plus its transitive dependency closure), identified by the content-addressed identity of #5965 §2 and #5997 §2 (platform tree hash + source closure hash + DLL closure hash + compiler digest + target). It is *not* a per-world-row record: a world that republishes new code gets a new entry; a world restored from a checkpoint keeps its entry.

Certification content (`requested`, `discovered_used`, `custom_platform`) is a property of the release and lives here. **Effective grants and runtime observations are not fields of this record**: they are properties of a *world instance* running the release (§4.2.1). Two worlds on one server can run the same release under different grants and ceilings — the ledger and ceiling are stored per `WorldCore` (`pkgs/id/src/world/core.rs:145-153`) and updated per world (`world/actor.rs:1769-1780`) — so a single release-keyed `effective_grant` would overwrite one worlds authority report with anothers.

### 4.2 Shape

```json
{
  "v": 1,
  "release": {
    "identity": "<blake3 hex>",
    "platform": {"name": "roc-world", "tree_hash": "<blake3 hex>", "abi_version": 1},
    "compiler": "<blake3 hex>",
    "analyzer": {"version": "<semver>", "policy": "<id>"}
  },
  "requested": {
    "tiers": ["standard"],
    "caps": ["time.now", "chat.say", "Screen", "Key"],
    "declared_at": "2026-11-08T09:15:00Z"
  },
  "discovered_used": {
    "state": "certified | not_certified | stale | unknown",
    "reason": null,
    "partial": null,
    "caps": ["time.now", "chat.say", "Screen"],
    "provenance": {
      "method": "call_graph",
      "analyzer": "<semver>",
      "analyzed_identity": "<blake3 hex>",
      "at": "2026-11-08T09:15:00Z"
    }
  },
  "custom_platform": null
}
```

### 4.2.1 The world-instance record (grants and observations)

Effective grants and runtime observations are recorded per **world instance**, keyed by `(world_id, release identity)`:

```json
{
  "v": 1,
  "world_id": "<world instance id>",
  "release_identity": "<blake3 hex>",
  "effective_grant": {
    "tiers": ["standard"],
    "caps": ["time.now", "chat.say", "Screen", "Key"],
    "ceiling": ["time.now", "chat.say", "Screen", "Key"],
    "computed_at": "2026-11-08T09:15:00Z"
  },
  "observed": {
    "window": "last 24h",
    "caps": ["time.now", "chat.say"],
    "runs": 412
  }
}
```

Admission and presentation join the two records on `release identity`: the release record says what the code *can* use and what its author *asked* for; the instance record says what *this* world may use now and what it has been seen using. Neither record alone is an authority report, and a consumer must never read a release record as if it carried a grant.

Rules that make the shape honest:

- **`discovered_used.state` is the first thing a reader sees.** `caps` is **absent or `null`** unless `state == "certified"`. A `null` set with `state: "not_certified"` and a `reason` is the honest representation of "we do not know"; `[]` is reserved for a completed analysis that proved no platform calls, and the schema makes that distinction explicit rather than by convention.
- **`partial`** carries any known subset the analyzer could report despite failure, explicitly labelled `non_authoritative: true`. Consumers must not use it to reduce a grant (#6000 §2).
- **`requested.tiers` (release record) and `effective_grant.tiers` (instance record, §4.2.1)** carry the tier decision, so a reader can see "this world asks for `sandbox.fs`, the server did not enable it, its derived set therefore cannot include `fs.*`" without reconstructing the arithmetic.
- **`custom_platform`** is `null` for a world on the server's stock platform, or `{tree_hash, requested_key, compiled_at}` for a world on a synthesised platform (§5).
- **`release.analyzer`** is part of the identity of the analysis, not of the artifact: a change of analyzer version stales every entry it produced (`state: "stale"`), forcing re-analysis. This is the "no silently stale certification" rule.

### 4.3 What the schema deliberately does not carry

- **Runtime usage counters as a used set.** `CapsReport.usage` (`world_caps.rs:1226-1230`, `:1426-1454`) stays what it is: local, unsigned, journaled observation. It is an input to `observed`, never to `discovered_used` (#6000 §2 states the same rule).
- **Signatures.** The registry record is server-local state, like grants and ceilings (`world_caps.rs:61-64`). Signing a capability registry is not in v1; a receiving server re-derives rather than trusting a foreign claim. If cross-server registry sharing is wanted later, it is a transfer of *derivation evidence*, not of a trusted verdict.
- **History.** The entry is the current fact for a release identity; history/#6017's diffs are a later schema addition, explicitly deferred to #6017.

## 5. The custom-platform compile path (S14, S15)

### 5.1 What "custom platform" means here

A platform is a tree: `platform.roc` (which `exposes` the module list and `provides` the host functions) plus its `.roc` modules plus `targets/`. The stock tree exposes all six modules and, under C2, no file or network host functions. A world that needs `fs.read` cannot get it by importing harder — the platform must *provide* a host function that performs the read, and the OS sandbox must permit the syscall. So S14 is not "pick a different prebuilt platform" (there is only one, §1); it is **synthesise the minimal platform tree that provides exactly the requested capability set**.

### 5.2 The requirement set is the cache key

```text
requested_key = BLAKE3(
  sorted(requested.caps)            ||  # canonical capability-name set
  sorted(requested.tiers)           ||
  platform_abi_version              ||  # the ABI the host implements
  platform_base_tree_hash               # the stock tree this is derived from
)
```

Two worlds that request the same set on the same ABI and base tree get the **same** synthesised platform — this is the "caching and sharing across identical requirement sets" the issue asks for, and it falls out of content addressing rather than a bespoke registry. The synthesised tree is stored under its own tree hash and recorded in the release's `custom_platform.tree_hash`; the compile cache key of #5965 §2.1 already includes the platform tree hash, so artifact caching composes with no new mechanism.

Narrowing the key: a world requesting `{standard, sandbox.fs}` is `{standard}`-plus-`fs.*`; a world requesting `{standard, net.out}` is a different key. The key is the request, never the *used* set — a world that requests file access but provably never calls it still gets the file-capable platform, because "provably never calls it" is a property of analysis, not of the ABI it was compiled against. (Compiling against the minimal platform its used set proves sufficient is a possible optimisation; it is deliberately not v1, because it would make the artifact's platform depend on analysis results and complicate re-certification.)

### 5.3 Synthesis is a second compile, and someone pays

Building the tree runs the platform build (Roc host sources + Zig/C host) — a real compile, minutes not milliseconds. The design therefore inherits #5965's economics rather than inventing a second budget:

- **Who pays:** the **requesting author**, in compile credits (the compile-credit budget of #5965 §4, separate from runtime fuel). Building a platform is gated to the contributor/patron/admin tiers of #5965 §5; a world author who cannot pay for a novel platform still gets a shared one if someone else already built the same `requested_key` (cache hit; cheap or free, because nothing is compiled).
- **Who approves:** a synthesised platform is executable code on the server. Enabling **any** non-standard tier on a server is an admin decision; a server with all extension tiers off refuses to synthesise a tree needing one, and such worlds fail at admission with `outside_tier`. First-party-only publishing (C2) means, for now, only first-party authors can publish worlds that use these trees.
- **What is cached:** the synthesised tree (by `requested_key` and tree hash) and the compiled world (by #5965's key, which now includes that tree hash). Eviction follows #5965's tiers (scratch, server LRU, pinned); a pinned custom platform is a premium operation and is visible in the admin surface as such.
- **Determinism:** the #5965 §2.2 determinism gate applies unchanged — cross-author reuse of a synthesised platform requires a byte-identical-rebuild proof for the (compiler, base tree) pair; until then it is author-scope only.

### 5.4 Expansive platform, minimal claim (S15)

A world may be given a platform wider than it uses. The design keeps the two facts distinct at every step:

- The **platform** grants the requested tier(s); it is the compile-time ceiling.
- The **registry** displays the requested set and the discovered-used set side by side (§4.2), so "it asks for all these, but it only uses this subset" is the literal rendering.
- **Enforcement** uses the effective grant, which is the world's ceiling and grants — not the platform's width. A world on a file-capable platform whose ceiling does not name `fs.read` still gets `outside_ceiling` at run, exactly as today's ceiling rules require (`world_caps.rs:1302-1320`).
- **The used set never reduces authority.** It is a disclosure, not a grant; the point of showing it is to let a user judge what they are connecting to, not to auto-narrow what the world may do.

## 6. What this decides for the issue's open questions

| Open question (issue) | Answer | Owner if split |
| --- | --- | --- |
| Discovery mechanism; reconciliation when static and profiling disagree | Static call-graph derivation certifies; profiling corroborates. Profile-outside-static is a hard fault and a certification revocation; static-outside-profile is normal over-approximation. (§3.1–3.2) | Tool internals: #5998; exception features: #6000 |
| Is the used set code-fixed or path-dependent; how to present a path-dependent set honestly | It is a property of the code (the reachable set), so it is path-independent; observations are a separate field and are never substituted. (§3.3) | Presentation: #6017 |
| Custom-platform compilation: who pays, how long, cached/shared | The requesting author pays in compile credits (#5965); minutes-scale build; keyed by `requested_key` so identical requirement sets share one tree and its compiled worlds. (§5.2–5.3) | Compile economics: #5965 |
| How the requested/used pair interacts with writer-set authority (#5545) | Separate axis; a capability grant is not a writer-set grant. (§7) | Writer-set mechanics: #5545 |
| Do we have more than one platform? (S9) | No: one tree, one catalog, six modules, two tiers. (§1) | — |

## 7. The writer-set authority interaction (#5545)

Writer-set authority answers **"whose writes count in a view"** — a signed, versioned object that admits authors to a projection (`doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/`, §9 of `doc/2026-11-04_id-program-essay-answers.md`). Capability grants answer **"what may this code call"**. They are different questions about different subjects, and the design keeps them on separate axes:

- **A capability grant is not a writer-set grant.** An author admitted to a writer set gets no filesystem or network authority by that fact; a world granted `net.out` gets no say in whose writes count.
- **A writer-set grant is not a capability grant.** Adding an author to a writer set does not widen the world's requested/used/effective sets.
- **The one intersection is admission of a capability-bearing world.** Publishing a world that requests an extension tier is currently first-party only (C2); when that changes, the natural rule is that the **publisher's writer-set authority over the world's own definition** is what makes the declaration trustworthy — but that is a future decision, not v1. v1 states only that the two checks run independently: writer-set admission for *content*, capability admission for *execution*.

If a later design wants the pair coupled (for example, "only the world's writer set may raise its requested ceiling"), that is an explicit follow-on under #5545; this note does not couple them by default.

## 8. Implementation boundary and follow-ups

**This note decides; it implements nothing.** The slices that follow, in dependency order:

1. **Vocabulary and tier table** — add the tier grouping and the seven v1 names to `world_caps.rs`, with the tier default assertion (`standard` on, all others off) as the C2 regression gate. Small, no behavior change while all extension tiers are off.
2. **Registry record** — the §4.2 shape, written by the compile/admission path; `discovered_used` starts as `unknown` until slice 3 lands.
3. **Derivation tool** — #5998, producing certified sets that fill slice 2.
4. **Admission checks** — the two invariants of §3 plus `outside_tier`; runs must keep working with `discovered_used: unknown` (fail-open for display, fail-closed for any extension-tier use without a certified set).
5. **Custom-platform synthesis** — §5, gated on #5965's cache and credit machinery.
6. **Observability** — #6017's surface over the slice-2 record, including the §3.3 presentation rules.

Filed as follow-ons because they are separate decisions, not because they are unclear: [#5998](https://github.com/developing-today/code/issues/5998) (derivation tool), [#6000](https://github.com/developing-today/code/issues/6000) (certification exceptions, PR [#7025](https://github.com/developing-today/code/pull/7025) open), [#6017](https://github.com/developing-today/code/issues/6017) (observability), [#5965](https://github.com/developing-today/code/issues/5965) (compile economics, `doc/2026-11-05_design_id5965.md`), [#5545](https://github.com/developing-today/code/issues/5545) (writer-set authority).

## 9. Risks and what would change this design

- **Tier granularity is a policy choice.** If admins want per-capability enablement without tiers, the table collapses to per-name defaults and the C2 assertion becomes a list. The registry schema is unaffected.
- **`host.fs.read` is the sharp edge.** The design deliberately makes it admin-root-bounded and tier-gated; if review concludes even that is too much for v1, drop the name and keep the tier empty rather than weakening the bounds.
- **Static certification depends on a closed namespace.** Any future platform feature that permits dynamic platform-function resolution voids certification by definition (#6000 §1); the design is built to say "unknown" rather than to guess.
- **The custom-platform cache key is the request, not the usage.** This trades some disk (platforms built for requests that under-use them) for a stable, analysis-independent artifact identity. If that trade is rejected during review, the alternative is usage-minimal platforms, at the cost of re-certifying whenever analysis changes.

## Sources (read at this checkout, commit `5ab0c06928`)

- `pkgs/id/crates/id-core/src/world_caps.rs` — `CATALOG_ENTRIES` (`:233`), six `PlatformModule` entries (`:454-501`), `DefaultCeiling` (`:119`), `CapLedger::effective` (`:1358`), `CapsReport` (`:1426`), sensor caps (`:1040`, comment `:55-59`), local-not-signed note (`:61-64`).
- `pkgs/id/src/world_compile.rs` — `platform_dir` and validation (`:49-70`), `platform_modules` (`:93-107`), `check_imports` (`:111-146`), platform staging (`:229`, `:559`), compile limits (`:26-35` in the #5965 record's citation).
- `pkgs/id/src/world_native.rs` — seccomp allowlist, no `open*`/sockets (`:1-15`, filter `:569`).
- `pkgs/id/examples/roc-world/platform.roc` — `exposes [Screen, Key, RocJson, RocGrid, RocCap, RocText]`; `platform-no-caps.roc` — negative-test fixture.
- `doc/2026-11-05_design_id5997.md` — derivation as call-graph reachability, declaration/derivation split, rejection of runtime-discovered capability checks.
- `doc/2026-11-05_design_id5965.md` — cache key, determinism gate, compile credits, tier gating.
- [PR #7025](https://github.com/developing-today/code/pull/7025) — certification exception policy; defers the canonical schema here.
- [Issue #5958](https://github.com/developing-today/code/issues/5958) — C2 ruling (2026-11-04 23:57 CST) in the issue comments.

## Change log

- 2026-11-08: Initial design record for #5958 (queue task 12856, lane 7b47536b-c162-444c-be8a-3a9538cc4cb5): S9 inventory, vocabulary v1, declaration vs discovery, registry schema, custom-platform path, writer-set axis.
