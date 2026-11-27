# #9208 next slice: canonical pair validation

Lane `code-i`; worker session `ses_e5de35c871effeA7Ms8VX4Tehj`; parent
`ses_e5de368ceedffeD0MeFrQD3PeQ`. Shortlist assignment, no queue claim.
Baseline: `aa906a4d9e6` (`origin/main` at start).

## Exit-criterion census

The parent is incomplete. A closed child or design record is not runtime proof.

| Parent criterion | Baseline state and evidence |
| --- | --- |
| Seven relation spellings and canonical direction | Partial: `work/relations.rs` `RelationSpelling`, `normalize`, and `pmo_dep_aliases_normalize_to_one_canonical_edge`; JSON validation did not enforce symmetric pair ordering. |
| Hard blockers affect eligibility and effective urgency; advisory edges do not block | Unmeasured end to end. `Edge::validate` restricts hard edges to `blocks`; this does not establish live scheduling integration. |
| Cycle prevention and resolved-history preservation | Partial: `EdgeRevision::validate` and transition tests preserve identity/history; graph concurrency/production cycle prevention not established by this slice. |
| Native GitHub links accurately projected; hierarchy distinct from hard dependencies | Unmeasured adapter behavior; canonical `Relation::Hierarchy` is distinct from `Blocks`, but DEP-04 #9444 remains open. |
| Queue titles/summaries independent after initialization | Unmeasured integration behavior; AUTH-01 #9429 remains open. |
| Private notes and append-only journal separate from public comments | Unmeasured integration behavior; JRN-02 #9464 remains open. |
| Explicit managed-content mirror with truthful concurrency guarantee | Partial: `work/authority.rs` denies body updates without verified conditional-write capability or explicit best-effort opt-in. This is pure policy, not adapter/live proof; managed comments #9430 remain open. |
| Priority label normalization from canonical score | Unmeasured here; belongs to the priority-distribution component, not this validator. |

Paths in this table are relative to `pkgs/fleet/crates/fleet-queue/src/`.

## Bounded acceptance

After this PR, `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue
--lib --no-default-features pmo_dep_wire_` rejects reversed
`related_to`/`duplicates` wire records, accepts canonical records, and leaves
ordered `blocks`/hierarchy/supersedes untouched. The oracle is DEP-02 D1:
authority byte order, then numeric work ID; an external end is the source.
The generated test checks distinct numeric work pairs; examples check authority
ordering and external ends. Rejection does not silently change duplicate roles.

This hardens the already-landed DEP-02 pure contract, not the separately gated
DEP-03/04 storage, graph or adapter implementations. No queue, schema, CLI,
GitHub write, deployment, or production adoption is added.

Next slice: after a follow-up PR, the DEP-04 pure native-link mapping fixture
shows GitHub `blockedBy` stays separate from parent/child hierarchy and never
infers Fleet completion from an issue close. Keep adapter/storage rollout gates
separate from that pure-contract evidence.
