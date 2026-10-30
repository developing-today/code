# Design: friendly-name reference resolution — local store (spec for the #2259 PoC)

Status: spec for the proof of concept tracked by developing-today/code#2259.
Input: the #2047 investigation sketch (queue task 3355, posted on #2047
2026-10-29). Program rule (epic #2056): nothing production is built on this
spec until the PoC reports; this document specifies the PoC, not a feature.

Resolved location: this file lives in the repo-root `doc/` tree.
`pkgs/id/doc` is a symlink to `../../doc`, so "the spec in `pkgs/id/doc/`"
and this path are the same file.

## 1. Reference representation

A stored reference is a span/object, never the typed phrase:

```text
Reference {
  target_id:    Hash,        // the object's stable ID; the meaning
  sigil_kind:   Actor | Topic,   // '@' | '#'
  surface:      String,      // phrase as the author typed/chose it
  locator_hints: Vec<StoreId>, // stores where the target was found
  author_context: ActorId,   // who authored the reference
}
```

- Plain-text/markdown degradation: `surface` + the ID as link target,
  e.g. `[surface](id://<hash>)`.
- The phrase is a query, never the reference. Once stored, ambiguity
  ends: the ID is the meaning, the surface is a label.

## 2. Per-store index (local store only, for the PoC)

Each store indexes its own objects. The PoC indexes exactly one local
store; there is no global index and no fan-out in this slice.

Index contents:

- object titles/names (notes/documents surface: `src/documents.rs`)
- alias tag claims (the `src/aliases.rs` claim set)
- tags (the tag-claim layer, `src/tag_index.rs`)
- author identities for `@` (account keys with their alias claims)
- the searcher's own petnames (`src/petname.rs`) — indexed locally on
  the client only, never replicated, never searchable by anyone else

A store only ever answers queries about objects the querier is entitled
to see in it (#2030): resolution must never become an existence oracle
for private objects.

## 3. Candidate and ranking types

```text
Candidate {
  target_id: Hash,
  kind:      Actor | Topic,
  match_kind: Petname | ExactClaim | ExactTitle | Prefix | Token | FullText,
  provenance: Provenance { store: StoreId, author: ActorId,
                           claim_state: Uncontested | Contested },
  surface:   String,   // display label per aliases.rs rules
}
```

Ranking within one viewer's candidate list, strongest first; boosts
never cross tiers:

1. Petname match (`@` only) — the viewer's private name for this ID.
2. Exact alias/title claim match in the viewer's own stores.
3. Exact match in shared stores, then public stores (PoC: local only,
   tiers 3+ collapse to the local store but keep their ordering slots).
4. Prefix, then token, then full-text matches, same provenance order.
5. Soft boosts within a tier only: current view's writer set (#2030),
   recently interacted, authored by someone the viewer has petnamed.

Contested rule (from `src/aliases.rs`, adopted wholesale): two or more
claims for the same alias/title surface as **contested — nothing is
picked silently**. There is no commit-on-Enter for a contested list;
the author picks, and the stored ID makes the choice permanent.

Resolution is viewer-relative: the same phrase may resolve differently
for different viewers, and that is the correct semantics.

## 4. Rendering states (PoC must demonstrate all)

- **Resolved:** render `surface` as a live link; hover/preview shows
  the target's *current* alias + provenance.
- **Renamed target:** link still works (by ID); rendering keeps the
  stored surface (historical fidelity); hover shows the current alias.
  Silent surface-rewriting is rejected.
- **Cleared target:** ID resolves to a clear/tombstone; render surface
  + "cleared" state — a citation degrades to its label, not an error.
- **Unresolved for this viewer:** surface as inert text with an
  unresolved marker. The viewer learns only the words the author
  typed — not the content, not metadata, not existence.

## 5. PoC acceptance mapping (#2259)

- Ranked candidates with provenance over the local store, for both
  sigils, in the notes/documents surface.
- Stored-reference insert + render, incl. renamed-target and
  cleared-target states above.
- Duplicate-title case: all candidates shown grouped by provenance,
  no auto-pick.
- Contested-alias case: shown as contested, no silent pick.
- Findings note on #2259 afterwards: what the PoC proved, what
  cross-store fan-out + locator fetch (v2, out of scope here) still owe.

## 6. Out of scope

Cross-store fan-out and locator fetch; hosted-store search; any change
to `aliases.rs` display rules; any production resolution feature.
