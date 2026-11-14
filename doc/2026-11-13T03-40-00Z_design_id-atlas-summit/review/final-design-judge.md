# Fresh final design judge — #9106 / task 15354

**Verdict: AGREE with merging the additive design/research set at the design
boundary reviewed here.** This is a whole-forest design judgment, not implementation,
issue-readiness, delivery, or task-15354 acceptance. I read the launch packet
(#9106 comment 6843006010), the proto and its 384 statements, `main.md`,
`answers.md`, the baseline audit at checkout `a0f3a529d3e167df0f614a598dc0c9f7aa532b0c`,
all five `versioned-design.md` and `research.md` pairs, repair dispositions,
inventory/source reviews, and crosscut sequencing. The working tree already had
unrelated worker manifest/preparation changes; I did not inspect or alter those
changing manifests. This judgment owns only this file.

## Correction follow-through

The initial review recorded two blockers: the Part B cache/origin expiry ambiguity
and the Part A numeric negative-order ambiguity. Follow-up edits now resolve both:
`part-b/research.md` agrees with the no-rerun-after-expiry and same-reservation
contract; `part-a-addressing/versioned-design.md` explicitly orders negatives by
descending absolute magnitude, then zero, then positives by ascending magnitude
within each numeric kind.

The parent then surfaced three independently verified PR-review defects. The
current stable design text and narrow oracles now address them:

- **R2 deletion:** `part-c/versioned-design.md:31` and `part-c/research.md:27,47`
  no longer claim conditional `DeleteObject`. They require a #9084 owner fence and
  immutable generation-qualified physical keys that are never reused; absent proof
  of both, deletion is disabled. Reference-vs-copy remains open.
- **Merged feed freshness:** `part-a-graph/versioned-design.md:128–130` defines a
  durable projection-ledger sequence for the complete feed scope, retains source
  sequences only as provenance, and pins cursors to the complete-view cutoff.
  `feed-cursor-oracle.py` covers colliding source sequences, cutoff, restart, paging
  and policy-generation invalidation.
- **Recommendation page identity:** `part-c/versioned-design.md:45` separates the
  signed parent `ParentSnapshotId` from each page's signed-envelope `EnvelopeId`;
  the cursor binds the parent ID plus position. `recommendation-page-oracle.py`
  recomputes the fixture IDs, checks restart determinism and rejects a wrong-parent
  cursor. It explicitly does not claim cryptographic verification or production
  state.
- **Budget wording:** `part-b/versioned-design.md:100` states spent counters are
  monotone non-decreasing, never refunded; only unused held reservation is
  refunded. This matches measured-use accounting in the design/model. The repair
  is reflected in the current stable design text.

No blocker remains for merging these additive designs/research records. The
verification above does not certify production implementations or delivery gates.

## Whole-forest findings

- **Coverage and homes:** the proto's fixed section counts total 384. The
  corrected inventory audit records 384 distinct statements, zero unmapped
  statement homes, and 39 home IDs; the design section scopes align: #9110/#9111
  cover S0/S1 non-specialness/S7, Part B covers S1.1–113, Part C S2/S6, and the
  two Part A designs cover S3/S4 and S5. The crosscut verifier reports all 39
  issue slots and checks the sequencing DAG/config/file oracles. I found no
  missing statement family or misplaced crosscut responsibility in the frozen
  design prose. This does not certify implementation-issue readiness.
- **V1 versus V2+:** the baseline and all five designs distinguish existing
  domain-local namespaces, sync/relay/query and separate stores from the proposed
  uniform contracts. V2/V3/V4 aspirations are not retroactively called
  implemented. Cloud/laptop measurement, provider winners and production
  performance are not claimed; measured runs remain valid future research work.
- **Security and identity:** signed admission is mandatory for local producers
  and remote inputs; derived retained/transported outputs require signed
  service-produced envelopes. Cross-part identity text separates payload/content
  IDs, signed-envelope IDs and stable logical IDs; signature preimages exclude
  their own signature/ID, and legacy JSON bytes are preserved. I found no
  remaining self-ID or hash-profile circularity contradiction.
- **Addressing and lifecycle:** the URI is internal/unregistered, uses a required
  local resolver hint, has human-readable unreserved byte slots, and distinguishes
  empty array, null and empty bytes. There are exactly two cores; other families
  derive from them. CAS/single-writer requirements and the GC reference-generation
  fence are explicit. The Part B V2 receipt is B19 (not B11), and non-group
  submissions always enter pending review, including on open attributes.
- **Trust, query and ownership:** budget accounting is multidimensional and
  scoped to query/requester/peer/tenant/node with explicit local-enforcement and
  Byzantine limits; claims/receipts do not exceed their evidence. Revocation and
  cache authorization constraints are present. SSO, tenancy, and ref-versus-copy
  decisions remain with #9077, #9083 and #9084; no unilateral settlement found.
- **Prior art/evidence corrections:** Part B research now narrows RFC 9750 replay
  claims, RFC 9942 receipt attribution, and OCapN attenuation to what the cited
  sources establish. Graph research reports the executable's SQLite JSONB and
  arbitrary-byte results as local toy evidence, not a backend-performance result.
  The original Part B 30-item list remains explicitly unrecovered; the replacement
  is transparent reconstruction, not invented provenance. Horizon work has
  review dates and is retained, not dropped.
- **Readiness boundary:** `main.md`, `integration-check.md`, and `verification.md`
  correctly keep the parked queue handoff, pending CI/merge/publication gates, and
  issue-readiness review separate from design correctness. I do not infer green CI,
  merged-to-main status, task completion, or live issue/queue coverage.

## Narrow checks run

- `uri_codec_reference.py self-test`: 6 vectors, 14 malformed cases and 10,000
  deterministic round trips passed.
- `federation_model.py`: fixed cases and 2,000 seeded accounting traces passed;
  this is not cryptographic or Byzantine/global-budget proof.
- `verify_crosscut.py`: 39 issue slots, 2 research slices and 17 implementation
  slices; DAG/file/config oracles passed.
- `benchmark-workload-oracle.py`: 11 reference transitions and 44 capability
  mappings passed; timings explicitly unmeasured.
- `graph-encoding-toy.py --check-oracles`: fixed hashes, JSONB and arbitrary-byte
  round trips passed for the stated local fixture only.
- `verify_inventory_regressions.py`: all 11 cases passed, including unknown map
  endpoint rejection.

This verdict does not waive separate issue-readiness, CI, main-merge,
native-parenting, queue-coverage, or overall task-acceptance gates.
