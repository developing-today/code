Status: design record for issue #9128, written 2026-11-23.
Program: #9106 (extensible primitives / federation / namespaces),
Drew directive 2026-11-12 18:29 CST, §5 (30-40). Parent epic: #9107
(Part A — ID core). This record decides the social app reference
model: how feeds, profiles, and bulletin boards are expressed over
the Part A substrate, and the freshness token their views carry.
It builds on the summit sketch (part-a-graph/versioned-design.md
§7), which it supersedes in detail; where this record and §5 of
the directive disagree, §5 governs and this record is the defect.

## 1. What this decides, and what it does not

Decides: the social object envelope and the four application
profiles (Feed, Profile, Board/Topic/Comment, Reaction) as shapes
over nested objects, triples, and append-lists; the freshness
token — its contents, its three forms, and the nothing-new check;
traversal and display defaults for unbounded nesting; the chosen
defaults for the open questions in §8.

Does not decide: the append-list submission protocol (sibling
#9127 — social objects are clients of it), versioning and
carried-forward comments (sibling #9129 — this record consumes
its `succeeds` edge and V3 carry-forward projection), moderation
and true-delete (sibling #9130 — removal here is that design's
tombstone, never a rewrite), the triples encoding (sibling
#9125) or query language (sibling #9126), or client-bound spaces
and petnames (sibling #9131 — profiles reference them).

## 2. Model: one envelope, nested without a data-model cap

A **social object** is immutable content in the content plane
plus signed relations, carried in one envelope:

```
SocialObject = {
  object_id,          // content hash of the content object
  author,             // signer key (identity per Part A)
  created_claim,      // signer-claimed time; untrusted, display only
  parent_ref?,        // the object this one nests under, if any
  content_ref,        // content-plane reference for the body
  schema_version,     // profile schema version (§3-§6)
  succeeds_ref?,      // version lineage, per #9129
  profile,            // feed-entry | profile | topic | comment | reaction
}
```

Three properties define the model:

1. **Nesting is a relation, not a container.** `parent_ref` is a
   signed assertion about two objects. The data model caps
   nothing: a comment nests on a comment to any depth, a topic
   nests in a board, a profile nests objects under itself. Depth
   caps, page sizes, and fold points are view/policy (§7),
   never admission rules.
2. **Acceptance is append-list admission.** Where a shape has an
   ordered public surface (a board's topics, a topic's comments,
   a profile's posts), the surface is an append-list attribute
   under #9127: objects are submitted signed, land in server
   order, and are hidden only by #9130 tombstones. The social
   layer adds no second ordering and no second moderation path.
3. **Order and ranking are projections.** Feed order, board
   order beyond append sequence, and "top" rankings are named
   projections with provenance (which inputs, which policy),
   computed over the graph — never an inherent property of the
   stored objects (summit §7, decision 1 of part-a-graph).

## 3. Feeds (Twitter-like)

A **feed** is a named projection, not a stored list. Its inputs
are: a set of source append-lists (authors' post lists, boards,
group attributes), a membership rule (follows, group standing,
explicit subscription — each itself a signed relation), and an
ordering policy (server append order across sources merged by
projection, or a named ranking). A feed view is a page of
`feed-entry` objects: each entry references the social object it
surfaces, so the same post appears in many feeds without copies.
Feed state a client holds between polls is exactly the view's
freshness token (§6); a feed has no server-side per-client state.

## 4. Profiles (Facebook/LinkedIn-like)

A **profile** is a root social object owned by one identity,
plus the user attributes attached at its path under #9127's
model: `name`, `about`, and `posts` are examples, not reserved
names. Structured profile fields (employer, school, links) are
attribute objects with their own schemas; the connections list
is a relation set (signed edges, projected by #9126 patterns),
displayed under the petname rules of #9131 — a viewer sees their
own name for a profile, not the profile's global claim. A
profile's public surface is the set of its attributes whose
audience admits the viewer; there is no separate "privacy
engine" in this layer.

## 5. Bulletin boards (phpBB/Usenet/Reddit/HN-like)

A **board** is an object with an append-list attribute
`topics`. An accepted submission to `topics` becomes a **topic**:
the submitted object, admitted under the board's append mode
(closed | open | moderated, per #9127's AttributeDefinition).
A topic carries an append-list attribute `comments`; a
**comment** is a social object whose `parent_ref` names the
topic or another comment, admitted to the topic's `comments`
list. Comment bodies are content objects — content-addressed,
living in the mutable area exactly like any object under
#9127 §2 — so a comment is citable, signable, and tombstoneable
independently of the thread rendering. Board views (topic list,
thread tree) are projections over the two append-lists plus
#9130 tombstone state; the tree shape comes from `parent_ref`,
the order from append sequence.

## 6. Freshness token

Forum and feed views are polled far more often than they change.
A view therefore carries a **freshness token**: a small,
server-issued statement that lets a client prove "nothing new"
on the next poll without the server scanning or refetching the
view, and without the client re-downloading it.

### 6.1 Contents

```
FreshnessToken = {
  view_id,        // hash of the canonical view definition:
                  // board/feed id + projection + policy revision
  frontier,       // the view's input frontier at issue time (§6.2)
  issued_at,      // server time of the check that issued it
  server,         // issuing server identity
  signature,      // server signature over the above
}
```

### 6.2 Frontier

The frontier is the vector of high-water marks of the view's
inputs: for each input append-list, its accepted head sequence
(#9127 §4) and the tombstone-set revision (#9130) that filters
it; for relation inputs, the projection checkpoint of the
triples adapter (#9125's `checkpoint`). Two frontiers for the
same `view_id` are comparable component-wise; a frontier is
**unchanged** iff every component equals the current one. The
frontier is a digest-sized value per input — checking it is a
lookup of current heads, never a scan of entries.

### 6.3 The check

A client polling with a token sends the token instead of a view
request. The server verifies the signature and `view_id`, reads
the current frontier, and answers one of:

- `nothing-new` — frontier unchanged. The answer is the token
  re-issued with a fresh `issued_at` (a **latest-check** proof:
  "as of this time, this view is exactly what you hold").
  Cost: one signature verify + one head lookup per input.
- `delta` — frontier advanced. The server returns the entries
  after the token's per-input sequences (append-read with
  `after-seq`, #9127 §3) plus a fresh token. The client never
  refetches the unchanged prefix.
- `resync` — the token's frontier is no longer reachable
  (compaction, policy revision change, tombstone-set rewrite):
  the client refetches the view once and receives a fresh
  token. Resync is a correctness fallback, not an error.

### 6.4 The three forms

The directive names three token uses; they are one mechanism:

- **latest check** — the poll above: "has anything changed
  since this token?"
- **since-date** — a token minted at a past frontier (a client
  returning after days offline) is just an old token: the check
  answers `delta` from its sequences, or `resync` if they aged
  out. Dates are display sugar over sequence frontiers; server
  order, not wall clock, is the authority (#9127 §4).
- **before-date** — a token also pins a historical frontier: a
  view rendered against it shows the view as of that frontier
  (entries with sequence ≤ the frontier, tombstones as of its
  revision). This is the archival/permalink form: a thread
  linked with its token renders the thread as linked, with
  later arrivals offered as a delta.

### 6.5 Trust and privacy

The token is a statement by one server about its own view; it
proves nothing across servers (federated views compose
per-server tokens, one per source server — §8.4). A token
reveals only the frontier of a view the holder could already
read; it carries no entries, no author identities beyond the
view definition hash, and no per-client state. A forged or
foreign token fails signature verification and is treated as no
token (full view fetch), never as an error to the client.

## 7. Traversal and display defaults

Unbounded nesting is representable; readers are bounded.
Defaults (policy may tighten, never silently loosen): max
display depth 32, max visited nodes 2,000 per page, max 8 MiB
response, continued traversal by bounded cursor. Traversal is
cycle-safe: a visited set, with cycles displayed as
back-references rather than recursed into. A folded subtree is
replaced by its freshness token for the subtree's inputs, so
expanding it later is the same §6.3 check, not a blind refetch.

## 8. Open questions, with chosen defaults

1. **Feed merge order across sources.** Default: stable merge
   by (source append sequence, source id) — deterministic and
   replayable; ranked orders are separate named projections,
   never the default view.
2. **Reactions.** Default: a reaction is a minimal social
   object (profile `reaction`, empty body, target as
   `parent_ref`) admitted to a per-target append-list
   `reactions`; counts are projections over that list. No
   separate reaction store.
3. **Profile fields.** Default: no reserved field schemas in
   this layer; well-known fields are ordinary attribute
   schemas registered like any other (a registry entry is an
   implementation follow-on, §9, not a protocol act).
4. **Federated freshness.** Default: per-server tokens
   composed client-side; a view is fresh iff every source
   token checks `nothing-new`. No cross-server token format in
   this record.
5. **Token lifetime.** Default: tokens do not expire by time;
   they become unusable only when their frontier is
   unreachable (§6.3 `resync`). Servers publish a retention
   horizon for frontiers as deployment policy.
6. **Comment identity across versions.** Default: comments
   attach to exact version ids; display on a successor is
   #9129's V3 carry-forward projection, not a re-targeting of
   the comment.
7. **Board hierarchy.** Default: boards nest by the same
   `parent_ref` relation (a board may be a topic in a parent
   board); there is no separate category tree structure.

## 9. Implementation follow-ons

Granular issues filed from this record (linked on #9128):

- #13094 — impl(id): freshness token type, frontier comparison,
  and the nothing-new/delta/resync verdict as pure logic in
  id-core, against #9127 sequences and #9125 checkpoints.
- #13095 — impl(id): social object envelope + the four profiles
  in id-core, with bounded cycle-safe nesting traversal per §7.
- #13096 — impl(id): board view builder — topic/comment tree
  projection over #9127 append-lists with #9130 tombstone
  filtering and per-subtree freshness tokens.

## 10. Acceptance mapping (issue #9128)

- Design section merged under doc/: this file.
- Open questions recorded with chosen defaults: §8.
- Granular implementation issues filed: §9.
- Parent epic link recorded: pointer comment on #9107.
