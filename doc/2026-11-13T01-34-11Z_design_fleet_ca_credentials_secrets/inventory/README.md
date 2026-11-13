# Fleet v0.3 statement inventory

This directory holds source-grounded inventory records. JSONL records use the coordinator handoff §8 traceability columns verbatim, plus the integration fields `source_id`, `source_lines`, `statement`, `normative_history_admin_class`, `canonical_requirement_ids`, and `primary_starter`, along with provenance/status helpers. `requirement_id` and `source_id` are the same stable cross-file identifier. Primary starters are domain issues from `note.md`, not granular proposals; proposal IDs and numeric issue receipts can be bound in follow-up.

## Files

- `supplemental.jsonl` — addendum, run handoff, decision review, backend research, implementing-agent handoff, PM companion conventions, package README, and SHA manifest.
- `coordinator_section7.jsonl` — the coordinator handoff's eight mandatory design areas.
- `coordinator_section8.jsonl` — traceability and coverage quality bar.
- `coordinator_run_contract.jsonl` — specific scope, staging, issue, dispatch, safety, supervision and reporting clauses outside §§7–8.
- `research_backlog.jsonl` — individually identified R-SEC-01 through R-SEC-18.
- `first_half_atomicity.jsonl` — explicit records for the §11.1 path matrix/negative cases, §19.2 typed schema, §21.3 DB tables, §22.2 generator policies, and §6.1 command families.
- `addendum_atomicity.jsonl` — atomic records for the addendum's provider, fnox, SOPS/age, root-custody, SecureStore/OpenBao, federation, Rust architecture and sequencing refinements.
- `current_evidence.jsonl` — current repo-evidence correction distinct from older intake-source statements.
- `source_context_coverage.jsonl` — source-specific explanatory, provenance, formatting and explicit out-of-scope mappings.
- `handoff_gate_records.jsonl` — ten exact source-line implementation-gate outcomes linked to the implementing-agent handoff aggregate.
- `spec-0-24.jsonl` — spec first half initial extraction.
- `spec00-13_lines045-299.jsonl` at the inventory root is preserved as an earlier assignment/draft artifact and excluded from canonical compilation; the reviewed refinement is `refinements/spec00-13_lines045-299.jsonl`.
- `spec-0-24-continuation.jsonl` and `spec-0-24-continuation-2.jsonl` — first-half roadmap/test refinement and additional semantic coverage.
- `refinements/spec00-13_atomicity_first_pass_draft.jsonl` — preserved child draft, explicitly excluded from compiler inputs; active refinements are the non-draft `refinements/*.jsonl` files.
- `SPEC_v03_sections25-49_appA_sec31.jsonl` — spec second half initial extraction.
- `SPEC_v03_continuation_27_39-47_28-29.jsonl` — second-half gap fill, including §47 historical input crosswalk.
- `SPEC_v03_atomicity_supplement.jsonl` — further §31/register atomicity records.
- `SPEC_v03_residual_context_mapping.jsonl` — mapped non-normative / structural spans; unresolved spans are cross-linked to semantic resolution records.
- `SPEC_v03_semantic_resolution.jsonl` and `SPEC_v03_register_atomic_audit.jsonl` — second-half semantic gap resolution and register atomicity audit.
- `statements.jsonl` — normalized canonical inventory with exact source excerpt, whole-source SHA-256, role, oracle and granularity status. Shards remain preserved as source records.
- `CANONICAL_FREEZE.md` — frozen mapper input path/hash and validation/gap receipt.
- `build_canonical_inventory.py` — deterministic builder pinned to `source-snapshots/intake-34faffbff2/sources/`; verifies each source hash against `source_snapshot_manifest.json` and excludes the formatted `sources/` view.
- `compile_refinements.py` — compiles reviewed gap refinements into the sole canonical file; preserves source parents and nests atomic members instead of minting source IDs per member.
- `audit_register_anchors.py` — §31 and Appendix A exact ID-to-line audit.
- `write_inventory_report.py`, `inventory_audit.json`, `inventory_audit.md` — counts and concrete atomicity-gap index.
- `input3_statement_inventory.jsonl` — historical third input's semantic records.
- `input3_coverage_closure.jsonl` — non-normative and formatting spans closing the historical input line map.
- `validate_inventory.py` — strict duplicate-key/schema/ID/range/excerpt/hash and nonblank-line coverage validator using only the pinned original snapshot; also checks nested member lineage/oracles.

## Interpretation and coverage discipline

The source precedence in `note.md` applies. An item classified as proposal, historical, research, explanatory, or administrative is not promoted to a binding implementation requirement by appearing here. Coordinator §7 / implementing-agent handoff obligations are future design/implementation obligations, not authorization to perform production operations in this design phase. Production safety gates remain explicit. Historical input 3 is traceability context; the v0.3 spec §47 reconciliation controls where it supersedes it.

Coverage is measured against the pinned original snapshot, never the upstream-formatted `sources/` view. The final validator passes with all eleven authoritative files' **4,104 nonblank lines** mapped to exact excerpts/source hashes. The canonical file contains **1,496 stable source IDs**, **1,345 logical grouping IDs**, and **1,825 nested, parent-linked member outcomes** (not source IDs or issue count). Source classes: **935 normative/proposed, 540 historical, 21 context/admin**. All normative rows and nested members have specific-oracle records; unsupported/research/approval statuses remain explicit. §31 and Appendix A have 158 register rows and 219 primary/split references audited with zero anchor errors, including SEC-003 at line 1990 and explicit PKI2-013 digest/key-usage and PKI2-014 profile-inheritance outcomes. The separate freeze receipt contains the canonical hash. The independent source auditor's review remains pending; proposal mapping to granular issues has intentionally not started.

Coordinator §8 column set: `requirement_id`, `source_file_and_section`, `source_precedence_resolution`, `user_outcome`, `severity_or_security_class`, `verified_current_behavior`, `proposed_architecture_or_API`, `primary_issue_number`, `dependent_issue_numbers`, `provider_capabilities_and_permissions`, `acceptance_test_or_benchmark`, `migration_or_rollback`, `evidence_status`, `current_status`. `source_lines` is a normalized machine-readable inclusive range or list of ranges; `canonical_requirement_ids` are stable grouping IDs and do not imply a granular proposal exists.
