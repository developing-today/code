# Fresh inventory judge — #9106 / task 15354

**Scope:** inventory artifacts only: the proto statement inventory and issue map, `inventory/`, and `inventory-live/`. This does not judge the overall design.

**Round 1 verdict (historical): DISAGREE with inventory acceptance as-is.** The statement count/ID inventory was independently supported, but the checker had an unknown-endpoint false-zero hole and the live hierarchy audit contradicted its JSON evidence. The following second-round review records the gap-fill and current scoped verdict; the first-round findings and checks remain below as history.

## Second-round re-review (after gap-fill)

**Current verdict: AGREE with the 384-statement numbered inventory and the numbered statements' live issue homes, within this limited scope.** I do not agree that this establishes overall task/program acceptance. Original Part B candidate provenance remains unrecovered, and sequencing completeness remains a separate open program obligation while the stated fill is in progress. Neither is silently counted as solved by this agreement.

### Rechecked evidence

- Re-ran preflight: clean, still on the designated worktree/branch.
- Ran `python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/inventory/check_inventory.py --source doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md --json doc/2026-11-13T03-40-00Z_design_id-atlas-summit/inventory/statement-map.json`: success; fixed oracle reports S0–S7 counts 17/120/19/38/93/64/17/16, 384 unique IDs, zero unmapped. JSON SHA-256 remains `35958df8783861275b9f0035b0b85f021eee2b6f83867390bbcf007217865d7a`, matching the recorded audit hash.
- Ran `python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/inventory/verify_inventory_regressions.py --source doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md`: all 11 cases passed: actual source, duplicate definition, missing mapped definition, valid-plus-unknown endpoint, malformed syntax, reversed range, invalid home, count mismatch, absent map, invalid CLI no-output, source/output alias protection.
- Independently exercised the CLI using temporary mutated copies of the full source: `S0.1, S0.999` rejected with `unknown map endpoint: S0.999`; `S0.1; S0.2` rejected as a malformed token; cross-section `S0.1–S1.2` rejected. Each exited nonzero and created no output. The full parser now consumes each comma-delimited token with `fullmatch`, checks both endpoints against actual IDs, rejects cross-section/reversed ranges, and only writes after all inventory checks. The fixed section/total oracle prevents source declarations from redefining the expected 384.
- Re-enumerated the live JSON: 52 requested records, 52 issue records, no missing records. The statement map has 384 rows, all with homes; its 39 distinct home IDs all exist in the captured live issue set. The corrected `semantic-audit.md` now reports the mixed hierarchy accurately: #9106 has six direct subissues; #9107/#9108/#9109 have 20/8/8 native children; total program hierarchy 42 edges, with #9113–#9148 component issues having no children of their own in this snapshot. `candidate-provenance.md` explains the different serialization shapes and corroborates this count.
- Inspected the cited live #9106 question comment [6843258938](https://github.com/developing-today/code/issues/9106#issuecomment-6843258938). It records bounded unsuccessful recovery and the transparent statement-complete replacement as the default, not as the recovered original 30. `candidate-provenance.md` likewise labels its 30 slices as a reconstruction, separates named from statement-derived items, and calls out the interop-category boundary ambiguity. Thus no provenance is invented; exact original candidate recovery remains unresolved.

### Separate obligations, not part of this agreement

- **Original Part B candidate provenance:** still not recovered. The replacement decomposition may be useful work, but it is not evidence of the original 30 candidate identities/boundaries or proof that the exact original slices are filed. Keep the gap and its recovery evidence explicit until reconciled or explicitly dispositioned by the program owner.
- **Sequencing:** the captured `semantic-audit.md` still identifies 12 previously omitted component IDs: #9121, #9122, #9123, #9128, #9129, #9130, #9131, #9132, #9136, #9137, #9145, #9146. Per the current task update, a cross-cut worker is filling all 39 components and a Part B worker is producing at least 30 actual slices, but that work is underway, not verified complete in the artifact snapshot I reviewed. Re-run the live #9112 sequence comparison when those changes land; do not mark schedule coverage complete based on this judge pass.

This is agreement only on **statement count/ID completeness, strict parsing/zero-unmapped check, and existence/assignment of numbered statement homes in the reviewed live snapshot**. It does not certify original-candidate provenance, completed granular implementation issue filing, complete sequencing, the remaining design-mill acceptance checklist, or the overall design.

## Independent count and ID oracle

Ran `bash "$(git rev-parse --show-toplevel)/scripts/worker-preflight.sh"`: clean; no shadowing `refs/heads/origin/main`.

Independently read the proto section 2 with a small inline `python3 -I` extractor (count headers and `Sx.n` definitions; no import/use of the worker checker or JSON map). Results:

| Section | Expected | Independently found |
|---|---:|---:|
| S0 | 17 | 17 |
| S1 | 120 | 120 |
| S2 | 19 | 19 |
| S3 | 38 | 38 |
| S4 | 93 | 93 |
| S5 | 64 | 64 |
| S6 | 17 | 17 |
| S7 | 16 | 16 |
| **Total** | **384** | **384** |

All IDs are contiguous from `.1` through the declared maximum; no duplicate, missing, or extra IDs. This independently confirms the numerical inventory claim (the proto’s section 2 source, not merely the checker’s result).

## Checker review and negative fixtures

Inspected `inventory/check_inventory.py`; then ran in-memory negative fixtures through its `parse()` and separately exercised its CLI. Outcomes:

