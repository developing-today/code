# FDR-PMO-003 — Lifecycle, claim compatibility, and lease fencing

- **Date:** 2026-11-13
- **Status:** accepted — local design contract only, not production authorization
- **Adopted:** Summit owner, task 15381, 2026-11-13; see `../reviews/component-contracts.md`. Unresolved #9178/#9179 rulings are not settled here.
- **Owner:** Summit design owner, task 15381 / session `ses_e5e2d10e06effeEylSDDVaN0PL` for this proposal; future maintenance by Fleet maintainers through #9180.
- **Decision maker:** Delegated Summit architecture owner for this design pass; adoption follows independent final design review by the parent and Fleet owner recording through #9180. #9178/#9179 owners retain their existing issue rulings; no human name is inferred.
- **Scope:** lifecycle/readiness/ownership separation, atomic claim, lease generations, compatibility with claim and park policy
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-PMO-001, -002, -006, -007

## Context and problem

The desired lifecycle CLI composes existing Fleet queue operations; it is not authority to rename stored states or bypass queue owners. At the census pin, queue-oriented commands and worker/lease metadata exist, but a complete lifecycle interface and monotone fence are not established. #9178 governs claim/manual takeover/resume semantics; #9179 governs park semantics. Persistent `manual_only` is an eligibility gate, not Do Now. Claim selection must be atomic with current eligibility; session creation and task binding are separate effects.

