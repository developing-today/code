# Design: friendly-name reference resolution v2 — cross-store fan-out (first slice)

- **Issue:** developing-today/code#5178 (follow-on to #2259; program epic #2056)
- **Date:** 2026-11-04
- **Status:** implemented in `pkgs/id/src/references.rs` (`FanOut`), queue task 9579
- **Predecessors:** `2026-10-30T13-25-00Z_design_friendly_name_references.md` (local-store
  PoC, #2259, PR #2756); the #2048 Zettelkasten design (merged PR #3885); the stable
  note-identity model of #3814/#3816 (merged PRs #3908, #4131)

## Carried decision (Q1 from #2259, answered for v2)

A stored reference targets a **stable logical note ID** — for notes, the create-event
ID of the #3814/#3816 model, under which revisions chain — and preserves the authored
surface verbatim. A reference may additionally carry an **optional exact-revision pin**
(`Reference.pinned_revision`): pinned means "this exact revision", unpinned (the usual
case) means "the logical target, current revision". The markdown degradation carries
the pin as `id://<logical-id>@<revision>`; unpinned references keep the exact #2259
shape, so existing stored text parses unchanged.

## Problem

The #2259 PoC resolves a typed `@`/`#` phrase against exactly one local index.
Resolution in v2 must query **several stores** (local notes/documents, other local
stores, later hosted stores) and merge their answers without weakening the PoC's two
load-bearing guarantees: boosts never cross match tiers, and a contested surface is
never silently picked.

## Fan-out contract

1. **Tier-first merge.** Every store answers the phrase under the existing
   `MatchKind` ranking (petname → exact alias claim → exact title → prefix → token →
   full-text). The merged list sorts tier-first, so a store-local ordering or boost
   can never lift a candidate past a better-tier candidate from another store.
2. **Deterministic total order.** Within a tier: folded surface, then the answering
   store's `StoreId`, then the target ID. The merged list is byte-identical
   regardless of the order stores were registered — merges are reproducible and
   testable, and two viewers with the same store set see the same list.
3. **Provenance per candidate.** Each candidate keeps the store that answered
   (`Provenance.store`, alongside the claim author). A target held by two stores
   appears once per store: duplicates are _shown_, never silently deduplicated,
   because telling duplicates apart is the author's decision, taken with provenance
   in front of them.
4. **Contested is cross-store.** For the exact tiers, the claimant set is the union
   over all stores of each store's claimants for the surface, keyed by target ID:
   - a surface claimed by distinct targets in different stores is **contested**,
     exactly as if both lived in one store, and `FanOut::insert_unambiguous`
     refuses it — the author must pick a candidate explicitly;
   - one target replicated in several stores is a **single claimant**: replication
     is not a contest, and a phrase naming it resolves unambiguously, with the
     stored reference gaining every holding store as a locator hint.
5. **One normalization.** Matching and claimant counting share a single normalized
   phrase (sigils stripped as a run, whitespace tolerated). Divergent normalization
   would let counting miss claims that matching found, silently clearing contests.
6. **Viewer-relative visibility.** Each store's index is scoped to the viewer before
   fan-out: their petnames, and the targets they may not see are hidden
   (`LocalIndex::hide`). A hidden target yields no candidate, is excluded from
   claimant unions (so its _existence_ cannot leak via a contested flag on a
   visible target's surface), and renders `Unresolved` with only the stored
   surface — the #2259 no-leak rule, extended across stores.

## What this slice deliberately does not do (later #5178 slices)

- **Locator fetch:** following a stored reference's locator hints to fetch/render a
  target that lives in another store, including renamed/cleared/unresolved render
  states across the boundary. This slice's contribution is that stored references
  now _record_ complete locator hints (every holding store).
- **Hosted-store search:** the hosted counterpart of the local index. A hosted
  store joins the fan-out by producing a viewer-scoped `LocalIndex`-equivalent
  answer set under the same contract; no contract change is expected.
- **Wiring to live stores:** fan-out operates over the in-memory per-store indexes,
  as the PoC did; production store adapters are follow-up work.

## Alternatives considered

- **Score-based merge (weighted sum across stores):** rejected — a strong
  lower-tier match in one store could outrank an exact match in another, breaking
  the tier guarantee the spec is built on.
- **First-store-wins / local-store priority:** rejected — order-dependent,
  non-deterministic across configurations, and silently picks among contested
  surfaces.
- **Deduplicate replicated targets in the candidate list:** rejected for display
  (provenance is information), accepted for _choice_: ambiguity is counted in
  distinct targets, not in candidate rows.

## Locator fetch (second slice, #5927)

Implemented in `pkgs/id/src/references.rs` (`FanOut::fetch_located` /
`FanOut::render_located`), queue task 9828.

- **Hints are the locator contract.** When a stored reference records locator
  hints, only those stores are consulted, in hint order; the first hinted store
  holding the target for the viewer decides — resolved, renamed, or cleared. A
  tombstone in the first holding store is an answer, not an absence to skip
  past. A reference with no hints (the markdown degradation carries none) is
  fetched against the viewer's store set in fan-out order.
- **States stay distinct across the boundary.** Resolved / renamed / cleared
  mean exactly what the single-store `render` means; the fourth state,
  unresolved-for-viewer, covers both "no hinted store holds it" and "a hinted
  store holds it but this viewer may not see it" — deliberately
  indistinguishable, rendered from the stored surface alone, naming no store,
  so an inaccessible target leaks no candidate, claimant, or existence
  information (fan-out contract rule 6, extended to fetch).
- **Pins are unaffected.** The fetch follows the stable logical ID; an
  exact-revision pin stays on the reference and does not change which object
  is fetched (the per-store indexes hold the logical object, not its revision
  chain).

## Hosted-store search + live-store wiring (third slice, #5178 remainder)

Implemented in `pkgs/id/src/references.rs` (`StoreView` / `StoreId::hosted` /
`FanOut::from_views`), queue task 13938.

- **Hosted search is an answer set, not a protocol.** A hosted store joins the
  fan-out by producing a viewer-scoped `StoreView` — its identity, the hosted
  rows the viewer may see as `IndexedObject`s, the IDs hidden from this viewer —
  which folds into a `LocalIndex`-equivalent answer set under the same
  `MatchKind` tiers. The fan-out contract is unchanged by the boundary:
  tier-first merge, per-candidate provenance, cross-store contested state, and
  the no-leak rule for hidden targets all hold across local/hosted exactly as
  among local stores. A hosted view carries no petnames (the viewer's petnames
  live in their local index only).
- **Live wiring is `FanOut::from_views`.** Production adapters snapshot one
  `StoreView` per store per viewer — local notes/documents rows, hosted rows —
  instead of hand-built in-memory indexes. The merge, the claimant union, and
  the locator fetch all operate on the folded indexes, so where the rows come
  from never changes what the fan-out decides.
