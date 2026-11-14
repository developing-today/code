# FDR-PMO-002 — Versioned priority profiles, ordering, and provenance

- **Date:** 2026-11-13
- **Status:** accepted — local design contract only, not production authorization
- **Adopted:** Summit owner, task 15381, 2026-11-13; see `../reviews/component-contracts.md`. #7030 is consumed, not re-ratified.
- **Owner:** Summit design owner, task 15381 / session `ses_e5e2d10e06effeEylSDDVaN0PL` for this proposal; future maintenance by Fleet maintainers through #9180.
- **Decision maker:** Delegated Summit architecture owner for this design pass; adoption follows independent final design review by the parent and Fleet owner recording through #9180. No human name is inferred.
- **Scope:** priority profile/versioning, deterministic selection order, input precedence, provenance, cross-target projections
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-PMO-001, -003, -005, -008

## Context and problem

Priority is distributed across human rulings, label/comment commands, queue metadata, dependencies, claimers, PM projections, and hosted targets. A score alone cannot encode importance, effective pressure, workflow, ownership, manual eligibility, or one-shot dispatch. The historic 450/High and 1000/default proposal must not silently replace live values. #7030 already ratified `expedite=15` and `next-up=25`; the priority adapter's nearest-value mapping currently changes those values (`15→10`, `25→20`) and is a compatibility defect, not permission to reratify.

