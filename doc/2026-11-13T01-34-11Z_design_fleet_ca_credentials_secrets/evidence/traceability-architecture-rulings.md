# Traceability owner rulings — proposed coverage is not implementation

These Summit rulings address a systematic error in draft mapping shards:
an original source cannot name proposal IDs created later, and a future
implementation issue need not already be implemented to own a requirement.
Neither absence is evidence of a missing design owner.

The mapping must still compare the actual clause, packet scope, component
design and acceptance oracle. It must not infer ownership solely from a
starter number or keyword. Where a coherent owner needs an additional
explicit acceptance clause, enrich the final issue **before** its reviewed
payload hash is frozen; do not mark an unrelated or uncovered outcome covered.

`implementation_status` remains not-implemented, research, verified-existing,
or the actual evidence state. Planned coverage is a separate dimension.

## Concrete owner decisions

| Source outcome | Proposed primary owner / support | Basis and oracle |
|---|---|---|
| `SPEC-R-26crates`, `26bench`, `26granularity` | ARCH-10; architecture seam comparison and ARCH-07 descriptor graph | Feature/dependency closure and measured size/startup/build report; explicit maintenance/license/unsafe-surface checks where needed; recorded rationale prevents crate-per-noun fragmentation. No claim the new crates already exist. |
| `SPEC-R-26resolver` | ARCH-01, with TRACE-02's compatibility corpus | Preserve the existing name-only entry and intentional stdout/error semantics while adding typed interfaces; fake-provider and golden CLI cases decide compatibility. |
| `SPEC-R-26singletruth` | ARCH-06; ARCH-07 and SEC-02 consume the common descriptors/types | Shared algorithm/error/material contracts rather than duplicate registries; concrete consumers have typed compile/schema cases. Split member ownership where the clause names distinct descriptor families. |
| `SPEC-0006` four roots and asymmetric root graph | ROOT-04, supported by ROOT-03's key/self-cert construction | Exact root edge set and key identity checks, followed by independent signature validation when implemented. The graph/schema and key-construction slices can be tested with controlled inputs, not a false mutual dependency. |
| `SPEC-0002-member-001` root-bundle definition | ROOT-02 schema contract; generation handled separately | A manifest/golden-SPKI fixture can assert four independent identities, correct variants and no one-key conflation without waiting for the production generator. Runtime credential IDs in ARCH-01 do not own the full PKI bundle merely because both concern identity. |
| `SPEC-0002-member-002` issuer-set definition | CERT-01, supported by ROOT-02's shared manifest contract | Typed purpose/partition/key/cert/artifact schema cases and invalid fixtures; actual issuer generation is a separate consumer. |
| `SPEC-0200` SSH adoption/config preservation | GEN-07/GEN-08, according to their specific adoption/file-edit boundaries | Native parsed-entry and byte-preservation fixtures, backup/rollback and unrelated-user-content negatives. No requirement that the planned verbs already ship. |
| `SPEC-0201` explicit root-creation/signing context | ROOT-03 with ROOT-05, ROOT-09 and OPS-10 support | Explicit source and exact authorized tuple before signing; generation/custody/activation are distinct phases. No implicit root lookup, actual vault write, or production authority follows from a mapping. |

Aggregate source witnesses may need a primary plus supporting owner references;
their independently named members still get explicit bindings. An unassigned
new subsystem or a genuinely missing API remains unresolved. A parameterized
case inside a clearly owned schema/profile is not a new subsystem merely
because its field name was not copied verbatim into a draft issue summary.

These are design-owner decisions to guide the final binding pass, not numeric
issue receipts, producer authorization or a claim that the entire matrix has
passed review.
