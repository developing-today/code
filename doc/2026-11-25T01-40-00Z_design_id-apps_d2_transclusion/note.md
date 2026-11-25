# D2: transclusion and embedding of ID objects (slice 1, with slice 3 where it bears)

Design record for #13887 (parent epic #13880, programme order 4).
Inputs: the D3 graph layer record
([`2026-11-24_design_id-graph-layer-typed-links-verbs/D3-graph-layer.md`](../2026-11-24_design_id-graph-layer-typed-links-verbs/D3-graph-layer.md))
and D1 slice 1, the `id://` URI scheme
([`2026-11-25T01-20-40Z_design_id-apps_d1_id-uri-scheme/`](../2026-11-25T01-20-40Z_design_id-apps_d1_id-uri-scheme/)).

Status: slice 1 (embed object semantics) and slice 3 (cycles and bounds)
are designed here and checked by a stdlib reference. Slice 4, the host
rendering contract, is in [`slice-4-host-contract.md`](slice-4-host-contract.md)
with its own checklist. Slice 2, the capability matrix and the delegated
view grant (off by default), is in
[`slice-2-capability-matrix.md`](slice-2-capability-matrix.md); §7 here
records the default it builds on.

## 1. Problem

Drew asked for "embedding ID objects inside other ID objects". The bulk
form of that is what Cobweb called transclusion: a post that shows a live
photo album, a note that shows a reply thread, a page that pulls in a
whole subgraph. Cobweb never had to answer who may see an embed. ID has
to, because every object sits in an area with its own readers.

D3 already has `quote` (show part of another object) and `attach` (carry
a file). Neither says whether the rendering follows the target's later
edits, neither can name a collection, and neither bounds what a render
may expand to. That is what this record adds.

## 2. Decisions

1. **A new D3 verb, `embed`.** An embed is a D3 link object whose verb is
   `embed`, so it is signed, authored, revocable by supersede, and
   visible to every reader rule D3 already has. Embedding is never a
   rendering hint in a payload (that alternative is rejected in §9).
   Adding a verb goes through D3's admission rule, so this needs Drew's
   sign-off (open question 1).
2. **Two modes, chosen per embed: `live` and `snapshot`.** Live follows
   the target's supersede head. Snapshot is bound to an exact object id,
   which is already a content digest under D3, so a snapshot freezes
   without copying anything.
3. **Two scopes: `object` and `collection`.** A collection is the D3 nest
   projection of a container (D3 §4), the same thing D1 reserves the
   `set/` URI shape for in its slice 2.
4. **Every collection embed states `bounds.max_count`.** A render can
   never be asked to expand an unbounded set.
5. **One `unavailable` state** for a target the viewer cannot read, one
   that is not stored, and one that was pruned. The render reveals
   nothing about which of the three it is.
6. **A snapshot pins one level.** Embeds inside a snapshotted object keep
   their own author's mode (§4.3).
7. **Embedding grants the viewer nothing.** The viewer's own read rights
   decide every slot (§7).

## 3. The embed link

An embed is a D3 `LinkObject` with `verb: "embed"` and one extra member,
`embed`. D3's schema has `additionalProperties: false`, so the member is
added to the schema with the verb; nothing else in `LinkObject` changes.

```json
{
  "object_id": "lnk-embed-thread-snapshot",
  "author": "ed25519:...",
  "created_claim": "2026-11-24T02:30:00Z",
  "verb": "embed",
  "source": "note-recap",
  "target": "post-meetup-thread",
  "embed": {
    "mode": "snapshot",
    "scope": "collection",
    "pin": {
      "root": "post-meetup-thread",
      "members": ["msg-photo-answer", "post-photo-published"],
      "members_digest": "sha256:aeff1c28..."
    },
    "bounds": { "max_count": 20, "max_depth": 2 }
  },
  "schema_version": 1
}
```

| Field    | Rule                                                                                                                                                                                                                                                         |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `mode`   | `live` or `snapshot`. Anything else is refused.                                                                                                                                                                                                              |
| `scope`  | `object` or `collection`.                                                                                                                                                                                                                                    |
| `pin`    | Required for `snapshot`, forbidden for `live`. Object scope: `{object}`, the exact object id shown. Collection scope: `{root, members[], members_digest}` with `root == target` and `members_digest = "sha256:" + hex(sha256(join("\n", sorted(members))))`. |
| `bounds` | Collection scope requires `max_count >= 1`. `max_depth` is optional everywhere (default 3). A snapshot may not pin more members than `max_count`.                                                                                                            |
| (other)  | Unknown members of `embed` are refused, matching D3's closed schema.                                                                                                                                                                                         |

