# FDR-PMO-005 — Typed dependencies, private journal, and field authority

- **Date:** 2026-11-13
- **Status:** accepted — local design contract only, not production authorization
- **Adopted:** Summit owner, task 15381, 2026-11-13; see `../reviews/component-contracts.md`. Existing fact owners and visibility rules remain binding.
- **Owner:** Summit design owner, task 15381 / session `ses_e5e2d10e06effeEylSDDVaN0PL` for this proposal; future maintenance by Fleet maintainers through #9180.
- **Decision maker:** Delegated Summit architecture owner for this design pass; adoption follows independent final design review by the parent and Fleet owner recording through #9180. No human name is inferred.
- **Scope:** typed graph, notes/events/checkpoints, field authority, managed public content and privacy boundaries
- **Live status scope:** `manual_only` remains a persistent no-automatic-claim eligibility property. A separately authenticated manual/owner claim may override it only where #9178 explicitly permits; Do Now alone never clears it or grants that override.
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-PMO-001, -002, -003, -004, -006

## Context and problem

Hierarchy, hard blockers, advisory links, notes, status reasons, public comments, and journal events have different semantics and authorities. Existing queue fields/events and #3424 are compatibility inputs, not proof of a complete typed graph or reconstructable event store. Private findings/checkpoints must not leak through automatic mirroring. GitHub comment/body endpoints reviewed in the component research do not document a conditional-write guarantee; a GET/compare/PATCH can race a human edit.

