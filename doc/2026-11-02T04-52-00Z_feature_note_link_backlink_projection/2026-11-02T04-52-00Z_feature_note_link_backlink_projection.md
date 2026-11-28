# Note links and the head-based backlink projection

## Request and intent

Implement Phase 2 of [issue #3816](https://github.com/developing-today/code/issues/3816), under the Zettelkasten design in [parent issue #2048](https://github.com/developing-today/code/issues/2048#issuecomment-6560927674), on top of the stable note identity and revision graph from Phase 1 ([issue #3814](https://github.com/developing-today/code/issues/3814), design record `2026-11-01T17-52-12Z_feature_stable_note_revisions`). Notes need typed, directed references to other notes — topic links — and a deterministic way to ask the inverse question, "which notes link here?" Generic tags do not define directed edges, and the existing metadata search is not a note graph.

This record is the design record the `pkgs/id` documentation protocol requires for the new signed event type and public projection API added here. It is written alongside the implementation in PR #4131 (Codex review finding on `pkgs/id/src/note_links.rs`: the change initially landed without this record).

## Design

- A signed `note.link` event carries a `NoteLinkOp::Assert` in the header extension (`pkgs/id/src/chain.rs`, `link` field): the source note's stable ID, the source revision that asserts the reference, the target note's stable ID, the authored surface verbatim, and an optional pin naming one exact target revision.
- References name stable IDs, never titles or paths. The authored surface is kept verbatim and is never used to resolve the link; resolution by phrase or title is the separate resolver work tracked by issue #2259, which this phase does not duplicate.
- The link projection (`pkgs/id/src/note_links.rs`, `project_links`) is a deterministic derived fold over the folded notes and the applied link events. It is never separately mutable canonical state: backlinks are computed as the inverse of the selected edges, not stored.
- Head-based semantics: an asserted link is an edge of the projection only while its asserting revision is a head of the source note. Revising a note so the asserting revision is no longer a head retires the edge; concurrent heads each contribute their own edges, and no head is silently preferred over another.
- A pinned reference keeps naming its pinned revision as the target's heads move on; an unpinned reference follows the target note as a whole.
- Determinism: edges are sorted by (source, target, surface, link event ID), so the projection is identical regardless of event arrival order. The projection rejects duplicate link event IDs, links whose source or target note has no create, asserting revisions that are not revisions of the source, and pins that are not revisions of the target (`LinkProjectionError`).

## Access filtering

Access filtering is a query-time view, not a second projection. The caller names the set of notes a viewer may access, and the filtered queries (`links_from_visible`, `backlinks_to_visible`) drop every edge that would expose an inaccessible note: an edge to an inaccessible target is dropped whole, because its existence is target metadata, and a backlink from an inaccessible source is dropped together with its surface, because the surface is the source's authored content. Nothing about an inaccessible note — not its content, not its existence — crosses the filter. Group and write authority are unchanged by this phase.

## Retention interaction

Pruning behind a causal cut (`EventStore::prune_behind_cut`) treats a link operation's references as dependencies, the same way it treats a revision's note references: a surviving link keeps its source note, asserting revision, target note, and pinned revision alive. Without this, pruning a target note's create on another author's chain would leave a surviving link whose projection rebuild then fails with `UnknownTarget` (Codex review finding on PR #4131, fixed in the same PR).

## Scope and exclusions

This phase covers the `note.link` event representation and validation, the local per-store link/backlink projection, access-filtered queries, retention interaction, and focused tests. Cross-store fan-out, a global index, full-text indexing, phrase-resolution UI or candidate ranking (#2259), and any change to group or write authority are out of scope.

## Key decisions

1. **Links are events, not note fields.** A link assertion is independently signed, ordered, and retracted by head movement; folding it into the note body would couple link history to content revisions.
2. **Backlinks are derived, never stored.** One canonical direction (the asserted edge) and a computed inverse cannot disagree; a stored inverse would be a second source of truth to keep consistent under concurrency and pruning.
3. **Existence does not leak.** Access filtering drops whole edges rather than redacting fields, because per-note existence is itself private metadata in this system.

## References

- [Issue #3816](https://github.com/developing-today/code/issues/3816)
- [Issue #3814](https://github.com/developing-today/code/issues/3814) and its design record `2026-11-01T17-52-12Z_feature_stable_note_revisions`
- [Decision-ready design record for #2048](https://github.com/developing-today/code/issues/2048#issuecomment-6560927674)
- [Documentation protocol](../DOCUMENTATION_PROTOCOL.md)
