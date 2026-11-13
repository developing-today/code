# ID/Atlas summit: extensible signed primitives, federation, namespaces

Program [#9106](https://github.com/developing-today/code/issues/9106), queue task
15354. Summit owner: `ses_e5e2d06f0ccffefg2Z4FGyW3mW`, intended head
`opencode-route/summit`. Workhorse workers produce bounded inventories, evidence,
drafts and verification; the summit owns cross-part decisions and final language.

**Status: integrated design review set, not final program acceptance.**
[PR #9238](https://github.com/developing-today/code/pull/9238) is the concrete
review deliverable. This is additive to the
[intake proto](../2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md),
not a replacement for its 384 statements. A closed intake issue does not establish
completion. The launching lane's parked queue state is a handoff, not a stop order;
stage records are committed here and posted as `agent-note` comments on #9106.

## 1. Reading order and acceptance evidence

| Record | Purpose | Acceptance state |
|---|---|---|
| [Baseline](baseline/v1-audit.md), [seams](baseline/seam-map.md) | Source-audited V1 and existing owner boundaries | Source profiles pinned; external Atlas snapshot checked, producer-source gap retained |
| [Inventory](inventory/audit.md) | Deterministic statement extraction and expanded issue map | Independent judge agrees: 384 IDs, zero unmapped homes; 11 regression cases pass |
| [Live homes](inventory-live/semantic-audit.md) | Live issue homes, hierarchy, Part B candidate provenance | 52 issue snapshots; 42 program hierarchy edges; original-30 gap explicitly documented |
| [Crosscut](crosscut/versioned-design.md), [sequence](crosscut/sequencing-draft.md) | Extensibility, modes, sequencing: #9110–#9112 | Frozen draft; verifier passes 39 slots and DAG/config/file oracles |
| [Addressing](part-a-addressing/versioned-design.md) | ID shapes and namespace families: #9113–#9124 | Frozen 35-slice draft; readable URI reference passes 10,000 round trips |
| [Graph](part-a-graph/versioned-design.md) | Triples, attributes, social substrate: #9125–#9132 | Frozen 16-slice draft; fresh worker review passed; toy evidence reproduced |
| [Federation](part-b/versioned-design.md), [repairs](part-b/repair-disposition.md) | Federation and trust: #9141–#9148 | Frozen 32-slice set; bounded findings applied; allocation model passes 2,000 traces |
| [Atlas](part-c/versioned-design.md), [repairs](part-c/repair-disposition.md) | Storage economics, search, recommendations: #9133–#9140 | Frozen 22-slice set; executable 11-transition reference fixture; empirical runs are explicit follow-ups |
| [Answers](answers.md), [integration check](review/integration-check.md) | Summit rulings, conservative defaults, readiness boundaries | Concrete checkpoint; no repeated reviewer-wait loop |
| [Verification](verification.md) | Copyable narrow commands, observed results and evidence limits | Runtime/production claims explicitly excluded |
| [Delivery preparation](delivery/delivery-plan.md) | Individually workable issue drafts and native/queue readback gate | Offline-only until final validation; no issue publication is implied |

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

### Shared identity and signature preimages

New profiles distinguish a raw `PayloadId` from a signed record's `EnvelopeId`.
Both are content IDs over their declared **exact bytes**, not a per-object registry
translation. A canonical body commits to payload identity/length and its protocol,
signer, authority and tenant context. Its signature covers a domain prefix and the
canonical body, excluding its own signature and envelope ID. The envelope ID then
hashes the complete canonical signed envelope, including the signature. Sidecar
snapshot IDs and graph assertion IDs follow that rule. Logical graph/space IDs
are separate stable names, never a circular hash of the assertion containing them.
New profiles use deterministic CBOR with an explicit version; existing artifact
JSON preimages remain a separate legacy profile, with no silent rehash/re-sign.

The audited initial raw content profile is Iroh's exact-byte BLAKE3-256. Existing
artifact SHA-256 and event/world commitments keep their domain meaning. Atlas's
checked-in package consumes `content_hash` but does not define its production
producer; that missing producer contract is a tracked integration input, not
permission to assign it Iroh semantics. Snowflake refers to the Arctic embedding
model, not an assumed Snowflake Cortex deployment. #9092 owns the second-embedder
evaluation; its candidate set is not a preselected model.

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
- 2026-11-13: draft PR #9238 opened. Independent inventory judge confirmed 384
  IDs, rejected a malformed-map false-success hole and incorrect hierarchy
  reporting; focused fixes now reject unknown endpoints and report 42 program
  hierarchy edges. Re-review is in progress; initial disagreement is retained.
- 2026-11-13: summit top-to-bottom pass requested concrete gap-fills: RFC-compliant
  nonambiguous URI examples, arbitrary-precision float profile, mandatory pending
  review for non-group append submissions, predecessor-to-successor `succeeds`,
  aggregate rather than per-path fan-out accounting, V2 emergency revocation,
  GC fencing against reference-add races, and model-backed evidence rather than
  invented production benchmark claims.
- 2026-11-13: bounded recovery did not find the original enumeration of 30 Part B
  candidates. The question and default are recorded on #9106 comment 6843258938:
  continue a transparently statement-derived replacement covering all S1 and the
  seven named categories; retain the provenance gap, never claim recovered text.
- 2026-11-13: owner directed bounded integration rather than repeated waits. One
  status pass consumed actual returned B/C reviews; nine shared contract checks
  were recorded, exact slice gaps assigned, and the parked-task checkpoint verdict
  was recorded on #9106 comment 6843573492 without a duplicate fallback claim.
- 2026-11-13: final-language pass corrected A's mistaken B11 receipt dependency to
  B19 (B11 is Diamond Types research), made normal URI byte slots human-readable,
  and removed ambiguous self-ID/profile-prefix wording. URI checks now pass six
  required vectors, 14 malformed cases and 10,000 deterministic round trips.
