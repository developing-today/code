# D2: transclusion and embedding, slice 2 (capability matrix)

Design record for #13887 (parent epic #13880), slice 2 of 4. It builds on
slice 1 in [`note.md`](note.md), whose §7 fixed the capability default
(embedding grants the viewer nothing), and on slice 4 in
[`slice-4-host-contract.md`](slice-4-host-contract.md), whose rules only
narrow what a host may show.

Executable matrix: [`fixtures/capability_matrix.json`](fixtures/capability_matrix.json),
run by [`fixtures/capability_reference.py`](fixtures/capability_reference.py)
(`python3 -I fixtures/capability_reference.py check`, stdlib only). It loads
the slice 1 graph plus a slice 2 overlay
([`fixtures/view_grants.json`](fixtures/view_grants.json)) and runs one check
per matrix cell and per worked attack. Today it reports 12 cells, 10 attacks
and 15 checks, with 0 failures. The slice 1 fixtures and `expected.json` are
unchanged.

## 1. Problem

The issue's acceptance for this slice is the leak test: embed a private
object into a public one, and show that the attack fails or that it is an
explicit, recorded design choice. Slice 1 showed it failing for one case. What
was missing is the full table: for each embed kind, what the embedder needs,
what the viewer needs, what the target's owner can take back, and whether
there is ever a sanctioned way to show restricted content in a public embed.

## 2. Decisions

1. **The matrix is data.** As with slice 4's checklist, every cell names a
   check, every check is named by a cell or an attack, and the run fails if
   either side drifts.
2. **Traversal starts from what the viewer can read.** A viewer who cannot
   read the source never reaches its embed links. The reference returns no
   render at all for such a root, rather than a tree of `unavailable` slots.
3. **A pin names only what its embedder could read.** New create rule: a
   collection snapshot may not pin a member its embedder cannot read. The
   pin's member ids sit in the embed link, which everyone who reads the
   source can read, so a pin naming a private member would publish that
   member's existence.
4. **Embedder rights never flow to viewers.** If the embedder later loses read
   on the target, no other viewer's render changes. The embed is a pointer,
   not a credential.
5. **The delegated scoped view is specified and off by default.** A target's
   manager may issue a _view grant_ that fills one snapshot embed's slot for
   readers of the source. It is written up in full (§4) so the choice is
   explicit and auditable, but no host turns it on until Drew decides open
   question 4 (§7). With the switch off, every grant is ignored and slice 1's
   render trees are byte-for-byte unchanged (check `grants_off_by_default`).

## 3. The matrix

Four embed kinds: live object, snapshot object, live collection, snapshot
collection. "Read" and "write" are composition A capabilities on an area
(D3 §8).

| Phase    | Party        | Needs                                                                                                     | Kinds      |
| -------- | ------------ | --------------------------------------------------------------------------------------------------------- | ---------- |
| create   | embedder     | Write on the source's area, authorship of the source, read on the target at create time.                  | all        |
| create   | embedder     | For a collection: read on the container. Members the embedder cannot read are not pinned (§2.3).          | snap. coll |
| traverse | viewer       | Read on the source's area. Without it the embed link is never reached.                                    | all        |
| render   | viewer       | Read on the object shown: the live head, or the pinned object.                                            | objects    |
| render   | viewer       | Read on the container. Each member needs its own read; an unreadable member is neither shown nor counted. | coll.      |
| render   | viewer       | Nothing from the embedder. The embedder losing read later changes no viewer's slot.                       | all        |
| render   | viewer       | Embeds inside a shown object render by the viewer's own rights, one level at a time.                      | all        |
| revoke   | target owner | Removing a viewer from the target's area turns the slot `unavailable` on the next render.                 | all        |
| grant    | issuer       | Manage on the target's area at issue time.                                                                | snapshots  |
| grant    | issuer       | The grant names one embed link and exactly its pin. Live embeds take no grant.                            | snapshots  |
| grant    | viewer       | Read on the source's area and a grant inside its window. The slot names the grant.                        | snapshots  |
| grant    | target owner | Revocation is non-renewal: a window of at most 7 days, after which the slot is `unavailable`.             | snapshots  |

The four `grant` rows apply only when delegated views are switched on.

## 4. The delegated view grant

A grant is a signed object, not a field on the embed, because the embedder
alone cannot confer a view: only someone who manages the target's area can.

```json
{
  "grant_id": "grant-plan",
  "issuer": "ed25519:bbbb2222forum",
  "area": "forum-public",
  "link": "lnk-embed-plan-snapshot",
  "covers": { "object": "doc-bo-plan" },
  "rights": ["read"],
  "issued_claim": "2026-11-24T05:10:00Z",
  "not_before": "2026-11-24T05:10:00Z",
  "not_after": "2026-11-30T00:00:00Z"
}
```

A grant is valid only if all of these hold:

- **The issuer manages the target's area** at issue time. An embedder who is
  not a manager cannot write one that counts (attack A2).
