# Decision: ID tag claims as atomic-triple projections

- Date: 2026-11-06
- Status: **design decision** for [#5912](https://github.com/developing-today/code/issues/5912)
- Baseline: `origin/main` at `a2b4345e80927cb1d8ac652756cb219b971bf635`
- Related: [#5688 §2.5](https://github.com/developing-today/code/issues/5688), [#2048](https://github.com/developing-today/code/issues/2048), [#5907](https://github.com/developing-today/code/issues/5907)
- Scope: design only; no event-format or implementation changes

## Decision

Keep signed `tag.set` and `tag.clear` events as the source of truth, and
represent their assertions in the canonical triple model through
claim/assertion nodes. Do **not** collapse each event directly into one
unprovenanced `(subject, tagged-with, tag)` row.

For a claim event `c`, the logical canonical representation is:

```text
c --subject--> subject-ID
c --predicate--> tagged-with-ID
c --object--> tag-term-ID
c --asserted-by--> signer-ID
```

An optional existing tag value remains attached to the assertion as a
typed value/payload; it is not silently folded into the tag-term identity.
A `tag.clear` remains a signed retraction naming the claim IDs it removes,
and is represented as a relation to those assertion nodes. The deterministic
live-claim projection then exposes the ordinary edge
`(subject-ID, tagged-with-ID, tag-term-ID)` once when at least one authorized,
live claim supports it. The edge retains its supporting claim IDs and
authors as provenance. Clearing one of several supporting claims does not
remove the visible edge while another live claim supports it.

This is a triple-shaped representation, but not a one-event/one-bare-triple
mapping. Claim identity, signer, causal visibility, clear authority, and
multiple independent supports are part of the meaning and must survive the
projection. The exact names of the assertion predicates, tag-term ID creation,
signer-key-to-ID mapping, and encoding of typed values belong with the still-open
triple decomposition design in #5907 before implementation.

## Distinguish tags from topic references

“Triple-shaped” describes the storage form; it does not make every relation a
zettelkasten topic link. Generic tags are label/category assertions. A typed,
directed topic reference is the separate structured reference described by
#2048: target note ID, reference type/sigil, authored surface, and optional
pinned revision (`doc/2026-11-01_design_id2048.md:42-46`). Do not infer a topic
link from a tag, or a tag from a topic link. Backlinks continue to derive only
from the typed outgoing references in active note revisions.

This reconciles the parent decisions without silently changing either one:

- #5688 §2.5 remains the canonical direction: tags are ordinary triples over
  IDs, vocabularies are data, and a feature that cannot be expressed that way
  must explain why.
- #2048 remains unchanged: generic tags are for labels and aliases, not the
  canonical typed topic-reference edge. They can be triples in the graph
  without becoming note-to-note links.
- #5912 supplies the compatibility seam: preserve claim/event semantics and
  derive the generic triple view from them.

No parent-decision change is proposed.

## Alternatives considered

1. **Flatten each live claim to one bare triple.** Rejected: it drops claim
   IDs, authorship, independent support, and the causal/authorized meaning of
   `tag.clear`. Duplicate claims also become indistinguishable, so clearing
   one writer's claim could erase another writer's still-live assertion.
2. **Replace `tag.set`/`tag.clear` with direct triple writes and deletes.**
   Rejected: it changes signed event semantics and wire compatibility, and a
   delete cannot express the current observed-remove rule without retaining
   retraction provenance.
3. **Keep tags outside the triple model permanently.** Rejected as the
   canonical end state: it contradicts #5688 §2.5. A separate compatibility
   reader is acceptable during migration, not as a second canonical model.
4. **Use display strings as globally unique tag IDs.** Rejected: names can be
   renamed or contested, and equal strings in different vocabularies need not
   denote the same term.

## Identity, vocabulary, and namespaces

- The canonical tag object is a stable term ID. `tagged-with` is itself a
  stable predicate ID. A display name is a label/alias for a term, not its
  identity.
- Vocabulary membership and labels are data expressed through ordinary
  triples, not special parser branches or schema columns. A term may be in a
  named vocabulary; identical display strings in distinct vocabularies remain
  distinct terms unless they explicitly share an ID.
- An unqualified legacy string is not globally resolvable by spelling alone.
  Migration must select an explicit legacy vocabulary and resolve a name
  unambiguously; otherwise it remains an unresolved legacy claim until a
  person or a later migration rule resolves it.
- A vocabulary's domain identity is distinct from an iroh-docs storage
  namespace and from an event's group/access scope. Those storage and
  authorization boundaries must not be inferred from a tag vocabulary.

Alternatives for term identity are: strings as identity (simple but unstable
and collision-prone); a deterministic ID derived from vocabulary plus label
(stable for that spelling but makes rename/duplicate-label policy part of the
ID algorithm); or an explicit stable term ID with labels and vocabulary
membership stored as data (**recommended**). The last keeps identity separate
from presentation and supports contested labels. #5907 should settle the
concrete ID-minting and legacy-name resolution rules before code depends on
them.

## Authoring, browsing, and projections

- Preserve existing `tag.set { subject, name, value? }` authoring and signed
  bytes during compatibility. New authoring that targets a vocabulary term
  must resolve/select its stable term ID explicitly; it must not silently
  treat a free-text name as globally unique.
- Keep alias/value claims as claims. An alias string is presentation data,
  not a note ID or tag-term ID (`pkgs/id/src/aliases.rs:1-17,27-42`). A
  value-bearing claim needs a typed value representation in the assertion
  model; this decision does not reinterpret its bytes as an ID.
- A tag browser/query can use the projected `(subject, predicate, term)` edge
  and filter by vocabulary/term ID. It should retain claim provenance and
  show ambiguous labels as ambiguous rather than selecting one silently.
- The triple view is derived and rebuildable per store. It must run after the
  existing applied-event, clear-causality, and clear-authority rules, and it
  must obey the same viewer/store access boundary as other ID projections.
  Do not introduce a global tag index or expose inaccessible target data.
- `tag.clear` removes only the claims it names that are in its causal past;
  concurrent unseen sets survive. Existing authorization remains in force:
  the claim author or a settled Admin of the subject's owning group may clear
  it (`pkgs/id/src/tag_claims.rs:9-13`; `pkgs/id/src/event_store/claims.rs:33-36,100-138`).
- Projection invariants: replicas given the same applied event set derive the
  same active assertions and visible edges; reordering delivery does not
  change the result; every visible edge has at least one live supporting
  claim; clearing one support preserves other supports; a topic reference
  never appears as a tag solely because both are triples.

The implementation's exact query/index strategy remains a derived-view
choice. Today `EventStore::tag_claims()` folds applied claims and
`TagIndex` rebuilds local indexes from that live set
(`pkgs/id/src/event_store/claims.rs:42-87`; `pkgs/id/src/tag_index.rs:1-16`).
The future triple projection should preserve that rebuildability rather than
turning the index into another replicated source of truth.

## Compatibility and migration

### Signed ID event model

The current event log is append-only. `tag.set` carries a subject hash, text
name, and optional text value; `tag.clear` names `tag.set` event IDs. Clears
remove only claims they have seen, while concurrent unseen sets survive
(`pkgs/id/src/tag_claims.rs:1-26,51-67`). The default view also filters clear
markers by authorship/group-admin authority
(`pkgs/id/src/event_store/claims.rs:33-36,100-138`).

This decision requires **no event rewrite, signature change, or wire-format
bump**. A future projection can be rebuilt from retained events, preserving
the original claim ID, signer, causal history, and clear references. It must
not invent a term mapping for ambiguous names or discard unresolved claims.
Any conversion that changes the event model, or any migration that cannot
preserve the above invariants, needs a separately versioned design and
acceptance tests.

### Separate iroh-docs metadata tags

`pkgs/id/src/tags.rs` is a different key/value system: it stores arbitrary
subject/key/value bytes in iroh-docs α/Ω indexes, with Global, Node, and
Custom storage namespaces (`tags.rs:1-38`). It has its own set/delete and
copy operations. `LocalNode` imports legacy `.meta` tags into that store, and
the importer is one-shot, leaves `.meta` untouched, and preserves link data
as payload bytes (`pkgs/id/src/local.rs:106-116`; `pkgs/id/src/tags.rs:1188-1232`).
It does not import those values into the signed ID event store.

Therefore no iroh-docs-to-triples migration is required by this decision, and
the existing `.meta` import remains unchanged. If interoperability is later
requested, it needs an explicit adapter defining storage-namespace scope,
subject identity, byte/text conversion, provenance, and access policy. Do not
equate an iroh-docs namespace with a tag vocabulary or silently merge the two
stores.

## Follow-up boundary

No implementation-ready delta is filed here: the repository has no canonical
triple table or settled tag-term ID/value encoding yet. #5907 is already open
for triple decomposition rules and is the right design home for those shared
contracts. Once it settles them, an implementation packet can narrowly cover
the rebuildable claim-to-triple projection and compatibility tests. This task
does not reopen #3814 or #3816 and does not absorb #3819's separate access
policy decision.

## Sources checked

- #5688 body §2.5 and decomposition sketch item 8; fresh GitHub read on
  2026-11-06.
- `doc/2026-11-01_design_id2048.md:17,42-46,58-66`.
- `pkgs/id/src/tag_claims.rs:1-26,51-67,81-94`.
- `pkgs/id/src/event_store/claims.rs:33-87,100-138`.
- `pkgs/id/src/aliases.rs:1-17,27-42`.
- `pkgs/id/src/tag_index.rs:1-16`.
- `pkgs/id/src/tags.rs:1-38,1188-1232`; `pkgs/id/src/local.rs:106-116`.
- Fresh GitHub state: #5912 open with no matching PR; #5688, #2048, #5906,
  and #5907 open; #3814 and #3816 closed; #3819 open with its separate
  blocked/escalated access-policy state; PR #3885 merged.
