# ID/Atlas summit: extensible signed primitives, federation, namespaces

Program [#9106](https://github.com/developing-today/code/issues/9106), queue task
15354. Summit owner: `ses_e5e2d06f0ccffefg2Z4FGyW3mW`, intended head
`opencode-route/summit`. Workhorse workers produce bounded inventories, evidence,
drafts and verification; the summit owns cross-part decisions and final language.

**Status: elaboration in progress, not acceptance.** This is additive to the
[intake proto](../2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md),
not a replacement for its 384 statements. A closed intake issue does not establish
completion. The launching lane's parked queue state is a handoff, not a stop order;
stage records are committed here and posted as `agent-note` comments on #9106.

## 1. Reading order and acceptance evidence

| Record | Purpose | Acceptance state |
|---|---|---|
| [Baseline](baseline/v1-audit.md), [seams](baseline/seam-map.md) | Source-audited V1 and existing owner boundaries | Initial audit; exact hash/overlap reconciliation in progress |
| `inventory/` | Deterministic statement extraction and expanded issue map | Worker verification in progress |
| `inventory-live/` | Live issue homes, hierarchy, Part B candidate provenance | Worker verification in progress |
| `crosscut/` | Extensibility, modes, sequencing: #9110–#9112 | Research/draft in progress |
| `part-a-addressing/` | ID shapes and namespace families: #9113–#9124 | Research/draft in progress |
| `part-a-graph/` | Triples, attributes, social substrate: #9125–#9132 | Research/draft in progress |
| `part-b/` | Federation and trust: #9141–#9148 | Research/draft in progress |
| `part-c/` | Storage economics, search, recommendations: #9133–#9140 | Research/draft in progress |

Before final acceptance: a fresh judge independently verifies inventory coverage;
research exists per component; versioned designs are reviewed and merged to main;
granular implementation/research issues are natively parented, linked and enqueued;
#9112 records complete sequencing; final judge and summit agree on total scope,
cohesion and correctness. No stage is inferred from a worker's success message.

## 2. Binding architecture and ownership

1. **Signed acceptance is a primitive invariant.** Transport authentication,
   payload signature verification, semantic validation, signer authority,
   tenant/scope authorization and replay handling are separate checks. All apply
   where relevant before acceptance. Local producers sign before the same gate;
   an in-process API, privileged hook or fully trusted peer is not an exception.
   A signature establishes attribution, not truth or access permission.
2. **Extensibility is architecture, not immediate breadth.** Stores, object
   buckets, streams and transports are independent adapter planes. Long-tail
   implementations remain deferred, but normal execution paths expose versioned
   extension seams from the start. A missing backend capability is a conformance
   gap, never grounds for silently weakening a required primitive.
3. **Two fully featured tier-1 editions.** Rust and hosted Workers must meet the
   same externally observable primitive contracts, without claiming identical
   local transactions, performance or physical durability. Unsupported atomicity
   must be implemented by an explicit protocol or rejected, not simulated by a
   misleading interface promise.
4. **Exactly two namespace cores.** Content addressing and array addressing
   supply the cores; names, numeric registries, streams and coordinate worlds
   derive from those cores. Address slots are null-or-bytes; stored values are
   separately type-tagged. Empty address, null slot and empty bytes are distinct.
5. **Scope precedes disclosure.** A content hash is not an access capability.
   Existence acknowledgments, sidecars, locations, derived graph/search records,
   cache entries and federation routing all obey the owning tenant/authority
   policy. “All metadata” means a complete version within its authorized scope,
   not a license to disclose another scope's private metadata.
6. **Immutable content, signed mutable associations.** Content bytes and mutable
   names, pointers, sidecar heads, version relations and stream completion are
   separate objects. Mutable transitions need revision/predecessor and conditional
   update semantics; timestamps alone are not concurrency control.
7. **Negative answers have scope.** A remote miss is not global absence; a receipt
   is not a perpetual retention promise. A contradiction requires comparable
   signed statements about scope, time, authorization and applicable retention or
   completeness obligations. Claims remain claims, not automatic verdicts.
8. **Resource safety is part of correctness.** Hops alone do not bound fan-out;
   breadth, total work, bytes, time and finite duplicate state need explicit
   accounting. Conceptually unbounded addresses, numbers, graphs and nesting do
   not require unbounded per-request allocation. Operational caps return explicit
   errors/continuations without changing the abstract data model.
9. **Conflicts are artifacts before merges.** Initial sync reports conflicts
   before any automatic merge, while unrelated work continues. CRDT research is
   not permission to silently replace that contract. Key revocation, isolation
   and blast-radius controls cannot wait for a V4 reputation engine.
10. **Sibling owners remain authoritative.** Identity/SSO is #9077; tenancy is
    #9083; the #9084 reference-versus-copy decision stays open. This program
    supplies addressing, scoped provenance, lifecycle and GC contracts to those
    owners. It does not pick their product decisions by implication.

## 3. Version layering

| Layer | Meaning | Must not be claimed |
|---|---|---|
| V1 | Source-audited current implementations, domain-local namespace/sync/relay and storage mechanisms, plus separately labeled existing designs | That all deployments run every source feature, or that general federation is already implemented |
| V2 | Stable adapter/admission contracts, both cores, identity profiles, signed sidecars/location indexing, safe peer classes/conflict reports, evidence harnesses | Unmeasured performance/economics superiority or universal compatibility without conformance |
| V3 | Bounded federated queries, names/lenses/streams, append/social surfaces, ID search and incremental recommendations built on V2 contracts | Global completeness, unrestricted public metadata or special first-party authority |
| V4 | Evidence-gated merge/reputation/world/graph-language horizons and long-tail adapters | That horizon work has been dropped or that research outcomes are predetermined |

The proto's V1 shorthand is not rewritten in place: the baseline adds precision.
ID already has domain-specific namespace, sync and relay mechanisms; absence of
one uniform cross-domain namespace/federation contract is the narrower finding.
Existing Iroh, event/world and Atlas storage paths are not one redb migration.

## 4. Research and verification discipline

Research precedes selection in each component. Dated primary-source citations,
credible alternatives and disconfirming limitations distinguish evidence from
recommendations. Synthetic codec/cost experiments are explicitly not cloud or
production benchmarks. CRDT research compares the named four-project set on a
common matrix; discovery of other useful options is retained.

Every proposed automated test identifies its oracle: a reference codec, abstract
state model, independently computed expected fixture or invariant. PBT applies to
address/encoding round trips and ordering; MBT to admission, append/moderation,
version/delegation, fan-out and GC lifecycles. Tests must cover both tier-1 editions
at the primitive contract, including crash/retry, revocation and authorization.

## 5. Questions and decisions

Technical defaults are decided in the mill and recorded with alternatives. Only
genuine owner choices are batched on #9106 with options and a recommendation.
No question blocks detached work. The five intake defaults remain provisional
until the corresponding evidence-backed component selection is reviewed.

## 6. Stage log

- 2026-11-13: `summit_started` checkpoint accepted; full launch comment and both
  source documents read; worker preflight clean. Empty task branch pushed.
- 2026-11-13: parallel mechanical inventory, live issue-home audit and V1 baseline
  audit started on explicit workhorse route; independent component research/draft
  workers started. First baseline inspected; follow-up requested on exact hash
  profiles and pre-existing phase-14 design overlap.
- 2026-11-13: launching lane instructed parked-task handoff; issue notes and
  committed artifacts remain the stage record. No queue claim is taken back by
  this session solely to make heartbeat/checkpoint calls succeed.
