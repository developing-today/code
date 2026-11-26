# Design: dynamic user functions — interface contract, compile pipeline, and v1 placement

- **Date:** 2026-11-16
- **Status:** design; no implementation is included here.
- **Issue:** [#5964](https://github.com/developing-today/code/issues/5964), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955). Statements S36–S38 and S45 of the 2026-11-04 brief.
- **Mechanism decision (settled, not re-opened here):** [#5996](https://github.com/developing-today/code/issues/5996) decision record, landed by PR [#7637](https://github.com/developing-today/code/pull/7637) (merged 2026-11-16): **v1 is instance-per-function restart** — a user function is a separately compiled Wasm module; a swap discards the old Wasmtime instance and instantiates the new module fresh at a tick boundary, with state handed over through the host's snapshot/restore seam.
- **Evidence this design is reframed by:**
  - [#5995](https://github.com/developing-today/code/issues/5995) Roc shared-library evidence (`doc/2026-11-11_roc_library_evidence_5995.md`, PR [#8478](https://github.com/developing-today/code/pull/8478)): the tracked Roc compiler has **no library/DLL output at all** (binary or Wasm outputs only); the legacy compiler's `roc build --lib` produces a `libapp.so` that is a **startup dynamic-link dependency**, not an in-process swap mechanism, and neither compiler line offers any runtime load/unload/swap support. **No part of this design rests on a DLL premise.**
  - [#5966](https://github.com/developing-today/code/issues/5966) per-function gas/timeout research (closed): per-function fuel budgets are the existing `sandbox.rs` metering model at per-function scope, inside the world's per-tick budget ([#5962](https://github.com/developing-today/code/issues/5962)).
  - [#5967](https://github.com/developing-today/code/issues/5967) per-user function mapping (the consumer): switches are rebinds that take effect only at tick boundaries; state survives a switch only by explicit handover.

## 0. What this settles, and what v1 defers

This doc fixes the five things #5964's Done criteria name: the interface contract (§1), the server compile pipeline (§2), placement/binding under the v1 mechanism (§3), swap semantics with failure fallbacks and isolation (§4), and the trust argument end-to-end (§5).

**The v1 mechanism, stated once:** publish compiles a new per-function `wasm32` module through the server pipeline; the host instantiates it and runs its init; only on success does it become the function's active version, at the next tick boundary; in-flight calls finish on the old instance; state crosses versions only through the snapshot seam. The mismatch/failure default throughout is #5964's stated one: **keep running what you'd already published.**

**Deferred past v1** (per the #5996 record's reopen cases, not silently dropped): component-model dynamic instantiation (the likely v2 if WASI dynamic linking matures — strictly better link-time mismatch errors, but the host has no component runtime today); per-function native processes (the native-worker tier is the only process-shaped target); any in-process native DLL swap (foreclosed by the #5995 evidence, not merely deferred).

**Not this:** the authoring hot-swap loop (`id world dev`, [#558](https://github.com/developing-today/code/issues/558) / [#5541](https://github.com/developing-today/code/issues/5541)) swaps _world_ code for its _author_ during development. This design swaps _per-user functions inside a live world for its players_. Where the mechanics coincide (compile → instantiate → activate) they are the same host seams; the trust model (§5) and the budget model (§4) are different, and neither is borrowed from the authoring loop.

## 1. Interface contract

### 1.1 Shape

A user function is a **per-function core-Wasm module** compiled against a **function platform** the world declares — the narrowing of `world_compile.rs`'s whole-world compile to function scope that the #5996 record calls for. The module's contract with the host is:

- **Exports (guest → host):** an init entry that builds the function's initial model, and a call entry that takes (model, input) and returns (model, output) in the platform's canonical encoding. The function is pure over its model: everything it can observe arrives as call input or through a granted platform capability (§5); it holds no host references between calls except its model.
- **Imports (host → guest):** exactly the platform capabilities the world author granted this function (§5). Imports are checked at compile/install time against the granted set (the existing `check_imports` posture in `world_compile.rs`); an import outside the granted set is a compile/install rejection, never a runtime surprise.
- **State:** the function's state is a single opaque **state blob** owned by the host's snapshot seam (`plaza_snapshot` / `plaza_restore` in `sandbox.rs`, which already rebuilds a freshly instantiated guest's model from a snapshot). The function's code never serializes its own state for handoff; the host snapshots and restores (§3.3).

### 1.2 Identity: name, version, interface hash

A published function version is identified by three coordinates, all recorded at publish time:

- **Name** — the binding name in the per-user function mapping (#5967): the world-side name the mapping resolves.
- **Version** — the publish sequence for that (user, name): monotonically increasing, assigned by the server at publish. A version is immutable once published; an edit is a new version.
- **Interface hash** — BLAKE3 over the canonical interface descriptor: the function's signature (call input/output shapes), the state-blob schema identifier, and the granted capability set. The interface hash is part of the artifact's identity alongside the existing provenance scheme: `module_hash` (BLAKE3 of the module bytes) and the compile provenance triple (source, platform, compiler digests) from `world_compile.rs` / `world_session`.

Discovery is by the mapping, not by the module: world-side code asks the mapping for (user, name) and receives the active version's coordinates. A module is never discovered by scanning artifacts, and a caller never names a module hash — hashes are for audit, replay, and cache keys, not for binding.

### 1.3 Mismatch behavior

Mismatch is an **activation-time** verdict, never a call-time one:

- A candidate version whose interface hash differs from the interface hash the binding expects is **not activated**. The previously published version keeps running — the stated default — and the publish is recorded as rejected with the two hashes, so the publisher can see exactly what diverged.
- There is no coercion, no adapter synthesis, and no partial activation: either the candidate matches the expected interface hash and the full §3 sequence runs, or nothing changes.
- A world that _wants_ a new interface publishes a new binding expectation first (a mapping change, itself versioned); only then can a candidate carrying the new hash activate. Interface evolution is therefore always an explicit two-step, never a side effect of a code publish.

## 2. Server compile pipeline

### 2.1 Settled constraints

- **C3 (source-only, server compiles):** users submit Roc **source**; the server compiles it. Pre-compiled user artifacts are never accepted, on any path — this is the root of the trust argument (§5).
- **Compiler line:** the **tracked** compiler, `wasm32` target. The #5995 evidence forecloses the alternative readings: the tracked compiler has no library output to build on, and the legacy compiler's `libapp.so` is startup linking (code replacement = new file + new process), which is a different feature than this one. The design therefore targets the Wasm output the tracked compiler already builds, and the host layer — not the toolchain — owns all swap semantics, exactly as the evidence doc concludes.

### 2.2 Pipeline

A publish runs the existing `world_compile.rs` pipeline at per-function scope:

1. **Intake limits** (inherited from the world pipeline as the v1 starting point): ≤8 files, 128 KiB/file, 256 KiB total, 120 s compile ceiling. Per-function tightening is an implementation measurement, not a design question.
2. **Compile** the source against the world's declared function platform for `wasm32`.
3. **Import check** against the function's granted capability set (§1.1).
4. **Identity:** compute `module_hash` and record the provenance triple (source digest, platform-tree digest, compiler-binary digest — `digest_sources` / `digest_tree` / `digest_file`).
5. **Interface hash** computed from the canonical descriptor (§1.2) and checked against the binding expectation (§1.3).
6. **Cache:** the artifact enters the **persistent, content-addressed compile cache** keyed by (provenance triple, target, interface descriptor) — the #5996 record's one real gap, since today's cache lives inside each job directory and nothing is warm across compiles. A cache hit never recompiles; a cache entry whose digest check fails is recompiled, never trusted.
7. **Hand to placement** (§3). Compilation never disturbs the running version: every failure in steps 1–6 is a rejected publish with the running version untouched.

## 3. Placement and binding under v1 instance-per-function restart

### 3.1 Publish, then swap

Following the host's existing atomic-install pattern (`WorldService::install_wasm`: compile, instantiate, and only then activate — "a failed compile or write leaves the running program unchanged"), narrowed to per-function scope:

1. The candidate module is **instantiated fresh** and its init runs.
2. On success — and only on success — the candidate is marked the function's **active version**, effective at the next tick boundary.
3. On any failure (compile, instantiation trap, init trap, interface mismatch per §1.3) the candidate is discarded and the previously published version continues, per the stated default.

### 3.2 Tick boundary, in-flight calls

- Activation happens **only at a tick boundary, never mid-tick** (the #5967 mapping model). The replay artefact stays small and auditable: (module hash, snapshot, tick inputs).
- Invocations already in flight at the boundary **finish on the old instance**. The old instance is discarded after the boundary once its in-flight calls drain; there is no in-place mutation of a live instance, ever.
- A swap is a rebind in the #5967 mapping: from the boundary onward, (user, name) resolves to the new version's instance.

### 3.3 State handoff — host-owned

- **Same function, new version:** the host snapshots the old instance's state blob and restores it into the fresh instance (`plaza_snapshot` / `plaza_restore`). A failed restore **rejects the swap**: the old version continues with its state intact, and the rejection is recorded. State handoff is the **host's** responsibility — not the function's (it cannot serialize what it cannot observe) and not the world's (the world owns the binding, not the blob).
- **Different function bound to a name:** the #5967 rule governs — fresh init, unless the user explicitly hands a state blob over. Schemas are never assumed shared between functions.

## 4. Failure fallbacks and isolation

The isolation unit is the **instance**: a fresh Wasmtime instance per function version, run under the existing `sandbox.rs` posture (fuel metering per guest call, memory limits, trap/fuel-exhaustion handling).

- **Crash (trap):** a trapping call loses its turn; the instance is poisoned and discarded. The world and every other function continue. The next tick re-instantiates from the last good version + its last good state snapshot.
- **Hang:** bounded by fuel, not by wall-clock hope: each function instance runs under **its own fuel budget** (the #5966 metering model), inside the **world's per-tick budget** (#5962). Fuel exhaustion is a trap-class event with the same handling as a crash. A function cannot consume another function's budget or the world's reserve.
- **Publish-time failures** (compile error, instantiation/init trap, interface mismatch, state-restore failure): §3.1 — the previously published version keeps running; nothing about the live world changes. This is the stated mismatch default applied uniformly: **keep running what you'd already published.**
- **No shared fate:** functions share no instance, no interpreter, and no metering pool. The one shared resource is the world's per-tick budget, and it is enforced as a ceiling over per-function budgets, not as a common pot a single function can drain.

## 5. The trust argument, end-to-end

1. **Provenance — the server compiled it.** User code arrives as source and is compiled by the server's own pipeline (§2). The server trusts the artifact because it produced it, and the artifact's identity (module hash + provenance triple: source, platform, compiler digests) makes that claim checkable after the fact. Where configured, installed modules additionally carry an Ed25519 `ModuleSignature` over the domain-separated digest, checked against configured module trust keys (`world_session/types.rs`).
2. **Purity by construction.** A function is pure over its model (§1.1) except for the imports it was granted. Imports are checked against the granted set at compile/install; an ungranted capability is not merely unused, it is **absent** — there is no ambient authority for compiled code to reach for.
3. **Capabilities are the world author's, passed deliberately.** The only non-pure things a user function can do are the **platform capabilities the world author passed to it**. The author grants per function, and the grant is part of the interface hash (§1.2), so changing a grant is a visible interface change, not a silent privilege drift.
4. **The author's trust is transitive and inspectable.** The world author trusts the chain because every link is theirs or the server's: their platform defines the capabilities; the server's compile is the only path from source to artifact; the mapping (#5967) is the only path from name to version. A player publishing a function cannot widen its capabilities, substitute a foreign artifact, or activate an interface the binding does not expect (§1.3) — each of those is a structural absence, not a policy check that could be skipped.
5. **Determinism closes the loop.** Given (module hash, snapshot, tick inputs), a tick replays exactly — module identity is content-addressed and snapshot/restore is the existing replay seam — so a disputed outcome is re-derivable from recorded coordinates alone.

## 6. Follow-ups this design implies (for filing from #5964, not designed here)

- Per-function compilation against a function platform (the §1 contract as a platform definition).
- The persistent content-addressed compile cache (§2.2 step 6).
- Per-function instances, publish/swap bookkeeping, and pending-activation state in the session layer (§3).
- The interface-descriptor canonicalization that the interface hash (§1.2) is computed over.
- Implemented: the interface-descriptor canonicalization and interface hash (§1.2) with the §1.3 activation check, as `FunctionInterface`, `FunctionBinding` and `check_activation` in `pkgs/id/crates/id-worlds/src/world_compile.rs`; this covers the item above.
- Implemented: the persistent content-addressed compile cache for function compiles (§2.2 step 6), as `compile_function_cached` in `pkgs/id/crates/id-worlds/src/world_compile.rs`, keyed by the provenance triple, target and interface hash in the shared `WorldCache` (`pkgs/id/crates/id-worlds/src/world_cache.rs`, domain-separated from world keys); this covers the cache item above for per-function compiles.
- Implemented: the publish/swap bookkeeping and pending-activation state (§1.2 versions, §1.3 versioned expectations and recorded mismatches, §3.1–§3.3 publish then swap at the tick boundary, rejected publishes recorded, a failed state restore rejecting the swap), as the pure `FunctionRegistry` state machine in `pkgs/id/crates/id-worlds/src/world_functions.rs`; real per-function instances, snapshot/restore wiring and per-function fuel remain with the per-function instances item above.

Implementation of the mechanism is explicitly out of this doc's scope, per the task packet.
