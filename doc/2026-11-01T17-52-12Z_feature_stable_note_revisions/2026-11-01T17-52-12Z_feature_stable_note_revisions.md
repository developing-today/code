# Stable note identity and revision graph

## Request and intent

Implement Phase 1 of [issue #3814](https://github.com/developing-today/code/issues/3814), under the Zettelkasten design in [parent issue #2048](https://github.com/developing-today/code/issues/2048#issuecomment-6560927674). A logical note needs a stable identity across edits, while each immutable signed revision remains independently addressable. Preserve existing `document.put` compatibility and keep note deletion/erasure out of this phase.

## Initial design

- A signed `note.create` event establishes the stable note ID; its event ID is that note ID.
- A signed `note.revise` event has its own immutable revision ID and names the stable note ID, verified blob content reference, and causal parent revision IDs.
- Concurrent revisions remain separate heads. A merge is an explicit signed revision naming the heads it joins; history and head selection must be deterministic and independent of event arrival order.
- Use the existing signed-event and blob-reference substrate. Validate operation shape and references before folding them into local note history. Existing non-note document operations retain their current wire representation and behavior.

## Scope and exclusions

This phase covers note event representation/validation, local history and head queries, explicit-merge semantics, and focused Rust tests for malformed references, content verification, revision lookup, concurrent heads, and merge. Editor/UI behavior, live collaboration, topic/backlink indexing, search, cross-store name resolution, shared-write policy, and deletion/erasure are not implemented here. The deletion promise and shared-write authority remain decisions for Drew as recorded in #2048; this phase does not make either decision implicitly.

## Initial implementation plan

1. Add typed note create/revise operations to the signed extension and wire model, with strict validation and compatibility coverage for existing document events.
2. Fold valid note events into local history/head queries, preserving concurrent heads and requiring explicit merge parents.
3. Add focused tests for validation, blob verification, deterministic history/heads, and explicit merge; run with the `world` feature because the no-default-features baseline is blocked by `petname` feature gating.

## References

- [Issue #3814](https://github.com/developing-today/code/issues/3814)
- [Decision-ready design record for #2048](https://github.com/developing-today/code/issues/2048#issuecomment-6560927674)
- [Documentation protocol](../DOCUMENTATION_PROTOCOL.md)
