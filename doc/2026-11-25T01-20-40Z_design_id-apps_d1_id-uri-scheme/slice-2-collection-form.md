# D1 slice 2: the collection form (`set/`)

Issue: #13886 (parent epic #13880). Builds on slice 1
(`2026-11-25T01-20-40Z_design_id-apps_d1_id-uri-scheme.md` in this folder), R1 rows 3,
8 and 16 with errata 3 and 5, the D3 verb registry, and the D5 view model
(`doc/2026-11-25T02-40-00Z_design_id-apps_d5_link-display/`).

Status: proposal. Slice 2 of 5. It fills the `set/` shape that slice 1 reserved.

Executable reference: `set_uri_reference.py` in this folder
(`python3 -I set_uri_reference.py self-test`, stdlib only). It reuses the slice 1
codec for everything outside the set path and prints:

```text
ok: 4 golden, 25 rejects, 512 round trips, corpus sha256 29679520…19b0; 8 D5 cases
match build_frame (longest URI 303 chars); prefix property over 10 members; bundle
scope 141 chars, 8 members, 17 objects, read back
```

## 1. The decision in one paragraph

A collection URI names a **query over the graph**, not a list of members. It is a
root object plus optional filters and bounds, written as `key=value` path segments in
one fixed order. Resolving it for one reader gives the members in the same
breadth-first order that D5 already draws, so a D5 view, a D4 bundle scope and a D2
collection embed are all the same URI. Anything omitted means "unbounded" or "any",
which fixes R1 errata 3 and 5 by construction. Time bounds filter on claimed time
and are marked untrusted. Nothing in a set URI grants access: members the reader
cannot read are absent, exactly as for a single object.

## 2. Grammar (ABNF, canonical form only)

```abnf
set-shape = "set/" set-root *( "/" set-param )   ; params in this order, each at most once
set-root  = ( "obj/" / "head/" ) cid             ; cid as in slice 1
set-param = "verbs="   verb *( "," verb )        ; ASCII-sorted, no repeats
          / "dir="     ( "in" / "out" )          ; omitted means both
          / "depth="   pos-u64                   ; omitted means unbounded
          / "within="  cid                       ; nest descendants of a container
          / "authors=" key *( "," key )          ; sorted, no repeats; omitted means any
          / "since="   u64                       ; unix seconds, claimed time
          / "until="   u64                       ; exclusive; since < until
          / "count="   pos-u64                   ; omitted means unbounded
          / "bytes="   pos-u64                   ; omitted means unbounded
verb      = %x61-7A *31( %x61-7A / DIGIT / "-" ) ; not "supersede"
key       = "ed25519." 64HEXLC
pos-u64   = %x31-39 *DIGIT                       ; 1 to 2^64-1, no leading zeros
```

Slice 1's `shape` line becomes `shape = obj-shape / head-shape / set-shape`.

Rules the ABNF does not carry, all enforced by the reference:

- **One spelling per query.** Parameters come in the order above, each at most
  once. Lists are sorted with no repeats. A default is never written: `dir=both` is
  `noncanonical_default`. Zero bounds are `invalid_bound`. The parser re-formats what
  it parsed and rejects any difference, as in slice 1.
- **Everything else is slice 1.** Resolver hint, namespace, `any`, `via=` hints,
  fragment, percent-encoding and the 8 KiB cap are unchanged. Identity drops hints and
  fragment. Set parameters use fixed alphabets, so they never need percent-encoding.
- **`any` is allowed.** A set URI is on the same footing as `head/`: the name is a
  fixed, self-contained query over self-certifying roots, and what it resolves to
  depends on what the resolver holds. So it is valid under `any/graph`.

## 3. What a set URI names

Resolution is defined for one reader. It reuses D5's graph model so the two cannot
drift apart:

1. **Nodes are chains.** Supersede chains fold into one node, as in D5 §3.3. Nothing
   the reader cannot read exists. An unreadable root and an absent one both answer
   `root_unavailable`.
2. **Roots.** `obj/<cid>` starts at the node holding that object. `head/<cid>` must
   name a chain's first object (`not_a_chain_root` otherwise) and says "this chain,
   whatever its current head".
3. **Filters prune before the walk.** `within`, `authors`, `since` and `until` decide
   which nodes may be members. The root always passes. Authors and times are read from
   the node's shown head.
