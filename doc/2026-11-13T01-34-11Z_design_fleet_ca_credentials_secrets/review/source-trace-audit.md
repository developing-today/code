# Independent source-trace audit — frozen canonical inventory

## Verdict

**AGREE with the audited source-inventory scope, with the review limits below.** The one concrete oracle gap in the prior review is **CLOSED** in this freeze: the two register child oracles and the linked §15.4 member now test the complete required issue fields, negative missing/vague-field cases, and the research/deferred-versus-committed distinction. I found no remaining known blocker in the scope audited here.

This is not a claim that I manually certified the semantics of all 935 normative/proposed records or all 1,825 nested outcomes. The validator establishes mapping, schema, source witnesses and oracle presence; it cannot by itself establish that every statement/oracle is semantically sufficient.

## Frozen identity and source-byte checks

- Canonical `inventory/statements.jsonl` SHA-256: **`e8158639602683cfb948e14fad4a774faa8169f6070f7c3f44e6e4062da86d58`**; 1,496 unique source IDs.
- The byte oracle is `source-snapshots/intake-34faffbff2/sources/` at source commit **`34faffbff26615e313488f6d04705a444342e41e`**, as recorded by `source-integrity.md` and `source-snapshots/provenance.json`. Formatted `sources/` was not used as an oracle.
- Independently recomputed authoritative snapshot hashes and checked every canonical record's source path, valid inclusive line range(s), and exact excerpt, including the documented `\n\n--- source span ---\n\n` join for non-contiguous excerpts. Result: all 1,496 records match. The separately snapshotted auxiliary `note.md` record also matches its recorded hash and excerpt.
- Independently checked all **80 §31** and **78 Appendix A** register rows: exact source-line record and exact row excerpt exist for each. Freeze reports 219 primary/split references and zero anchor errors. No source ID/range change was made as part of the final oracle correction.
- Freeze/validation receipt reports 1,345 grouping IDs, 1,825 linked member outcomes, and 4,104/4,104 nonblank source lines mapped. These are structural results, not an issue count or a substitute for semantic review.

## Targeted semantic and precedence review

- **§31 and Appendix A:** compared register outcomes with canonical witnesses and siblings. Rechecked SEC-003 at line 1990; PKI2-013 digest/signature/key-usage and no-forced-SHA-256 constraints; PKI2-014 certificate/CSR handling and inherited profiles; daily initial inventory and reviewed plan; complete-scan deletion rules; immutable `zz.delete` linkage/retention; native-ID-preserving updates; exact 2050/2032 dates; issuer/root graph; output manifest metadata; and future-work classification. The known outcomes are represented in binding requirements or linked source-specific members, not only context/history records. No new omission was confirmed in the 158 register rows.
- **F-class:** rechecked all 16 previously identified children. They are now `nonblocking research`; a pinned-source scan found zero §31/Appendix A records labelled `binding requirement` on an F row. Future work remains represented without promotion to baseline implementation.
- **Addendum:** read all ten original sections and reviewed all 52 records. Three-provider separation/authorization, benchmark-not-anecdote, fnox/SecureStore/OpenBao evaluation status, exactly three initial root escrow destinations, R2 research-only, same-host failure-domain limits, selective replication, freshness/revocation and redaction are retained. No false production/deployment claim or material clause loss was found in this pass.
- **Implementer handoff:** all ten `HANDOFF-GATE-01`…`10` records map individually to original lines 87–96 and have distinct deciders/expected invariants. They remain future implementation/safety obligations, not current implementation evidence or authorization. Prior omnibus-linkage concern is resolved.
- **Precedence/input 3:** reviewed source-precedence rules and all 16 listed resolutions against the pinned inputs. The 61 input-3 records are historical proposal/context and retain per-record §47 reconciliation plus links to integrated PKI/V3 outcomes where applicable. §47 is treated as a crosswalk, not a substitute for the original input. No blanket out-of-scope or blanket current-normative classification was found.

## Final condition rechecked and closed

Compared the corrected records directly with pinned source §15.4 line 953, which names user goal, prerequisites, public/security implications, full CLI/library surface, versioned data model/format if needed, observable output, failure modes, minimal test, documentation, relative complexity, priority/sequence, acceptance criteria, and explicitly separates research from committed engineering while allowing actionable high-effort deferral.

- `SPEC-Q-pki065a` now uses a future-roadmap packet validator with a complete row, each missing-field negative variant, unjustified N/A, vague-only row, and research/committed/deferred statuses; expected results require every named field and status distinction.
- `SPEC-Q-pki065b` now checks prerequisite IDs, qualitative complexity, priority/sequence, and rejects missing/unknown/vague/calendar-substitute cases.
- Nested `SPEC-0016#atomic-083` now checks the full §15.4 field set, reasoned N/A, each missing-field negative case, and distinct research/committed/deferred examples; all negative cases fail and valid cases pass.

These are discriminating, source-specific oracles rather than “assert the stated behavior” placeholders. **Prior finding: CLOSED.** The freeze states IDs/member cardinality remain unchanged; the current canonical SHA above was independently recomputed.

## Review limits

I independently checked exact byte provenance for all 1,496 canonical records, all 158 register rows, the corrected F classifications, all addendum sections, all ten handoff gates, input-3 precedence handling, and the targeted high-risk semantic groups listed above. I inspected representative linked members and concrete invariants, including rotation recovery, expiry, manifest metadata, and the revised roadmap oracles.

I did **not** read and independently adjudicate every one of the 935 normative/proposed statements or all 1,825 member oracles. Therefore this is agreement with the audited scope and the absence of a known blocker in it—not exhaustive human semantic certification. Granular proposal/issue mapping and production verification are separate stages and were not assessed.