Sources: [architecture, “Identity and authority” and “Ordering and distributed priority”](../architecture.md); [source 01 lines 39–43, 51–58, 71](../sources/01_handoff_pm_stack_fleet_orchestration.md); [source 02 §§3–9](../sources/02_charter_v1_1.md); [Track A census, “Track A evidence matrix”](../census/track-a.md); [#9199, “V1 — observed baseline,” “Independent dimensions,” “Candidate order and rank semantics,” and “Triage mechanics”](../components/9199-distributed-priority.md); [#9200, “V1 — observed producer/consumer and state baseline,” “Authority and precedence table,” and “Replay/version semantics”](../components/9200-priority-distribution.md); [#9204, “V1 — observed baseline” and “Eligibility and deterministic normal selection”](../components/9204-queue-selection-bounded-exploration.md); [prior art A, “Four distinct architecture shapes”](../research/track-a-state-of-art.md).

## Options considered

1. **Versioned named profiles, canonical intrinsic values, derived effective pressure, explicit tie rank (recommended).** Preserves meaning and permits deterministic comparable snapshots without forcing unique scores.
2. **One unversioned numeric scale with producer-side normalization.** Simple at first, but current nearest snapping already demonstrates silent semantic drift; cannot represent independent dimensions.
3. **Multi-master or last-arrival-wins priority.** Easy ingress, but clocks/delivery order do not establish human intent or field authority.
4. **Full event-sourced priority aggregate now.** Excellent replay history, but larger reducer/version/retention and projection cost than needed; compact accepted decision journal suffices initially.
5. **Do nothing.** Leaves adapter mismatch, unstable final tie behavior, and non-comparable projections unresolved.

**Tradeoff comparison:** Versioned profiles add schema/fixture and compatibility work but make cross-target behavior extensible and auditable. A single unversioned scale is simpler and cheaper initially, yet loses meaning at adapters and cannot scale semantically across new dimensions. Multi-master improves write locality at the cost of conflict correctness/reliability. Full event sourcing maximizes replay extensibility but materially raises storage, read, migration, and operations cost without measured need; doing nothing preserves that low cost but leaves known correctness defects.

## Proposed operational decision

All routine/manual/automatic precedence is subordinate to Drew's existing protected
expedite policy. The versioned owner-policy observation, canonical membership (not a mirror
label), exact authenticated owner action, atomic policy/work-version check and typed denials
in [`architecture.md`](../architecture.md#protected-owner-priority-policy) are normative.
Generic priority permission and high confidence cannot amend protected score/rank/labels or
membership. Test every entry surface and stale policy/membership version; do not ask for
#7030 ratification again or adopt the obsolete band-90 table as numeric authority.

Fingerprint framing, exact command-intent/full-order DTOs, purpose domains and lowercase
`sha256:` wire encoding follow
[`architecture.md`](../architecture.md#digest-and-serialization-contract). Full order hashes
bind snapshot/graph/policy/protection/lane inputs and all ordered comparator keys, not merely
display IDs. Numeric work-ID comparison preserves `9 < 10` despite string encoding. This is
new-interface compatibility work, not a legacy creation-outbox hash migration.

Keep intrinsic urgency, nullable importance, effective scheduling pressure, workflow, ownership, peer rank, and operational directive distinct. Use named/versioned profile and taxonomy mappings. Preserve exact ratified `15`/`25` values losslessly; do not re-ask #7030 or reinterpret `expedite=15` as Do Now. Historic `450`/`1000` stays a source profile, not live score migration.

For a fixed authority/lane snapshot, require both task requirements and caller filter; apply hard eligibility before ordering. Order eligible work by effective urgency (lower sooner), explicit exact peer rank within equal urgency, creation time, and canonical identity: authority IDs bytewise, queue unsigned decimal work IDs by arbitrary-precision numeric order (9 before 10), not lexical string order. Opaque provider/session IDs are not parsed. Record policy/profile, taxonomy, graph, item revisions, eligibility snapshot, comparator version, and as-of time. Cross-target order is promised only when all these inputs and canonical IDs match. Claim transaction revalidates or returns stale/reselects. This comparator is separate from RFC 8785's UTF-16 JSON property-key ordering.

Manual before/after is preserved as a requested capability, with rank changes constrained within urgency peers unless urgency is explicitly changed. Rank representation/rebalance mechanics remain implementation choices subject to schema compatibility. Automatic triage must be bounded, evidence-backed, provenance-bearing, and reversible; derived blocker/deadline pressure does not silently rewrite intrinsic score.

## Rationale and evidence boundary

**Measured/source facts:** Track A census records broker ascending priority order and missing final ID tie; list adds `id ASC`; dispatch validator uses discrete values and snaps 15/25. #7030 evidence documents ratification and dated readback; current staffing acceptance remains separate. Census is pinned source/issue evidence, not current deployment assurance.

**Inference:** explicit versioning and deterministic snapshots are the least disruptive means to preserve value across producers and consumers. Full event sourcing is not required to retain auditable accepted decisions. No queue distribution, scale, or latency benchmark is asserted here.

## Consequences, compatibility, and migration

- Adapter fix must preserve 15/25 exactly and provide typed outcomes for unsupported profile/version; no hidden rounding or score renumbering.
- Stable tie behavior may alter only previously unspecified final ties; compare list/claim results and record compatibility fixture before enabling.
- Rank/schema work waits for #3424/#8575/#8579 and QTI/#9241 disposition. Pure reference evaluator/fixtures can proceed before schema.
- Lower-is-sooner and Do Now separation are fixed semantics; `manual_only` is persistent eligibility, never a one-shot directive.
- No production policy, threshold, hysteresis constant, fairness weight, or historical score migration is adopted by this proposal.

## Negative cases and test oracles

- **Golden/profile oracle:** exact `15` and `25` round-trip; 15 never aliases 10, 25 never aliases 20; historical 450/1000 profile is not consumed as live values.
- **Comparator oracle:** equal urgency is legal; permuting input rows yields byte-identical canonical order for pinned inputs; final ties resolve by documented creation/ID order; authority IDs use bytewise order and work IDs compare numeric strings 9 before 10 (never lexical). JSON intent digest property ordering is separately tested against RFC 8785 vectors.
- **Authority oracle:** stale revision conflicts; manual command and authorized external label follow distinct precedence; arrival timestamp alone never wins.
- **Eligibility oracle:** both requirements and filter must pass; `manual_only` excludes ordinary claims; Do Now changes directive, not urgency or hard gates.
- **Replay oracle:** fixed policy/taxonomy/as-of/graph/item snapshot reproduces the same order hash; unknown major version is typed unsupported/quarantined, not coerced.
- **Migration oracle:** shadow comparison preserves live stored values; no write migration occurs before compatibility gate.

## Revisit triggers

Revisit profile boundaries when owner-approved policy changes, #7030 mapping source changes, measured queue scale requires a different selection strategy, existing rank fields are found and alter implementation approach, or fairness goals are quantified. Revisit event sourcing only if audit/replay needs exceed append-only accepted decisions and versioned snapshots.

## Implementation, research, and source links

- Existing: [#7030](https://github.com/developing-today/code/issues/7030), [#9199](https://github.com/developing-today/code/issues/9199), [#9200](https://github.com/developing-today/code/issues/9200), [#9204](https://github.com/developing-today/code/issues/9204), [#8097 lane policy](https://github.com/developing-today/code/issues/8097).
- Existing granular component keys: #9199 `PRI-01`–`PRI-07`; #9200 `DST-01`–`DST-07`; #9204 `SEL-01`–`SEL-07`. Reconcile shared aliases to the exact issue index before filing; no extra backlog keys are allocated here.
- Sources and code evidence: `pm-stack/dispatch/src/validate.ts`; `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` at pinned commit above; census `track-a.md` and component #9199/#9200.
- Serialization evidence: [RFC 8785 evidence note](../research/canonical-json-evidence.md); its UTF-16 property ordering is not the queue's numeric work-ID comparator.
