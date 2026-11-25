# D1: the ID URI scheme, slice 1 (grammar and object form)

Issue: #13886 (parent epic #13880). Inputs: R1 Willow URI review
(`doc/2026-11-25T01-10-34Z_research_id_apps_r1_willow_uri_review/`), D3 graph layer
(`doc/2026-11-24_design_id-graph-layer-typed-links-verbs/D3-graph-layer.md`), the
`id://` design record for #9117 and summit ruling A-9117.1
(`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/`).

Status: proposal. Slice 1 of 5. Slices 2 (collections), 3 (failure matrix), 4
(carriers and hints) and 5 (display forms) follow; this record reserves room for them
and sketches only what the grammar needs from them. Slice 2 has landed as
`slice-2-collection-form.md` in this folder: it fills `set/` and closes R1 rows 3, 8
and 16.

Executable reference: `graph_uri_reference.py` in this folder
(`python3 -I graph_uri_reference.py self-test`, stdlib only). It holds the golden
strings, the rejection codes and a pinned 512-URI round-trip corpus.

## 1. The decision in one paragraph

We already have a URI scheme. `id://` is implemented on main (`id_uri.rs`,
`id_uri_parse.rs`, `id_uri_format.rs`) and its V1 grammar is a settled summit ruling.
D1 therefore does **not** invent a new scheme; it adds a `v2` version segment to
`id://` that names D3 graph objects. A bare object URI names one immutable object by
its `object_id`. Mutability is a separate, explicit "chain head" form. Location hints
live in the query and never change what is named. Nothing in a URI ever grants access.

## 2. Grammar (ABNF, canonical form only)

```abnf
id-graph-uri  = "id://" resolver-hint "/" namespace "/v2/" shape
                [ "?" hints ] [ "#" fragment ]
resolver-hint = alias / "any"                    ; "any" reserved, see §4
alias         = alnum-lc *62( alnum-lc / "." / "_" / "-" )   ; same as V1
namespace     = 1*( unreserved / pct-encoded )   ; UTF-8, not "." or ".."
shape         = obj-shape / head-shape           ; slice 2 adds set-shape
obj-shape     = "obj/" cid [ "/bytes/" byte-range ]
head-shape    = "head/" cid [ "/at/" cid ]
cid           = "raw-blake3-256." 64HEXLC        ; V1 content-id spelling
byte-range    = u64 "-" [ u64 ]                  ; from (incl.) - to (excl.)
u64           = "0" / ( %x31-39 *DIGIT )         ; no leading zeros, <= 2^64-1
hints         = via *( "&" via )                 ; ordered, max 8
via           = "via=" via-kind ":" 1*( unreserved / pct-encoded )
via-kind      = "broker" / "pkarr" / "bundle" / "https"
fragment      = 1*( unreserved / pct-encoded )
pct-encoded   = "%" UPHEX UPHEX                  ; only for bytes outside unreserved
alnum-lc      = %x61-7A / DIGIT
64HEXLC       = 64( DIGIT / %x61-66 )
```

Rules the ABNF does not carry:

- **One spelling per value.** The parser re-formats what it parsed and rejects the
  input if the two differ (`noncanonical_spelling`). This rejects lower-case percent
  escapes, escapes of unreserved bytes, leading zeros, upper-case hex digests and an
  empty range.
- **Identity is the path.** `identity(uri)` drops the query and the fragment. Two URIs
  with the same identity form name the same thing. LinkObjects, bundle indexes and
  dedupe keys store the identity form, never hints or fragments.
- **Critical vs non-critical.** The version segment and the shape tokens are critical:
  an unknown one is rejected (`unknown_critical_shape`, `unsupported_critical_version`),
  as in V1. In slice 1 the `via` kinds are a closed list too; slice 4 decides whether
  unknown hint kinds are skipped instead of rejected.
- **Length.** The 8 KiB cap from #9117 applies. The portable identity forms are fixed
  length: 101 characters for an object, 185 for a chain head with an expected head.
- **V1 is untouched.** `v1` URIs keep their meaning and their parser. A `v2` parser
  rejects `v1` and the reverse.

## 3. The object forms

| Form               | Example (portable, digest shortened)                    | Names                                                                                                                                          |
| ------------------ | ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Object             | `id://any/graph/v2/obj/raw-blake3-256.0001…1f`          | One immutable D3 object, ContentObject or LinkObject. The digest is the `object_id`.                                                           |
| Object slice       | `…/obj/raw-blake3-256.0001…1f/bytes/100-200`            | Bytes 100 up to 200 of that object's `body` (never its signed envelope). `100-` means to the end.                                              |
| Chain head         | `id://any/graph/v2/head/raw-blake3-256.0001…1f`         | "The current version of the object this chain started from", resolved through `supersede` links (D3 §5).                                       |
| Chain head, pinned | `…/head/raw-blake3-256.<root>/at/raw-blake3-256.<head>` | As above, with the head the writer saw. A resolver that finds a newer head says so (stale link) instead of silently showing different content. |