Sources: [architecture, “Identity and authority,” “Ordering and distributed priority,” and “Lifecycle and cross-system reliability”](../architecture.md); [source 02 §§3, 10, 13–16](../sources/02_charter_v1_1.md); [Track B census, “Finding,” “Evidence anchors,” and “Deliberate boundaries”](../census/track-b.md); [#9206, “Domain state,” “Pinned executable baseline,” and “Selection and peer-order boundary”](../components/9206-work-item-lifecycle-fleet-subcommands.md); [#9209, “Claim semantics,” “Fence boundary limitation,” and “End-to-end states/errors”](../components/9209-leases-fencing-outbox-direct-completion.md); [prior art B, “Work identity, lifecycle, and CLI surface” and “Leases and fencing”](../research/track-b-state-of-art.md).

## Options considered

1. **Extend existing queue lifecycle and claim semantics; add reviewed fence/version checks where compatible (recommended).** Reuses current command/storage authority and preserves #9178/#9179 rulings.
2. **Replace queue states and claim model with a new lifecycle service.** Could regularize the domain but creates competing authority and broad migration risk.
3. **Keep worker-ID/expiry lease only.** Minimal change, but an expired worker can still write after reclaim; expiry alone is liveness, not stale-write safety.
4. **Adopt a new workflow runtime.** Durable orchestration could help long-running sagas but does not fence GitHub and adds platform/migration complexity without evidence it is required.
5. **Do nothing.** Leaves lifecycle composition and stale local mutation risks unresolved.

**Tradeoff comparison:** Extending the broker minimizes architecture and keeps claim reliability local, while serial writes may bound throughput and require atomic transaction support. A new lifecycle service/runtime offers independent scaling and richer retries but increases deployment, consistency, and migration complexity without fencing remote APIs. Lease-only is simplest/fastest but fails stale-writer safety. Do-nothing avoids change cost while retaining the known correctness gap. Fence generations improve extensibility of local operations, not remote guarantees.

## Proposed operational decision

Keep four distinct dimensions: lifecycle/workflow, readiness/eligibility, ownership/execution evidence, and provider-native remote state. Map current queue vocabulary loss-aware; do not silently substitute proposal states for stored states. `completed` requires the designated policy and verified terminal receipt, not a queue label alone.

At claim, selection and mutation occur through one broker-authoritative transaction. Revalidate task requirements **and** caller filter, hard dependencies, active lease/concurrency, ownership/manual-only policy, item revision, graph revision, active policy/profile revision and relevant lane inputs against the intended snapshot using broker-authoritative time for time-sensitive rules. A changed input triggers recompute/reselection or typed stale result; if adapter cannot do this atomically, it cannot advertise the capability. Do Now bypasses selection delay only; it does not bypass hard gates, lease, concurrency, or manual-only policy.

Where approved and supported by the storage owner, use monotone per-work-item fence generation for local writes. Expiry allows takeover but does not stop a process; every local mutation checks current generation/version transactionally. GitHub does not accept Fleet fence tokens; remote effects require their own preconditions and operation receipts (FDR-PMO-006). Do not infer token/fence semantics from current worker ID. A `manual_only` marker excludes automatic claim; it does not prohibit an explicit, authenticated owner/manual claim where the current #9178 policy permits that override. Do Now alone never clears `manual_only` or supplies that permission.

## Rationale and evidence boundary

**Facts:** Track B identifies exact queue command surface and stored states at pinned SHA and notes worker/lease fields but no proven fence token/generation. Existing claims are atomic conditional selection/update in cited queue code; issue #9178/#9179 are behavior anchors. This record has not re-adjudicated those live issue rulings or deployment.

**Inference:** extending the existing broker avoids split-brain and permits a small transition model. Monotone local fencing addresses stale local writes; it cannot provide remote exactly-once or stop already in-flight external calls.

## Consequences, compatibility, and migration

- First reconcile the exact #9178 claim/manual/takeover/resume rules and #9179 park behavior; unresolved semantics remain upstream, not silently settled here.
- Maintain current queue-state compatibility. Additive fence/schema changes wait on #3424/#8575/#8579 and QTI provenance/#9241 gate; legacy rows must not receive fictitious history or token values.
- Claim boundary requires whole-input validation, including graph/policy revision; projections and sampled candidates never reserve work.
- No live queue status/schema change, new service, migration, deployment, or security credential is authorized here. A future versioned loss-aware work projection/enum is not prohibited by this FDR; it must remain separate from renaming stored queue values and pass the #9178/#9179/schema compatibility gates.
- Batch caps, lease duration, takeover timing, and DB clock guarantees remain unset unless established by the existing owner and measured environment.

## Negative cases and test oracles

- **MBT oracle:** two concurrent claimers for one unchanged candidate produce one holder; second reselects or receives typed conflict.
- **Conjunctive filter oracle:** each of requirement-only pass/filter-fail and filter-only pass/requirement-fail is unclaimable.
- **Snapshot race oracle:** change item, graph, policy, owner, lease, or deadline between selection and claim; stale candidate cannot be returned as claimed.
- **Fence invariant:** after generation N+1 takes over, generation N cannot mutate local state, notes, relation, cursor, or lifecycle; late remote effect is recorded only against its operation.
- **Compatibility oracle:** #9178 manual/takeover/resume and #9179 park fixtures remain true; manual_only is not bypassed by Do Now.
- **Clock oracle:** broker time, not client clock, determines expiry eligibility; clock uncertainty fails conservatively.

## Revisit triggers

Reopen if #9178/#9179 owner rulings change, actual storage cannot support required atomic validation, measured lease/claim contention exceeds existing broker capability, or QTI identifies an incompatible claim schema. Alternative coordination runtime requires measured unmet reliability/scale need and a migration/authority plan.

## Implementation, research, and source links

- Existing issues: [#9178](https://github.com/developing-today/code/issues/9178), [#9179](https://github.com/developing-today/code/issues/9179), [#9206](https://github.com/developing-today/code/issues/9206), [#9209](https://github.com/developing-today/code/issues/9209), [#9198](https://github.com/developing-today/code/issues/9198), [#9241](https://github.com/developing-today/code/issues/9241).
- Existing granular component keys: #9206 `CLI-01/04/05`; #9204 `SEL-01/02`; #9209 `REL-01/02`. QTI/schema compatibility remains an external gate, not another local behavior key.
- Pinned baseline: `a038f084dbeb525494d7bb928ad91f597db2ac51`; see Track B census, not deployment proof.
