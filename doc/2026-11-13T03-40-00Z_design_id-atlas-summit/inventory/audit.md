# ID/Atlas statement inventory audit

This is a mechanical inventory audit of the proto design, not a review or completion claim for the full design.

## Inputs and reproducibility

- Source: `doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md`
- Source SHA-256: `51c331696f822f7751bf56f1e6729806ec49ff4a30e9b17124b3544c818985ba`
- Statement-map SHA-256: `35958df8783861275b9f0035b0b85f021eee2b6f83867390bbcf007217865d7a`
- Oracle: section 2's declared per-section counts require precisely the contiguous IDs `Sx.1..Sx.N`; the document's declared total must equal the parsed definition count. Section 3 ranges are expanded inclusively against those definitions. A statement is mapped when the map cell assigns it one or more bold issue homes (`**#NNNN**`).
- Independent fixed oracle in the checker: S0 17, S1 120, S2 19, S3 38, S4 93, S5 64, S6 17, S7 16 (total 384). Editable source declarations are checked against, not used to define, this oracle.

Run from the repository root:

```sh
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/inventory/check_inventory.py \
  --source doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md \
  --json doc/2026-11-13T03-40-00Z_design_id-atlas-summit/inventory/statement-map.json
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/inventory/verify_inventory_regressions.py \
  --source doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md
```

Both commands completed successfully. The regression suite passed 11 deterministic cases. Re-running the checker produced the same JSON SHA-256. A malformed invocation with an undeclared `--bogus` option exited 2 before reading/writing inputs, confirming strict argument rejection.

## Results

- Declared counts: S0 17, S1 120, S2 19, S3 38, S4 93, S5 64, S6 17, S7 16 (sum 384).
- Parsed: 384 definitions, 384 unique IDs; no missing or duplicate definitions; declared total 384 agrees.
- Expanded every comma-delimited range token in section 3: 0 unmapped statements. Malformed terms, unknown endpoints/sections, reversed ranges, missing/invalid bold homes, and a missing/empty map section are rejected. The only non-range row is the explicitly recognized administrative `Sequencing of all of the above` row.
- The checker stops reading map rows at the next top-level section, rejects source/output path aliasing (including hardlinks when both paths exist), and validates the entire CLI before source reads or output writes.
- The JSON has one row per statement, preserving its text and source line. `issue_homes` contains distinct bold home issue IDs; references that are not bolded homes (for example, amendments/consumers in parentheticals) are intentionally not treated as assignments.

## Mapping overlaps and semantic review flags

The map deliberately/explicitly gives multiple homes to these ranges; this checker reports overlaps and does not adjudicate ownership:

- S1.114–S1.120: #9106 program-level non-specialness and #9110 conformance audit.
- S4.1–S4.18: #9116 namespace model/registry and #9117 `id://` URI design.
- S4.29–S4.33: #9118 array-address mechanics and #9119 shorthand/default-index behavior.
- S5.1–S5.12: #9125 triple encoding and #9126 graph query/visualization.
- S4.20 is listed twice within the same `S4.19–S4.22, S4.20` map row, yielding a repeated #9120 assignment. The output JSON normalizes this to one distinct home; the source redundancy is retained and called out here.

Core ownership merits a design-owner decision: the map assigns S4.19–S4.22 (including both the content-addressed core S4.20 and array-address core S4.21) to #9120, while S4.23–S4.40 mechanics are assigned to #9118 and S3.1–S3.10 ID shapes (including content-addressed hash statements) to #9113. This may be intentional layering, but the content-addressed core's ownership boundary is not self-evident from the map. The duplicated S4.20 token should also be confirmed as intentional or removed in a later authored source change. No proto source was modified in this inventory pass.

Other shared ranges above are sensible cross-cutting seams but should retain explicit acceptance criteria across both homes. No conclusion about the adequacy of any issue body, requirements, architecture, implementation, or overall design follows from this count/map check.

## Regression cases

`verify_inventory_regressions.py` runs 14 cases with only the Python standard library: valid real source, duplicate definition, missing map coverage, a later-section fake mapping that cannot mask missing section-3 coverage, valid real IDs plus unknown endpoint `S0.999`, malformed map syntax, reversed range, malformed home, declared-count mismatch, absent map section, invalid CLI with no output, direct source/output alias protection, hardlink alias protection, and symlink alias protection. It uses `TMPDIR` when set (otherwise `/home/user/.local/share/opencode/tmp/opencode`) for temporary fixtures and checks rejected fixtures do not produce JSON output or mutate aliased source bytes.
