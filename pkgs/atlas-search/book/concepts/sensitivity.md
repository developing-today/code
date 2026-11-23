# Sensitivity tiers and client ceilings

Source: https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/filters.rs, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/disclosure.rs, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/manifest.rs, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/shards/mod.rs, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/space.ts

Atlas holds Drew's own conversations, notes, and working records, and it
serves them to callers that are not equally trusted: Drew himself, his
agents, and hosted surfaces reachable from outside the laptop. Sensitivity
is therefore not a label for tidiness — it is the access-control model, and
it is enforced by the server, inside the search queries, before anything is
ranked or rendered. This chapter defines the two ladders the model uses,
the mapping between them, and the machinery that keeps material at the tier
it was classified into. The corpus the ladders govern is described in
[The corpus: provenance and content addressing](corpus-and-provenance.md).

Two absolute rules frame everything below. Candidate secret content is
never reproduced in issues, chat, or logs — findings are reported as
counts, classes, and locations only. And secret text is never sent to a
hosted embedder. This chapter follows both rules: it describes the shape of
the secret tier and nothing of its content.

## Two ladders, one model

The model has two vocabularies because it answers two different questions:
*where may this content live?* and *how much may this caller see?*

**Corpus tiers** — where content lives. Three tiers, in ascending
sensitivity: `public`, `internal`, `secret`. A tier is a physical placement
as much as a classification: content of a tier lives in that tier's
databases. At the 2026-10-31 census the placement was 28 internal shards
holding 107,001 chunks and 1 secret shard holding 638 chunks; there were no
public-tier shards. The standing rule for the top tier is that secret
material may exist only in the two secret databases (the secret scan shard
and its secret content store). One recorded exception to that rule is
stated at the end of this chapter.

