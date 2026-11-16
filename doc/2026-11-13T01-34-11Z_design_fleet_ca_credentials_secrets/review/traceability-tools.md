# Traceability and manifest tools — independent DRAFT review

**Decision: DISAGREE for final manifest/coverage gating; tool review remains DRAFT.** The suites now pass, and the CLI, authored-input parser, canonical-inventory, and publisher-receipt compatibility findings are fixed. Traceability binding now appends reviewed enrichment clauses and primary/support roles to issue bodies, but final verification does not require those clauses/roles in the exact published manifest body; the policy loader also still accepts duplicate keys. Source/member bindings and proposal content remain under review; I did not certify the 154 proposals or run a final matrix.

## Reviewed snapshot and tests

Read the current `tools/prepare_manifest.py`, `tools/build_traceability.py`, both test files, `traceability/normalization-policy.json`, the traceability README, the canonical inventory freeze note, and the current publisher receipt contract. SHA-256 of reviewed tool/policy snapshot:

- `prepare_manifest.py`: `dabda039295d9ef80a7c8574b78841f20dd5c198927b1888a08a721955da023b`
- `build_traceability.py`: `645fa56795e5801553467521831aa50f177df82c63e600dc0c4326e5c8a2df3d`
- `test_prepare_manifest.py`: `375f70cc8289afdb16bb800ec49971a70aaf66ad2aa105e0a9a39316c8092f15`
- `test_build_traceability.py`: `4bf6f866896f892714343ce6414dc847e206b8086fa1a0df823ef063a782d713`
- `normalization-policy.json`: `2bf0ca83ffe2dda5055ba62e5f402e37663907c622282717f4c7cf0b695c4ef9`

Ran both mocked suites under `python3 -I -B`: `test_prepare_manifest.py` **10/10 passed** and `test_build_traceability.py` **33/33 passed**. No Git mutations, GitHub calls, or queue-service calls were made; only the required local worker preflight ran.

## Findings

1. **RESOLVED — strict normalizer CLI and authored-input duplicate-key rejection.** The duplicate `--traceability-matrix` registration is gone (`prepare_manifest.py:42-58`), and `_read` now uses `_unique_object` (`86-103`). The current mocked tests exercise parser refusal and normalization behavior.

2. **RESOLVED — final canonical inventory is pinned.** Final mode requires the sole `inventory/statements.jsonl` input, exact frozen SHA-256, 1,496 unique source IDs, 1,825 unique nested member IDs, class counts, and 1,345 canonical groups (`build_traceability.py:65-69, 710-738`). A one-row subset is explicitly rejected (`test_build_traceability.py:363-370`). Draft mode can remain partial.

3. **RESOLVED — final receipt verification matches the current publisher contract.** `_verified_publication` delegates plan/digest/event/receipt validation to the actual publisher module (`build_traceability.py:664-707`); it reads separate `issue_verified.issue` and `task_verified.task`, uses `readback_scope.matched_row`, and derives blocker issue numbers from verified blocker edges. The publisher receipt reader enforces the exact pilot queue, same-row task ID/key/status/priority, and bounded scan conditions. Current fixtures are generated in the publisher's event schema (`test_build_traceability.py:51-106`) and test a dependent issue.

4. **PARTIALLY RESOLVED — owner/gap rulings now reach normalized issue bodies.** Coverage rulings require exact primary/support proposal IDs and an explicit enrichment list (`build_traceability.py:484-614`). `prepare_manifest.bind_traceability` fans each row out to primary/support proposals, renders ownership roles, and appends enrichment clauses as planned acceptance (`prepare_manifest.py:440-475, 487-521`). Final rows include support issue numbers from verified receipts and keep implementation status separate from publication.

5. **BLOCKER — final verification does not require coverage-ruling enrichment/support roles in the published body.** Coverage rulings store `acceptance_enrichment` and `supporting_proposal_ids`, and `prepare_manifest.bind_traceability` now renders them in primary/support bodies. But `_verify_issue_body_binding` still checks only source/canonical IDs, source clause, design target, oracle, and mapping reason (`build_traceability.py:95-105`, called at `862-864` and `977-980`). It receives no expected enrichment, support IDs, or owner role, so it cannot reject a stale/omitted clause in the exact published manifest.

   **Counterexample (reproduced):** a bound row with primary `P`, support `Q`, and clause “Reject an alias collision without replacing the persisted identity” produces the clause and role labels in both normalized bodies. Removing only the clause from the published body and calling `_verify_issue_body_binding` still succeeds, because the verifier does not receive or test that value. Final mode can therefore report the ruling as covered although the issue body lacks it.

   **Required fix and oracle:** pass expected enrichment clauses, support proposal IDs, and expected primary/support role into final body verification; assert all are in the corresponding manifest body. Add a coverage-ruling test with one primary, one support issue, and a distinct clause, then remove the clause and assert final mode fails. Keep wording explicitly planned/not implementation evidence.

6. **GAP — duplicate keys in the normalization policy itself are still silently accepted.** Authored proposal JSON is strict, but `prepare_manifest.load_policy` still calls ordinary `json.loads` (`61-65`). Reproduced with a temporary policy: two `publisher_kind` fields for the `design` mapping were accepted and silently selected `implementation`, contrary to the row's “Pure design deliverable” reason.

   **Required fix and oracle:** use the same duplicate-key-rejecting JSON hook for policy loading; add duplicate kind and priority-band policy fixtures and assert a controlled pre-normalization error.

## Checks that look sound in this DRAFT

- Proposal/dependency IDs and references preserve exact case/bytes; optional authored fields are included in the administrative metadata body, unknown fields are rejected, and urgency band/priority/projection fields are cross-checked against the policy.
- Kind normalization is explicit, with no prefix fallback; per-ID overrides are tied to exact authored kinds and retain reasons. This is not semantic approval of mutable proposal content.
- The source commit (`34faffbff26615e313488f6d04705a444342e41e`) is kept distinct from snapshot locator (`40af815bf0`). Source proof checks exact inclusive lines, current file SHA-256 against provenance, exact excerpts, and the `\n\n--- source span ---\n\n` multi-span join; the auxiliary note has a single pinned exception and path traversal is rejected.
- Source/member IDs are emitted as separate row types; member mappings require their own parent, proposal, and oracle. Partial/unresolved/unmapped rows and mapping collisions fail final mode relative to the supplied inventory.
- External issue references are restricted to #9077/#6767/#2056 and remain distinct from existing-code evidence and local numeric receipts. `implementation_status` is copied only from source fields or `not-recorded`, not inferred from issue publication; planned acceptance language says it is not a passed test.

## Scope boundary

This is **not** a review of the evolving bindings or a final matrix. Before final use, the acceptance-enrichment body binding (and policy duplicate-key control) must be closed, the tools rehashed, and tests rerun against a reviewed final normalized manifest and current publisher receipt schema. The output must retain planned/design coverage separately from code/test completion; a green unit suite or partial source map is not approval of source coverage.