The target is written in D1 form when it leaves the graph: an object
embed is `id://any/graph/v2/obj/<id>` for a snapshot and
`id://any/graph/v2/head/<id>` for a live embed, and a collection embed
uses D1's reserved `set/` shape once D1 slice 2 defines it.

**Create rule.** An embed is valid only if:

- its author is the author of its source (you embed into your own object,
  as with every D3 link that changes how a source renders), and
- that author could read the target when the embed was created. This is
  the anti-exfiltration rule: you cannot point a public object at
  something you were never allowed to see. A pruned target cannot be
  re-checked, so a pruned target on an existing embed is accepted and
  renders as `unavailable`.

## 4. Live and snapshot

### 4.1 Object embeds

- **Live** resolves the target's supersede head for this viewer (D3 §5,
  reader rule 3: the newest readable successor). If the head is not the
  target, the slot carries the badge `edited`. If the supersede chain
  forks, it carries `fork`, as D3 already requires of readers.
- **Snapshot** shows exactly `pin.object`. If the viewer can read a newer
  head, the slot carries `changed-since-embed`, so a reader is never
  misled into thinking a frozen view is current. If the pinned object is
  a tombstone, the slot is `removed`.

### 4.2 Collection embeds

- **Live** shows the nest projection now, oldest first, restricted to
  members the viewer can read, cut at `max_count`, with the remainder
  shown as `more: <n>`.
- **Snapshot** shows the pinned members, minus any the viewer can no
  longer read. If members were added since the pin, the slot carries
  `changed-since-embed`. The digest makes a tampered member list a
  refused link, not a different render.

### 4.3 What a snapshot freezes

The issue asked whether a snapshot is digest-bound or render-bound.
This record picks **digest-bound, one level deep**:

- A snapshot freezes the identity of what it shows: one object id, or one
  member list. D3 objects are immutable and content-addressed, so the
  bytes behind those ids cannot change. Nothing is copied.
- It does not freeze how that object's own embeds render. If a pinned
  post has a live embed of a photo, the photo still follows its head.
  Freezing the whole tree would mean copying every nested target, or
  rewriting other authors' embed links, which an embedder has no right
  to do. The fixture `note-embeds-thread` shows this: the recap note's
  snapshot of the thread still shows the post's live photo at its
  edited head.
- A render-bound snapshot (freezing the rendered pixels or HTML) is
  rejected for v1 (§9 and open question 2).

## 5. Render states

Every embed slot renders to exactly one state:

| State         | Meaning                                                                                                                 |
| ------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `shown`       | The target (object or members) is shown, possibly with badges.                                                          |
| `unavailable` | The viewer cannot read it, it is not stored, or it was pruned. Same state, same shape, no mode or scope, no hint which. |
| `removed`     | A snapshot pins a tombstone the viewer can read.                                                                        |
| `cycle-cut`   | The target is already on the render path (§6).                                                                          |
| `depth-cut`   | The depth bound ran out.                                                                                                |
| `budget-cut`  | The host-wide render budget ran out.                                                                                    |

Badges on a `shown` slot: `edited` (live head moved), `fork` (supersede
fork), `changed-since-embed` (snapshot is behind). Unreadable members of
a collection are left out and not counted in `more`, so a collection
reveals neither what nor how many objects a viewer cannot see.

Slice 4 turns these into host obligations (sizes, lazy loading,
not-yet-synced) in [`slice-4-host-contract.md`](slice-4-host-contract.md).

## 6. Cycles and bounds (slice 3, as it bears on embeds)

- **Cycles.** The renderer carries the path of objects it is inside.
  An embed whose target, or whose resolved live head, is already on the
  path renders `cycle-cut` and stops. A collection member already on the
  path does the same. Detection is per render, so two notes embedding
  each other are both valid links, and each renders the other once.
- **Depth.** Each slot has a depth budget, the smaller of the parent's
  remaining depth and the embed's `bounds.max_depth` (default 3). An
  embed at zero renders `depth-cut`.
- **Budget.** A host-wide budget (64 slots in the reference) caps one
  render whatever the individual bounds say. Members past the budget are
  added to `more`; later embeds render `budget-cut`.

Together these make every render finite: depth bounds the height,
`max_count` and the budget bound the width.

## 7. Capability default (slice 2 builds the matrix on this)

