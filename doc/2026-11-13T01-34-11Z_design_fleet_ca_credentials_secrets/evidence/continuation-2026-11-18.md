# Continuation record, 2026-11-18 (task 17413)

**Owner:** `ses_e5e1d09d167ffeS8xtQLnExKKn`, the single Summit design-mill writer for
#9155. Prior writer `ses_e5e2d72b22affeq0U488RlilTg` stays superseded. This file is an
add-only record; `continuation-checkpoint.md` is the prior writer's and is unchanged.

**Start state:** branch `design/fleet-ca-v03-buildout`, head
`6e39fcc9c11a9a99ff6b9b0016a5dbf34f57fdc7`, clean tree, worker preflight clean.

## 1. ADD-A-023 ruling applied (proposed, pending expert review)

Source of the ruling: scoped source-mapping review on #9155,
`issuecomment-6909803610`, 2026-11-16T07:04:51Z. It says to keep the mapping "as
proposed pending expert review". It is therefore recorded in its own file,
`evidence/coverage-rulings-source-mapping-review.json`, and not in the expert-reviewed
ruling files.

- Primary owner FUT-10 (#9166); OPS-10 (#9165) supporting policy only.
- Disposition `research`, matching the source's own class ("nonblocking research").
- FUT-10's authored body is unchanged. The acceptance refinement is carried as one
  planned oracle and three enrichment clauses, which the normalizer renders into the
  FUT-10 (primary) and OPS-10 (supporting) issue bodies.
- Publication and implementation status are unchanged: `not-published`, `not-recorded`.

**Locator reconciliation.** No source text or locator changed. The statement is line 53
of the immutable snapshot
`source-snapshots/intake-34faffbff2/sources/ADDENDUM_secrets_architecture_performance_infrastructure.md`
(SHA-256 `a84b7885cc6c19707b6ff0a8f77460af06b9c8ad80cd554053f01dc40942ea20`, equal to
the inventory row's `source_sha256`). Line 54 is only its position in the reformatted
`sources/` view (SHA-256 `4126e302…`), which the builder never reads.

**External fact checked.** Cloudflare's S3 compatibility page
(<https://developers.cloudflare.com/r2/api/s3/api/>, dated 2026-11-10, read 2026-11-18)
lists `GetBucketVersioning` and `PutBucketVersioning` as unimplemented and documents
conditional `PutObject` headers as evaluated atomically against committed bucket state.

**Known mislabel.** The builder labels every applied ruling "expert" (summary line
"Applied expert owner/gap rulings: 158"; "Expert planned coverage oracle" in rendered
blocks). For ADD-A-023 that label is not yet true. The row's reason text states
"PROPOSED OWNER, PENDING EXPERT REVIEW". An expert reviewer must confirm or replace this
ruling before any final manifest is cut.

## 2. Committed outputs were stale against the committed builder

Before the ruling was added, regenerating from the committed inputs with the committed
`build_traceability.py` did not reproduce the committed outputs:

- every one of the 3,321 rows gains `source_specific_oracle` and `expert_test_oracle`;
- `SPEC-0122` moves from `needs-review` to `reviewed-specific-oracle`, because the
  builder now reads the specific expert oracle already present in
  `coverage-rulings-pki.json`;
- row set and row order are identical; `acceptance-enrichment.md` was identical.

So the matrix digest `b16122435a1f…` named in the 2026-11-16 approval request described a
stale artifact, and the preview digest `10b158831ff8…` described a 154-proposal preview
that lacked ARCH-11, ARCH-12 and TRACE-03. Both are **superseded and withdrawn**. The
ruling itself changes exactly one row (ADD-A-023) relative to the regenerated baseline.

Regeneration used: `inventory/statements.jsonl`, every `proposals/*.json`,
`traceability/mapping.json` plus every `traceability/mappings/*.json`, every top-level
`traceability/member-mappings/*.json` (not `archive/`), `mapping-resolutions.json`, every
`evidence/coverage-rulings-*.json`, an empty receipts file, draft mode (no `--final`).

## 3. Current digests (draft; not approved, not a final manifest)

| File | SHA-256 |
|---|---|
| `traceability/traceability.jsonl` | `f0ac056751cd7971f4d1890f756e312b2512f1e89fa3beb001dd8c9772de4052` |
| `traceability/traceability.csv` | `06e15cc38ec66bd004588d8d8d2b8d6e464f589e2be68b9821d333b0deb33c8c` |
| `traceability/traceability-summary.md` | `ccd2416e0c25fa4391c300521b394307bb00dbeb0ddbf1fa36b85e7951a58dcb` |
| `traceability/acceptance-enrichment.md` | `d9c60a8e06db04d5468bc80610f186441eca18003afa4f313513c2ca0edf26ba` |
| `evidence/normalizer-preview.json` (157 proposals, unenriched) | `1cc96dee26419e8e3f859aadf0b87cf6cdfebaf61d80ebd91e3b53f7efcec48c` |
| `evidence/coverage-rulings-source-mapping-review.json` | `f7ab8bd3fd362ef3929cc49439418cb023cce6628b09b557c16b24cb820c1178` |
| `inventory/statements.jsonl` (unchanged) | `e8158639602683cfb948e14fad4a774faa8169f6070f7c3f44e6e4062da86d58` |

No enriched manifest exists yet, so no enriched-manifest digest exists and no approval is
requested.

## 4. What still blocks the enriched manifest

The normalizer (`prepare_manifest.py --traceability-matrix`) exits 2 at the first row that
names a proposal without a reviewed, outcome-specific oracle. It now stops at
`traceability.jsonl:35` (ADD-A-025); before this pass it stopped at line 33 (ADD-A-023).
Eight such rows remain, in three groups:

**A. Owner ruling needed (expert).** `ADD-A-025`, `ADD-A-027`: SecureStore evaluation,
mapped to OPS-04, whose own reason says OPS-04 does not cover it. Candidate for the
reviewer, not applied: PERF-05 "Complete bounded SecureStore and AEAD-envelope
evaluation" (#9160). The rows' domain starter is #9166, so the owner is a cross-domain
call.

**B. Oracle ruling needed (expert).** `SPEC-0010-split-04` (ROOT-02, partial): the oracle
is generic boilerplate. `SPEC-0122-member-003` (ROOT-03): the `rsa-4096` member is paired
with a validity-time-boundary decider, which does not test that member.

**C. Tool-contract decision needed.** `COORD-ADMIN-0002` (TRACE-01, administrative),
`SPEC-C-0106` (SEED-12, historical), `SPEC-PKI-012` (CERT-02, superseded), `SPEC-PKI-073`
(CERT-12, historical). These are context rows that name a proposal. The traceability
README allows a `proposal_id` on any disposition; the normalizer refuses to bind a row
whose oracle is `not-applicable`. The other 458 rows with a `not-applicable` oracle name
no proposal.
*Open question, default chosen:* do not change these four mappings without a ruling.
Recommended resolution for the reviewer: clear the proposal target on these four rows,
since each row's own reason says the named proposal does not own it.

**Separate from the normalizer:** final coverage is still far off. 704 source outcomes are
`unresolved-cross-domain` and 120 are partially covered; 608 member outcomes are
unresolved and 696 partial. Rows with no proposal target are skipped by the normalizer, so
clearing the eight rows above would let it emit a manifest, but that manifest would omit
those outcomes and would not be the exact final enriched manifest.

## 5. Tests at this head (offline, fixtures only)

`test_build_traceability.py` 36/36 (one test added:
`test_coverage_ruling_reassigns_owner_and_binds_supporting_role`, the first end-to-end
test of the ruling path); `test_prepare_manifest.py` 11/11; `test_publish_issues.py`
48/48. Nothing was published; no issue, queue task, account, credential or key was
touched.