Choices in these forms:

- **The kind is not in the URI.** Content and link objects share one `obj/` shape.
  The object says what it is once fetched and verified, so a URI cannot lie about it,
  and D3 can add kinds without a URI change.
- **Slices are identity.** A quote of bytes 100 to 200 names something different
  from the whole object, so the range is in the path, not the query. Willow's
  truncation rule is kept at resolve time: a range past the end resolves to the bytes
  that exist, possibly none, and is never an error. An empty or reversed range is a
  spelling error and is rejected at parse time.
- **No digest expectation on `obj/`.** The `object_id` already is the digest. The
  expectation earns its place only on the mutable form, as `/at/`.
- **Forked chains.** If two valid supersedes of one object exist, the head is not
  unique. The resolver returns every readable head and marks the result ambiguous;
  choosing between them is D3's rule, not the URI's.
- **Hash agility.** The profile token (`raw-blake3-256`) already tags the algorithm,
  as the content-addressing record asks. If the estate moves to WILLIAM3 the new
  profile token (for example `william3-256`) becomes legal in the same grammar; D3
  object ids re-hash, the scheme does not change.

## 4. The authority: resolver hint, plus a portable `any`

V1 (A-9117.1) makes the authority a **local resolver alias** that must equal the
caller's selected resolver, otherwise `resolver_context_mismatch`. That is right for
V1 names, which only mean something inside one resolver. Graph object names are
self-certifying (the name is the hash), so they can travel. Slice 1 proposes one
reserved alias:

- `any` means "any resolver that can fetch and verify this". It is valid only with
  the `graph` namespace, and the `graph` namespace under `any` carries only
  self-certifying shapes (every v2 shape in slice 1). Slice 2's `set/` is allowed
  on the same footing as `head/`: a fixed query over self-certifying roots.
- `resolve_context("any", selected)` returns the selected resolver instead of failing.
- `any` is a legal V1 alias today, so reserving it needs a check that no deployed
  resolver is named `any`.

This touches a settled ruling, so it is a proposal for Drew, not a decision (§9).

## 5. Hints, fragment and capabilities

- **Hints** are `via=<kind>:<value>`, ordered, repeatable, untrusted and droppable.
  Kinds in slice 1: `broker:` (a broker name), `pkarr:` (z-base32 key), `bundle:`
  (a D4 bundle id), `https:` (a door). Because the name is a hash, a hostile hint can
  waste time but cannot substitute content. Slice 4 adds per-carrier rules, the
  resolver privacy policy R1 asks for (following a hint tells its host someone is
  resolving this name), and decides on `mailto:`.