- **Embedder:** must be able to read the target at create time (§3).
- **Viewer:** sees a slot only if the viewer can read the object shown.
  An embed confers no view of its own. This is the default the issue
  asks for: embedding grants nothing the embedder could not already
  delegate, because it delegates nothing.
- **The leak case.** Fixture `private-target-outsider`: a public forum
  post embeds a document in its author's private area. A reader outside
  that area gets `unavailable`, with no mode, scope or title. The author
  sees the document (`private-target-owner`). The attack fails by
  design, not by a host's discretion.

Slice 2 ([`slice-2-capability-matrix.md`](slice-2-capability-matrix.md))
gives the full matrix and specifies a delegated, scoped, read-only view
grant, off by default, so public embedding of restricted content can only
ever be a deliberate, recorded choice by the target's manager.

## 8. Fixtures and the reference

[`fixtures/`](fixtures/) holds a small graph, a stdlib reference
renderer, and the expected render trees:

```
python3 -I fixtures/embed_reference.py check    # ok: 8 embeds, 8 cases, 5 refusals
python3 -I fixtures/embed_reference.py render   # print the render trees
```

`graph.json` has four authors, six areas (three public, a three-person
room and two private areas), 19 stored objects, one pruned object, a
supersede, nest links and eight embed links.

| Case                           | Viewer | What it shows                                                                                                                                            |
| ------------------------------ | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `post-embeds-photo`            | anyone | Live object embed follows the supersede head; badge `edited`.                                                                                            |
| `note-embeds-thread`           | anyone | Snapshot collection of a reply thread; a member added later gives `changed-since-embed`; the pinned post's own live embed still follows its head (§4.3). |
| `page-embeds-collection`       | ana    | Live album, `max_count` 3, one private member left out and not counted: three shown, `more: 1`. A snapshot photo embed beside it is behind its head.     |
| `page-embeds-collection-owner` | cyp    | The same page for the album owner: the private image is in the list, `more: 2`.                                                                          |
| `private-target-outsider`      | ana    | Public post embedding a private document: `unavailable`.                                                                                                 |
| `private-target-owner`         | bo     | The same post for the document's owner: shown.                                                                                                           |
| `cycle`                        | anyone | Two notes embedding each other: the second hop is `cycle-cut`.                                                                                           |
| `pruned-snapshot`              | anyone | Snapshot of a pruned object: `unavailable`, same shape as unauthorised.                                                                                  |

The checker also mutates the graph and requires each of these to be
refused: a live embed with a pin, a collection embed without bounds, a
tampered member digest, an unknown mode, and an embed whose author
cannot read its target.

## 9. Alternatives

- **Copy-on-embed only.** Safe and simple, but loses the live property
  that makes transclusion different from attachment, and duplicates data
  the graph already holds. Snapshots here get the same safety by pinning
  immutable ids instead of copying.
- **Reuse `quote` with a mode flag.** A quote shows part of one object
  and carries the quoted span; it has no collection scope and no bounds.
  Overloading it would change how every existing quote is read.
- **Embed as a rendering hint in the payload.** No author, no signature,
  no revocation, no reader rules: unauditable. Rejected.
- **Render-bound snapshots.** Freezing the rendered form needs a stored
  rendering that differs per host and per viewer, and nested targets
  would have to be copied. Deferred (open question 2).
- **Deep snapshots.** Freezing nested embeds would require copying their
  targets or rewriting links other authors own. Rejected for v1.
- **Viewer must hold a grant for every embed.** That is §7's default
  for restricted targets; for public targets it changes nothing, so
  public embedding still works. The delegated-view variant is slice 2.

## 10. Open questions

1. **Admit `embed` as a D3 verb** (decider: Drew, under D3's admission
   rule). The alternative is a typed `quote`, which §9 argues against.
2. **Render-bound snapshots later?** (decider: Drew, with D5 as the
   first host that would need them.) This record ships digest-bound only.
3. **Should a collection snapshot also badge removed members?** Answered
   by slice 4 §3.5: no. Additions only; a host shows removals from the
   nest's own history.
4. **Delegated scoped view**: specified by slice 2 §4 as a manager-issued,
   snapshot-only, expiring view grant and kept off by default. Whether to
   turn it on is Drew's decision (slice 2 §7).

## 11. Compatibility

No existing verb, link or object changes. The `embed` member is new and
only valid with the new verb. Readers that do not know `embed` treat it
as an unknown verb, which D3 already requires them to skip, so old hosts
show the source without the embed rather than misrender it.