- **It names one embed link**, and that link is a snapshot. A live embed would
  let the grant cover whatever the head becomes, content the issuer has never
  seen, so live embeds take no grant (A3).
- **`covers` equals the pin exactly**: `{object}` for an object snapshot,
  `{root, members_digest}` for a collection snapshot. Not another object, not
  the whole area, not a different member set (A4).
- **`rights` is `["read"]`** and nothing else.
- **It is stored in the source's area**, so every reader of the source can
  fetch it and nobody else needs to.
- **Its window is at most 7 days** and starts no earlier than the embed.
  Revocation is non-renewal, as in composition A: the issuer stops reissuing
  and the slot goes `unavailable` when the window ends (A6). The cap bounds how
  long a revoked view can linger.

When a valid grant is inside its window and the viewer cannot read the target
by their own right, the slot renders the pin as `shown`, with one extra key:
`"via": "<grant_id>"`. It carries no badges, because the grant covers the
pin and not the target's history: `edited` or `changed-since-embed` would
reveal private edits. A viewer who can read the target by their own right
gets the ordinary slot with no `via`; a grant never adds to an existing right.

What a grant does not do:

- It fills its own slot and nothing else. The same object opened directly, or
  embedded a second time, is still `unavailable` to an outsider (A5, A7).
- Embeds inside the granted object render by the viewer's own rights (A8),
  the same one-level rule as a snapshot (note §4.3).
- A collection grant covers the pinned member set. Members nested later are
  not shown through it (A9).

## 5. The worked attacks

Each attack is a check in `capability_reference.py`.

| Id  | Attack                                                                  | Outcome                                                            |
| --- | ----------------------------------------------------------------------- | ------------------------------------------------------------------ |
| A1  | Embed a private document into a public post and show it to an outsider. | `unavailable`, with no mode, scope or title.                       |
| A2  | Embedder writes a grant for a target area it does not manage.           | Grant ignored; `unavailable`.                                      |
| A3  | Grant on a live embed, so it covers whatever the head becomes.          | Grant refused; `unavailable`.                                      |
| A4  | Grant covers more than the pin.                                         | Grant refused; `unavailable`.                                      |
| A5  | Reuse a grant on a second embed of the same object.                     | Grant does not apply; `unavailable`.                               |
| A6  | Keep showing after the window ends, or issue an over-long window.       | `unavailable` after `not_after`; an over-long window is refused.   |
| A7  | Use the grant to read the target outside the embed.                     | Refused; the grant only fills its slot.                            |
| A8  | Reach a second private object through an embed in the granted one.      | Inner slot `unavailable`.                                          |
| A9  | Grow a granted collection after the grant to slip new members in.       | Only pinned members show; later ones need the viewer's own read.   |
| A10 | Turn delegated views on for one host only.                              | With the switch off every grant is ignored and A1 holds unchanged. |

So the leak test passes in both readings the issue allows. With grants off
(the default), the attack fails by design. With grants on, showing restricted
content in a public embed is possible only as a signed, scoped, expiring,
revocable decision by the target's manager, visible in the render as `via`.

## 6. What changes elsewhere if grants are adopted

- **Slice 4:** `shown` gains the optional key `via`, and §3.8 gains one line
  of wording ("Shared by <issuer> until <date>"). Nothing else in the host
  contract changes. Not done here, because grants are off.
- **D3:** a grant is a new object kind under the admission rule, not a new
  verb.
- **Composition A:** the issuer's manage right is a Meadowcap capability over
  the target's area. Issuing a grant is an attenuation of it to one object and
  read only, which Meadowcap can express; the fixtures model it as a
  `managers` list per area.

Residual exposure, unchanged by this slice: an embed link names its target's
id, so a reader of the source learns that some object with that id was
embedded. Ids are opaque digests, as with D3's `attach` and `quote` links,
which name their targets the same way.

## 7. Open questions

1. **Adopt delegated views?** (decider: Drew.) This record specifies them and
   proves them leak-tight in the reference, but they stay off until adopted.
   The cost of adopting is one key in the host contract and one object kind.
   The cost of not adopting is that public embedding of restricted content is
   impossible, which is the SSB-style all-or-nothing the issue warns against.
2. **Window cap.** 7 days matches a weekly renewal at broker check-in. A
   shorter cap tightens revocation; a longer one means fewer reissues.
   (decider: Drew, if grants are adopted.)

## 8. Alternatives

- **Grant as a field on the embed link.** Rejected: the embedder would be
  writing its own permission, and a grant could never be renewed without
  superseding the embed.
- **Grants on live embeds, scoped to the target's area.** Rejected: the
  issuer would approve content that does not exist yet (A3).
- **Viewer-bound grants (one grant per reader).** The purest capability story,
  but useless for a public post, whose readers are unknown. A source-area grant
  reaches exactly the people who can already read the post.
- **No grant at all, ever.** This is the default and stays available: if Drew
  declines open question 1, slice 2 still stands as the matrix and the attack
  proofs for the default.
