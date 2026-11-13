# Fleet v0.3 statement inventory

This directory holds source-grounded inventory records. JSONL records use the coordinator handoff §8 traceability columns verbatim, plus the integration fields `source_id`, `source_lines`, `statement`, `normative_history_admin_class`, `canonical_requirement_ids`, and `primary_starter`, along with provenance/status helpers. `requirement_id` and `source_id` are the same stable cross-file identifier. Primary starters are domain issues from `note.md`, not granular proposals; proposal IDs and numeric issue receipts can be bound in follow-up.

## Files

- `supplemental.jsonl` — addendum, run handoff, decision review, backend research, implementing-agent handoff, PM companion conventions, package README, and SHA manifest.
- `coordinator_section7.jsonl` — the coordinator handoff's eight mandatory design areas.
- `coordinator_section8.jsonl` — traceability and coverage quality bar.
- `research_backlog.jsonl` — individually identified R-SEC-01 through R-SEC-18.
- `current_evidence.jsonl` — current repo-evidence correction distinct from older intake-source statements.
- `spec-0-24.jsonl` — spec first half initial extraction.
- `spec-0-24-continuation.jsonl` and `spec-0-24-continuation-2.jsonl` — first-half roadmap/test refinement and additional semantic coverage.
- `SPEC_v03_sections25-49_appA_sec31.jsonl` — spec second half initial extraction.
- `SPEC_v03_continuation_27_39-47_28-29.jsonl` — second-half gap fill, including §47 historical input crosswalk.
- `SPEC_v03_atomicity_supplement.jsonl` — further §31/register atomicity records.
- `SPEC_v03_residual_context_mapping.jsonl` — mapped non-normative / structural spans; unresolved spans are cross-linked to semantic resolution records.
- `SPEC_v03_semantic_resolution.jsonl` and `SPEC_v03_register_atomic_audit.jsonl` — second-half semantic gap resolution and register atomicity audit.
- `input3_statement_inventory.jsonl` — historical third input's semantic records.
- `input3_coverage_closure.jsonl` — non-normative and formatting spans closing the historical input line map.
- `validate_inventory.py` — schema, stable-ID and per-source nonblank-line coverage check; exits nonzero on gaps/errors.

## Interpretation and coverage discipline

The source precedence in `note.md` applies. An item classified as proposal, historical, research, explanatory, or administrative is not promoted to a binding implementation requirement by appearing here. Coordinator §7 / implementing-agent handoff obligations are future design/implementation obligations, not authorization to perform production operations in this design phase. Production safety gates remain explicit. Historical input 3 is traceability context; the v0.3 spec §47 reconciliation controls where it supersedes it.

Coverage must be measured per source line/span and record, not inferred from section headings or line counts. A line may be covered by an exact range mapping without every word being a requirement; explanatory/history statements are explicitly mapped as non-normative. The input3 worker reports continuous physical coverage 1–1796 after its closure file; semantic completeness still requires an independent review. The second-half spec worker reports that prior 42 semantic gaps now have resolution records and that all six of its files have the integration fields; these are source interpretations, not implementation verification. The first-half spec worker has supplied further records but a final exact coverage/status audit is pending. **Do not claim overall inventory completeness until the first-half and input3 schema/semantic audits are checked and all source gaps are either resolved or explicitly reported.**

Coordinator §8 column set: `requirement_id`, `source_file_and_section`, `source_precedence_resolution`, `user_outcome`, `severity_or_security_class`, `verified_current_behavior`, `proposed_architecture_or_API`, `primary_issue_number`, `dependent_issue_numbers`, `provider_capabilities_and_permissions`, `acceptance_test_or_benchmark`, `migration_or_rollback`, `evidence_status`, `current_status`. `source_lines` is a normalized machine-readable inclusive range or list of ranges; `canonical_requirement_ids` are stable grouping IDs and do not imply a granular proposal exists.
