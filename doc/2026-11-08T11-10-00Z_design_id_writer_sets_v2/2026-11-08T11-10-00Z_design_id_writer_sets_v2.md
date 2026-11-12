# Design: writer sets v2 — view-as-object, set algebra, federation, discovery v2

- **Date:** 2026-11-08
- **Status:** design spec only — no implementation in this document's PR.
  Per the program rule (epic
  [#2056](https://github.com/developing-today/code/issues/2056)), a spec is
  not a build order: implementation slices are named in §8 and must be
  filed as their own issues.
- **Issue:** [#5789](https://github.com/developing-today/code/issues/5789).
  **Extends:** [#5545](https://github.com/developing-today/code/issues/5545)
  (v1 view projection + discovery) and the
  [#2238](https://github.com/developing-today/code/issues/2238) spec
  (`doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/`).
- **Relations:** [#5672](https://github.com/developing-today/code/issues/5672)
  (selective sharing), [#5675](https://github.com/developing-today/code/issues/5675)
  (remote query), [#2033](https://github.com/developing-today/code/issues/2033)
  (abuse primitives — owns every number this spec deliberately leaves out),
  [#5732](https://github.com/developing-today/code/issues/5732) (typed
  similarity/recommendation — views as a signal source).
- **Method:** v1 state read at `origin/main` `e1b77bba0d` on 2026-11-08.
  The v1 projection this spec extends has landed:
  `pkgs/id/src/idset/view.rs` (`expand_set`, frontier-pinned
  `ExpandedWriters`) and `pkgs/id/src/idset/expand.rs` (`Expander::expand`,
  `Expander::expand_retro`, one-level, non-recursive group expansion).
  Design choices are labelled **Decision** with their reasons; they are
  proposals for the review round, not settled API.

`pkgs/id/doc` is a symlink to the repo-root `doc/`, so this file is
reachable as `pkgs/id/doc/2026-11-08T11-10-00Z_design_id_writer_sets_v2/`
exactly as the issue's convention expects.

---

## 0. The rule v2 may not trade away

V1's founding rule stands unchanged: **ingest is a validity gate, never an
authority gate; a view counts only its writer set.** Composition adds
exactly one new idea — a view may count _through_ another view — and every
decision below is checked against the v1 corollaries: stored ≠ counted;
counting is per view, decided by the view owner, and reversible; every
computed view names its inputs; set changes are signed, versioned,
reviewable events; no silent history rewriting.

## 1. View-as-object

### D1 — The referenced object is a signed view descriptor plus a frontier snapshot, not a bare set and not a frozen result

**Decision:** a new signed event kind, `idset.view`, published by the view
owner, carrying `{ view_id, owner, set_ref, config_hash, frontier }` in the
event body (the v1 D1 reasoning applies verbatim: the entry payload cannot
live in `ChainExt`). `set_ref` is the v1 triple `(set_id, seq, head_hash)`;
`frontier` is the causal frontier the published projection was computed
at; `config_hash` commits to the view config that produced it.

- Not the **set alone**: a set without its config and frontier is not a
  view. Two views over one set count differently, and a reference that
  cannot say which one it means is ambiguous by construction.
- Not a **frozen tuple/result**: copying a computed result into the
  referencing view forks the truth — the copy can never be corrected when
  the source view is. The reference names inputs; the referencing view
  recomputes (or reuses a cached expansion keyed by those inputs).

### D2 — Two reference modes: pinned (default) and tracking (explicit)

A view reference in a writer set is a new entry shape:

```rust
View { view_id: Hash, mode: RefMode, include_history: bool }
enum RefMode { Pinned, Tracking }
```

- **Pinned.** The referencing view counts exactly the referenced view's
  published snapshot: its `set_ref` version and its `frontier`. When the
  source view changes — new set version, new frontier — **nothing changes**
  in the referencing view until its owner republishes against the new
  snapshot (a repin is itself a signed set-version event, v1 D2). A
  published, citable view is therefore stable: anyone holding the
  reference can reproduce the count.
- **Tracking.** The referencing view counts through the source view's
  _current_ version at evaluation time. What propagates is only what
  passes the source's own set at its new version: if the source removes an
  author, that author stops counting through this reference at the next
  evaluation (counting is reversible, §0). Tracking never widens the
  source's set — it follows it.
- In both modes the referencing owner's own `excludes` apply **on top of**
  whatever the source counts (§2). A reference is an admission path, never
  an override of the referencing owner's exclusions.

## 2. Set algebra

### D3 — Derived sets are signed objects whose operands are pinned content references

**Decision:** a new signed event kind, `idset.derived`, published by the
deriving owner, carrying
`{ derived_id, owner, op, operands: Vec<SetRef | ViewRef>, excludes }`
where `op ∈ {Union, Intersection, Difference}` and every operand is a
pinned reference (`(set_id, seq, head_hash)` for a set; the §1 descriptor
reference for a view). Evaluation is set algebra over the operands'
**expanded author sets at their pinned versions**, using the v1 expander
unchanged: group expansion stays one level deep _inside_ each operand —
algebra composes expansions, it does not deepen them.

- **Provenance.** Every author in a derived set's expansion carries a
  derivation path: the chain of `(object, op)` steps that admitted them.
  The path is computed deterministically at evaluation, not stored per
  author; anyone can replay the expansion from the signed originals and
  check the path, so a derivation chain is auditable end-to-end to signed
  source objects.
- **Exclusions still win — at every level.** The derived object's own
  `excludes`, and the excludes of any view that references the derived
  set, are applied after the algebra, on top of its result. An outer
  exclusion cannot be undone by an inner union, and a source's exclusion
  is part of what the source _is_: difference and intersection never
  resurrect an author a source operand excluded at its pinned version.
- **Difference is over expansions, not over spellings.** `A − B` removes
  authors in B's expansion however B admitted them (key, group, or view
  reference). Removing "everyone B counts" must not require knowing how
  B spelled its entries.

### D4 — Cycles are refused by construction, and evaluation defends in depth

An operand reference is a content hash of an object that must already
exist before the derived object can name it. A cycle would require an
object whose hash is computed over a body that already contains its own
hash — not constructible, not merely rejected. On top of that, two
evaluation rules hold for robustness against hand-crafted corpora:

- A derived set whose operand reference does not resolve to a known
  object is **stored but uncountable through that operand**; the missing
  operand appears on the uncounted side with reason `unresolved operand`
  (§4) until it arrives. It is never silently treated as empty (that
  would corrupt intersections and differences) — an unresolved operand
  makes the whole derived expansion report itself partial.
- Evaluation carries a visited set and the #2033-owned depth/fan-out
  budget (§5); exhausting either is a partial result with the reason
  named, never a panic and never a silent truncation.

## 3. Federation

### D5 — A set may name an author it has never seen; naming is a claim about membership, nothing more

The v1 `Key` entry already names a 32-byte verifying key, and keys are
global — no new entry shape is needed to _name_ a remote author. What v1
lacks is resolution and a claim discipline:

- **Resolution** goes through
  [#5675](https://github.com/developing-today/code/issues/5675)'s remote
  query: the local server asks the author's home server (located via the
  discovery layer, or an explicit home hint recorded alongside the entry)
  for that author's chain. What comes back is shared under
  [#5672](https://github.com/developing-today/code/issues/5672)'s
  selective-sharing rules: **a set entry is a request, never a grant.**
  The home server shares only what its sharing policy and the author's
  own grants allow, and a refusal is a normal outcome, not an error.
- **What a set may claim about an unseen author:** exactly one thing —
  "this key is a member of my set", the set owner's signed claim,
  verifiable locally from the set object alone. It may not claim the
  author's history, display identity, consent, or liveness. The author's
  _events_ count only when actually fetched and verified through the
  ordinary validity gate; until then the author appears on the uncounted
  side with reason `named, not yet resolved` (§4) — counted neither
  silently in nor silently out.
- Fork proofs, chain validity, and the store's `applied` predicate work
  identically for fetched remote events: federation changes _where events
  come from_, never _what makes them stand_.

## 4. Discovery v2

### D6 — Views and sets are discoverable objects, and the honesty rule inherits through composition

The v1 discovery query returns counted and uncounted claims grouped by
author, with why-not-counted. V2 extends it in two directions:

- **Objects, not just subjects.** `idset.writer`, `idset.view`, and
  `idset.derived` objects are themselves discoverable: a client can find
  views and sets (by owner, by operand, by subject coverage) — "who
  curates well" becomes a query over published views, and the typed
  relations of [#5732](https://github.com/developing-today/code/issues/5732)
  can treat a view as a signal source without new plumbing.
- **Inherited honesty.** A derived view's uncounted side is the union of
  three labelled lists: what the derived view itself excluded, what each
  source excluded at its pinned version, and what could not be resolved
  (unresolved operands, unresolved remote authors, budget exhaustion).
  Exclusions are **never laundered through composition**: if a source
  view excluded an author, a view built on that source must be able to
  show that exclusion, attributed to the source that made it. A derived
  count that cannot explain its sources' exclusions does not satisfy the
  honesty rule and must not be presented as a v1-grade count.

## 5. Abuse shape: composition as amplification

Composition does not create authority — counting stays per-view — but it
amplifies three costs, and the design names the valve for each. All
numbers are owned by
[#2033](https://github.com/developing-today/code/issues/2033); this spec
fixes only the shape, consistent with v1's §5 posture.

- **Attention amplification.** One flood author admitted by a popular
  source view is re-counted by every tracking view built on it. Valve:
  the referencing owner's exclusions win over every source (§2, D3) — one
  exclusion at the derived level silences the flood everywhere it
  propagates, and pinned references (the default) do not propagate at
  all until repinned.
- **Compute amplification.** Naive evaluation re-expands every source per
  derived view. Valve: expansion results are cached per
  `(set version, frontier)` — the v1 `Expander` already carries a cache —
  and evaluation cost is charged to the evaluating view's owner under
  #2033's budgets, with the §2 depth/fan-out budget as the hard stop.
- **Fetch amplification (federation).** A popular remote author in many
  sets must not make every local server independently hammer the home
  server. Valve: remote resolution is cached per
  `(author, chain head)` and revalidation is head-check-first through
  #5675; #5672 sharing responses are themselves cacheable signed objects.

Spam is still made ineffective, not free, exactly as in v1: uncounted
remote data costs storage and retention, never a new authority gate.

## 6. The issue's open questions, answered

**Q1 — Is a derived view's projection still a pure function of a tuple,
and what extends the tuple?**
Yes. Purity is preserved by extending the v1 tuple
_(view config, writer-set version, frontier, resolution)_ with a
**per-source frontier vector**: for each view/set operand, the
`(operand id, operand set version, operand frontier)` actually used.
Tracking mode does not break purity — evaluation materialises the
per-source frontiers into the tuple at evaluation time, so any computed
derived view still names all of its inputs and replays deterministically.

**Q2 — Retro-import (`include_history`) through a derivation chain: whose
flag governs?**
The most restrictive flag on the path governs (logical AND). An author's
history is imported through a derivation only if _every_ link that
admitted them — the source entry, each intermediate derivation/reference
— carries `include_history`, _and_ the referencing view's own reference
carries it. No intermediary can grant history the source set did not
flag (the source owner set the cost/consent terms), and no source can
force history on a deriving owner (their storage bill, their choice).
Direct v1 entries are unchanged: there, the single entry's flag is the
whole path. This is why `include_history` is spelled on the §1 `View`
entry shape as well as on v1 entries.

**Q3 — Do derived sets need their own signing owner, or is derivation
unsigned-but-reproducible?**
Both, in layers. **Publication requires a signer:** a derived set that
others can reference is a signed `idset.derived` object, and its owner
is claiming "this op over these pinned operands is my derivation".
**Trust does not require trusting the signer:** the derivation is
reproducible — anyone can recompute the expansion from the signed
operands and verify the claim, so the signature buys referenceability
and accountability, not authority over the result. Unsigned derivations
remain legal as _local_ computation (a private view config already is
one); they simply cannot be published, referenced, or discovered until
someone signs them.

## 7. Explicitly deferred, with reasons

- **Exact CBOR layouts** for `idset.view` / `idset.derived` bodies —
  implementation-slice work; this spec fixes shapes and semantics, and
  the layout must be reviewed against the v1 body-encoding rules when a
  slice is filed.
- **All numeric caps** (derivation depth, fan-out, fetch rates, budgets)
  — owned by [#2033](https://github.com/developing-today/code/issues/2033);
  duplicating numbers here would fork the abuse posture.
- **Remote-query wire details** — owned by
  [#5675](https://github.com/developing-today/code/issues/5675); this
  spec fixes only what a set may assume about the answer.
- **Selective-sharing policy grammar** — owned by
  [#5672](https://github.com/developing-today/code/issues/5672); this
  spec fixes only that an entry is a request, never a grant.

## 8. Implementation slices (to be filed separately)

1. `idset.view` object: encode/decode + `WriterSet::check`-style
   validation of the descriptor against its `set_ref`.
2. `View` entry shape in expansion: pinned resolution against the v1
   expander, with `expand_retro` honouring the §6/Q2 AND rule.
3. `idset.derived` evaluation: union/intersection/difference over pinned
   expansions, provenance paths, unresolved-operand partial results.
4. Discovery v2: object discovery + inherited uncounted-side reporting.
5. Federation resolution hookup: #5675 fetch + head-check caching for
   named-but-unresolved authors (depends on #5675 and #5672 landing).

## References

- [Issue #5789](https://github.com/developing-today/code/issues/5789) — this design's tracking issue
- [Issue #5545](https://github.com/developing-today/code/issues/5545) — v1 view projection + discovery (landed in `pkgs/id/src/idset/`)
- [Issue #2238](https://github.com/developing-today/code/issues/2238) — v1 writer-set spec, extended by this document
- [Issue #5672](https://github.com/developing-today/code/issues/5672) — selective sharing
- [Issue #5675](https://github.com/developing-today/code/issues/5675) — remote query
- [Issue #2033](https://github.com/developing-today/code/issues/2033) — abuse primitives and budgets
- [Issue #5732](https://github.com/developing-today/code/issues/5732) — typed similarity/recommendation
- [Issue #2056](https://github.com/developing-today/code/issues/2056) — parent epic