Sources: [architecture, “Identity and authority,” “Cycle-safe hard-dependency pressure,” “Lifecycle and cross-system reliability,” and “Implementation locality and ownership”](../architecture.md); [source 02 §§6, 12–13](../sources/02_charter_v1_1.md); [#9208, “Scope, code ownership, and evidence boundaries,” “Typed relationship contract,” “Notes, checkpoints, and append-only journal,” and “Field authority and external synchronization”](../components/9208-typed-dependencies-notes-journal-field-authority.md); [prior art B, “Typed relationships” and “Dependencies, journal, notes”](../research/track-b-state-of-art.md); [Track B census, “Finding” and “Compatibility and ownership”](../census/track-b.md).

## Options considered

1. **One typed relation fact, append-only journal facts, private-by-default notes, explicit managed projection (recommended).** Keeps domain semantics and authority visible while extending existing storage after crosswalk.
2. **Flat blocker list plus parent issue prose.** Simple representation but conflates hierarchy with eligibility and loses edge provenance/resolution history.
3. **Mirror all status reasons/notes into GitHub.** Convenient visibility but leaks private content, floods comments, and cannot safely overwrite user-owned text.
4. **Replace entire GitHub body on every update.** Straightforward remote API shape but can clobber unrelated human edits; pre-read is not CAS.
5. **Full event sourcing or second notes/graph database.** Rich reconstruction but duplicates authority and increases migration/version/retention burden before existing `events`/#3424 fields are crosswalked.
6. **Do nothing.** Retains ambiguous relationship and content ownership; cannot support safe readiness or audit.

**Tradeoff comparison:** Typed edges and append-only facts increase modeling and migration work but improve graph reliability, explainability, and future consumer extensibility. Flat lists are operationally cheap but do not scale semantically to hierarchy, provenance, or concurrent cycle checks. Automatic mirroring and whole-body replacement reduce UX steps but increase privacy and lost-update risk. Full event sourcing/parallel stores offer replay/query flexibility at much higher operations and migration cost; the proposed bounded journal retains audit value without that scale burden.

## Proposed operational decision

Represent relationships as typed, versioned, provenance-bearing edges with canonical orientation. Active hard `blocks` edges alone affect readiness and effective pressure; parent/child hierarchy and advisory relations do not block. Serialize graph revision validation and insertion so opposite-edge races cannot jointly create a cycle. Preserve resolved/removed history; incomplete graph evaluation is partial/unknown and fails closed for claim/completion.

Keep canonical task/work state in Fleet; GitHub owns its issue/PR body and discussion; provider owns actual session state; Project writes remain orchestrator-only. Separate private Fleet notes, structured status reasons, append-only observed/journal events, and public comments. Corrections append superseding facts; do not silently rewrite history. Untrusted issue/comment text is data, never authority-bearing instruction.

Private notes/checkpoints stay private by default. Public status projection is allowlisted; explicit note mirroring is a separate authorized operation. Default automatic managed-body-section write capability is unsupported unless a conditional-write capability is verified. A separately approved best-effort mode must disclose human lost-update risk and retain before/after evidence; GET/compare/PATCH is not optimistic concurrency. Prefer explicit known managed comment IDs where useful, still without claiming remote exactly-once creation.

## Rationale and evidence boundary

**Facts:** charter distinguishes native hierarchy from `blockedBy`, private notes from public comments, and explicit managed mirror. Census says existing `blocked_deps`, `events`, and related fields do not establish typed graph/journal semantics. Research reviewed comment APIs and found no documented conditional compare-and-swap in those endpoint contracts; this is bounded evidence, not a claim about every future endpoint.

**Inference:** separating field authority and relation types reduces accidental readiness changes and privacy leaks. A compact append-only journal alongside current state is a narrower evolution than full event sourcing.

## Consequences, compatibility, and migration

- Crosswalk all six #3424 pieces and #8575/#8579 against existing schema before adding parallel columns/tables. QTI remains unlocated; #9241 research and owner disposition gate schema-coupled changes.
- Do not invent table names, storage ownership, graph materialization, or full replay claims here. Pure relation/reference model and privacy fixtures can precede schema.
- Imported ambiguous/cyclic edges require visible quarantine/uncertainty and conservative readiness; do not silently repair by deletion.
- Managed-body mutation remains opt-in and capability-gated; no race-free claim absent verified CAS. Public writes have their own operation receipt (FDR-PMO-006).
- No new database, journal service, automatic mirror, retention policy, or production permission is authorized.

## Negative cases and test oracles

- **Graph model:** parent edge never blocks; only active hard blocker does; opposite concurrent inserts produce one accepted edge and one cycle conflict; unresolved hard external ref is not assumed satisfied.
- **Journal oracle:** correction appends supersedes reference; replay from partial records is not advertised as complete reconstruction.
- **Privacy oracle:** private note/checkpoint absent from Project/GitHub/public hydration unless explicit authorized scoped operation exists.
- **Authority oracle:** actor field cannot authorize; field owner table rejects unauthorized cross-owner mutation; Project write goes only through orchestrator.
- **Managed content oracle:** without verified conditional-write capability, automatic body-section write is unsupported; best-effort mode displays risk and detects divergence but test does not assert prevention of race.
- **Migration oracle:** compatibility fixture covers #3424 fields and current relation/event semantics before schema change.

## Revisit triggers

Reopen on authoritative #3424/#8575/#8579/QTI crosswalk, verified GitHub conditional update capability, changed Project ownership, measured graph scale requiring a different representation, or approved privacy/retention policy change. A stronger mechanism must preserve the same ownership and explicit failure semantics.

## Implementation, research, and source links

- Existing: [#9208](https://github.com/developing-today/code/issues/9208), [#9199](https://github.com/developing-today/code/issues/9199), [#9209](https://github.com/developing-today/code/issues/9209), [#3424](https://github.com/developing-today/code/issues/3424), [#8575](https://github.com/developing-today/code/issues/8575), [#8579](https://github.com/developing-today/code/issues/8579), [#9241](https://github.com/developing-today/code/issues/9241).
- Existing granular component keys: #9208 `DEP-01`–`DEP-05`, `JRN-01`–`JRN-03`, `AUTH-01`–`AUTH-05`, `VAL-01`, and `DOC-01`; #9206 `CLI-07/08`; #9209 `REL-05/06`. No third backlog is created here.
- Pinned code baseline `a038f084dbeb525494d7bb928ad91f597db2ac51`; see Track B census and #9208 for bounded line-level anchors.
