# Program ecosystem — v3 design: the public gallery for #6294

Status: v3 design (design only; no implementation code in this
record, per the #6294 charter).
Ladder step: v3 of the Phase 14 Program ecosystem core topic #6278
(core design of record:
`doc/2026-11-10T21-24-00Z_design_id_program_ecosystem/`; v1:
`doc/2026-11-11T05-01-15Z_design_id_program_ecosystem_v1/`; v2:
`doc/2026-11-11T05-24-00Z_design_id_program_ecosystem_v2/`).
Entry condition: met — v1 #6291 CLOSED (design landed via PR #8541,
merge 44fd6a414c) and v2 #6293 CLOSED status:done (design landed via
PR #8547, merge 3646bb6c9e), verified by triage task 15239 and
re-verified by this task (16804).
This record builds on v2's D6 (the gallery as a registry) and the
already-filed gallery registry implementation #8281; it does not
re-decide them. All operator policy below is layered on D6, never
protocol — the line v2 drew for this step.
Principles: #1240 — local-first always, hostable by anyone at any
scale, p2p behaviour identical to hosted behaviour, our hosted
instance is a public backstop and never a requirement.

## What v3 is, and is not

In scope: our hosted public gallery as an operated instance on the
main public endpoint — categories, search, featured collections,
author pages keyed by public key, plus the curation posture, abuse
handling, and scale elements v2 explicitly deferred here.

Out of scope: federation and lineage-aware discovery at scale (v4,
#6295); payments; browsing accounts or logins (identity is keys);
any change to the D6 registry protocol, the v1 bundle/import/fork
mechanisms, or the v2 channel/update machinery.

## Decisions

**G1 — The public endpoint is a read surface over D6, unchanged.**
The endpoint serves the registry's list/fetch operations plus
search (G3) over records exactly as D6 defines them. Operating a
gallery adds no protocol: a client reading our endpoint and a
client reading any other host's endpoint speak the same protocol,
and a record fetched from either verifies identically (it is
signed and content-addressed). This is the #1240 p2p-identical
rule applied to the gallery.

**G2 — Categories: author-asserted in signed metadata; operator
overlay is separate and unsigned-by-the-author.** Category
assignments travel in the record's signed discovery metadata
(author-asserted, per D6). The taxonomy itself is a governed list
published as a registry-adjacent object by the operator. Where the
operator disagrees with an author's category (miscategorised,
missing), it may attach an _operator overlay_ — its own tags,
stored and served as operator data, visibly distinct from the
signed metadata, never rewriting it. A different host's overlay
differs; the signed record never does.

**G3 — Search: full-text over signed metadata, from a rebuildable
index; ranking is deterministic.** The operator indexes the signed
discovery metadata (name, description, categories, author key).
The index is a cache, not a source: it is rebuildable from registry
records at any time and holds no data that exists nowhere else.
Result order is deterministic and documented: exact-name match,
then category match, then most recent version. **There is no
engagement ranking in v3** — no download counts, no personalised
order, no promoted placement outside G4's labelled collections.
Changing this requires a Drew ruling (see Open questions), because
engagement ranking is the point where a gallery stops being a
registry and starts being a storefront.

**G4 — Featured collections are operator-signed objects, labelled
as the operator's.** A collection is a named, ordered list of
record references, published as its own object signed by the
_operator's_ key and rendered with that attribution ("Featured by
<operator>"). Any host may publish collections the same way;
featuring is a property of a host's editorial voice, not of the
protocol, and a collection never alters the records it lists.
Collections are how curation shows on the endpoint — there is no
unlabeled promotion anywhere else in the surface.

**G5 — Author pages are keyed by public key.** An author page's
canonical address is the author's public key; it lists every
record that key has published to the registry, plus the operator
overlay for that author (G2). Names are display data from signed
metadata, never addresses — two authors may share a display name;
keys may not collide. Key rotation follows the programme's
identity rules (the page follows the identity's key lineage as
recorded in the signed metadata); a lost key's page stays as the
historical record of what that key published — it is not
reassignable.

**G6 — Curation is admission, and admission decisions are public.**
D6's validity-vs-admission line governs: the operator of our
endpoint decides what _it_ lists (admission), and that decision
binds only our endpoint — a refused or removed record continues to
exist in the registry ecosystem and on other hosts. Every removal
from our endpoint is written to a public transparency list (record
identity, date, reason class: illegal content / malware / spam /
policy), so admission is auditable rather than silent. Appeals and
reinstatement are operator process on the same list. The operator
never edits a record it did not author (G2's overlay is the only
operator layer, and it is labelled).

**G7 — Scale is a caching property, not a design change.**
Records are signed and content-addressed, so list/fetch responses
are cacheable at any CDN or proxy tier without trust implications
— a cache cannot forge a record that verifies. Search is the only
stateful surface; its index shards by category and key prefix and
rebuilds from records (G3). The same stack therefore runs at
single-node scale for a self-hoster and at our endpoint's scale
unchanged: hostable by anyone at any scale, per #1240, is a
consequence of G1 + G7 rather than a separate engineering track.

## Open questions / decisions needing Drew (explicit)

1. **Default admission posture for our instance:** open admission
   (anything valid is listed until removed under G6) vs reviewed
   admission (records appear after an operator pass). G6 is written
   to work either way; the default is an operator-policy ruling.
2. **Featured collections:** who holds our operator key and the
   editorial cadence for G4 collections.
3. **Ranking:** confirm G3's deterministic-only order for v3, or
   rule that download counts may be displayed (display only, not
   ordering) in this iteration.
4. **Abuse handling SLA:** response-time commitment for the G6
   reason classes (malware fastest) and the transparency list's
   publication cadence.

## Follow-on implementation work (filed as linked issues)

- Public endpoint read surface: D6 list/fetch + G3 search over the
  rebuildable index (builds on #8281's registry implementation).
- Author pages (G5) + featured collections (G4) + category overlay
  (G2) as operator-layer objects.
- Admission & transparency: G6 removal workflow + public
  transparency list + operator overlay administration.

## Sibling relations

Core topic #6278; v1 #6291 (closed), v2 #6293 (closed), v4 #6295
(federation and lineage-aware discovery — consumes this record's
endpoint and collection shapes, does not change them). Registry
implementation: #8281. Principles: #1240.
