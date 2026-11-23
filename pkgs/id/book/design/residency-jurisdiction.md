# Design: residency, jurisdiction and boundaries

This chapter covers 11 design records, the smallest design theme in the book. The survey keeps it as its own chapter rather than dissolving it into the hosted-editions chapter, for a stated reason: residency is a named programme thread with its own blocker chain, #3399 → #3400–#3404. It describes the system as designed in the programme's design archive. The final section states what runs today, including the one part of this design that is written, landed code with no surface wired to it yet.

## The framework: declarations, evidence, enforcement

The framework record is `2026-11-08_design_id5683.md` (2026-11-08): the residency and boundaries framework — declarations, evidence, and an enforcement skeleton. The design's three-part structure is the chapter's structure:

1. A declaration states where data of a given class may reside and which boundaries it may cross.
2. Evidence states the facts a verifier can check: where a store is, where a request was routed, which jurisdiction a node claims and can prove.
3. Enforcement acts on declarations plus evidence: admit, route, restrict or refuse.

The evidence model has its own decision record: `2026-11-09_design_id5876_residency_evidence_model.md` (2026-11-09), the residency evidence model for ID. The enforcement inputs are designed in `2026-11-17T21-55-00Z_design_id8851_boundary_inputs.md` (2026-11-17): boundary enforcement inputs — declaration residence, evidence sources, and the verifier (#8851). A boundary decision in this design is a function of a stored declaration and named evidence sources, checked by a named verifier; it is not an operator's judgement call at request time.

The audit side is designed in `2026-11-08T09-15-00Z_design_boundary_audit_appeals.md` (2026-11-08): boundary audit and appeals UX — a boundary refusal produces an audit record, and the person refused has an appeals path with a designed interface. The sanctions boundary is a data model, decided in `2026-11-10T19-52-00Z_design_id_sanctions_boundary` (2026-11-10): the sanctions boundary data model, decision record for #5878. Sanctions enter the design as data with a model, not as a hard-coded list in a request path.

## Jurisdiction-aware stores and partitioned worlds

Two records of 2026-11-01, plus a third under a plainer name, design the hosted store side:

- `2026-11-01_design_id2283.md` (2026-11-01): jurisdiction-aware per-user hosted stores.
- `2026-11-01_jurisdiction_aware_user_store_design.md` (2026-11-01): jurisdiction-aware hosted user stores — a second record on the same subject, filed without the issue-number naming of the first.
- `2026-11-01_design_id3397.md` (2026-11-01): jurisdiction-partitioned hosted worlds and world blobs — a hosted world and its blobs reside in one jurisdiction partition, as a property of the world, not of each request.

The platform mechanism underneath the store design is investigated in `2026-10-29T11-43-02Z_investigation_do_jurisdiction_pinning` (2026-10-29): Durable Object jurisdiction and data-location pinning for per-user stores — whether the hosted platform can pin a per-user object to a jurisdiction at all, which the store design depends on.

`2026-11-01_design_id3398.md` (2026-11-01) extends boundaries beyond the hosted store: residency boundaries for the native bridge, federation and relays. A boundary in this design applies to the bridge between hosted and native nodes, to federation traffic, and to relays, not only to data at rest. `2026-11-01_investigation_shared_world_residency.md` (2026-11-01) investigates the hardest case: shared-world residency and federation — a world with participants in more than one jurisdiction has a residency question that no single-partition rule answers, and the record investigates the options instead of deciding one.

The hosted edition that consumes these designs is covered in [Design: hosted and serverless editions](hosted-editions.md); the sync machinery that moves data across the boundaries is in [Design: sync, replication and FileOps](sync-fileops.md).

## The #3399 → #3404 chain

The jurisdiction chain is tracked as #3399 → #3400–#3404. Within the chain, #3404 is the fenced jurisdiction migration control plane: the component that migrates data between jurisdictions under the declaration and evidence rules above, with fencing so that a migration in progress cannot be raced by ordinary writes.

The status of #3404, stated plainly as of 2026-11-23: the migration control plane core has landed. The complete slice — `migration_control.rs` in `id-core`, 1,202 lines with 17 tests, plus 3 wiring edits — reached `main` on 2026-11-23 via PR #13347, after two sessions ended without landing it, a successor lane's transplant attempt tripped the platform execution-risk flag, and the owner's approval naming #3404 cleared the hold. The landed code is the core only: no CLI command and no hosted surface invokes it, and #3404 remains open for the hosted wiring (#3399/#3400/#3402), so this chapter does not describe jurisdiction migration as something an operator can run. The rest of the chain's record-level detail (#3400–#3403) is unverified in the survey material beyond the chain's existence and #3404's state.

## What exists today

Grounded in the code and configuration surveys, not the design records:

- Boundary configuration ships in the server config: the TOML server config carries a `[residency]` table and `[[boundaries]]` tables, with their forms owned by `crates/id-cli/src/server_config/boundaries.rs` (#8851). The design's declaration side therefore has a shipped configuration surface.
- Boundary cryptography ships as named domains: the boundary evidence domain is `id.boundary.evidence.v1` and the boundary declaration hash domain is `id.boundary.declaration.v1`.
- The hosted worker enforces jurisdiction at routing, in local form: the hosted scaffold routes each user only through the jurisdiction-restricted Durable Object namespace, objects are named by `binding_id` (32 lowercase hex characters, never an email or provider subject), there is no unrestricted fallback, and every UserDO route first checks that the runtime jurisdiction matches the jurisdiction bound immutably by the first routed request. Because local workerd does not implement jurisdiction restrictions, the scaffold fails closed locally with 503 `jurisdiction_unavailable` for every fully valid request, by design.
- Deployment of that enforcement is gated: the hosted worker's config states that deployment and the real-account jurisdiction canary are gated on #2285 and #3403.
- Landed as library code but not wired to any surface: the #3404 migration control plane core, as stated above. The remainder of the residency framework — declarations driving automated enforcement across federation and relays, and the audit-and-appeals interface — is design, with the records cited in this chapter as its complete statement in the archive.