4. **The walk.** Breadth-first from the root over links whose verb is in the set,
   both directions unless `dir` says otherwise. `out` follows links the node is the
   source of; `in` follows links pointing at it. Neighbours are taken in D5's order:
   verb rank, inbound before outbound, claimed time, then id.
5. **Bounds take a prefix.** `depth` stops the walk at that many hops. `count` and
   `bytes` keep the longest prefix of the canonical order that fits; `count` includes
   the root, and `bytes` sums the encoded size of each member's shown object. Every
   bound that removed something is reported as a cut.
6. **The result** is the members in order, the links between members, the cuts,
   `claims_untrusted` when a time bound was used, and any named verbs the resolver
   does not know.
7. **Unbounded is legal.** A resolver may stop at its own cap. When it does it must
   report the result as cut, never as complete.

A set URI is live: its members change as the graph grows. To freeze one, D4 writes
the resolved objects into a bundle and D2 pins the members in a snapshot embed. Both
keep the set URI alongside as the description of what was frozen.

## 4. Parameters

| Parameter  | Omitted means                  | Notes                                                                                                                                                                  |
| ---------- | ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `verbs=`   | every followable registry verb | Today that is `reply`, `quote`, `embed`, `mention`, `attach` and `nest`. A verb admitted later joins it; a URI that must not change with the registry lists its verbs. |
| `dir=`     | both directions                | `in` gives "what points here" (replies, children). `out` gives "what this points at".                                                                                  |
| `depth=`   | unbounded                      | Hops from the root. D5's `radius` maps to it.                                                                                                                          |
| `within=`  | the whole graph                | Keeps nodes nested, transitively, under one container. The root must be inside (`root_outside_within`).                                                                |
| `authors=` | any author                     | Ed25519 keys, so the filter cannot be spoofed by a display name.                                                                                                       |
| `since=`   | no lower bound                 | Claimed time, so a display filter, never a security bound. Sets `claims_untrusted`.                                                                                    |
| `until=`   | no upper bound                 | Exclusive.                                                                                                                                                             |
| `count=`   | unbounded                      | Members, root included. D5's `budget` maps to it.                                                                                                                      |
| `bytes=`   | unbounded                      | Encoded size of the members' shown objects. Links and older versions are not counted; a bundle writer still checks D4's own caps.                                      |

## 5. Acceptance

The reference loads D5's `view_reference.py` and D4's `bundle_reference.py` and
uses D4's BLAKE3 and Ed25519 to give every fixture object a real digest (over
sorted-key JSON, a stand-in for D3's canonical encoding) and every fixture author a
real key.

**D5's notes and programme graphs.** Every one of D5's eight cases is written as a
set URI, parsed back, and resolved by a walk written from §3, not copied from D5. For
all eight, the members, their hops, the links, the cuts and `claims_untrusted` equal
what D5's `build_frame` gives. Examples:

| D5 case                   | Set URI parameters after the root                              | Length |
| ------------------------- | -------------------------------------------------------------- | -----: |
| `notes-replies-first`     | `verbs=attach,mention,quote,reply/depth=2/count=200`           |    156 |
| `notes-budget-cut`        | `verbs=attach,mention,quote,reply/depth=2/count=4`             |    154 |
| `notes-time-window`       | `depth=2/since=1793750400/count=200`                           |    140 |
| `programme-within-epic`   | `verbs=embed,mention,reply/depth=2/within=<cid>/count=200`     |    236 |
| `programme-author-filter` | `verbs=embed,mention,reply/depth=1/authors=<2 keys>/count=200` |    303 |

The same checks also show that `count` and `bytes` give exact prefixes of the
unbounded order at every size, that `dir=in` and `dir=out` split the neighbours as
§3 says, and that an unreadable root fails the same way as a missing one.

**A D4 bundle index.** The bundle scope is the design epic's thread, everything
nested in it or replying into it:

```text
id://any/graph/v2/set/head/raw-blake3-256.<epic>/verbs=nest,reply/dir=in/count=4096
```

It is 141 characters. For an anonymous reader it resolves to the epic, the six
records nested in it and one triage reply, in that order. The bundle carries those 8
members, the 7 links between them, and the rest of each member's chain (the second
D1 version and its supersede link): 17 objects. The reference writes that
bundle with D4's writer, reads it back with D4's reader, and parses the scope from
the manifest back to the same query. `count=4096` matches D4's entry cap, so a
member count that fits is the first check a writer makes.

