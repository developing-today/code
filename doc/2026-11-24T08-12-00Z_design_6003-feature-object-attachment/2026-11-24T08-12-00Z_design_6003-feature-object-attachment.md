# Design: runtime feature-object library attachment (#6003)

Date: 2026-11-24T08-12-00Z
Source issue: https://github.com/developing-today/code/issues/6003
Program: #5955 (worlds design program) · Topic: #5964 (dynamic user functions)
Prior deliverables: design note 2026-11-10 (task 13815, on #6003); adoption decision 2026-11-22 (archer, lane S, under FDR-0013); implementation follow-on #12640 (open)
Status: design record to the sufficient-design standard (9 points). No implementation in this record.

## 1. Problem + context

Entities in a running world cannot gain library behavior without recompiling the consumer. Today a world program is replaced atomically as a whole (`pkgs/id/crates/id-worlds/src/world/actor.rs` — replacement `WorldProgram` install path; `WorldProgram` is a single active-program interface), and whole-module compilation is the only composition path (`CompileSpec` whole-module compile; cf. `pkgs/id/tests/provider_registry_exit_proof.rs`, `pkgs/id/src/lib.rs` re-export of `id_worlds::world_compile`). There is no partial library attach/detach operation.

The LambdaMOO precedent (https://www.moo.mud.org/moo-faq/lcore-getting-started.html, cited in #6003) shows the missing shape: feature objects as runtime-attached libraries — create a reusable object implementing verbs, attach it to any entity, gain those commands with no consumer recompilation.

This design is distinct from the #5955 taxonomy child (attachment as classification) and from #5964 (per-user _functions_ via server-compiled DLLs/hot-swap). This is per-entity _behavior_ attach/detach inside a running world. The compile-economics design #5965 (closed via #6230) is an input (DLL cache/linkage), not an implementation of runtime attachment.

## 2. Goals and non-goals

Goals:

- Decide whether feature-object attachment is a v-next mechanism (decided: YES, 2026-11-22).
- Settle attach/detach authority and timing, in-flight invocation semantics, capability composition, and state retention on detach/version change.
- Keep the mechanism privilege-safe and tick-consistent.

Non-goals (this record does not authorize):

- Implementation (owned by #12640, citing this note).
- Compile-time composition changes or whole-program replacement semantics.
- Unrestricted attach, per-user function hosting (#5964), taxonomy/classification, licensing/SPDX tooling (#5979), presentation assets.

## 3. Alternatives considered

1. **Do nothing (compile-time-only composition).** Rejected: preserves whole-recompile for every behavior reuse; contradicts the worlds library model. Kept as the fallback if v-next is later reversed.
2. **Unrestricted runtime attach (anyone may attach anything, anytime).** Rejected with reason: privilege amplification (borrowing a library borrows its author's privileges) and tick-inconsistency (behavior pulled from under running verbs).
3. **Server-compiled DLL hot-swap only (#5964 path).** Valuable but different layer: per-user functions, not per-entity behavior libraries. Kept as complementary; alignment required on the DLL contract, not substitution.
4. **Adopted: constrained runtime attach (this design).** Tick-boundary, authorized, intersection-only capabilities, namespaced inert state. Matches LambdaMOO precedent with modern capability ceilings.

## 4. Chosen approach

Interfaces/contracts (to be fixed in #12640; semantics frozen here):

- `attach(entity, library, version, authorizer)` — takes effect at the next tick boundary only, as does detach; atomic, so failure leaves no partial state visible. Authorizer must be the entity owner or a world author holding an explicit attach capability on that entity class.
- `detach(entity, library)` — takes effect at the next tick boundary only.
- Attachment set snapshot at tick start: both attach and detach apply only at tick boundaries; in-flight invocations finish under the tick snapshot they started with. Neither operation changes behavior for a running verb.

Data model:

- Library state namespaced per `(entity, library, major_version)`, kept but inert on detach or version change (no silent migration, no silent drop; migration is an explicit later operation).

Security/capability analysis:

- Intersection-only composition, checked at attach time: attached library may exercise only `cap(entity) ∩ cap(world ceiling)`. Borrowing a library never borrows its author's privileges.
- Attach that would widen the entity beyond the world ceiling is refused, naming the widening capabilities.
- Must agree with #5958 (capability composition) and invocation budgets with #5966; must align with the broader DLL contract in #5964 once Roc shared-library feasibility #5995 and hot-swap alternatives #5996 land (both open at triage time; V3 staging under #5955 — this design does not block V1/V2).

## 5. Dependency-ordered build slices

For #12640 (implementation), in order:

1. Attach/detach authority + tick-boundary snapshot + atomicity; acceptance: owner/author-only attach, both attach and detach effective next tick, in-flight verbs finish under start set.
2. Attach-time capability intersection check with named refusal; acceptance: widening attach refused with capabilities named; no amplification.
3. Namespaced `(entity, library, major)` state with keep-inert on detach/version change; acceptance: detach/version-change keeps state inert, explicit migration only.
4. Conformance with #5958/#5966/#5964 contract alignment; acceptance: composition and budget checks agree, DLL contract cited.

Each slice is one lane/session with its own acceptance criteria above. Compile-time composition and unrestricted attach stay out of scope.

## 6. Rollout + rollback

- Rollout: design record first (this file) → link from #6003 → implementation proceeds in #12640 slice order under #5955 V3 staging (after #5995/#5996 inform the approach).
- Rollback: mechanism is v-next; nothing shipped changes. Reversal is a later design note or PR under #5955, or Drew's decline comment on #6003, superseding the 2026-11-22 adoption. Detach path already keeps state inert, so rollback is detach + refuse new attaches.

## 7. Verification plan

- Design-level: read back this doc from #6003; confirm all 9 standard sections present, slices dependency-ordered with per-slice acceptance, open questions naming deciders.
- Implementation-level (#12640): tests covering attach-time capability intersection, in-flight invocation behaviour at detach, and state handling on version change; out-of-scope paths (compile-time composition, unrestricted attach) have no tests because they are rejected.

## 8. Open questions (named deciders)

- Roc shared-library feasibility (#5995) and hot-swap alternatives (#5996) outcomes — decider: owners of #5995/#5996, consumed by #12640 implementer.
- DLL contract alignment with #5964 — decider: #5964 owner with #5955 program.
- Capability-composition agreement with #5958; invocation-budget agreement with #5966 — deciders: respective issue owners.
- Program-direction reversal, if ever — decider: Drew on the #5955 program (FDR-0013: fleet records, Drew can supersede).

## 9. Record location and links

- This file: `doc/2026-11-24T08-12-00Z_design_6003-feature-object-attachment/2026-11-24T08-12-00Z_design_6003-feature-object-attachment.md`
- Source issue: #6003 (link back posted as a comment in the same pass).
- Implementation: #12640 (already open, cites #6003 and this note).
- Prior notes: [2026-11-10 design note, task 13815](https://github.com/developing-today/code/issues/6003#issuecomment-6779453926) and [2026-11-22 adoption decision](https://github.com/developing-today/code/issues/6003#issuecomment-7082893313); triage 2026-11-17 (task 18419).
- Landing provenance (lane code-i, 2026-11-24): copied the pre-existing untracked draft from the primary checkout without changing that original; clarified both operations' tick timing from the November 10 note. This PR completes #6003's design-only acceptance criterion; implementation remains in #12640.

## References

- #6003, #5955, #5964, #5958, #5966, #5995, #5996, #5965/#6230, #12640, #13873/#13871.
- `pkgs/id/crates/id-worlds/src/world/actor.rs` (atomic program replacement; no partial attach path).
- `pkgs/id/src/lib.rs` (`world_compile` re-export); `CompileSpec` whole-module compile path.
