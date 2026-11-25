# R1 — Willow URI spec deep review: per-element verdicts for D1

Status: research record for issue #13884, written 2026-11-25 by lane 6
(queue task 21546). Parent epic: #13880 (ID apps on one graph), programme
order 1. Consumer: D1, the ID URI scheme (#13886).

Source read: the full text of the `willow://` URI specification
("Willow URIs", status **Proposal**), from the private willowprotocol.org
mirror on the laptop (`specs/uris/index.html`, mirrored 2026-11-23; see
`doc/2026-11-24T00-53-05Z_research_p2panda-investigation/MIRROR-NOTES.txt`),
together with the same mirror's `specs/willow25` (the default parameters,
including the URI parameters), `specs/grouping-entries` (Area,
AreaOfInterest) and `specs/data-model` (Path limits, Timestamp). Public
canonical URL: https://willowprotocol.org/specs/uris/. Everything below is
paraphrased; short phrases in quotes are the spec's own words.

Axes, as defined on #13884:

- **A1 revocation** — composition A of #13779: short-window grants renewed
  at broker check-in; revocation is non-renewal plus supersession
  (`doc/2026-11-23T23-18-28Z_design_p2p-composition-options-compared/`).
- **A2 carriers** — #13752: one signed ticket format over broker,
  pkarr/DHT, email, QR, sneakernet bundle; plus the URI-in-the-wild cases
  added on #13884 (IRC-style chat line, D4 bundle index, QR, composed page
  in the core app #14037).
- **A3 graph model** — D3 (#13885,
  `doc/2026-11-24_design_id-graph-layer-typed-links-verbs/D3-graph-layer.md`):
  immutable, signed, content-addressed ContentObjects and LinkObjects;
  mutability only through the `supersede` verb; links are objects.
- **A4 human factors** — read, typed, pasted, wrapped, displayed in
  terminals (D5).

Length and alphabet figures come from `supplements/uri_lengths.py`
(stdlib, dummy 32-byte values, no real keys); its output is
`supplements/uri-lengths.json`.

## 1. What the spec actually says (the parts that matter)

- **Two URI kinds.** An Entry URI names one Entry, optionally plus one
  contiguous slice of its Payload. An Area URI names an AreaOfInterest.
  Both share `willow://<namespace>.<subspace>/<path…>?<query>#<fragment>`;
  the Area form is marked by an `area` query token.
