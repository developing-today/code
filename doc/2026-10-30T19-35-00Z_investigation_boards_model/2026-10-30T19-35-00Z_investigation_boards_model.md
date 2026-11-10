# ID boards — a modern phpBB on ID identity: board model sketch

**Date:** 2026-10-30 · **Issue:** #2052 (LIST 15a) · **Epic:** #2056 · **Feeds synthesis:**
#2054 (boards v1, with playable-posts investigation #2053)
**Scope:** Board objects in the ID graph (creation, membership, posting pictures/audio/
video), moderation under the accept-all-signed-objects trust model, theme format plus
copy/share mechanics between boards, federation between boards hosted on different
servers/IDs. Out of scope: embedded playable posts (#2053 owns those) and the boards v1
synthesis itself (#2054 consumes this).
**Method:** this doc records the board-model sketch first produced by queue task 3357
(2026-10-29, on the issue) as the wave-pattern investigation document, re-verified
against `origin/main` (`ad18a0bf`): no forum-board implementation exists in the tree,
the trust model is taken from #2030's issue text (its investigation doc is PR #2801,
still open at write time), and moderation/transport neighbours are the #2058 bridge and
abuse-controls findings. This is a model sketch, not an implementation and not a PoC.

---

## TL;DR verdict

A board is not a server-side forum with accounts. It is a **signed object graph rooted
at a Board object**: the board's identity is its creator's Iroh ID plus a board ID;
forums/threads/posts are signed child objects in the ID graph; "joining" is subscribing
to the board's object set and (optionally) publishing a signed membership attestation —
there is no registration step anywhere. Themes are **content-addressed, signed theme
objects** referenced by hash from the board object, so copying a theme is
referencing/forking the same object, never exporting files. Moderation is **not
deletion from a central store** — under accept-all/trust-filter, servers accept every
validly signed object; moderation is the board's signed moderation log plus each
reader's/board's _view filter_ (allowed-writer set, role attestations, hide/label
actions). Federation is **replication of the same signed object set between hosts**
(Iroh sync / public ID servers / Workers-edition HTTP-WS subset), with the board root
hash as the shared address — a board is portable because its authority lives in
signatures, not in the host.

---

## 1. Board objects in the ID graph

All objects are validly signed by their author's Iroh ID, content-addressed, and linked
by hash — the same envelope/graph conventions as the rest of ID.

| Object                         | Author/signer                                         | Contents (sketch)                                                                                                                                                      | Notes                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ------------------------------ | ----------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Board` (root)                 | Creator ID                                            | board ID (hash of creator ID + nonce), display name, description, `theme_ref` (hash), `rules_ref` (hash of a Rules object), visibility mode, creation timestamp        | The root hash IS the board's address; shareable as an ID link/QR. Renaming/retheming = a new signed version object superseding the prior version, same board ID.                                                                                                                                                                                                                                                             |
| `Rules`                        | Board owner (or current moderators, per board policy) | posting policy, media limits, role definitions, federation policy (open / listed / invite-only-replication)                                                            | Versioned like the root; clients pin to a rules version when rendering a thread, so mid-thread rule changes don't rewrite history.                                                                                                                                                                                                                                                                                           |
| `Forum` (category/sub-board)   | Owner/moderator attested role                         | name, ordering, parent board ref                                                                                                                                       | phpBB's forum/category layer; optional — a small board can be one flat forum.                                                                                                                                                                                                                                                                                                                                                |
| `Thread`                       | Any poster the board's view admits                    | forum ref, title, opening post ref, sticky/locked flags (via moderation log, not mutation)                                                                             | Threads are containers; ordering by first/last post object timestamps in the view layer.                                                                                                                                                                                                                                                                                                                                     |
| `Post`                         | Any poster                                            | thread ref, parent post ref (reply-to; enables threaded + flat rendering), body (markdown), attachment refs (content hashes), edit history = superseding post versions | Posts are immutable; an "edit" is a new signed version citing the old hash. Deletion by the author = a signed tombstone request; enforcement is view-layer (see §5).                                                                                                                                                                                                                                                         |
| `Attachment`                   | Poster                                                | content hash (blob in the ID store), media type, size, alt text                                                                                                        | Pictures/audio/video are blob refs, never inline payloads — keeps graph objects small and lets hosts apply size/budget policy (LIST 8 budgets) at the blob layer. Large media reuses the chunked retrieval pattern (#2050).                                                                                                                                                                                                  |
| `Membership` (optional)        | The member themselves                                 | board ref, member ID, display-name claim, avatar ref                                                                                                                   | Self-issued, not approved: "membership" is a public statement "I read/post here", used for member lists and role applications. Boards that want a gate put the gate in the view filter, not in admission.                                                                                                                                                                                                                    |
| `RoleAttestation`              | Board owner / existing moderators                     | board ref, subject ID, role (`owner`, `moderator`, `pinned-poster`, …), expiry/revocation ref                                                                          | This is how authority exists without accounts: a role is a signed claim by an already-authoritative ID, verifiable by any client or host from the graph alone. Succession (phpBB's perennial dead-admin problem) = owner issues a successor attestation before disappearing; an owner-less board is still readable/replicable, just unmoderated — and forkable (anyone can root a new Board citing this one as predecessor). |
| `Theme`                        | Theme author (anyone)                                 | see §4                                                                                                                                                                 | Referenced by the Board root; forkable independently of the board.                                                                                                                                                                                                                                                                                                                                                           |
| `ModerationAction`             | Moderator-attested ID                                 | board ref, target object hash, action (`hide`, `label`, `pin`, `lock`, `ban-from-view`), reason, timestamp                                                             | Append-only signed log; see §5.                                                                                                                                                                                                                                                                                                                                                                                              |
| `BoardFork` / predecessor link | New board's creator                                   | predecessor board ref, reason                                                                                                                                          | Succession games and community forks (cf. #2053 succession forks for saves) use the same pattern at board level.                                                                                                                                                                                                                                                                                                             |

**Identity of a board across hosts:** board ID + root object hash. Two hosts holding the
same signed object set hold _the same board_; there is no "primary server" copy in the
model (see §6).

---

## 2. Registration-free identity via Iroh ID

- There is no sign-up, no username table, no password, no email. Your Iroh ID (public
  key) is the account; signing a post _is_ logging in. A first post from a new ID is
  indistinguishable in mechanism from a post by a ten-year member — reputation
  differences live in attestations and trust filters, not in the identity layer.
- **Display names are claims, not registrations.** A profile object (signed by the ID)
  claims a display name/avatar. Name collisions are possible by design (two IDs may both
  claim "Drew"); clients disambiguate with the ID fingerprint/short-hash, exactly as
  key-based systems (and phpBB-era impersonation problems) require. A board may
  additionally show role badges from `RoleAttestation`s, which _are_ unforgeable. (Tree
  note: `petname.rs` already establishes the local-only display-name convention —
  private aliases, never replicated.)
- **Key loss = identity loss**, same as the rest of ID; boards add no recovery backdoor.
  Mitigation available in-model: an ID may pre-publish a signed "successor ID"
  attestation (key rotation), and boards' role checks accept the successor chain.
- **Sybil reality, stated plainly:** free identities mean free throwaway posters. The
  model does not pretend otherwise — §5's trust-filter layer, not identity scarcity, is
  the spam defence. Boards that need a cost can set view policy (e.g. posts from IDs
  younger than the board's first-seen record, or without any attestation, render only in
  a probationary view).

---

## 3. What replaces phpBB's database

phpBB = central DB + PHP rendering + admin panel. The ID equivalent: graph store + sync
(existing ID substrate), rendering in the ID web/TUI clients, and "admin panel" = a
client view over the board's own signed objects (root versions, rules versions, role
attestations, moderation log). No board-specific server code is required for a basic
board; a host is a generic ID host that happens to replicate this board's object set.

---

## 4. Theme format + sharing (concrete)

A theme is a signed object, not a zip of templates:

```json
{
  "id": "<hash>",
  "author": "<ID>",
  "name": "...",
  "version": 1,
  "parent_theme": "<hash | null>",
  "tokens": {
    "colors": { "bg": "...", "surface": "...", "text": "...", "accent": "...", "link": "..." },
    "fonts": { "display": "...", "body": "...", "mono": "..." },
    "radius": "...",
    "density": "...",
    "avatar_shape": "..."
  },
  "layout": "phpbb-classic | compact | threaded",
  "flags": { "show_signatures": true, "show_avatars": true },
  "assets": [{ "ref": "<hash>", "slot": "logo | banner | background | emoji-set" }],
  "css_extra_ref": "<hash | null>"
}
```

Rules that make themes copyable/shareable _by construction_:

1. **Apply = reference.** A board uses a theme by putting its hash in `Board.theme_ref`.
   "Copying" another board's theme is one click and zero file movement — you reference
   the same object.
2. **Modify = fork.** Changing any token mints a new Theme object with `parent_theme`
   set. The original author cannot be edited out of the lineage; themes acquire a visible
   family tree (which is the phpBB-styles-gallery culture, but with provenance).
3. **No executable templates in v1.** phpBB themes were PHP/HTML templates — arbitrary
   code per board is a security and portability hole (and breaks the TUI/SSH renderers,
   which have no HTML). V1 themes are tokens + layout enum + assets, renderable
   identically on web, TUI, and SSH. `css_extra` is the single escape hatch,
   web-renderer-only, sandboxed (no external URLs except declared font/asset refs), and
   boards on non-web clients ignore it. Custom HTML templates are an explicit non-goal
   for v1 and a named open question for the synthesis (#2054). (Tree note: the existing
   `web/` "themes" are CRT terminal colour schemes — unrelated; board themes are a new
   object kind, not an extension of those.)
4. **Theme packs:** a Theme may cite a `ThemeCollection` object (curated list) — the
   "share my board's whole look" case is just sharing the board root, which already
   carries theme + rules refs.

---

## 5. Moderation under accept-all / trust-filter (concrete)

Inherited axiom (#2030, LIST 7): **servers accept ALL validly signed objects; spam is
defeated by filtering views, not by rejecting storage.** Boards inherit this wholesale —
a board host does not get to silently drop validly signed posts, and a moderator cannot
reach into other hosts and erase bytes. What moderation _is_ in this model:

1. **Three view layers, composed in order:**
   - _Personal filter_ — the reader's own allowed-writer set / blocks (#2030, LIST 8
     controls). Always wins for that reader.
   - _Board view_ — the default rendering a board offers: root + rules + role
     attestations + the moderation log applied on top of the raw object set. `hide`
     removes a post from the board view (the object still exists and is visible in
     "raw/unfiltered" mode with the moderation action shown — moderation is transparent
     and auditable, not memory-holing).
   - _Host view_ — a public host/relay may apply its own legal/safety policy to what
     _it serves_ (e.g. not serving illegal blobs), which is a statement about that host,
     not about the board; other hosts may serve the same board differently. This is the
     honest federation answer: authority over a board is portable, liability stays with
     the server operator.
2. **Actions are objects.** Every moderation act is a signed `ModerationAction` in an
   append-only log citing the target hash and the actor's role attestation. Undo = a
   superseding action. Anyone can audit who hid what and why; a moderator who goes rogue
   can be _distrusted at the view layer_ (clients drop their attestations) and their
   actions stop applying — no server admin required.
3. **Bans are view-bans.** `ban-from-view` = board view stops rendering that ID's
   objects. The banned ID can keep signing; nobody is obliged to look. Evasion via new
   IDs is bounded by board view policy (probationary rendering, attestation requirements
   for e.g. posting media or links).
4. **Rate/volume abuse** is not a moderation question at all — it is the LIST 8
   substrate (per-ID/per-board budgets, rate limits), applied by hosts and relays before
   objects ever reach a board view.
5. **What this model refuses to promise:** global deletion, retroactive history
   rewriting, and a single canonical "what the board says" independent of a stated view.
   The synthesis (#2054) should ratify or amend the three-view composition explicitly —
   it is the board model's most consequential decision.

---

## 6. Federation between boards on different servers

- A board federates by **replication, not by protocol gateway**: hosts that choose to
  carry a board sync its signed object set (Iroh/ID sync natively; the Workers edition
  via the HTTP/WS signed-object subset per the Workers-fit investigation — no QUIC in an
  isolate, so Workers hosts are partial peers: they can serve and accept pushes, they
  cannot do native gossip).
- **Discovery/listing:** a host publishes a signed `BoardListing` (board ref + host
  contact) — directories are just indexes over listings; being listed is opt-in per board
  rules (`federation policy` field).
- **Cross-board identity is free:** the same Iroh ID posts on boards hosted anywhere;
  role attestations are per-board objects, so moderator on board A means nothing on
  board B unless B's owner says so.
- **Conflicts:** concurrent versions (root, rules, post edits) resolve by the graph's
  version/supersede links, not by host clocks; a host that lags shows a stale view plus
  a staleness indicator, never a silently different board.
- **Exit/portability:** because authority is signatures, a board can move hosts by
  replicating elsewhere and re-listing; the old host keeping a copy is a feature
  (archive) unless the board's federation policy says listed-only, which is enforceable
  only socially/by delisting — stated here so the synthesis doesn't overpromise takedown.

---

## 7. Media posts

Bodies are markdown; pictures/audio/video are `Attachment` blob refs (§1). Playable/
embedded content (HTML5 mini-games, async challenges, succession saves) is deliberately
_not_ modelled here beyond one hook: a Post may carry a typed `embed_ref` to a signed
embeddable object — #2053 owns that object's model, and #2054 merges the two. Defining
the hook now keeps the boards model from needing a breaking change when playable posts
land.

---

## 8. Open questions handed to synthesis #2054

1. Ratify the three-layer view composition (§5.1) and the transparency rule (hidden ≠
   erased, moderation log public) — or carve out a sealed-action variant for e.g.
   illegal content reports.
2. Custom HTML themes: ever, or is tokens+layout the permanent ceiling? (Security/
   portability vs the phpBB nostalgia for total restyle.)
3. Probationary/new-ID view policy defaults: ship a recommended default rules object, or
   leave every board to invent one?
4. Private/closed boards: this sketch models public boards. A closed board needs
   encrypted object sets (group keys from LIST 9/10 work?) — in scope for v1 or a
   follow-on?
5. Board-root succession when the owner ID is lost with no successor attestation:
   fork-only (as sketched) vs a moderator-quorum recovery attestation.

---

## 9. Follow-on routing

- **Synthesis:** #2054 (ID boards v1) — this investigation + #2053 feed it by link; it
  stays gated until #2053 lands.
- **Follow-up issues from this write** are filed as candidates for the #2054 synthesis
  to route, not as implementation go-aheads: per program rules, nothing here is a PoC or
  a permanent-feature decision, and any buildable follow-on is commissioned by/after
  the #2054 synthesis review round.
- **Not a PoC:** no code was built; the model above is the deliverable.

## Verification note

Checked at `origin/main ad18a0bf`: issue #2052 open with no owning or duplicate PR;
the only `board` matches in `pkgs/id/src` are tic-tac-toe game boards (`sandbox.rs`,
`world_fixture.rs`) — no forum-board code exists; `web/` themes are terminal colour
schemes, unrelated to §4's Theme objects; #2030's trust model taken from its issue text
(its doc PR #2801 was still open at write time); #2053 and #2054 open as the scoped
partners. Prior sketch: task 3357's comment of 2026-10-29, recorded here as the
wave-pattern doc with no change to the model.