- **Duplicate definition:** parser returns duplicate `S0.1` entries. The CLI’s `actual != expected` check rejects it (the reported error is the generic count/ID-range mismatch; it also prints duplicate IDs). Rejects, though a direct duplicate error would be clearer.
- **Map points to missing definition:** changed the map target to `S0.2` while the only definition remained `S0.1`. Expansion ignores the nonexistent target; `S0.1` remains unmapped and the CLI’s unmapped check rejects. Rejects this simple case.
- **Count mismatch:** declared 2 with only `S0.1`. CLI range check rejects. Rejects.
- **Unknown map endpoint (BLOCKER):** map expression `S0.1, S0.999` with a valid assignment for `S0.1`. `MAP_RANGE.findall` accepts only the `S0.1` substring; the unknown endpoint is silently ignored, the real definition is mapped, and the checker reports zero unmapped. No validation requires each non-separator token in a map expression to be a recognized, in-range endpoint. A typo/unknown extra endpoint can therefore coexist with a false-zero-unmapped success. Parse/validate the complete range expression, reject malformed syntax and any endpoint whose section/number is undeclared, and add a regression fixture for a valid range plus an unknown endpoint.
- **Invalid CLI side effects:** invoked with nonexistent source and `--bogus x`; exit 2, usage/error emitted, and the designated output path was not created. The argument parser runs before source read/output write, so this tested malformed invocation has no file side effect.

The successful normal checker run recorded in `inventory/audit.md` is plausible and its reported 384/zero-unmapped result matches the JSON on inspection. But the unknown-endpoint hole means the checker is not yet an adequate independent proof of zero unmapped under map-edit errors. The JSON map has one row per expected definition and assigned homes; spot-checked boundary/shared ranges, including S1.114–120, S4.1–18, S4.29–33 and S5.1–12, match the proto’s stated home rows. The duplicate S4.20 token is transparently disclosed: it repeats the same #9120 home and the JSON deduplicates it. Keep that as a non-blocking source cleanup/ownership confirmation, not a count error.

## Live existence, hierarchy, and ownership

Independent JSON enumeration of `inventory-live/live-issues.json` found 52 records for 52 requested issue numbers, no `missing:true` records, and no requested number absent. Issues #9107–#9148 are OPEN in this snapshot; #9106 is CLOSED. Parent evidence in the records is consistent with the declared program/part split: #9107/#9108/#9109 have parent #9106, and their respective component children have parent #9107/#9108/#9109. The #9106 closed state is correctly qualified as intake closure, not design completion.

**Hierarchy audit inconsistency (BLOCKER):** `semantic-audit.md` lines 68–69 says the snapshot has “six native child edges (the six children of #9106)” and says the component issues have no native child issues. The JSON does show six *direct* children on #9106, but also lists 20 native children on #9107, 8 on #9108, and 8 on #9109: **42 native-child edges total**, of which 36 are the component issues. These are also corroborated by the child records’ `parent.number` values. Correct the audit to distinguish six direct edges from the 42 total, and state that the 36 component issues have no children of their own. This is a mismatch between audit claim and its cited snapshot, not a missing-issue finding.

Other live-audit conclusions needing disposition:

- **Exact 30 Part B candidate set/provenance (BLOCKER for claiming the candidate inventory is complete):** the proto claims 30 candidate sub-items but identifies only seven category labels (key lifecycle, replay protection, acknowledgment primitive, prioritization scoring, reputation-abuse research, privacy surface, interop kit, plus “and more”). The live snapshot confirms no native child issues under #9141–#9148, but does not establish that no candidates were filed elsewhere. Neither artifact contains the original task-15354 intake research list. Preserve the audit’s appropriately cautious wording; recover and cite the original listing, or explicitly mark exact-set reconciliation as unresolved. Do not manufacture the remaining 23.
- **Twelve sequencing omissions (BLOCKER for claiming a complete schedule):** comparison of the proto/live #9112 now/next/later lists against components #9121–#9148 finds these absent from every slot: **#9121, #9122, #9123, #9128, #9129, #9130, #9131, #9132, #9136, #9137, #9145, #9146**. The audit correctly calls this out. Give each a slot or explicit disposition in #9112; no component should disappear from the schedule.
- **Semantic ownership:** sampled and cross-checked the issue descriptions and map ranges against the snapshot. The displayed shared ownerships are materially sensible when described as split responsibilities (e.g. S4.1–18 #9116 registry/model vs #9117 URI; S5.1–12 #9125 representation vs #9126 query/viz; S4.29–33 #9118 core semantics vs #9119 shorthand). Explicitly keep these seams in the audit/designs. #9120’s duplicate S4.20 listing is same-home redundancy, not competing ownership. Candidate follow-up labels (ack/prioritization/abuse/interoperability) must remain provisional until their provenance is recovered and actual issue filing verified.

## Commands / evidence record

1. Preflight: `bash "$(git rev-parse --show-toplevel)/scripts/worker-preflight.sh"` → clean.
2. Inline `python3 -I` source extractor over the proto → section totals above, 384 IDs, no duplicates/gaps/extras.
3. Inline `python3 -I` JSON enumeration over `inventory-live/live-issues.json` → 52/52 present, no missing records; printed all #9106–#9148 states, parents, child lists. Child counts: #9106=6, #9107=20, #9108=8, #9109=8; 42 total.
4. Inline import/in-memory fixture checks against `check_inventory.py:parse()` → duplicate/missing-target/unknown-endpoint/count outcomes above. In particular `S0.1, S0.999` silently accepted as a valid `S0.1` mapping.
5. `python3 -I check_inventory.py --source <nonexistent> --json <temp-output> --bogus x` in a temporary directory → exit 2, output absent.

No worker artifact or Git state was changed. This review file is the only deliverable from this judge pass.