**Client ceilings** — how much a caller may see. Four levels, in ascending
order, named in code (`SENS_ORDER`) `shareable < personal < local-only < secret-bearing`.
The canonical terminology for the four classes is
`public < internal < private < sensitive`, in the same order — `shareable`
is public, `personal` is internal, `local-only` is private, and
`secret-bearing` is sensitive. Where this book speaks of the classes
themselves it uses the canonical names; where it quotes a flag value, a
config cap, or a stored label it spells the value as the code does, because
the code spellings remain the literal values until the identifier rename
(#13506) lands with alias/migration handling for the stored values.
A *ceiling* is a maximum: a search run at ceiling `personal` may return
chunks classified `shareable` or `personal`, and nothing above. The default
ceiling is `personal` — a caller who asks for nothing gets `personal`, and
`local-only` and `secret-bearing` material is excluded unless a ceiling
above the default is both requested and granted.

The mapping between the ladders is fixed:

| Client ceiling (code spelling) | Canonical name | Corpus tier it reaches |
|---|---|---|
| `shareable` | `public` | `public` |
| `personal` | `internal` | `internal` |
| `local-only` | `private` | `internal` |
| `secret-bearing` | `sensitive` | `secret` |

Note the deliberate pinch in the middle: two ceiling levels map to the
internal tier. The ceiling ladder is finer-grained than physical placement,
because query-time filtering works per chunk while placement works per
database. The two vocabularies are disjoint value sets in the schemas — a
staging database's `sensitivity` column speaks tier words and defaults to
`internal`, while the published `chunks` and `session_labels` columns speak
ceiling words — and the translation happens at promotion, in the aggregator,
in one direction, and at shard selection, in the other. No schema declares
the mapping; the two code points above are where it lives. Whether the tier
vocabulary and the ceiling vocabulary are one ladder or two is a deliberate
open decision, recorded on #13506: the canonical ceiling names now share
`public` and `internal` with the tiers, which makes the question sharper,
not settled.

A third, related vocabulary appears at the output boundary: the
*disclosure context*, which names the destination surface rather than the
caller. Each context carries its own ceiling — `fleet-internal` allows up
to `local-only`, `hosted-per-user` up to `personal`, `public-surface` up to
`shareable`, and `local-owner` up to `secret-bearing` — and the effective
ceiling for a response is the *minimum* of the caller's granted ceiling and
the context's ceiling. Neither widens the other. An absent or unrecognized
context fails closed to `public-surface`. One classification sits outside
the ladders entirely: content marked `excluded_personal_life` is ineligible
in every context, including `local-owner`.

## Enforcement: server-side, per client identity

A ceiling requested by a client is a request, never a grant. The service
holds a named identity for each client, each with a cap, and clamps:
effective ceiling = min(requested, the identity's cap). The clamp is
recorded on the search job and reported back in the result envelope, so a
caller can always see the ceiling it was actually served at. The named
identities in service today:

| Client identity | Who it is | Ceiling cap |
|---|---|---|
| `drew-cli` | Drew's own CLI on the laptop | `secret-bearing` |
| `sandbox-scripts` | Muse lanes and scripts on the sandbox seat, via the tailnet | `personal` |
| `cf-worker` | The Cloudflare Worker, calling the service | `personal` |

The design of record's v1 client set matches this shape: Drew's CLI is the
only identity capped above `personal`; OpenChamber sessions and fork /
OpenCode agents were likewise specified at `personal`. Raising a cap is a
local edit to the service configuration by Drew — there is no request a
client can make that raises its own cap.

Enforcement happens at three depths, so that no single layer's failure is
sufficient to leak:

1. **Inside the branch queries.** The ceiling is a clause in the SQL that
   gathers the full-text and vector candidate pools — `{sensitivity} IN
   (allowed set)` — applied before ranking. A chunk above the ceiling is
   never retrieved, never counted, and its existence is not leaked by a
   gap in the results. A chunk with NULL sensitivity never passes.
2. **At the shard set.** Tier ceilings are physical: a shard above the
   caller's ceiling — or of a tier for which no credential is held — is
   never connected, never probed, never scanned. Each tier has its own
   read-only token, and the secret tier's token is never derived from
   another tier's. Because the default ceiling `personal` maps to the
   internal tier, a default fan-out can never include the secret tier; it
   takes an explicit `secret-bearing` ceiling *and* the secret credential.
3. **At the output boundary.** Retrieved details are checked again against
   the effective ceiling (caller grant intersected with disclosure
   context) before rendering, in every output shape — flat, grouped, and
   raw. The served set recorded in the query trace is collected from the
   rendered envelope, so cached and paged payloads inherit the filter.

Fail-closed defaults run through the model. Unknown, missing, or malformed
sensitivity — in a classification record, a staged artifact, or a chunk's
stored value — resolves to `SecretBearing`, the most restrictive level.
When several classification records cover one chunk, the effective
classification composes conservatively: the sensitivity is the maximum
over the records, and an exclusion, once recorded by any of them, sticks.

The Worker surface pins its side of this in code, not configuration: its
ceiling (`personal`) and disclosure context (`hosted-per-user`) are
compile-time constants with no request field, header, or environment
variable able to change them, and it verifies the service's reported
effective ceiling before any result is allowed into its cache — a result
computed above `personal` can never be cached, let alone served. That
surface is covered in [The service and the Worker](service-and-worker.md).

## The classification machinery

Classification is assigned at ingest and then defended by several
independent passes, because a single classifier's mistake at the secret
boundary is the failure this whole model exists to prevent.

**The classification manifest (#5410).** A small table in the corpus
database declares the filterable dimensions — kind, client, topics,
projects, creation time — with their storage shapes and match semantics,
and carries the per-source tier rules. Its safety invariant is *narrow,
never widen*: the manifest schema has no sensitivity keys at all (the
names `sensitivity`, `max_sensitivity`, `ceiling`, and `sens` are a
reserved namespace the loader rejects), its column shapes bind only to
code-known label columns, and any unknown shape, semantics, column, or key
fails validation, in which case the caller falls back to the built-in
manifest — loudly, and without widening anything. A manifest can change
how content is filtered; it cannot change how much of it a caller may see.

**Revalidation (#6021, closed).** A corpus-wide revalidation programme
checked that nothing sensitive sits at the wrong tier, combining three
independent signals: deterministic secret-pattern scanning (gitleaks),
a classifier pass, and a vector-neighbourhood check — internal chunks that
embed *close to* secret chunks are candidates for review, a check that is
only meaningful because one embedding model and pipeline serve every tier.
The Layer-1 scan report (2026-11-15) records counts, classes, and ids
only, ran gitleaks with redaction, and used private copies that were
deleted after the scan.

**The Layer-1 review (#10412, closed).** The scan's candidates were then
reviewed one by one: 47 candidate chunks carrying 67 findings, plus 45
candidate messages, each ruled on and the review closed under Drew's
2026-11-22 Atlas order. It is a review record, not a design statement —
its value to this chapter is that the secret tier's membership has had a
human-scale pass over its boundary cases, not only automated scans.

**The Presidio NER pass (#6048).** A design record for a named-entity
recognition pass over the corpus — recognizer set, tier scoping, cost, and
a review queue — aimed at the personal and financial detail that pattern
scanning for credential shapes does not catch. It is recorded as design
only, with no code enabled; treat it as part of the machinery's agenda,
not as a pass that runs today. A revalidation-cadence design (#6051)
similarly proposes rule-triggered rescans and sampled human audits as
standing practice.

**Boundary-safe classification.** Where a source's producer declares
classification segments over byte offsets, staging composes them through
the boundary splitter described in
[Ingest today: staging, classification, promotion](ingest.md): chunks are
cut only at declared boundaries, never across one, and any text whose
classification is undeclared, contradictory, or uncertain becomes a single
`secret-bearing` chunk marked uncertain — doubt resolves upward, to the
most restrictive tier.

## The hosted-embedder rule

Embedding is the one step that would ordinarily send text to an outside
service, so it has its own rule: **secret text is never sent to a hosted
embedder.** Atlas's production embedder runs in-process on Drew's hardware,
which satisfies the rule for the corpus as built; the rule's teeth are in
ingest. Until a dedicated local secret-tier embedder lands (#6059, designed
but not built), secret-tier source classes are held out of automated
ingest sweeps entirely — held, not rejected: held items form a queue that
re-offers automatically once a compliant route exists. No sweep may route
around the hold by embedding secret material anywhere else.

## The open boundary defect: #8479

One defect is on the record, stated here in shape only. The shard set's
boundaries are clean — but the same **638 secret-bearing chunks also live
in the production monolith and in a local replica**, alongside pre-tiering
unclassified snapshots. Outside the shard set, then, the standing claim
that secret material exists only in the two secret databases does not
currently hold: query-time ceiling enforcement still governs what any
caller is *served* from those stores, but the physical placement boundary
— the second layer of the defence above — is not intact there.

The remediation is filed (#12406: inventory, a verified destination copy
and re-embed, a visibility switch, replica and cache purge, trace-safe
tombstones, and post-run absence checks), but the threshold decision —
purge the monolith copies, or reclassify — is **Drew's, and it is
undecided**. This book records the defect and the options; it does not
pick one. Any claim that Atlas's tier boundary fully holds should be read
with #8479 attached until that decision is made and the migration has run.