- **Any other query parameter is rejected** (`undeclared_query_parameter`, as in
  #9117). That is how "no capability in a URI" is enforced: there is no place to put
  one. A carrier that needs access too, such as a QR invite, carries the URI and a
  separate signed ticket.
- **The fragment** is opaque, client-side, never sent to resolvers, and not part of
  identity. This diverges from V1, which rejects fragments; v2 allows them because a
  composed page (#14037) needs "this object, scrolled to that part".

## 6. QR and paper

The portable identity form uses only lower-case letters, digits and `/:.-`. Its
upper-case spelling (`ID://ANY/GRAPH/V2/OBJ/RAW-BLAKE3-256.0001…`) is entirely in
the QR alphanumeric alphabet, which is about 1.45 times denser than byte mode. The
object form fits QR version 5 at level M (101 characters); a pinned chain head fits
version 8. The equivalent Willow pinned entry needs version 11 in byte mode (R1).

The QR form is a transport encoding, defined only for portable identity forms with no
hints or fragment: the reader lower-cases the whole string and parses it normally.
Non-portable forms are excluded because their namespace may be case-sensitive.
Slice 4 covers paper and email survival in full.

## 7. Failure semantics, only as far as the grammar needs them (slice 3 owns the matrix)

- Parse failures are local and reveal nothing; they use the codes in the reference.
- Resolution failures reuse the V1 AddressError envelope. A holder who is not
  authorised gets one coalesced answer for "not authorised" and "not here", so a URI
  cannot be used to probe whether something exists.
- Under composition A, a lapsed grant is "not authorised", never a different code, so
  non-renewal reads the same as never having had access.
- A pruned or tombstoned object is visible as such only to a holder who could read it;
  the chain-head form surfaces a tombstone as the head ("removed"), per D3 §5.
- Naming never pins: no resolver keeps data alive because a URI to it exists.

## 8. R1 verdicts, consumed row by row

| R1 row                                   | R1 verdict                       | D1 slice 1                                                                                                                                                             |
| ---------------------------------------- | -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1 Generic RFC 3986 skeleton              | adopt                            | Adopted: `id://authority/path?query#fragment`.                                                                                                                         |
| 2 Authority = namespace.subspace keys    | reject                           | Rejected. Authority is the V1 resolver alias plus portable `any` (§4). No keys in the host, so no DNS label overflow.                                                  |
| 3 Path with dot-segments                 | reject / adapt in collections    | Rejected for objects: every path segment has a fixed alphabet except the namespace, which forbids `.` and `..`. Slice 2: no selector; a display alias is the fragment. |
| 4 Entry form                             | adapt                            | `obj/<object_id>`: one URI, one immutable object. Links named the same way (§3).                                                                                       |
| 5 Slice form                             | adapt                            | `/bytes/from-to` in the path, body bytes only, truncation at resolve time; the name `bytes` cannot collide with time bounds (R1 erratum 4).                            |
| 6 Expected digest                        | adapt                            | Only on the mutable form, as `/at/<cid>`; algorithm-tagged by the profile token.                                                                                       |
| 7 Prune fails                            | adopt principle, adapt semantics | Naming never pins (§7). The cause matrix is slice 3.                                                                                                                   |
| 8 Area form                              | adapt                            | `set/` shape, filled by slice 2: root plus verbs, direction, depth, container, authors, time window, count and bytes.                                                  |
| 9 Hint sequence                          | adapt                            | Typed `via=` hints in the query, ordered, max 8, outside identity (§5).                                                                                                |
| 10 Fragment                              | adopt                            | Adopted, client-side, outside identity (§5).                                                                                                                           |
| 11 Parameterised encodings               | reject                           | Rejected: one encoding per field, fixed by the `v2` segment.                                                                                                           |
| 12 Willow'25 hex codec                   | reject                           | Diverges in part, see §8.1: lower-case hex kept, mixed case and `+`/`-` prefixes rejected, QR density recovered by the upper-case transport form.                      |
| 13 Many codes per value                  | reject                           | Rejected harder than R1 suggests, see §8.1: non-canonical input is rejected, not accepted.                                                                             |
| 14 RFC 3986 classes and percent-encoding | adopt                            | Adopted, upper-case escapes, only in namespace, hint values and fragment.                                                                                              |
| 15 Dot-segment normalisation             | reject                           | Rejected; `.`/`..` cannot appear (row 3).                                                                                                                              |
| 16 Relative references                   | reject for identity              | Rejected: the parser accepts only absolute `id://` strings. Slice 2 keeps collections absolute too.                                                                    |
| 17 Scheme name `willow://`               | reject                           | Rejected; proposal is to keep `id` (§9).                                                                                                                               |
| 18 No capability in the URI              | adopt                            | Adopted and enforced by the closed query vocabulary (§5).                                                                                                              |
| 19 Mutable-link semantics                | adapt                            | Explicit `head/` form, never the default (§3).                                                                                                                         |

### 8.1 The two places this record departs from R1's recommendation

- **Hex, not base32 (row 12).** R1 notes unpadded base32 saves 12 characters per
  digest. We keep the V1 content-id spelling (`raw-blake3-256.` plus 64 lower-case hex)
  because the same digest then reads the same in a `v1/content/` URI, a `v2/obj/` URI,
  a log line and a `b3sum` output. Two spellings of one hash inside one scheme would
  cost more in confusion than 12 characters. R1's real objection, QR density, is met
  by the upper-case transport form (§6).
- **Reject, not accept, non-canonical input (row 13).** R1 suggests parsers accept
  non-canonical spellings but never emit them. V1 already rejects them, and doing the
  same here means a string either is the name or is an error: no tool can store a
  non-canonical spelling it was handed, and a URI damaged in transit fails cleanly
  instead of being silently "repaired" into something else.

## 9. Open questions for Drew

1. **Scheme name.** Proposal: keep `id`. It is already implemented, short, and typed
   by humans today. Alternatives would need a second parser for the same objects.
2. **Portable `any` authority.** Proposal: reserve `any` (with namespace `graph`) as a
   resolver hint that every resolver accepts for self-certifying names. This adds one
   exception to summit ruling A-9117.1's "hint must equal the selected resolver" rule.

Everything else in this record is the designer's call and can be changed by review.

## 10. What slices 2 to 5 inherit

- Slice 2 (landed, `slice-2-collection-form.md`) fills `set/`: a root plus optional
  verbs, direction, depth, container, authors, claimed-time window, count and bytes,
  where omitted means unbounded or any. It decides the verb filter (sorted verb tokens,
  `supersede` never walked) and the "collect" question below.
- Slice 3 writes the failure matrix against #13779's revocation decision points,
  starting from §7.
- Slice 4 adds per-carrier hint rules, hint-kind extensibility, resolver privacy and
  `mailto:`, with one worked URI per carrier.
- Slice 5 hands D5 the display rules: angle-bracket delimiting in plain text (RFC 3986
  Appendix C), elision of the digest middle, and the "edited / removed" badges the
  head form makes possible.
- The core-app PoC (#14037) has a "collect" action with no D3 verb yet. Slice 2
  recommends a `nest` projection, not a new verb: the collection is
  `set/obj/<collection>/verbs=nest/dir=in/depth=1`.
