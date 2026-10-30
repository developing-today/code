# Design: writer-set objects, view projection, and the discovery query for `pkgs/id`

- **Date:** 2026-10-30
- **Status:** **design spec only — no permanent feature built from it.** Per the program rule
  ([epic #2056](https://github.com/developing-today/code/issues/2056)), a spec is not a build
  order: nothing here is implementable on top of the unreviewed
  [#2030](https://github.com/developing-today/code/issues/2030) investigation until the review
  round accepts it. Implementation slices are named in §7 and must be filed as their own issues.
- **Issue:** [#2238](https://github.com/developing-today/code/issues/2238). **Parent
  investigation:** [#2030](https://github.com/developing-today/code/issues/2030) (result posted
  on that issue 2026-10-29, queue task 3346).
- **Consumers / cross-refs:** [#2028](https://github.com/developing-today/code/issues/2028) (ID
  core design v1 — holds the questions this spec cannot settle, §6),
  [#2031](https://github.com/developing-today/code/issues/2031) (did-it tracker, the motivating
  app), [#2032](https://github.com/developing-today/code/issues/2032) (abuse-control budgets —
  owns every number this spec deliberately leaves out), [#2029](https://github.com/developing-today/code/issues/2029)
  (tracker data model — the event kinds that get projected).
- **Method:** every substrate claim below was read at `origin/main` `72545cb9` on 2026-10-30 and
  is cited `path:line`. Design choices are labelled **Decision** with their reasons; they are
  proposals for the review round, not settled API.

`pkgs/id/doc` and `pkgs/id/docs` are both symlinks to the repo-root `doc/` (`pkgs/id/doc ->
../../doc`), so this file lives at `doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection.md`
and is reachable as `pkgs/id/doc/…` exactly as the issue asks.

---

## 0. The rule everything follows

From #2030, verbatim in effect: **ingest is a validity gate, never an authority gate; every
object that verifies is stored; a view counts only its writer set.**

This spec exists to make the third clause buildable. It is not allowed to trade away any of
these, and each is checked against the substrate in §1:

1. **Stored ≠ counted.** Storage accepts anything validly signed, attributed to its signer.
2. **Counting is per view, decided by the view owner, and reversible.**
3. **Every computed view names its inputs** — *(writer-set version, frontier)* — so two people
   comparing "the same number" can see whether they compared the same thing (§2.4).
4. **Spam is made ineffective, not free.** The cost of uncounted data is paid in storage and
   retention policy (§5), never by quietly re-introducing an authority gate at ingest.
5. **No silent history rewriting.** Set changes are signed, versioned, reviewable events.

---

## 1. Verified current state — what the substrate already gives us

The #2030 verdict ("this is not new architecture") holds at the code level. Five of the seven
pieces this spec needs already exist in some form; the two that are genuinely missing are named
in §1.7.

### 1.1 The stored object and its author

An event is a signed header plus a separately stored body
([`chain.rs:262`](../../pkgs/id/src/chain.rs)): `id` is BLAKE3 over the bytes with the signature
included, `bytes` is the encoded header (what is stored and sent), `header` is the decoded
header. The author is recovered from the header's verifying key —
`author_of(event) = *event.header.verifying_key.as_bytes()`
([`event_store/resolve.rs:60-62`](../../pkgs/id/src/event_store/resolve.rs)) — and `AuthorKey` is
that 32-byte value. There is no separate `AuthorId`, endpoint id, or account handle in the
counting path: **the writer identity a writer set matches on is the 32-byte Ed25519 verifying
key.**

The signed extension is the extension point a new object kind uses
([`chain.rs:97-118`](../../pkgs/id/src/chain.rs)):

```rust
pub struct ChainExt {
    pub kind: String,            // free string, ≤ MAX_KIND_BYTES = 64
    pub group: Option<Hash>,     // routeable without decrypting the payload
    pub after: Vec<Hash>,        // causal frontier, ≤ MAX_AFTER = 32
    pub tag: Option<TagOp>,
    pub document: Option<DocumentRef>,
    pub removes: Option<Hash>,
}
```

Two hard limits constrain any writer-set format built on this envelope
([`chain.rs:30-50`](../../pkgs/id/src/chain.rs)): `MAX_KIND_BYTES = 64`, `MAX_AFTER = 32`, and the
CBOR decoder's declared limits — text ≤ 512 bytes, arrays ≤ 512 elements (`chain.rs:41-42`). **A
writer set of any real size cannot live in the signed header.** That single fact drives Decision
D1 in §2.

### 1.2 Stored vs counted already exists — as `events` vs `applied`

The store's tables ([`event_store/mod.rs:86-112`](../../pkgs/id/src/event_store/mod.rs)) already
separate "present" from "in force":

| Table | Key → value | Meaning |
|---|---|---|
| `events` | `author‖seq` (40 bytes, `POSITION_LEN`) → bytes | **every** accepted event, including ones past a fork point |
| `hashes` | event ID → position | ID lookup |
| `chains` | author → next seq, head hash, frozen flag | the author's **applied prefix** |
| `forks` | fork-point position → the two conflicting headers | fork proofs |
| `applied` | event ID → — | the resolved in-force set |
| `groups` | group hash → positions (`MultimapTable`) | per-group membership index |
| `actions` | event ID → membership action | group ops, for resolution |

`EventStore::event_applied(id) -> bool` ([`event_store/mod.rs:444-450`](../../pkgs/id/src/event_store/mod.rs))
is the existing "does this count?" predicate. The resolution that fills `applied` is a pure
function of the event set plus fork proofs, so it does not depend on arrival order
([`event_store/resolve.rs:1-20`](../../pkgs/id/src/event_store/resolve.rs)).

Its three rules matter because they are *authority-free* for content: fork exclusion, group-op
effectiveness, and "any other event applies" — which says outright that a group with no create
has no authority to lose, so **its content always applies**, and "content is kept on removal"
(`resolve.rs:1-20`). That is #2030's "the store keeps it either way" in code.

**Consequence for this spec:** `applied` is a *store-wide* predicate, not a per-view one. It
answers "may this stand?" — never "does this count for Alice's metric?" The writer-set layer sits
strictly above it and must not be confused with it (Decision D2).

### 1.3 Claims are already author-attributed, and an author index already exists

A live claim is `Claim { id, author, subject, name, value }`
([`tag_claims.rs:80-91`](../../pkgs/id/src/tag_claims.rs)); `author` is the same 32-byte key.
`EventStore::tag_claims()` folds the applied `tag.set`/`tag.clear` events into `Vec<Claim>`
([`event_store/claims.rs:39-60`](../../pkgs/id/src/event_store/claims.rs)).

Critically, `TagIndex` is built **three times over**, and one of the three is already the
author-indexed projection this spec's §3 requires
([`tag_index.rs:1-21`](../../pkgs/id/src/tag_index.rs)):

- α: `(subject, name, value, claim)` — tags of X
- Ω: `(value, name, subject, claim)` — subjects with value V
- **Author: `(author, name, value, subject, claim)`** — "the claims by A"

and `search::Term::Author(AuthorKey)` already queries it
([`search.rs:46-60`](../../pkgs/id/src/search.rs),
[`search.rs:124-131`](../../pkgs/id/src/search.rs)). This is decision-grade evidence: **the
author→objects index is not a new structure to invent; it is a shape the codebase already
builds and throws away on each call.** Today the index is rebuilt per query and not persisted
(`tag_index.rs:11-12,16`); §3 turns that into a stated cost model rather than a new design.

### 1.4 Two prior-art predecessors this spec must not be confused with

There are already two "who is allowed" mechanisms and one per-viewer projection in the tree. None
of them is a writer set, but each is close enough that a reader will ask, so the difference is
stated here rather than discovered later.

**`access::AccessPolicy` is a server-side write gate, not a view.** It is a *flat allowlist* of
`EndpointId`s, applied to mutating operations (put/delete/rename/copy/tag writes/blob pushes),
with `--open-writes` disabling it
([`access.rs:1-24`](../../pkgs/id/src/access.rs)). It keys on **`EndpointId`, not the event
author's verifying key** — a different identifier — it is set by the *server operator* rather than
a view owner, and its module doc says outright it is "deliberately a flat allowlist, not a
capability system" and is superseded by the `doc/` platform design (`access.rs:20-24`). A writer
set is the opposite on all three axes: keyed on the author, owned by the view owner, and
per-view. **They must not be merged**; `AccessPolicy` answers "may this peer mutate my node",
which is orthogonal to "does this author's claim count in my metric".

**`world::project_event_for` is the only per-viewer event projection that exists.** It takes a
`WorldScopes` and returns `Option<WorldEvent>`, dropping chat without `CHAT`/`HISTORY` and zeroing
an install seed ([`world/mod.rs:954-980`](../../pkgs/id/src/world/mod.rs)). It is a genuine
predecessor in *shape* — one function every path routes through, so the rule cannot be bypassed
(`world/mod.rs:956-957`) — and that is the discipline §4.3 copies. But it projects by **scope
capability**, not by **author set**: it never asks who wrote the event. The writer-set projection
is the author-keyed analogue of it.

Note that no event-store analogue exists: grep for `view|metric|projection|aggregate` in
`event_store/` returns only "symmetric **diff**erence" false positives, and grep for `writer` in
`event_store/`, `chain.rs` and `tag_claims.rs` returns **zero hits**.

### 1.5 Groups are signed, versioned by causal position, and nest one level today

Group ops are events (`Body::{Create, Join, Policy, Member, Delegate, Cross, Veto, Device}`,
[`groups/mod.rs:99-128`](../../pkgs/id/src/groups/mod.rs)), state is an order-independent
deterministic fold ([`groups/fold.rs:3-11`](../../pkgs/id/src/groups/fold.rs),
`fold(events) -> Result<Projection, FoldError>` at `fold.rs:350`), and membership roles are
ordered `Pull < Read < Write < Manage < Admin`
([`groups/mod.rs:64-75`](../../pkgs/id/src/groups/mod.rs)).

Three facts the writer-set design depends on:

1. **Nesting is representable but discarded.** `GroupMember::Group(id)` is encoded
   ([`membership/body.rs:182-191`](../../pkgs/id/src/membership/body.rs)) and decoded, but the
   projection flattens it: `AuthId::Group(_) => None`
   ([`groups/fold.rs:1381`](../../pkgs/id/src/groups/fold.rs), also
   [`event_store/resolve.rs:282`](../../pkgs/id/src/event_store/resolve.rs)). So a group
   reference in a writer set resolves to a flat set of keys today, which is exactly the
   one-level semantics Decision D4 wants — it must be *stated*, not merely inherited by accident.
2. **The depth limit is in the dependency.** `MAX_NESTED_DEPTH = 1000` in p2panda-auth
   (`p2panda-auth/src/group/crdt/mod.rs:26`); `pkgs/id` itself has no depth constant (grep over
   `groups/` for `depth|nest|MAX_` finds none). A 1000-deep transitive expansion is not a
   writer-set semantics.
3. **Point-in-time queries exist.** `is_member_at(author, group, after)` and
   `access_at_position(after, group, member)`
   ([`membership/layer.rs:338-364`](../../pkgs/id/src/membership/layer.rs)) take a causal
   frontier and resolve membership at that position; `members(group)` is the current-state
   snapshot (`layer.rs:287-289`). This is what lets a view name *which* set version it used.

### 1.6 There is no exclusion entry, and no `public` sentinel in membership

Grep over `groups/` and `membership/` for `exclude|deny|block`: nothing. The nearest
exclusion-shaped things are all about the author's own rights, not a viewer's exception:

- `Body::Veto { grant }` voids a grant from a causal position on
  ([`groups/mod.rs:121-125`](../../pkgs/id/src/groups/mod.rs));
- `MemberOp::Remove { member, successor }` removes a member
  ([`groups/mod.rs:175-183`](../../pkgs/id/src/groups/mod.rs));
- `Refusal::NotActive / NotAMember / AlreadyMember`
  ([`groups/fold.rs:51-61`](../../pkgs/id/src/groups/fold.rs)).

**Absent: a view-level exclusion that removes one individual admitted via a group reference
without leaving the group.** #2030 abuse case 6 requires it (§4). `Join::Open`
([`groups/policy.rs:21-22`](../../pkgs/id/src/groups/policy.rs)) is the closest thing to
`public`, and it is an admission policy, not a writer-set sentinel — the two must not be
conflated (Decision D5).

### 1.7 Genuinely missing (this is the spec's actual workload)

| Piece | Status | Evidence |
|---|---|---|
| Writer-set **object** (signed, versioned, owned) | **absent** — no `writer`/`audience`/`reader_set` type outside prose | grep `writer\|audience\|reader_set\|viewer_set` over `groups/`: prose only (`policy.rs:170`) |
| **Per-view** projection (counted vs uncounted for *this* owner) | **absent** — `applied` is store-wide | §1.2 |
| View-level **exclusion entry** | **absent** | §1.6 |
| **`public` sentinel** for a metric | **absent** | §1.6 |
| **Discovery query** returning counted + uncounted with why-not-counted | **absent** — `search` returns live claims only, undifferentiated | [`search.rs:1-20`](../../pkgs/id/src/search.rs) |
| **Retention tiering** (hot counted / cold uncounted, expiry, pin) | **absent** for events; only blob access control exists | grep `retention\|expiry\|quota` in `src`: waiting-pool age caps and auth-expiry only; [`blob_gate.rs:1-15`](../../pkgs/id/src/blob_gate.rs) is access control, not retention |
| A **generic signer** for a new object type | **absent** — only `chain::sign` is body-opaque; artifacts and invites are fixed layouts | [`chain.rs:282`](../../pkgs/id/src/chain.rs), [`artifact.rs:226`](../../pkgs/id/src/artifact.rs), [`invites/event.rs:17`](../../pkgs/id/src/invites/event.rs) |
| **Persisted, kind-generic** author index | **absent** — `TagIndex.by_author` covers tags only and is rebuilt per call | [`tag_index.rs:11-12,16`](../../pkgs/id/src/tag_index.rs), [`event_store/claims.rs:50-53`](../../pkgs/id/src/event_store/claims.rs) |

---

## 2. Writer-set object format

### D1 — A writer set is its own signed event kind, with the entry list in the body

**Decision:** `kind = "idset.writer"`, carrying `{ set_id, version, owner, scope?, entries: Vec<Entry>, excludes: Vec<Entry> }`
in the **event body** (CBOR, BLAKE3-committed by the header), never in `ChainExt`.

Reasons:

- The header cannot hold it: text ≤ 512 bytes and arrays ≤ 512 elements (`chain.rs:41-42`), and
  `MAX_AFTER = 32`. A writer set for a dog-care household is fine; one for a public figure is
  not. Putting it in the body keeps the signed-header format — and therefore every existing
  verifier — unchanged, which is the same reasoning the tracker design used for its kinds
  ([`didit` §D0](../2026-10-29T13-04-50Z_investigation_didit_tracker_data_model/2026-10-29T13-04-50Z_investigation_didit_tracker_data_model.md)).
- Reusing `chain::sign`/`chain::decode` means the existing `verify_strict` path
  ([`chain.rs:11-16`](../../pkgs/id/src/chain.rs)) checks the set object with no new crypto.
- A distinct kind keeps writer sets from colliding with `tag.*`, `document.*`, `device.*` and
  the tracker's `didit.*`.

The body must be canonically encoded and length-prefixed the way `DocumentRef` content is, so a
set object is content-addressed and dedupes like any other body.

### D2 — `version` is the owner's chain sequence, not a field the owner invents

**Decision:** `version` is the `seq_num` of the publishing event in the owner's own chain
(`Header.seq_num`, `p2panda-core/src/operation/header.rs:63`), and `set_id` is a stable derived
ID (BLAKE3 over `(owner, purpose)`), so successive versions of the *same* set form a contiguous
prefix of one author's chain.

Reasons: an owner-invented counter can fork or repeat; the chain already enforces "seq 0 has no
backlink, seq > 0 has one" ([`chain.rs:290-294`](../../pkgs/id/src/chain.rs)) and the store
already keeps `chains: author → next seq, head, frozen` (`event_store/mod.rs:90-91`). A view can
therefore name a set version as `(set_id, seq, head_hash)` and anyone can verify it is an
ancestor of the head they hold — no new versioning machinery.

**Rejected:** a mutable "current set" row. There is no mutation path in the substrate
(`event_store/mod.rs:1-30`, append-only) and a mutable set would make §2.4's "which set version
produced this number" unanswerable.

### D3 — Entries are a union of three shapes

```rust
enum Entry {
    Key(AuthorKey),                    // 32-byte verifying key, explicit
    Group { id: Hash, role_min: Role },// one-level reference into groups/
    Public,                            // the everyone-sentinel
}
```

- **`Key`** matches `author_of(event)` directly (§1.1).
- **`Group`** resolves through `membership::Members::members(group)`
  ([`membership/layer.rs:287-289`](../../pkgs/id/src/membership/layer.rs)) and filters by
  `role_min` against the ordered roles (`groups/mod.rs:64-75`). Resolution is **one level deep**
  (D4).
- **`Public`** matches every author. It is deliberately a *distinct entry*, not "the absence of a
  set", so "this metric is public" is a signed, visible choice rather than a default that happens
  when someone forgets to configure anything.

`role_min` is included because `Read`/`Write`/`Manage` already distinguish who may read content
from who may write it (`groups/mod.rs:64-75`); a writer set that ignored roles would count
`Pull`-only peers who cannot read the payload — an incoherent metric.

### D4 — Group references expand exactly one level, and the expansion is pinned

**Decision:** a `Group` entry contributes the members at the set's stated frontier, flattened to
keys, and **does not recurse**. A group that itself contains a group contributes only its direct
key members.

Reasons: today the code flattens one level by dropping `AuthId::Group(_)`
([`groups/fold.rs:1381`](../../pkgs/id/src/groups/fold.rs)) — so one level is already the de-facto
behaviour, and stating it makes it a contract instead of an accident. The dependency's
`MAX_NESTED_DEPTH = 1000` (`p2panda-auth/src/group/crdt/mod.rs:26`) is the opposite of a
writer-set semantics: it would let one careless admin three groups away silently change who
counts in my view. A view that wants the nested group's members names that group as its own
entry, explicitly.

**Pinning:** the set object records the frontier it was expanded at (the publishing event's
`after`, ≤ `MAX_AFTER = 32`), so later group membership changes do **not** silently change an
old set version. Re-expansion is a new set version. Without pinning, "set version 7" would mean
different things on two devices, which breaks §2.4.

### D5 — `Public` is not `Join::Open`

`Join::Open` ([`groups/policy.rs:21-22`](../../pkgs/id/src/groups/policy.rs)) governs who may
*join a group*; `Entry::Public` governs who may *count in a view*. They are different questions
with different blast radii, and the spec forbids reusing one for the other. In particular, a
`Public` writer set is never a default for a metric: the #2030 investigation flagged that a
public *metric* is an invitation to game it, and left that question to #2028 (§6, OQ-1).

### D6 — Exclusions are entries too, and they win

```rust
struct Excludes(Vec<Entry>);  // Key | Group{..} | Public
```

**Decision:** `excludes` is evaluated after `entries`, and any author matched by an exclusion
does not count even if an entry (explicitly or via a group) admitted them.

Reason: this is the only mechanism that answers #2030 abuse case 6 — "my view references a
group; its admin adds a spammer" — without forcing me to leave the group. `Body::Veto` looks
similar but is not it: `Veto` voids a *grant* from a position on
([`groups/mod.rs:121-125`](../../pkgs/id/src/groups/mod.rs)) and requires authority in the group.
An exclusion requires only that I own the view.

`Excludes(Public)` is deliberate, but it means exactly one thing: **count nobody**. Because
`Public` matches every author and exclusions win over every admission, a set with
`entries = [Public], excludes = [Public]` is empty by construction. It is the emergency "stop
counting everything in this view" switch, not a moderation tool.

The moderated public feed Codex-style readers will expect — everyone except a few blocked
authors — is the opposite shape and is spelled with the two fields doing different jobs:
`entries = [Public]`, `excludes = [Key(a), Group(g)]`. That is "everyone, minus these", and it
needs no special sentinel: the entries admit the crowd and the exclusions carve out of it. This
distinction is stated explicitly because the two configurations differ by one entry and produce
opposite results.

---

## 3. Set-change semantics

### 3.1 Prospective by default; "include history" is explicit

**Decision:** adding an entry counts that author's *future* claims by default. Counting their
already-stored claims requires the set object to carry `include_history: true` for that entry.

Reasons: the substrate has no time in the envelope — `ChainExt` has no timestamp, and time
appears only as the local arrival clock on waiting rows
([`event_store/waiting_rows.rs:39-40`](../../pkgs/id/src/event_store/waiting_rows.rs)). So
"history" cannot mean "before a wall-clock instant"; it must mean **"claims in the causal past of
this set version"**, which is exactly what `after` expresses. `include_history: true` therefore
counts that author's claims **in** the set version's causal past **as well as** everything
prospective — a precise, order-independent definition that every device computes identically.
Without the flag, only claims outside that past (i.e. written after the set version) count, which
is the "future claims" default.

This is also the honest default for a *new* friend: you probably do want their past dog walks,
but you must say so, because the same switch applied to an attacker silently imports their whole
backlog into your metric.

### 3.2 Removal stops future counting and never rewrites the past

**Decision:** removing an entry stops counting from that set version on. Claims already counted
under earlier versions stay counted *for those versions*. Erasing them requires an explicit
`rescind` (§3.3).

Reasons: this is the only rule under which two viewers can compare numbers at all. If removal
retroactively un-counted history, then "Alice walked the dog 40 times in October" would change
answer depending on when you asked, with no event to point at. The substrate already refuses
mutation (`event_store/mod.rs:1-30`), so retroactive erasure would have to be a projection lie
rather than a data change — precisely the silent rewrite §0.5 forbids.

### 3.3 `rescind` is an explicit, visible set-version event

**Decision:** `kind = "idset.rescind"`, body `{ set_id, from_version, target: Entry, through: Option<Hash> }`.
It excludes the target's claims from `from_version`'s position, optionally only up to a named
event (`through`), and it is itself a new set version.

Reasons: #2030 abuse case 7 ("removed writer disputes history") needs a real answer for both
directions — the walker says "my past walks must still count", I say "they must be erased". §3.2
gives the walker their answer by default; `rescind` gives me mine, as a signed act that shows in
the set's version history instead of happening invisibly. It is a separate kind, not a flag on
the set, so its authorship, position and reason are all auditable.

### 3.4 Every computed view names *(set version, frontier)*

**Decision:** a projected result carries `{ set_id, set_seq, set_head, frontier }` — never just a
number. The frontier is the view's own `after` at evaluation
([`chain.rs:97-118`](../../pkgs/id/src/chain.rs)).

Reasons: the group layer already made this choice — `is_member_at`/`access_at_position` take an
`after` and resolve at that position ([`membership/layer.rs:338-364`](../../pkgs/id/src/membership/layer.rs))
— so the pattern is established. A bare count that cannot name its inputs is not comparable, and
#2030's whole point is that comparability is what makes "the same metric" meaningful across
people who trust different sets.

---

## 4. View projection

### 4.1 Counted vs uncounted, computed per view

**Decision:** a **view** is `{ view_id, owner, set: (set_id, set_seq), subject_scope, kind_scope, exceptions }`.
Projection is **admission-history-aware, not a single-snapshot membership test**:

```
admitted(view, claim)  ⇔  ∃ v ∈ versions(view.set_id), v.seq ≤ view.set_seq,
                            author_of(claim) ∈ expand(v)
                          ∧ ¬rescinded_at_or_before(view, author_of(claim), claim, v)
counted(view, claim)   ⇔  applied(claim)
                          ∧ (author_of(claim) ∈ expand(view.set) ∨ admitted(view, claim))
                          ∧ ¬rescinded(view, claim)
                        ∨ claim ∈ view.exceptions        // one-off "count this claim"
uncounted(view, claim) ⇔  applied(claim) ∧ ¬counted(view, claim)   // stored, attributed, zero effect
```

The `∨ admitted(...)` clause is load-bearing and was missing from the first draft: §3.2 says a
removal stops *future* counting and never rewrites the past, so a claim Alice wrote while set
version 1 admitted her must stay counted under version 2 — where she is gone — unless there is an
explicit `rescind`. A plain `author ∈ expand(current set)` test would silently un-count her whole
history on removal, which is precisely the retroactive rewrite §3.2 forbids and abuse case 7
promises not to do.

So projection must fold the **set's version history**, not one snapshot:

- `admitted` walks the set versions in `(set_id)` order up to `view.set_seq` — they are a
  contiguous prefix of the owner's chain (§D2), so this is a bounded, ordered walk, not a scan of
  all chains.
- An author admitted at version *v* is counted for claims whose causal position is at or after
  *v*'s frontier, and un-counted from the version that removed them onward — unless a `rescind`
  (§3.3) explicitly reaches back.
- `expand(v)` is evaluated **at *v*'s pinned frontier**, never at head (see §4.5).

The cost model in §4.2 still holds, because the number of *versions* is the owner's own history
(small, and cached), while the number of *claims* is what §4.2's author-index bounds.

Note the two predicates use different bases and that is the point: `applied` is the store-wide
membership resolution (§1.2), `counted` is the view's writer set. A claim can be `applied` and
`uncounted` — that is the normal case for a stranger's claim.

### 4.2 Cost is proportional to the writer set, not the store

**Decision:** projection reads an **author-keyed event index** restricted to the expanded writer
set, rather than scanning the store. The precedent for the *shape* is `TagIndex`'s author key; the
precedent for the *scope* is the `events` table's `author‖seq` ordering. Neither is sufficient as
it stands (see "two gaps" below), so this slice is a **generic author-event index**, not a reuse
of `TagIndex`.

Reasons: `tag_claims()` opens `EVENTS` and walks **every row**, then decodes each one
([`event_store/claims.rs:44-60`](../../pkgs/id/src/event_store/claims.rs)) — cost is O(store). The
author key `(author, name, value, subject, claim)` is already built
([`tag_index.rs:9-12,16`](../../pkgs/id/src/tag_index.rs),
`by_author: BTreeMap<Vec<u8>, Entry>` at `tag_index.rs:46`) and `search::Term::Author` already
queries it ([`search.rs:124-131`](../../pkgs/id/src/search.rs),
`TagIndex::author(author)` at `tag_index.rs:161`). A view with *n* writers therefore costs
O(Σ author rows) — proportional to writer-set size, not to global spam volume. That is the
mechanism #2030 named as the defence against a Sybil flood becoming a compute DoS on every view.

**And the store already has the locality to build that cheaply.** The `events` table is keyed
`author‖seq` — `POSITION_LEN = 40`, "a position key is the author's 32 bytes followed by the
sequence number in big-endian, so one author's events sort in sequence order"
([`event_store/mod.rs:24-25,114-115`](../../pkgs/id/src/event_store/mod.rs)). So one author's
events are a **contiguous key range**, and the code already ranges over it for exactly this
purpose in `applied_events(author, after, until, limit)`
([`event_store/mod.rs:1298-1300`](../../pkgs/id/src/event_store/mod.rs)) and `lowest_proof`
(`mod.rs:1257-1259`).

**Two gaps make "just persist `TagIndex`" wrong, and the first draft of this spec made that
mistake:**

1. **`TagIndex` covers tags only.** `tag_claims()` skips every non-tag event —
   `if !is_tag_kind(kind) || !applied(id) { continue }`
   ([`event_store/claims.rs:50-53`](../../pkgs/id/src/event_store/claims.rs)) — and `tag_index()`
   builds exclusively from that result (`claims.rs:84`). The did-it tracker's walks are
   `didit.record` events with CBOR bodies, a different kind entirely
   ([tracker §3.2](../2026-10-29T13-04-50Z_investigation_didit_tracker_data_model/2026-10-29T13-04-50Z_investigation_didit_tracker_data_model.md)).
   A projection built on `TagIndex` would therefore return **an empty tracker** — it would work
   for tags and silently return nothing for the first real consumer.
2. **`TagIndex` is rebuilt per call** (`tag_index.rs:11-12,16`), so per-view projection would pay
   the O(store) scan on every render.

**Decision (corrected):** the index this spec specifies is `AuthorEvents: (author, seq) →
(event_id, kind, subject?)`, built by ranging the existing `events` table per author in the
writer set and filtering by `kind_scope`. It is generic over kind, so it serves `tag.*`,
`didit.*` and any future kind; it is persisted and incrementally maintained; and where a
kind-specific projection (tags → `TagIndex.by_author`) is cheaper, that becomes an *adapter on
top* of it rather than the only source. The tracker consequently reads real `didit.record`
events, which is the test slice 7 exists to prove.

The cost claim and its test are unchanged: a view query must touch only writer-set authors, and
slice 3's test asserts *that*, not merely the answer.

### 4.3 Counted and uncounted are strictly separated in the type system

**Decision:** `Projection { counted: Vec<Hit>, uncounted: Vec<UncountedHit>, provenance: Provenance }`,
and the two collections never share a type. `UncountedHit` carries `author`, `signature_ok`
(true by construction from `chain::decode`) and `reason: WhyNotCounted`.

Reasons: #2030 abuse case 4 (confusion attack) says counted status is *computed from the set,
never displayed from claim-supplied data*. Making it two Rust types means a UI cannot render an
uncounted claim inside the counted collection without a deliberate conversion — the invariant
survives contact with a careless template.

`WhyNotCounted` is a closed enum: `AuthorNotInSet`, `RescindedSince(version)`,
`ScopeMismatch(subject|kind)`. A free-text reason would drift.

### 4.4 Exceptions are view-level and separate from set changes

**Decision:** `exceptions: Vec<Hash>` on the view counts a named claim one-off, recorded at the
view level (a `view.exception` event). It never edits the writer set.

Reasons: #2030 draws the line explicitly — *count this claim* and *add this writer* are different
actions and collapsing them is how writer sets quietly bloat. Keeping them as separate kinds is
the code-level expression of that paragraph.

---

### 4.5 Group expansion must be frontier-aware, not `members(group)`

**Decision:** `expand(v)` resolves group references **at *v*'s pinned frontier** using
`is_member_at(author, group, after)` / `access_at_position(after, group, member)`
([`membership/layer.rs:338-364`](../../pkgs/id/src/membership/layer.rs)), and **must not use
`Members::members(group)`**.

Reasons: `members(group)` is explicitly a **current-state** query — "The members of `group` and
their access, in the current state" — and takes no frontier
([`membership/layer.rs:286-289`](../../pkgs/id/src/membership/layer.rs)). Using it to expand a
pinned set version would let a group's later additions or removals silently change the expansion
of a set version that has not changed — exactly the reproducibility guarantee §D4 promises, and
exactly the group-capture surface abuse case 6 is about.

The cost difference is real and is stated rather than hidden: a frontier-aware expansion does
historical replay over the group's causal past instead of one snapshot read. For a set with *g*
group entries this is *g* replays, cached against `(group, frontier)` — the frontier is immutable,
so every cache entry is permanent. `members(group)` remains correct for *current-state* questions
(e.g. "who is in this group right now"), and the spec uses it only for those.

## 5. Discovery query, and how it stays readable under flood

### 5.1 Subject/topic-first, returning both sides

**Decision:** `discover(subject_or_terms, view) -> Discovery { counted: Vec<Hit>, uncounted: Vec<UncountedGroup>, }`
where uncounted results are **grouped by author**, with `{ author, petname?: String, claim_count, claims: Vec<UncountedHit> }`.

Reasons: the existing search is subject-first and already returns `Hit { subject, claims }`
([`search.rs:62-69`](../../pkgs/id/src/search.rs)), so the query shape is not new. What is new is
that it returns the uncounted side **at all**, which is what makes "uncounted" mean *findable*
rather than *discarded* — #2030's "uncounted must not be a bin".

Grouping by author handles the **one-key** flood: 10,000 claims from one key read as **one
cluster with a count**, not 10,000 facts. That is a real mitigation and it is a *layout* rule on
the result type so it cannot be lost in a UI.

**But it does not answer a multi-key flood, and the first draft overclaimed by implying it did.**
A Sybil flood of 10,000 *distinct* keys yields 10,000 clusters — grouping has bought nothing,
because the attacker spread across keys rather than claims. #2030 abuse case 2 describes exactly
the multi-key form ("an attacker mints 10,000 keys"). So:

- **Bounded, ordered, paginated discovery** is required, not optional: `Discovery` carries
  `total_authors` and a cursor, and a page is capped. §5.1's result type is therefore
  `Discovery { counted, uncounted_page: Vec<UncountedGroup>, total_authors, next_cursor }`.
- The page cap and any bucket-width values are **delegated to #2032** with the other budget
  numbers (§6) — this spec sets the *shape* (bounded + cursor + honest total), not the number.
- **Retained residual risk, stated plainly:** multi-key discovery flooding is **not solved** by
  this spec. Grouping and pagination keep the UI *usable* under flood, but an attacker willing to
  mint keys can still fill many pages and consume the viewer's attention. Author-clustering
  heuristics (shared timing, identical bodies, IP/relay correlation) are the plausible next
  mechanism, and are **out of scope here** — they belong with #2032/#2033's abuse controls, not
  with the projection layer.
- The §9 slice-5 test must therefore cover **both** shapes: 10k claims from 1 key (one cluster)
  **and** 10k keys with 1 claim each (10k clusters, bounded page, honest `total_authors`). The
  second is the one that fails today, so it is the test that matters.

### 5.2 `petname` is viewer-local and must never be presented as identity

`Petnames::get(viewer, key)` returns the *viewer's own private* name for a key, stored per node
in `petnames.jsonl` ([`petname.rs:257-264`](../../pkgs/id/src/petname.rs),
[`world_store.rs:387-392`](../../pkgs/id/src/world_store.rs),
[`directory_view.rs:107-108`](../../pkgs/id/src/directory_view.rs)). `Label::Alleged` and
`Label::SharedPetname` exist precisely because a name claim may be *someone else's assertion*
([`petname.rs:268-285`](../../pkgs/id/src/petname.rs)).

**Decision:** the discovery result shows `author` (the 32-byte key, rendered as a fingerprint) as
the identity and `petname` only as a viewer-local annotation labelled as such. A shared or
alleged name is rendered with its `Label` variant, never flattened into "the author's name".

### 5.3 No ambient leakage

**Decision:** uncounted claims never appear inside the counted view — no greyed rows, no "plus 3
hidden" badge that functions as an advertisement. A per-view opt-in indicator is allowed and
**default off for metrics**.

Reasons: a badge that advertises stranger claims converts discovery into a nudge, which is the
delivery mechanism spam actually wants. Discovery is a place you go.

---

## 6. Storage tiering interface

### D7 — Counted is hot/permanent; uncounted is cold with published expiry, unless pinned or counted

**Decision:** the interface is a `Tier` decision per object, evaluated by any server:

```rust
enum Tier { Hot, Cold { expires_at: RetentionWindow } }
// Hot  if counted by any view this server serves, or pinned by its owner,
//      or named by a view exception.
// Cold otherwise.
```

Reasons: #2030's storage-exhaustion answer is exactly this, and it must be an *interface* here
because the numbers are not this spec's to set. Two constraints from the code:

- **Expiry of uncounted events has no mechanism today.** Grep finds retention only for the
  waiting pool (`waiting.rs:24`, `expire` at `:353`) and auth expiry
  ([`directory_auth.rs:180`](../../pkgs/id/src/directory_auth.rs)). Blob access control
  ([`blob_gate.rs:1-15`](../../pkgs/id/src/blob_gate.rs)) decides *who may fetch*, not *how long
  it is kept* — it is not a tiering system and must not be reused as one.
- **Expiry must not break the chain.** An event's `after` and `backlink` name other events
  (`chain.rs:97-118`), and resolution refuses an `after` it cannot place
  ([`event_store/resolve.rs:68-70`](../../pkgs/id/src/event_store/resolve.rs)). So expiry of an
  uncounted event may strand a dependent that *is* counted. **Decision:** an object is never
  expired while it is referenced by a hot object's `after`, `backlink`, or `removes`; the
  reference holds it hot. This is the one place tiering needs real care and is called out as a
  review question (OQ-3).

**There is already a worked precedent for exactly that rule, and it is unwired.**
`EventStore::prune_behind_cut(cut, protected)` prunes events behind a causal cut while keeping
"every event back to the causal past of each open removal … and open frontier" and "events that
are dependencies or active backlinks of surviving events", then repairs `events`, `hashes`,
`groups`, `actions`, `applied` and the chains together
([`event_store/mod.rs:527-545`](../../pkgs/id/src/event_store/mod.rs)). Its only callers are
`tests/retention_pruning.rs` — it has **no production caller** — and the p2panda adapter's
`prune_entries_tx` still returns `Unsupported` ("pruning is an open design question",
[`store_traits.rs:463`](../../pkgs/id/src/store_traits.rs)).

That changes the shape of slice 6 in §9: the tiering work is not "invent retention", it is
"give `prune_behind_cut` a policy that distinguishes counted from uncounted, and a caller". The
reference-holding rule is already the function's stated contract; this spec only adds the
counted/uncounted dimension to `cut`/`protected`. Recorded here so the implementation does not
rebuild what exists.

**Values deliberately absent.** Window lengths, per-topic and per-author MB caps, quota refusal
codes and `Retry-After`-equivalent framing all belong to
[#2032](https://github.com/developing-today/code/issues/2032)/[#2033](https://github.com/developing-today/code/issues/2033)
and are set once, together. This spec defines only the seam. Inventing a "30 days" here would
create the second source of truth the issue's scope explicitly forbids.

**Statement, not enforcement:** a tiering refusal says "no room" and never "not allowed". The
object stays valid and storable elsewhere — that is the difference between a resource policy and
the authority gate §0 forbids.

---

## 7. Abuse-case trace (#2030 cases 1–7)

Each case gets: the mechanism in this spec, or an explicitly accepted residual risk.

| # | Case | Mechanism here | Verdict |
|---|---|---|---|
| 1 | **Montana stranger** claims they walked my dog | Signed claim is `applied` (content always applies, `resolve.rs:1-20`) and stored; `counted()` is false because `author_of ∉ expand(set)` (§4.1); discovery shows it attributed (§5.1). Owner may ignore, `view.exception` it, or add the writer (§4.4). | **✅ wholly answered** |
| 2 | **Sybil flood** — 10k keys flood claims about my dog | Metrics: none count (§4.1). Compute: projection is author-indexed, so cost ∝ writer set, not flood (§4.2). Readability: bounded, cursor-paginated discovery with an honest `total_authors`; single-key floods collapse to one cluster, **multi-key floods do not** (§5.1). Storage: cold tier + #2032 quotas (§6). | **⚠️ metrics + compute answered; discovery readability only *bounded*, not solved — multi-key flood retained as an open risk routed to #2032/#2033** |
| 3 | **Storage exhaustion** against a relay | Validity is not a storage entitlement: `Tier::Cold { expires_at }` (§6) plus #2032's per-IP/per-ID/per-topic MB windows. Reference-holding rule (§6) prevents expiry stranding hot dependents. | **⚠️ holds only with #2032 budgets in place — this is the model's real weak point, stated not buried** |
| 4 | **Confusion attack** — spam crafted to look counted | `Projection.counted` and `.uncounted` are different types (§4.3); counted status is computed from the set, never read from the claim; every uncounted hit renders author + `WhyNotCounted`. | **✅ if the type split and the render rule hold** |
| 5 | **Writer-set poisoning** — I'm talked into adding an attacker | Set changes are signed `idset.writer` versions (§D1/D2), so the version history is a reviewable list ("your set changed 3× this week"); `rescind` exists (§3.3); per-entry scoping (`Entry::Group{role_min}`, §D3) bounds blast radius. | **⚠️ accepted residual: the model makes an owner decision visible and reversible; it cannot make it wisely** |
| 6 | **Group-reference capture** — a group admin adds a spammer | `Excludes` entries win over group admissions without leaving the group (§D6); group expansion is one level (§D4) so capture does not cascade; the set version names the frontier it was expanded at (§D4), so a later add is a *new version*, not a silent change. | **✅ answered — this is the mechanism the issue's point 1 asks for** |
| 7 | **Removed writer disputes history** — "my past walks must still count" / "must be erased" | §3.2 keeps past claims counted (they were true under the set then); `rescind` (§3.3) is the owner's explicit erasure, itself a visible version; the walker's stored claims remain their evidence because removal deletes nothing (`resolve.rs:1-20`, "content is kept on removal"). | **✅ both directions answered without silent rewrite** |

---

## 8. Open questions this spec cannot settle (for #2028, not decided here)

These are routed, not defaulted. Each names why it is not this spec's to answer.

1. **OQ-1 — May a `Public` writer set back a *metric* at all?** #2030 flagged that a public
   metric is an invitation to game it and suggested public views may need to be read-only
   discoveries rather than metrics. This spec provides `Entry::Public` (§D3/D5) and
   `Excludes(Public)` (§D6) but sets no policy on where `Public` may be used. **→ #2028** (with
   [#2033](https://github.com/developing-today/code/issues/2033) for the abuse-control side).
2. **OQ-2 — Is a writer set per *person*, per *app*, or per *question*?** A did-it view and a
   chat audience may want different sets for the same person. This spec makes `set_id` derived
   from `(owner, purpose)` (§D2), which permits all three, but does not pick the default the core
   design should present to users. **→ #2028.**
3. **OQ-3 — How does cold expiry interact with causal completeness?** §6 forbids expiring a
   referenced object, but a long-dormant counted event may reference a chain of uncounted
   ancestors. Whether expiry must be transitive-closure-aware, or whether uncounted ancestors of
   hot events are implicitly hot, is a real design choice with a storage cost. **→ #2028** and
   [#2034](https://github.com/developing-today/code/issues/2034) (ephemeral/live-data retention,
   which already owns the retention model).
4. **OQ-4 — Do writer sets need their own confidentiality, or is membership the boundary?**
   Today reading content requires `Read` (`groups/mod.rs:64-75`) and a set object names authors
   in a body that is only as private as the group it rides in. A set listing dissidents is
   sensitive. Whether a writer set should be encrypted to the owner only is not answerable
   without the core design's view of who may see a view's configuration. **→ #2028.**
5. **OQ-5 — What is the identity a user *sees* when author keys rotate?** A writer set names
   32-byte keys (§1.1); groups already carry `device.add`/`device.revoke`
   ([`groups/mod.rs:41-46`](../../pkgs/id/src/groups/mod.rs)) for multi-device accounts. Whether a
   writer set entry should be an account (a set of keys) rather than a key is a modelling
   question this spec deliberately leaves open — it makes `Entry::Key` the primitive and does not
   invent an account type. **→ #2028.**
6. **OQ-6 — Do view exceptions need their own audience?** A `view.exception` currently counts one
   claim for the view owner. Whether an exception can be *shared* (so two people agree to count a
   third party's claim) is unresolved; sharing it turns an exception into a mini-set. **→ #2028.**
7. **OQ-7 — How is a multi-key flood actually detected?** This is the residual risk §5.1 and
   abuse case 2 retain rather than claim: pagination keeps the UI usable, but nothing here
   recognises that 10,000 keys are one actor. Author-clustering (shared timing, identical bodies,
   relay/IP correlation) is the plausible mechanism and is deliberately **not** specified here —
   it belongs with the abuse controls (#2032/#2033), and it needs a decision on whether clustering
   is a *server* signal (which the viewer may not trust) or a *client* heuristic (which the server
   cannot force). **→ #2032 / #2033, with #2028 for where the signal lives.**

---

## 9. Implementation routing (file as their own issues — nothing built here)

Sequenced so no slice enables a counting path before the set object and its versioning exist.
Each is independently reviewable and small enough to land alone.

1. **`idset` schema types + canonical body encoding + `chain::sign`/`decode` round-trip tests.**
   `WriterSet`, `Entry`, `Excludes`, `Rescind`, with golden-header tests in the style of
   `chain.rs:733-852`. Library only; no projection yet.
2. **`expand(set_version) -> BTreeSet<AuthorKey>`** — frontier-aware via
   `is_member_at`/`access_at_position` (**not** `members(group)`, §4.5), one-level flattening and
   `role_min` filtering (§D3/D4). Property test: expansion is order-independent, frontier-pinned,
   and **unchanged by a later group-membership event** (the reproducibility guarantee).
3. **`AuthorEvents` index** — kind-generic `(author, seq) → (event_id, kind, subject?)`, persisted
   and incremental, built by ranging the `events` table per writer (§4.2). Test asserts a view
   query touches only writer-set authors (the cost claim, not just the answer) **and that
   non-tag kinds are indexed** — a `didit.record` must come back, which is where `TagIndex` fails.
4. **`Projection` + `WhyNotCounted` + `UncountedHit` types and the `counted`/`uncounted` split**
   (§4.1–4.3), including tests that a stranger's `applied` claim is `uncounted`, that a
   `rescind`ed claim stops counting from its version, and — the corrected one — that **a claim
   counted under set v1 stays counted under v2 after its author is removed** (admission history,
   §4.1).
5. **`discover()` bounded and cursor-paginated, grouped by author, with viewer-local petname
   labels** (§5). Two flood tests, because they differ: 10k claims from one key → one cluster;
   **10k distinct keys → 10k clusters, one bounded page, honest `total_authors`** (the one that
   fails today).
6. **`Tier` decision function** (§D7) with the reference-holding rule (§6), built on
   `prune_behind_cut`'s existing "protect dependencies and active backlinks" contract
   ([`event_store/mod.rs:527-545`](../../pkgs/id/src/event_store/mod.rs)) — give it a
   counted/uncounted policy and a caller, rather than writing new pruning — and wired to *no*
   scheduler: the scheduler and all window values come from #2032/#2033.
7. **The did-it tracker's first view** as the first real consumer, in
   [#2031](https://github.com/developing-today/code/issues/2031)'s PoC — the proof that one
   identity's view excludes a third party's claims while keeping them retrievable.

None of these is a "permanent feature on the unreviewed #2030 investigation": the review round
accepts this spec first, then these land against it.

---

## 10. Review dispositions

First review round on PR #2739 (Codex). Six findings, all accepted and fixed in the doc; none
dismissed. Recorded here because a spec whose defects are silently patched teaches the next reader
nothing.

| # | Finding | Disposition |
|---|---|---|
| 1 | `include_history` was defined as claims **not** in the set version's causal past — the exact inverse of what the option promises. | **Fixed.** §3.1 now says the flag counts claims **in** the past, in addition to everything prospective; the default (no flag) is the future-only set. |
| 2 | `Excludes(Public)` was described as "everyone except these", but with exclusions winning it produces an **empty** set. | **Fixed.** §D6 now states `entries=[Public], excludes=[Public]` means "count nobody", and the moderated feed is `entries=[Public], excludes=[Key…]` — two configs one entry apart, spelled out. |
| 3 | Projection tested membership only in the *current* set snapshot, so removing an author un-counted their whole history — contradicting §3.2 and abuse case 7. | **Fixed, and it was the most serious.** §4.1's predicate is now admission-history-aware (`∨ admitted(view, claim)` over the set's version prefix), with a test for it in slice 4. |
| 4 | Group expansion prescribed `Members::members(group)`, a **current-state** query, so a later group change would alter an unchanged set version's expansion. | **Fixed.** New §4.5 requires frontier-aware `is_member_at`/`access_at_position`, states the replay cost honestly, and keeps `members(group)` for current-state questions only. |
| 5 | Author-grouping was claimed to answer a 10k-**key** flood, but 10k distinct keys yield 10k clusters. | **Fixed by narrowing the claim.** §5.1 now scopes grouping to single-key floods, specifies bounded cursor pagination with an honest `total_authors`, and **retains the multi-key flood as an explicitly unsolved risk** routed to OQ-7 / #2032-#2033. The abuse-case table's row 2 verdict was downgraded accordingly. |
| 6 | `TagIndex` covers tags only, so the did-it tracker's `didit.record` events would project to nothing. | **Fixed.** §4.2 now specifies a kind-generic `AuthorEvents` index built by ranging `events` per author, with `TagIndex` demoted to a tag-specific adapter on top; slice 3's test asserts a `didit.record` comes back. |

Three citation errors found in self-review were also corrected (`chain.rs` decoder limits are at
`:41-42`; the `TagIndex` rebuild note is `:11-12,16`; `TagIndex.by_author` is `:46`). All 78
internal links were checked programmatically and resolve.

---

## Sources

- Issue [#2238](https://github.com/developing-today/code/issues/2238) (the spec's five points and
  acceptance criteria) and its scope exclusions.
- Investigation [#2030](https://github.com/developing-today/code/issues/2030) — result comment
  2026-10-29, queue task 3346 (verdict, three-layer model, writer-set representation, seven abuse
  cases, follow-on routing).
- Code read at `origin/main` `72545cb9`, 2026-10-30, cited inline:
  `pkgs/id/src/chain.rs` (envelope, `ChainExt`, domain tag, decoder limits, golden tests),
  `pkgs/id/src/event_store/mod.rs` (tables, `POSITION_LEN`, `event_applied`),
  `pkgs/id/src/event_store/resolve.rs` (three apply rules, `author_of`, roster flattening),
  `pkgs/id/src/event_store/claims.rs` (`tag_claims` full-store walk),
  `pkgs/id/src/event_store/waiting_rows.rs` (no time in the envelope),
  `pkgs/id/src/tag_claims.rs` (`Claim`, kinds, limits),
  `pkgs/id/src/tag_index.rs` (α/Ω/author index), `pkgs/id/src/search.rs` (`Term::Author`, `Hit`),
  `pkgs/id/src/groups/mod.rs` (ops, roles, `Veto`, devices), `pkgs/id/src/groups/policy.rs`
  (`Join::Open`), `pkgs/id/src/groups/fold.rs` (fold, `Group(_) → None`),
  `pkgs/id/src/membership/layer.rs` (`members`, `is_member_at`, `access_at_position`),
  `pkgs/id/src/membership/body.rs` (nested group encoding), `pkgs/id/src/membership/gate.rs`
  (admission only, no authority), `pkgs/id/src/petname.rs` + `directory_view.rs` +
  `world_store.rs` (viewer-local petnames), `pkgs/id/src/blob_gate.rs` (access, not retention),
  `pkgs/id/src/pull_policy.rs`, `pkgs/id/src/access.rs` (`AccessPolicy`, the flat `EndpointId`
  allowlist), `pkgs/id/src/world/mod.rs` (`project_event_for`, the one per-viewer projection),
  `pkgs/id/src/local_events.rs` (`frontier()`), `pkgs/id/src/event_store/sync.rs`
  (`sync_entries`, whole-store set reconciliation), `pkgs/id/src/store_traits.rs`
  (`prune_entries_tx` = `Unsupported`).
- Sibling wave-1 records this spec depends on and does not re-explain:
  [did-it tracker data model](../2026-10-29T13-04-50Z_investigation_didit_tracker_data_model/2026-10-29T13-04-50Z_investigation_didit_tracker_data_model.md)
  (event kinds, fold-as-evaluation),
  [comms abuse controls](../2026-10-29T10-55-00Z_investigation_comms_abuse_controls/2026-10-29T10-55-00Z_investigation_comms_abuse_controls.md)
  (the budget values §6 defers to),
  [grant lifecycle spec](../2026-10-30T11-55-00Z_design_grant_lifecycle.md) (spec shape precedent).
- Program: [epic #2056](https://github.com/developing-today/code/issues/2056); directive
  hatch-personal `memory/topics/id-program-2026-10-28.md`.