**A D2 collection.** A collection is its `nest` children, one level down:

```text
id://any/graph/v2/set/obj/raw-blake3-256.<album>/verbs=nest/dir=in/depth=1
```

## 6. Decisions

- **The verb filter is a list of verb tokens.** It is sorted by ASCII, not by
  registry order, so the canonical spelling does not move when the registry grows.
  `supersede` is rejected (`verb_not_followable`): supersede links fold chains and are
  never walked. A well-formed verb the resolver does not know is accepted and
  reported back, because a filter on an unknown verb can only narrow the result.
- **"Collect" is a `nest` projection, not a new verb.** The core-app PoC's "collect"
  action (#14037) writes a `nest` link from the item into the collection, and the
  collection is the set URI above. D3's admission rule asks for an existing verb
  first, and `nest` already means "is part of". This is a recommendation to D3's
  owner, not a registry change.
- **No path selector inside collections (R1 rows 3 and 16).** R1 allowed an
  optional display-alias selector. It is not needed: a member is named by its own
  absolute `obj/` URI, and "this collection, scrolled to that member" is the
  fragment (`…/verbs=nest/dir=in/depth=1#photo-3`), which slice 1 already keeps
  client-side and outside identity. So there are still no dot-segments and no
  relative references anywhere in v2.
- **Authors are keys, not names.** Display names are not unique and can be spoofed.

## 7. R1 rows closed by this slice

| R1 row                   | Slice 2                                                                                                    |
| ------------------------ | ---------------------------------------------------------------------------------------------------------- |
| 3 Path with dot-segments | Closed: no selector in the path; a display alias belongs in the fragment (§6).                             |
| 8 Area form              | Adapted: root plus verbs, direction, depth, container, authors, claimed-time window, count and bytes (§2). |
| 16 Relative references   | Closed: members are named by absolute URIs; the collection URI is absolute too (§6).                       |
| Erratum 3 (bare area)    | Fixed by construction: a set with no parameters is everything reachable, never empty.                      |
| Erratum 4 (from/to)      | Avoided: time bounds are `since=`/`until=`, bytes bounds are `bytes=`; no name means two things.           |
| Erratum 5 (any author)   | Fixed by construction: omit `authors=`.                                                                    |

## 8. Alternatives weighed

| Alternative                                         | Verdict  | Why                                                                                                                       |
| --------------------------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------- |
| Parameters in the query string                      | Rejected | The query is outside identity. Two collections with different filters are different names, so the filters are path.       |
| A member list in the URI (`set/<cid>,<cid>,…`)      | Rejected | Grows without bound and duplicates D4 and D2 pins, which already freeze members.                                          |
| A new `collect` verb                                | Rejected | `nest` already means it; a second verb would split every collection reader in two.                                        |
| Registry-ordered verb lists                         | Rejected | The canonical spelling would change whenever a verb is admitted.                                                          |
| Time bounds as security bounds                      | Rejected | `created_claim` is untrusted (D3). Time only filters what is shown, and the result says so.                               |
| D5-style defaults (radius 2, budget 200) in the URI | Rejected | R1 erratum 3: a reader must be able to tell "no bound" from "default bound". D5 writes its defaults out when it uses one. |

## 9. What changes elsewhere if this is adopted

- Slice 1's parser stops returning `reserved_shape_not_in_slice_1`. Its script is
  left as it is, so its pinned corpus stays valid; `set_uri_reference.parse` is the
  combined parser.
- D4's optional `scope` record can hold a set URI. It already only requires an ASCII
  `id://` string, so nothing in D4 changes.
- D5 can serialise any view as a set URI: `radius` becomes `depth=`, `budget` becomes
  `count=`, and `radius` 0 becomes `count=1`.
- D2's collection embeds name their target with a set URI; snapshot embeds still pin
  members as D2 slice 2 specifies.
- The Rust `id://` code is unchanged until an implementation slice picks this up.

## 10. Open questions

None needs Drew alone. Two are for review:

1. **Registry growth.** An omitted `verbs=` follows the registry as it grows. The
   alternative is "the six verbs of today, forever". The reference takes the first
   because a collection is usually meant to include new kinds of relation.
2. **`bytes=` weight.** It counts shown objects only. Counting chains and links too
   would make it a closer fit to a bundle's size, at the cost of a less obvious rule.
