# Bounded summit integration checkpoint

2026-11-13, after the owner's no-repeated-waits steer. This records a bounded
inspection of `part-b/versioned-design.md`, `part-c/versioned-design.md` and their
slice manifests. It is **not** a full-program judge agreement or proof of runtime
behavior. Existing reviewer reports remain intact, including disagreements.

## Method and results

One status pass consumed the outstanding reviewers' actual final messages. No
additional `subagents_wait` loop was used. A local isolated Python inspection read
the two design files and parsed their JSON manifests, checking the following
shared contracts were explicitly present:

| Cross-part contract | Bounded result |
|---|---|
| Immutable origin query and separately signed relay grant | Present in B |
| Same admission gate for local producers and remote peers | Present in B |
| Cached responses reauthorize; expired origin queries cannot rerun | Present in B, but old test wording also contradicted it; precise repair requested |
| Payload/envelope identity and legacy signature bytes remain distinct | Present in B and C |
| #9083 tenancy and #9084 lifecycle ownership preserved | Present in B and C |
| GC uses physical generations and an atomic owner fence, or remains disabled | Present in C |
| Cache source/policy watermarks and explicit bypass | Present in C |
| Snowflake Arctic model, owner-undecided second embedder | Present in C |
| Independently retained/transported derived pages require service signatures | Present in C |

Manifest references were expanded as ranges, not compared as literal strings.
The inspected C manifest covered all 36 S2/S6 statements. The inspected B
reconstruction had 30 rows but omitted foundational peer-class/gate statement
references (S1.1–S1.16, S1.20, S1.26–S1.27); a count of 30 is not exhaustive
implementation coverage. The writer was directed to add foundational slices and
complete those references. S1.114–S1.120 remains crosscut #9110's responsibility.

## Returned review dispositions

- [Inventory judge](inventory-judge.md): agrees on numbered inventory and live
  homes after fixes; 384 statements, zero unmapped issue homes. This does not
  accept design completeness or issue delivery.
- [Research-source judge](research-source-judge.md): source qualification repairs
  requested for MLS/COSE/OCapN and stale experiment prose. Updated component
  artifacts must preserve the exact, narrower evidence claims.
- [Part B judge](part-b-judge.md): per-client/peer accounting, S1.27 attribution,
  exact single-PR/test ownership and research-kind repairs assigned. Summit also
  required nonce-bound request IDs, accountable-sender rather than purported-
  signer blame, and consistent cache-versus-origin expiry semantics.
- [Part C judge](part-c-judge.md): six precise repairs assigned: executable
  reference semantics instead of fixture-label assertions; explicit local
  benchmark-run slice; #9092 recall-result ownership; validator/strict-CLI
  ownership; eliminate shared-file collision; total recommendation tie-break.
- Graph writer's fresh read-only reviewer reported no blocking findings after
  corrected 16-slice coverage and executable experiment checks. The original
  source-fidelity review is not silently overwritten by that result.

## Conservative defaults for genuinely unavailable evidence

1. No cloud/laptop latency or economics winner without the planned measured
   evidence. Adapter conformance and synthetic arithmetic are not benchmarks.
2. #9092 owns search recall/shard recommendations; storage-only measurements do
   not establish those conclusions or select the second embedder.
3. GC deletion remains disabled unless #9084 supplies the atomic fence and
   lifecycle predicate; this program does not settle copy versus reference.
4. Unknown key status does not authorize a signed world or accepted assertion.
5. The original 30-item Part B enumeration remains unrecovered. The replacement
   is labeled statement-derived; later original scope must receive extra slices.

## Bounded repairs applied before the review-ready commit

The writers froze their corrected batches; repair dispositions are retained at
`../part-b/repair-disposition.md` and `../part-c/repair-disposition.md`. B now has
32 slices (original reconstructed B01–B30 plus foundational B31/B32); C has 22,
including distinct local and cloud benchmark-evidence tasks. Neither the returned
judge files nor their earlier blocking verdicts were overwritten.

The final-language pass also corrected the cross-part receipt dependency to B19
(B11 is Diamond Types research), made B19 a V2 contract rather than waiting for
V3 fan-out, and kept readable unreserved URI octets literal. It removed ambiguous
sidecar self-ID/hash-prefix wording, required trusted-class **and** explicit relay
authority, and added completed-reservation tombstones to the accounting model.
Unknown CLI arguments are rejected before the B/C standalone checks run or read
fixtures. These are concrete repairs, not a substitute for final independent
whole-package/delivery acceptance.

Observed narrow checks after those edits: URI 6 vectors/14 malformed/10,000
round trips; B fixed checks plus 2,000 seeded accounting traces; C 11 computed
reference-store transitions and 44 capability labels; synthetic cost arithmetic;
strict extra-argument rejection for the three B/C check scripts. No benchmark
timings or production-adapter correctness are established by these results.

The first strict publication-gate run correctly refused to emit issue drafts:
its snapshot saw 124 records/38 component homes, missing exact metadata and eight
unattributed statement IDs. A bounded annotation repair then assigned those eight
IDs to their existing responsible crosscut slices. An independent range-expanded
aggregate now finds **124 records covering all 384 statement IDs**, with no
statement exemptions. This repairs attribution, not issue-body readiness. The
publication helper still has disclosed schema/heuristic limitations and concrete
ownership/dependency metadata gaps; its nine offline tests pass, but no issue was
created or enqueued. Full final review and publication remain separate gates.

## Checkpoint and next concrete deliverable

Queue checkpoint `review_ready_integration` was attempted once as
`repo-worker-code`. It returned the expected parked-handoff `not claimed` verdict;
no fallback claim was created. #9106 comment 6843573492 records the stage and exact
next step. PR #9238 preserves the work; no changes were reset or discarded.

Next: commit/push the frozen, integrated V1–V4 component set with its executable
design checks and explicit readiness matrix. Publication of granular issues,
native hierarchy/blocker verification and live queue coverage is a separate
delivery gate. Neither this checkpoint nor a review-ready PR completes task 15354.