- **Common fields.** NamespaceId (host, before the dot), SubspaceId (host,
  after the dot), a URIPath (Willow Path plus RFC 3986 dot-segments), an
  ordered, possibly empty sequence of hint URIs ("typically not Willow
  URIs"), and an optional opaque application fragment.
- **Entry-only.** Optional expected PayloadDigest (`digest=`); optional
  `from=` / `to=` U64 byte indices with truncation (end beyond length →
  end of payload; start beyond length → empty slice at the end; start >
  end → empty slice; one index alone defaults the other).
- **Area-only.** `count=` (max_count), `size=` (max_size), `from=` /
  `to=` (TimeRange, end open if absent). Omitted count/size/from encode
  the value **0**.
- **Prune-fails.** A digest-pinned Entry URI matches only an entry with
  that payload; the spec says using the URI does not "pin" the entry and
  retrieval "deliberately fails" if it was pruned.
- **Parameters.** Each instantiation supplies host-safe encodings for
  NamespaceId and SubspaceId and a query-safe encoding for PayloadDigest.
  Willow'25 uses base-16 (lowercase recommended, uppercase allowed), with a
  `+`/`-` prefix on the namespace marking communal/owned.
- **Encoding is a relation, not a function.** Query parameters may appear
  in "arbitrary order", decimals may carry "arbitrarily many leading
  zeros", hex case is free, and `count=0` / `size=0` / `from=0` may be
  written or omitted. One value has many valid strings.
- **Conformance.** Character classes match RFC 3986 (unreserved,
  sub-delims, pct-encoded, reg-name, query, fragment); dot-segment removal
  follows RFC 3986; URI references (relative links) are inherited and
  implementations "should" support them.

## 2. Errata found while reading (worth knowing before anyone copies it)

These are defects in the Proposal text, verified against the mirrored
page. None changes a verdict, but D1 must not inherit them by copying.

1. **Area marker bytes disagree with the prose.** The Area encoding steps
   emit the bytes `[38, 97, 114, 101, 97]`, which spell `&area`, and emit
   no `?`; the prose says the query "always starting with `?area`" and
   every example uses `?area`. A literal implementation produces
   `willow://ns.ss/blog&area`, which RFC 3986 parses as a path component.
2. **`size=` is misspelled in bytes.** The Area `size=` step lists
   `[115, 105, 112, 101, 61]`, which spells `sipe=`.
3. **A bare `?area` denotes an empty set.** Omitted `count`/`size` decode
   to 0, and an AreaOfInterest includes an entry only if it is among the
   max_count newest (unless max_count = 2⁶⁴−1). So by the letter of the
   spec, `willow://family.alfie/blog/ideas?area` (one of its own examples)
   names an area of interest containing nothing. The intended default is
   evidently "no limit" (2⁶⁴−1).
4. **`from=`/`to=` mean different things in the two kinds.** Byte indices
   into a payload for Entry URIs; TAI-microsecond Timestamps for Area
   URIs. The only switch is the presence of the `area` token.
5. **Area URIs cannot say "any subspace".** The data model's Area allows
   `subspace_id: SubspaceId | any` (the "full area" uses `any`), but the
   Area URI requires a concrete SubspaceId in the host. Cross-author areas
   are unnameable.
6. Small: the dot-segment note cites "RFC 3968" (should be 3986); the
   examples use 4-byte digests (`b287afb0`), which are not valid codes
   under Willow'25 (32-byte WILLIAM3 digests, 64 hex characters).

## 3. Per-element verdict table

Verdicts: **adopt** (take as is), **adapt** (take the idea, change the
form), **reject** (do not take). Each verdict names the axis that decided
it.

| #   | Element                                                                                                 | Verdict                                                         | Deciding axis | Reason                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| --- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- | ------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | **Grammar skeleton**: `scheme://authority/path?query#fragment`, RFC 3986 generic syntax                 | **adopt**                                                       | A2, A4        | Generic-syntax URIs survive every parser, linkifier fallback, and HTML attribute we will meet; a bespoke syntax would need its own parsers in every app (#14037, #2051).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| 2   | **Authority = `namespace.subspace`** as two key encodings in the host                                   | **reject**                                                      | A3, A4        | D3 objects are not located by (namespace, subspace, path) slots; they are named by `object_id`. Putting 65- and 64-character hex keys in the host gives labels over the 63-octet DNS label limit (script: longest label 65), so any tool that treats the host as a DNS name breaks, and the host alone is 130 characters before any path.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| 3   | **Path / URIPath with dot-segments**                                                                    | **reject** (for object naming) / **adapt** (inside collections) | A3            | Content-addressed immutable objects have no hierarchy, so `..` has nothing to climb; the dot-segment machinery only adds normalisation cases. The single place a path is meaningful is a display alias inside a named collection (a `nest` projection), which D1 may keep as an optional, non-identifying selector.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| 4   | **Entry form** (the URI names one record)                                                               | **adapt**                                                       | A3            | Keep "one URI names one object", but the name is the D3 `object_id` (content hash), not a mutable slot. That makes a bare object URI immutable by construction. The Willow behaviour "this path's current value" maps to a different, explicit form: "head of the supersede chain from X" (see row 6 and §4 Q2). Links (LinkObjects) must be nameable exactly like content objects, which falls out for free because both carry `object_id`.                                                                                                                                                                                                                                                                                                                                                                                                              |
| 5   | **Slice form** (`from=`/`to=` byte range with truncation)                                               | **adapt**                                                       | A3, A4        | Useful for D2 quote/attach (quote a byte range, show part of a large body). Keep the truncation rules (they make a slice URI never "invalid", only empty), but give the parameters names that cannot collide with time bounds (erratum 4), and apply slices to `body` bytes of a ContentObject or to an out-of-band blob referenced by digest, never to the signed envelope.                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| 6   | **Expected digest** (`digest=`)                                                                         | **adapt**                                                       | A3, A1        | For an object URI the `object_id` already is a digest, so a separate expectation is redundant there. It earns its keep only on mutable forms: "head of X's supersede chain, but only if the head is this digest" (stale-link detection), and on blob references. The digest must be **algorithm-tagged** (Willow's is untagged; the algorithm is fixed by the instantiation and invisible in the string). The content-addressing record (`doc/2026-11-24T03-31-54Z_research_content-addressing/`) already recommends tagged digests from day one so a BLAKE3→WILLIAM3 move is a re-hash, not a redesign.                                                                                                                                                                                                                                                  |
| 7   | **Prune-fails choice** (a pinned URI fails rather than pins)                                            | **adopt the principle, adapt the semantics**                    | A1            | "Naming does not keep data alive" composes with composition A: a URI must neither pin data nor extend a grant. But Willow has one failure cause (the entry is gone). Under non-renewal there are at least four (pruned or tombstoned via `supersede`; grant lapsed; never existed; not authorised), and the holder of a URI needs to tell some of them apart without the resolver leaking existence to the unauthorised. Willow's model gives no guidance here; it is D1 slice 3's matrix. Willow's "deletion wins over links" is the right default; what it must not import is any reading where a URI implies a permanent right to resolve.                                                                                                                                                                                                             |
| 8   | **Area form** (`?area`, count/size/time bounds)                                                         | **adapt**                                                       | A3            | The idea (a URI can name a bounded dataset slice, sized for sync budgets) maps directly onto D3: a collection URI naming a subgraph expression. But the Willow axes (subspace × path prefix × time) are the wrong axes for a graph; ours are root object + verb set + direction + depth, author, and time (`created_claim` is untrusted, so time bounds are display filters, not security bounds). Keep the resource bounds (max count, max bytes) because they are what make a collection URI safe to resolve over a slow carrier. Fix errata 3 and 5 by construction: an omitted bound means unbounded, and "any author" is expressible.                                                                                                                                                                                                                |
| 9   | **Hint sequence** (ordered, possibly empty, nested URIs, percent-encoded, `;`-joined)                   | **adapt**                                                       | A2            | Ordered, untrusted, try-in-order location hints are exactly what the #13752 carrier set needs, and because the name is a content hash, a hostile hint can waste time but cannot substitute content. Change the form: hints should be **typed and compact** (`broker:<name>`, `pkarr:<z-base32 key>`, `bundle:<bundle id>`, `mailto:` reply address, `https:` door) rather than arbitrary percent-encoded URIs, because percent-encoding every reserved byte of a nested URI inflates it (98 → 108 characters for a modest https hint) and two hints push a pinned URI to 387 characters (script), at the edge of one IRC line. Add a resolver privacy rule Willow lacks: following a hint tells the hint's host that someone is resolving this name, so resolvers need a policy (hint allowlist, or broker-only when offline-verifiable is not required). |
| 10  | **Fragment** (opaque application data)                                                                  | **adopt**                                                       | A4            | RFC 3986 fragment semantics (client-side, not sent to resolvers) are what a composed page (#14037) needs for "this object, scrolled to that part". Keep it opaque to the scheme.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| 11  | **Parameterised id/digest encodings** (instantiation picks the codec)                                   | **reject**                                                      | A2, A4        | A URI string that means different things under two instantiations is not self-describing; a URI carried by sneakernet or printed on paper cannot rely on the reader knowing the parameters out of band. D1 should fix one encoding per field in the scheme itself (versioned by the scheme name or a version tag).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| 12  | **Willow'25 codec: base-16, either case, `+`/`-` prefix**                                               | **reject**                                                      | A4, A2        | Hex costs 64 characters per 32-byte value; an unpadded base32 costs 52, which also fits a 63-octet DNS label if ever needed. Mixed case defeats QR alphanumeric mode, which only takes upper-case letters, digits and ` $%*+-./:` (script: no Willow URI qualifies; a pinned entry needs byte mode at QR version 11, level M). The communal/owned prefix is Willow-namespace semantics D3 has no use for.                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| 13  | **Encoding relation with many codes per value** (any order, leading zeros, either case, optional zeros) | **reject**                                                      | A3, A1        | D3 signs and content-addresses objects; a URI stored inside a LinkObject or a D4 bundle index must have **one canonical form** or two semantically equal links hash differently and dedupe fails. D1 should define a canonical encoding (fixed parameter order, no leading zeros, one case) and say whether parsers accept non-canonical input (recommended: accept, but never emit or sign it).                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| 14  | **RFC 3986 character classes and percent-encoding rules**                                               | **adopt**                                                       | A2            | Precise and standard. Percent-encode only where a field can contain arbitrary bytes (display aliases, fragments), not in fixed-alphabet fields.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| 15  | **Dot-segment normalisation**                                                                           | **reject**                                                      | A3            | Follows from row 3: no hierarchy, no dot-segments. A conformant RFC 3986 resolver will still remove them, so D1's grammar should simply forbid `.`/`..` components in the identifying part.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| 16  | **URI references / relative resolution**                                                                | **reject** for identity, **adapt** for collections              | A3, A4        | Relative references ("`../image.png` from this entry") assume a path tree. In a composed page (#14037) relative references would silently re-resolve when the page moves. Embeds (D2) should carry absolute object names; a collection-relative selector, if kept, is explicitly scoped to its collection URI.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| 17  | **Scheme name `willow://`**                                                                             | **reject** (as our name)                                        | A3            | D1's own non-goal: not the Willow scheme renamed. The name itself is Drew's decision (#13886 open question).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| 18  | **No capability in the URI** (naming never grants access)                                               | **adopt**                                                       | A1            | Willow URIs carry no read capability; access is Meadowcap's business. This is exactly #13886's "a URI must not become a capability leak". D1 should state it as a rule: no grant, token or secret ever appears in a URI; a carrier that needs both (a QR invite) carries a URI and a separate signed ticket (#13752), not one string.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| 19  | **Mutable-link semantics** (an Entry URI without digest follows the slot to its latest value)           | **adapt**                                                       | A3, A1        | Cobweb's "mutable links without link rot" is the reason the Entry form exists. D3 reaches the same outcome through `supersede`: name the original object and ask for its chain head. Making that an explicit form (not the default) keeps the bare object URI immutable and lets the resolver show "edited" or "removed" state honestly. Supersede authority is per D3 §5 (author or moderation grant), so a stranger cannot hijack a mutable link, which Willow handles by subspace ownership.                                                                                                                                                                                                                                                                                                                                                           |

## 4. URI-in-the-wild cases (added on #13884)

| Carriage                                                                                                  | What breaks with the Willow form                                                                                                                                                                                                                                                                                                           | Bearing on verdicts                                                                                                                                                                                                                           |
| --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **IRC-style chat line** (#2049 → #2051), plain text, ~400–450 usable bytes per line, client-side wrapping | Most linkifiers do not recognise an unknown scheme at all; those that guess stop at whitespace and strip trailing `.`, `)` and `'`, all legal here (`'()` are sub-delims, paths may end in `.`). A pinned entry with two hints is 387 characters (one line, barely); wrapped by the client, it becomes two fragments and neither resolves. | Rows 9, 12, 13: short canonical form, no trailing-punctuation-legal characters at the end, compact typed hints. D1 display rules should mark the end of a URI unambiguously (angle brackets, as RFC 3986 Appendix C suggests for plain text). |
| **Sneakernet bundle index** (D4, #13889)                                                                  | The bundle already carries the bytes; hints are dead weight, and an instantiation-dependent codec (row 11) cannot be decoded by a reader who only has the bundle.                                                                                                                                                                          | Rows 9, 11, 13: hints optional and droppable; one fixed codec; canonical form so the index hashes stably.                                                                                                                                     |
| **QR code**                                                                                               | Lower-case hex and `?=&;` force byte mode; a digest-pinned entry needs version 11 at level M (251 bytes), with two hints version 15 (412).                                                                                                                                                                                                 | Row 12: a case-insensitive alphabet (upper-case base32 plus `:`/`-`/`.`) would allow alphanumeric mode, roughly 1.45× denser. D1 should state whether the canonical form or a QR form is upper-case.                                          |
| **Composed page in the core app** (#14037)                                                                | Relative references re-resolve when the page moves; mutable Entry URIs make an embed change under the reader without a visible "edited" signal.                                                                                                                                                                                            | Rows 16, 19: absolute object names in embeds; mutable "chain head" only as an explicit form D2 chooses per embed (live vs snapshot).                                                                                                          |
| **Email header** (from the #13752 carrier set)                                                            | URIs contain no whitespace, so header folding cannot break them; a 225-character URI exceeds the 78-character recommended line and survives only in an unfolded or angle-bracketed form. Body text wraps at about 72–78 characters.                                                                                                        | Rows 9, 12: shorter canonical form; email carriage should use angle brackets in headers and should not depend on hints.                                                                                                                       |

## 5. Questions D1 must answer that the Willow spec does not

1. **Failure taxonomy.** Which dereference failures are distinguishable to
   the URI holder (pruned/tombstoned, grant lapsed under non-renewal,
   never existed, not authorised), and what may a resolver reveal to an
   unauthorised holder without confirming existence? Willow has a single
   cause and no disclosure rule. (Decides on the #13779 decision points
   only via #13779.)
2. **Immutable vs mutable naming.** Is the default URI the immutable
   `object_id`, with "supersede-chain head" as an explicit form, or the
   reverse? This review recommends immutable-by-default (rows 4, 19).
3. **Collection expression grammar.** Which subgraph selectors (root +
   verbs + direction + depth, author set, time window, resource bounds)
   and whether verb-filtered collections are allowed (#13886's open
   question, gated on D3's registry).
4. **Canonical form.** One canonical string per value: parameter order,
   case, digit rules, and whether non-canonical input is accepted.
5. **Digest and key codecs.** Algorithm-tagged digest format, the key
   encoding (base32 variant, case), and a scheme version mark so a codec
   change is not a silent reinterpretation.
6. **Hint types and resolver privacy.** The typed hint vocabulary per
   #13752 carrier, which hints a resolver may follow automatically, and
   whether hints are ever signed (a carrier-supplied hint list is
   unauthenticated).
7. **Authority component.** With no namespace/subspace host, what (if
   anything) goes in the authority: nothing (`scheme:` without `//`),
   an author key, or a world/topic id. Drew decides (#13886).
8. **Link-object naming in practice.** Whether a LinkObject URI renders
   differently (D5) and whether a collection can be the target of a link
   (a reply to a whole thread view).
9. **Length budget.** A hard upper bound for the canonical form so it fits
   one IRC line and one QR version at a stated error-correction level.

## 6. Hand-offs

- **D1 (#13886):** consumes §3 row by row and §5. Errata (§2) are for D1's
  reference only; this record does not report them upstream.
- **D2 (#13887):** rows 5, 16, 19 (slices for quote/attach; absolute
  names in embeds; explicit chain-head form for live embeds).
- **D4 (#13889):** rows 9, 11, 13 (droppable hints, fixed codec, canonical
  form in bundle indexes).
- **D5 (#13888):** §4 chat and QR rows (display delimiters, case).
- **#13779:** no verdict here changes a composition-A decision point; row 7
  states the URI-side consequence of non-renewal only.
