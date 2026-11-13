# Canonical inventory freeze receipt

- Mapper input: [`statements.jsonl`](statements.jsonl)
- SHA-256: `7221bed174f221b1177a3512f3aa04c60c7a99b367aa5926acbd9f40daa32f27`
- Authoritative source root: `source-snapshots/intake-34faffbff2/sources/`
- Source commit: `34faffbff26615e313488f6d04705a444342e41e`; auxiliary intake note hash/path are recorded in `source_snapshot_manifest.json`.
- Source IDs: **1,496 unique**. Canonical logical grouping IDs: **1,345**. Separately enumerated, parent-linked member outcomes: **1,825**; these are nested members, not additional source IDs.
- Source-class counts: **935 normative/proposed, 540 historical, 21 context/admin**.
- Coverage: all **4,104 nonblank lines across the eleven authoritative inputs** map to exact snapshot excerpts/source hashes. The auxiliary intake note is retained separately from the eleven inputs.
- Register audit: **158** §31/Appendix A rows and **219** primary/split references, zero wrong anchors. SEC-003 maps to line 1990; PKI2-013 digest/key-usage and PKI2-014 profile-inheritance clauses have explicit sibling outcomes.
- Validation: `validate_inventory.py` passes structural, duplicate-key/ID, schema, excerpt/hash/range, source-coverage, oracle and atomicity checks against the pinned snapshot. `audit_register_anchors.py` passes all register anchors. These are builder checks; independent reviewer audit remains pending.
- Future/F-class inventory entries are labeled nonblocking research rather than binding baseline. The ten future implementation handoff gates have gate-level source records linked to their aggregate handoff witness.
- Member outcomes are nested under stable parent source IDs, not counted as new source IDs or proposal/ticket count; command/schema families use parameterized members where one shared contract is testable. The independent reviewer may still challenge granularity/member grouping.
- The exact original intake snapshot is the byte-level oracle. The formatted `sources/` view remains untouched and is not used for excerpt/hash generation.
- Granular proposal/issue mapping has **not** been performed; source members retain parent source IDs and canonical grouping IDs for the separate mapper.

Raw extraction shards and refinement files remain preserved as inputs. Earlier assignment/refinement drafts are retained and explicitly excluded by the builder. Parent mapping should use this canonical file by the SHA-256 above.
