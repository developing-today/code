# Design: writer-set objects, view projection, and the discovery query for `pkgs/id`

- **Date:** 2026-10-30
- **Status:** **design spec only — no permanent feature built from it.** Per the program rule
  ([epic #2056](https://github.com/developing-today/code/issues/2056)), a spec is not a build
  order: nothing here is implementable on top of the unreviewed
  [#2030](https://github.com/developing-today/code/issues/2030) investigation until the review
  round accepts it. Implementation slices are named in §9 and must be filed as their own issues.
- **Issue:** [#2238](https://github.com/developing-today/code/issues/2238). **Parent
  investigation:** [#2030](https://github.com/developing-today/code/issues/2030) (result posted
  on that issue 2026-10-29, queue task 3346).
- **Consumers / cross-refs:** [#2028](https://github.com/developing-today/code/issues/2028) (ID
  core design v1 — holds the questions this spec cannot settle, §8),
  [#2031](https://github.com/developing-today/code/issues/2031) (did-it tracker, the motivating
  app), [#2032](https://github.com/developing-today/code/issues/2032) (abuse-control budgets —
  owns every number this spec deliberately leaves out), [#2029](https://github.com/developing-today/code/issues/2029)
  (tracker data model — the event kinds that get projected).
- **Method:** every substrate claim below was read at `origin/main` `72545cb9` on 2026-10-30 and
  is cited `path:line`. Design choices are labelled **Decision** with their reasons; they are
  proposals for the review round, not settled API.

`pkgs/id/doc` and `pkgs/id/docs` are both symlinks to the repo-root `doc/` (`pkgs/id/doc ->
../../doc`), so this file lives at
`doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/2026-10-30T13-15-00Z_design_id_writer_set_view_projection.md`
and is reachable as `pkgs/id/doc/…` exactly as the issue asks. It is in a folder rather than flat so
its `../../pkgs/id/...` evidence links resolve.

---

## 0. The rule everything follows

From #2030, verbatim in effect: **ingest is a validity gate, never an authority gate; every
object that verifies is stored; a view counts only its writer set.**

This spec exists to make the third clause buildable. It is not allowed to trade away any of
these, and each is checked against the substrate in §1:

1. **Stored ≠ counted.** Storage accepts anything validly signed, attributed to its signer.
2. **Counting is per view, decided by the view owner, and reversible.**
3. **Every computed view names its inputs** — *(view config, writer-set version, frontier,
   resolution)* — so two people comparing "the same number" can see whether they compared the same
   thing (§D2, §3.4).
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
ID (BLAKE3 over `(owner, purpose)`), so successive versions of the *same* set are a **subsequence**
of one author's chain — ordered, not necessarily contiguous, because one owner may publish several
sets (§4.1's set filter, OQ-2).

Reasons: an owner-invented counter can fork or repeat; the chain already enforces "seq 0 has no
backlink, seq > 0 has one" ([`chain.rs:290-294`](../../pkgs/id/src/chain.rs)) and the store
already keeps `chains: author → next seq, head, frozen` (`event_store/mod.rs:90-91`). A view can
therefore name a set version as `(set_id, seq, head_hash)` and anyone can verify it is an
ancestor of the head they hold — no new versioning machinery. Two sets interleaving in one chain
costs nothing here: `set_id` is in the body, so which versions belong to which set is decided by
the signed body, not by adjacency.

**Rejected:** a mutable "current set" row. There is no mutation path in the substrate
(`event_store/mod.rs:1-30`, append-only) and a mutable set would make §D2's "which set version
produced this number" unanswerable.

### D3 — Entries are a union of three shapes

```rust
enum Entry {
    Key { author: AuthorKey, include_history: bool },       // 32-byte verifying key, explicit
    Group { id: Hash, role_min: Role, include_history: bool }, // one-level reference into groups/
    Public { include_history: bool },                       // the everyone-sentinel
}

// include_history is spelled on EVERY shape because §3.1 makes it per entry, and §4.1's
// `expand_retro` restricts a version's expansion to the entries that carry it. Omitted, it is
// false — the prospective default — so a config written before the flag existed means exactly
// what it meant then.
```

- **`Key`** matches `author_of(event)` directly (§1.1).
- **`Group`** resolves through frontier-aware expansion (§4.5), filtering by `role_min` against the
  ordered roles (`groups/mod.rs:64-75`). Resolution is **one level deep** (D4). `include_history`
  on a `Group` entry applies to every member that expansion yields at that frontier — which is why
  it is a field on the entry and not a set-wide flag: a group with one flagged member and one
  unflagged member is expressed as two `Group` entries, distinguished by `role_min` or by splitting
  the group. That limitation is stated rather than hidden (§8, OQ-8).
- **`include_history`** is per entry (§3.1), so importing one friend's backlog never imports
  another's. `expand_retro(v)` (§4.1) is `expand(v)` narrowed to the flagged entries.
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
different things on two devices, which breaks §D2's "which set version produced this number".

A frontier is necessary and **not sufficient**: a fork proof that invalidates a membership grant
changes the expansion of an unchanged group at an unchanged frontier, because resolution drops the
excluded action before the membership layer sees it (§4.5). So the reproducible unit is the pair
*(frontier, `R`)*, and §4.5's cache key says so. This is the one place the spec corrects its own
earlier "the frontier is immutable, so the expansion is permanent" reasoning.

### D5 — `Public` is not `Join::Open`

`Join::Open` ([`groups/policy.rs:21-22`](../../pkgs/id/src/groups/policy.rs)) governs who may
*join a group*; `Entry::Public` governs who may *count in a view*. They are different questions
with different blast radii, and the spec forbids reusing one for the other. In particular, a
`Public` writer set is never a default for a metric: the #2030 investigation flagged that a
public *metric* is an invitation to game it, and left that question to #2028 (§8, OQ-1).

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

`Excludes(Public)` is deliberate, but it means exactly one thing: **admit nobody**. Because
`Public` matches every author and exclusions win over every admission, `expand(v)` is empty for a
set with `entries = [Public], excludes = [Public]` (the expression is in §4.1). It is the emergency
"stop counting everything in this view" switch, not a moderation tool.

**It stops *prospective* admission, not past totals, and saying otherwise was an overclaim.**
§4.1's `seen_by` resolves each claim against the latest admission-bearing version in *its own*
causal past, so a claim Alice wrote while v1 admitted her keeps resolving to v1 under v2 — an empty
v2 cannot reach back and un-admit her, exactly as §3.2 requires. A claim she writes after seeing v2
is not counted. So the switch has two halves, and an owner who wants both publishes both:

1. the empty v2, which stops everything from v2 on; **and**
2. an `idset.rescind` (§3.3) with `target: Public`, which erases every claim in its causal past —
   one signed event, not one per author, and it appears in the set's version history instead of
   happening invisibly.

That two-act shape is deliberate rather than inconvenient: it keeps "stop admitting" and "erase
what was admitted" as separate, auditable decisions, which is §0's fifth principle applied to the
emergency case. The doc previously said the single empty version was sufficient; it is not, and
the correction is here rather than in a footnote.

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
rather than a data change — precisely the silent history rewriting principle 5 of §0 forbids.

### 3.3 `rescind` is an explicit, visible set-version event

**Decision:** `kind = "idset.rescind"`, body `{ set_id, from_version, target: Entry, through: Option<Hash> }`.
It excludes the target's claims from `from_version`'s position, optionally only up to a named
event (`through`), and it is itself a new set version.

Reasons: #2030 abuse case 7 ("removed writer disputes history") needs a real answer for both
directions — the walker says "my past walks must still count", I say "they must be erased". §3.2
gives the walker their answer by default; `rescind` gives me mine, as a signed act that shows in
the set's version history instead of happening invisibly. It is a separate kind, not a flag on
the set, so its authorship, position and reason are all auditable.

Because §4.1 clause 3 is a term in the predicate, its meaning has to be as precise as the rest:

```
rescinded(S, c)  ⇔  ∃ r ∈ versions(S):                                   // r is a rescind version
                      r.kind = idset.rescind
                    ∧ r.body.set_id = S.set_id         // §4.1's set filter: a rescind of another
                                                       // set never erases this set's claims
                    ∧ author(c) ∈ expand({ entries: [r.target], excludes: [] })
                                                                         // target is entry-shaped,
                                                                         // so target: Public erases
                                                                         // everyone's claims
                    ∧ c ∈ causal_past(r)                                 // r saw the claim
                    ∧ ( r.through is None ∨ c ∈ causal_past(r.through) ) // the optional bound
                    ∧ r.from_version.seq ≤ S.set_seq
```

Three things this pins down that the prose left open:

- **`target: Public` is a valid and useful rescind.** It erases every claim in the rescind's causal
  past, which is the second half of the emergency stop in §D6: §D6's empty version stops prospective
  admission, this erases what was already counted. One signed act, not one per author.
- **It erases only claims `r` had actually seen.** A claim written *after* the rescind is untouched
  by it, exactly as a removal does not stop claims it never saw (§3.2). Erasing those takes a later
  rescind — a new signed act, not a silent widening.
- **`through` bounds the erasure to claims in that event's causal past.** It is the "up to here"
  form, so an owner can draw a line at a point rather than at a moment — the only option available
  in a substrate with no timestamps in the envelope (§3.1).
- **A rescind is a set version but not an admission version.** It carries no `entries`, so
  `expand(r)` and `expand_retro(r)` are empty. That is exactly why §4.1's `seen_by` and `retro`
  quantify over `adm_versions(S)` — the `idset.writer` versions only — rather than over
  `versions(S)`: a rescind is not an admission state, and letting it stand in for one would make a
  version that admits nobody the latest thing its author had seen, rejecting every admitted writer's
  claims until the next `idset.writer` arrived. Only clause 3 reads a rescind. A rescind that tried
  to add an entry is rejected at decode: the two kinds are separate for precisely this reason.

### 3.4 Every computed view names *(view config, set version, frontier, resolution)*

**Decision:** a projected result carries
`{ view_id, config_seq, set_id, set_seq, set_head, frontier, resolution }` — never just a number.
`(set_id, set_seq)` and `frontier` pin *who may count and over what claims*; `(view_id, config_seq)`
pin *which view computed it*, because two views can share a writer-set version and a frontier and
still disagree: `subject_scope`, `kind_scope` and `exceptions` are part of the view, and §4.4 lets
one view count a stranger's claim the other does not. `resolution` names the fork-proof state
`applied` was evaluated against (§4.6): `APPLIED` is rebuilt against every known fork proof on each
`refresh` ([`event_store/ingest.rs:890-937`](../../pkgs/id/src/event_store/ingest.rs)), so a proof
arriving later would otherwise silently change a saved total with no change to any named input.
Without the view configuration in the provenance, two different totals would carry identical inputs
and neither could be reproduced or told apart; without `resolution`, two *different* totals would
carry identical provenance.

Reasons: a projection is a pure function of *(view configuration, set version, frontier,
resolution)*. Naming only part of that tuple is what made earlier drafts' provenance insufficient
twice over: first the view's own scope and exception list was missing, which changes rarely but
*does* change without any writer-set event; then the resolution state was missing, which changes
with no act of the owner at all. `config_seq` is the owner's chain position of the latest
view-configuration event, so it is immutable and comparable the same way `set_seq` is;
`resolution` is identified by the set of fork proofs it contains, which is likewise immutable and
hashable (§4.6).

The frontier is the view's own `after` at evaluation
([`chain.rs:97-118`](../../pkgs/id/src/chain.rs)), and it is **not decorative**: §4.1's
`past(view.frontier, claim)` clause makes the claim set closed under it, so re-evaluating the same
*(set version, frontier, resolution)* triple always yields the same total. Pinning only the set
version would leave the claim set open and the number irreproducible.

The group layer already made the frontier part of this choice — `is_member_at`/`access_at_position` take an
`after` and resolve at that position ([`membership/layer.rs:338-364`](../../pkgs/id/src/membership/layer.rs))
— so the pattern is established. A bare count that cannot name its inputs is not comparable, and
#2030's whole point is that comparability is what makes "the same metric" meaningful across
people who trust different sets.

---

## 4. View projection

### 4.1 Counted vs uncounted, computed per view

**Decision:** a **view** is `{ view_id, config_seq, owner, set: (set_id, set_seq), frontier, resolution, subject_scope, kind_scope, exceptions }`.
Projection is a pure function of that whole tuple, or it is **undefined** — there is no third case.
It is stated once, in full, because eight review rounds showed that patching clauses one at a time
keeps opening a hole somewhere else.

```
// ---- inputs, all immutable, all named in provenance (§3.4) -----------------
//   C  = view config       (view_id, config_seq, scopes, exceptions)
//   S  = set version       (set_id, set_seq)  — the selected writer-set version
//   F  = claim frontier    — no claim after it is eligible
//   R  = resolution state  — the fork-proof set that fixes applied()

past(F, c)      ⇔  c ∈ causal_past(F)

// versions(S) is NOT "every idset.writer event the store holds". It is TWO restrictions,
// and both are load-bearing:
//   (a) the owner's APPLIED PREFIX as of R, truncated at S.set_seq — a linear sequence,
//       because `chains` stores the applied prefix and `forks` holds the conflicting
//       headers outside it (event_store/mod.rs:90-91), and
//   (b) a FILTER ON `set_id`: only versions whose body says set_id = S.set_id.
// (b) is not cosmetic. OQ-2 explicitly permits one owner to publish several purpose-
// specific sets, so the owner's applied prefix interleaves the dog set, the chat set and
// the dog set again. Without the filter, seq 2 of the *chat* set would be an admission
// version of the *dog* set: a stranger's dog claim that saw seq 2 but not the dog set's
// seq 3 would resolve `seen_by` to the chat set's Public and count. A set version is a
// position in an owner's chain, but a *version of S* only if S is the set it publishes.
versions(S)     ⇔  { v : v ∈ applied prefix of S.owner's chain under R, v.seq ≤ S.set_seq,
                        body(v).set_id = S.set_id }
                    // `applied prefix ... under R` is the same linearity guarantee as before;
                    // the set_id filter makes it linear *and about one set*. Both kinds
                    // (idset.writer and idset.rescind) carry a set_id, so one filter serves
                    // both — a rescind of the chat set can never erase a dog claim.
                    // if S.set_seq is NOT in this sequence, project() is undefined for this
                    // view: the view's own set version is forked, so there is nothing to
                    // project. Refusing is the answer; silently projecting the surviving
                    // branch would attribute a total to an input the owner did not select.
adm_versions(S) ⇔  { v ∈ versions(S) : v.kind = idset.writer }
                    // admission-bearing versions only. A rescind is a version but carries no
                    // entries (§3.3), so counting it in `seen_by` would make the newest thing an
                    // author had seen a version that admits nobody — and empty `expand(r)` would
                    // then reject every admitted writer's claims, world-wide, until the owner
                    // happened to publish another `idset.writer`. Rescinds act through clause 3
                    // alone.
seen_by(S, c)   ⇔  max{ v ∈ adm_versions(S) : v ∈ causal_past(c) }  // the version IN FORCE at c
admitted_at(S, c)  ⇔  seen_by(S, c) is defined
                        ∧ author(c) ∈ expand(seen_by(S, c))   // that version's expansion, not any
retro(S, c)     ⇔  ∃ v ∈ adm_versions(S):  c ∈ causal_past(v)     // v saw the claim
                        ∧ author(c) ∈ expand_retro(v)     // only entries marked include_history
expand(v)       ⇔  { author(a) : a ∈ v.entries } \ { author(x) : x ∈ v.excludes }
                    // D6's rule, in one place: exclusions win over entries, including Public.
                    // expand_retro(v) is the same expression over the entries carrying
                    // include_history, minus the same exclusions.
in_scope(C, c)  ⇔  subject(c) ∈ C.subject_scope ∧ kind(c) ∈ C.kind_scope
                    // exceptions are handled by clause 5, not inside in_scope,
                    // so there is exactly one place they bypass anything

// ---- the predicate ---------------------------------------------------------
counted(C, S, F, R, c) ⇔
      applied(R, c)                                  // 1. validity under the PINNED resolution
   ∧  past(F, c)                                     // 2. closed under the claim frontier
   ∧  ¬rescinded(S, c)                               // 3. explicit rescind
   ∧  (
        ( in_scope(C, c)
          ∧ ( admitted_at(S, c) ∨ retro(S, c) )      // 4. admitted in force, or imported history
        )
        ∨ c ∈ C.exceptions                           // 5. one-off exception (validity still required)
      )

uncounted(...) ⇔ applied(R, c) ∧ ¬counted(...)        // stored, attributed, zero effect
```

Each clause exists because a review round proved its absence broke something:

1. **`applied(R, c)` — validity under a *pinned* resolution state.** An event at or above a fork
   point never applies ([`event_store/resolve.rs:7-8`](../../pkgs/id/src/event_store/resolve.rs)),
   and `R` is part of the input tuple, not the live store: `APPLIED` is rebuilt against **every**
   known fork proof on each `refresh`
   ([`event_store/ingest.rs:890-937`](../../pkgs/id/src/event_store/ingest.rs)), so a conflicting
   header arriving *later* flips `applied` for an already-saved view. Evaluating against `R`
   ("the resolution state as of the saved frontier") makes a historical total reproducible;
   querying live validity is a separate, explicitly-named operation (§4.6). Exceptions do not
   escape this gate either — they lift scope and the writer-set requirement, never validity.
2. **`past(F, c)` — the claim set is closed.** Pinning the set version fixes *membership*; only
   this fixes *which claims* are eligible. Without it, re-evaluating a saved view after new claims
   arrive returns a different total.
3. **`¬rescinded(S, c)`** — §3.3's explicit erasure, bounded by the selected set version.
4. **`admitted_at` OR `retro`** — two distinct admission paths, and conflating them was the
   round-2/round-3 error. Note the quantifiers differ and that is deliberate:
   - **`admitted_at`** is the *prospective* path, and it takes the **`max`** version the author had
     seen, not `∃`: a claim Alice writes after v1 admits her resolves to v1 → counted; a claim she
     writes after v2 removes her resolves to v2 → not counted. Quantifying existentially here is
     round 2's finding 7 — v1 stays in her causal past forever, so `∃ v` would keep granting her
     *new* claims off one old admission. `max` is what makes removal prospective.
   - **`retro`** is the *retrospective* path: a version whose causal past already contains the
     claim, whose `expand_retro` admits the author. `expand_retro(v)` is `expand(v)` **restricted to
     entries carrying `include_history: true`**, because §3.1 makes the flag *per entry*, not per
     set: a friend added with the flag imports their backlog while a second friend added without it
     correctly does not. Using plain `expand(v)` here would make one flagged entry import the
     untagged backlog of every other member too.
     Existential is correct **here**, because §3.2 says a claim counted under an earlier version
     stays counted for later ones: a later removal must not un-import a backlog already brought in,
     only `rescind` (§3.3) may. This is the only way a claim that predates any admission can count —
     the promise §3.1 makes and that a prospective-only predicate could never keep.
   - **`seen_by` draws from the *applied* prefix, not from every `idset.writer` event in the
     store.** `chains` keeps each author's applied prefix and `forks` holds conflicting headers
     outside it ([`event_store/mod.rs:90-91`](../../pkgs/id/src/event_store/mod.rs)), so the prefix
     is linear and `max` is a function. Without that restriction a set chain that forked at some
     `set_seq` would give two versions the same `seq`, `max` would be ambiguous, and §4.1 would stop
     being a function of *(C, S, F, R)* at all — a forked, never-applied set version could admit an
     author or satisfy `retro`. This is the same discipline clause 1 applies to claims, applied to
     the set's own history.
   - **…and only from versions *of this set*.** The applied prefix is the *owner's*, not the
     set's: OQ-2 lets one owner publish several purpose-specific sets, so the prefix interleaves
     them. Filtering by `body(v).set_id = S.set_id` (round-8 finding 26) is what keeps a second
     set's admission from governing the first — without it, a chat set admitting `Public` at seq 2
     would admit the world into the dog view, and a chat `rescind` would erase dog claims. Both
     kinds carry `set_id`, so one filter covers admissions and rescinds alike.
   - **If `S.set_seq` itself is not in `versions(S)` under `R`, `project` is undefined.** The
     owner selected a set version that a fork proof has since invalidated, so the view's own input is
     gone; refusing is correct and silently substituting the surviving branch would report a total
     against an input nobody chose. §4.6's `validity_now` is what tells the owner *why* — the same
     split that keeps a saved total reproducible keeps an unresolvable one from being invented.
   - `seen_by` is also bounded by `v.seq ≤ S.set_seq`, so a version the author has seen but the view
     has **not** selected cannot govern. That is the same pinning as everywhere else: the view names
     its set version, and `S` — not the author's newer knowledge — decides. §3.2's removal begins at
     the selected version, which is exactly what "removal stops future counting" means for a view
     that has not yet advanced.
   - **`retro` is monotone, and clearing `include_history` later does not un-import.** Because the
     disjunct is existential over the applied prefix, a later version that drops the flag leaves an
     already-imported backlog counted. That is intended (§3.2: only `rescind` retracts), but it is
     worth stating: the flag is an import switch, not a standing filter. An owner who wants the
     import *undone* uses `rescind` (§3.3), which is visible in the set's version history.
5. **`in_scope(C, c)`, with one exception path outside it.** A view scoped to one dog must not count
   the other dog's walks even though the same author wrote both. `subject_scope` and `kind_scope`
   were in the view configuration from the first draft but were never applied by the predicate
   (round-4 finding 15); `kind_scope` alone filtered at retrieval, which is not the same thing and
   did nothing for subjects. Round 5 removed that retrieval filter entirely, so both scopes are
   decided in one place (§4.2). Exceptions are a **separate** disjunct and deliberately not folded into
   `in_scope`: they must bypass scope *and* the writer-set requirement alike, and having one
   bypass point instead of two is what keeps the two from drifting apart. Validity (clause 1) is
   above both, so an exception still cannot count a fork-invalidated claim.

**Candidate retrieval must match the predicate's reach, or the predicate never sees the claim.**
This is the failure mode that produced findings 6, 11, 13 and 14: each round fixed a clause and left
retrieval behind it. The predicate can count a claim through four routes — current admission (clause
4's `admitted_at`, a subset of the historical union), historical admission (an author admitted by an
earlier version and removed since), retrospective admission (clause 4's `retro`), and a named
exception (clause 5) — and §4.2 specifies retrieval as exactly those sources and nothing narrower.

Note the two predicates use different bases and that is the point: `applied(R, ·)` is the
store-wide membership resolution (§1.2) as of the pinned resolution state, `counted` is the view's
writer set. A claim can be `applied` and `uncounted` — that is the normal case for a stranger's
claim.

### 4.2 Cost is proportional to the writer set, not the store

**Decision:** projection reads an **author-keyed event index** restricted to the authors the
selected set version admits — currently, historically or retroactively — plus the named exception
IDs, rather than scanning the store. The precedent for the *shape* is `TagIndex`'s author key; the
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
(event_id, kind, subject?)`, built by ranging the existing `events` table per author over that
historical writer union, recording `kind` but filtering on none. It is generic over kind, so it serves `tag.*`,
`didit.*` and any future kind; it is persisted and incrementally maintained; and where a
kind-specific projection (tags → `TagIndex.by_author`) is cheaper, that becomes an *adapter on
top* of it rather than the only source. The tracker consequently reads real `didit.record`
events, which is the test slice 7 exists to prove.

**Candidate retrieval is three queries covering four admission routes, and it must match the
predicate's reach.** §4.1 can count a claim through current admission (a subset of historical),
historical admission, retrospective admission and a named exception:

1. **Author ranges of every author *historically* admitted up to the selected version** — the union
   of `expand(v)` over `v ∈ versions(S)` (so bounded by `S.set_seq` **and** by `v.set_id = S.set_id`,
   §4.1), not the current expansion alone. A removed author's earlier claims stay counted (§3.2,
   clause 4's `admitted_at`), so their ranges must still be scanned or the predicate can never
   preserve them. This is the correction for round-4 finding 13, and it is strictly larger than the
   set in force today.
2. **Past claims of a retro-admitting version** (`include_history: true`, §3.1). Those claims are by
   definition older than the version that admits them and may be authored by someone no version
   *prospectively* admitted, so they are reachable only through source 1's historical ranges — plus,
   for a claim that predates every version, through the version's own causal past. This is round-4
   finding 14: without it `include_history` cannot work at all, because the predicate had no
   retrospective branch and retrieval had nothing to feed one.
3. **Exception IDs looked up directly.** §4.4 lets an owner name a *stranger's* claim, and the whole
   point is that the stranger is **not** in the writer set — so neither author scan would return the
   claim and the exception could never fire. Each `view.exceptions` entry is fetched by ID
   ([`event_store/mod.rs:488`](../../pkgs/id/src/event_store/mod.rs)): O(log n) each, O(|exceptions|)
   in total, a bounded owner-authored list.

**Neither scope is applied at retrieval.** `in_scope` (§4.1 clause 5) owns that decision, and
applying it a second time in the query is how the two drift apart. This is a correction to an
earlier draft, which had `kind_scope` narrowing the index *and* `in_scope` filtering again: a
claim of the wrong kind would then never be a candidate, so `OutOfScope(kind)` could never fire and
the uncounted side would silently lose every claim the owner filtered out — the exact "uncounted
must not be a bin" failure §5.1 exists to prevent. The index stores `kind` so the predicate can
test it cheaply; it does not filter on it.

That leaves the cost claim honest: the index holds rows the predicate may reject, and the rejection
is a per-row test, not a per-row decode — which is what makes the cost proportional to writer-set
rows rather than to store size.

The cost claim and its test are therefore precise: **candidate retrieval may read only the
historical writer-set author ranges, the retro-admitting versions' past claims, and the named
exception IDs.** Slice 3's test asserts exactly that tuple — not merely the answer, not a blanket
"only current writer-set authors" that loses removed authors' history, and not a set so tight that
exceptions or `include_history` are unreachable.

**The cost is O(historically admitted authors × their rows), which is larger than O(current
writers)** and the difference is stated rather than hidden. The bound is still *set-history-shaped*,
not store-shaped: an author enters the union only by a signed set version, removal does not shrink
it, and an owner who wants the set to shrink that far uses `rescind` (§3.3) — which §4.1 clause 3
honours, so rescinded claims drop out of the *count* even though their ranges are still scanned.
The alternative, scanning current writers only, is precisely what made removed authors' history
vanish (finding 13); trading a bounded, owner-controlled growth for correct totals is the right
side of that trade, and §6's `Tier` policy is where a very long set history would be paged.

**One entry shape breaks that bound, and it is stated rather than glossed.** `Entry::Public`
(§D3/D5) admits *every* author, so the historical author union is every author — source 1 becomes a
store-shaped scan and "cost ∝ writer set, not the store" stops being true for that configuration.
Pagination and the `AuthorEvents` index keep it *linear in rows* rather than quadratic, and the
index is persisted so it is a scan of indexed rows, not a decode of every event; but the asymptotic
claim in this section is about keyed and group entries, and a `Public` view is the case OQ-1 asks
whether that should back a *metric* at all. §7 row 2's compute verdict is quoted against a keyed set for
the same reason.

### 4.3 Counted and uncounted are strictly separated in the type system

**Decision:** `Projection { counted: Vec<Hit>, uncounted: Vec<UncountedHit>, provenance: Provenance }`,
and the two collections never share a type. `UncountedHit` carries `author`, `signature_ok`
(true by construction from `chain::decode`) and `reason: WhyNotCounted`.

Reasons: #2030 abuse case 4 (confusion attack) says counted status is *computed from the set,
never displayed from claim-supplied data*. Making it two Rust types means a UI cannot render an
uncounted claim inside the counted collection without a deliberate conversion — the invariant
survives contact with a careless template.

`WhyNotCounted` is a closed enum, one variant per clause of §4.1 that can fail *after* `applied`
succeeded:

```rust
enum WhyNotCounted {
    AfterFrontier,                    // clause 2 — written after the view's frontier
    RescindedSince(SetSeq),           // clause 3 — the rescind version that erased it
    AuthorRemoved(SetSeq),            // clause 4 — seen_by saw this version, which removed them
    HistoryNotImported,               // clause 4 — the claim precedes the author's first
                                      //            admission and no admission carrying
                                      //            include_history covers it
    OutOfScope(ScopeMismatch),        // clause 5 — subject or kind
}
```

The set is **exactly the failure modes reachable through §4.2's retrieval**, which is why there is
no `AuthorNotInSet`: an author no version ever admitted is in no historical author range, so their
claims are never candidates and can never appear in either collection. A variant for that case
would be an arm a test can never reach.

`HistoryNotImported` covers **both** shapes of "too early", and an earlier draft only covered one:
a claim that predates *every* set version, and — the case finding 25 caught — a claim written while
v0 admitted nobody, by an author who is admitted later **without** `include_history`. v0 never
removed them and the claim need not predate every version, so neither `AuthorRemoved` nor a
"predates everything" reading fits. Naming it by the *cause* ("this author's history was not
imported") rather than by one of its instances is what makes the variant cover the reachable set.

The two clause-4 reasons that remain stay distinct because the user-facing messages differ — "you
removed them" (where §3.2's `rescind` affordance belongs) is not "you never imported this" (where
`include_history` belongs).

A free-text reason would drift, and collapsing these into one code would erase an affordance.

**A fork-invalidated claim is in neither collection**, and that is deliberate: `applied(R, c)` is
the outermost gate, so such a claim is not `uncounted` — it is *not in force* (§1.2). Folding it
into `uncounted` would make an enum whose members all explain a counting decision also carry a
validity verdict, which is the counted/uncounted blurring this section exists to prevent. It is
reported by `validity_now` (§4.6) instead, which is also the only place a *previously counted* claim
can be flagged after a later fork proof.

### 4.4 Exceptions are view-level and separate from set changes

**Decision:** `exceptions: Vec<Hash>` on the view counts a named claim one-off, recorded at the
view level (a `view.exception` event). It never edits the writer set.

Reasons: #2030 draws the line explicitly — *count this claim* and *add this writer* are different
actions and collapsing them is how writer sets quietly bloat. Keeping them as separate kinds is
the code-level expression of that paragraph.

An exception is a **disjunct inside** §4.1's predicate, not a bypass of it: it lifts clause 5's
scope and clause 4's admission, and nothing else. It cannot count a claim after the frontier
(clause 2), a rescinded claim (clause 3), or a claim that does not apply under the pinned
resolution (clause 1). It also cannot *hide* the claim it counts: exceptions are per view, so the claim is still
uncounted — and still listed — for the owner's *other* views that admit its author, and for any
view of anyone else who has that author in their set. What it cannot do is make a stranger's claim
visible to a third party for whom the claim is neither admitted nor excepted: §4.2 never fetches it
there, and no mechanism in this spec should (that would be the ambient leak §5.3 forbids).

`view.exception` is a **view-configuration event**: it advances `config_seq` (§3.4), because
§4.1's input tuple contains `exceptions` and provenance is only meaningful if every change to `C`
moves the pin. Recording it as a config event rather than as a loose side-table row is what makes
"these two totals had the same provenance" a true statement.

An exception for a claim that does **not exist** locally is kept and simply does not fire — the
view stores the hash, the lookup misses, and the result is identical to not having it yet. That is
the same forward-reference behaviour set versions already have, and it means the owner can name a
claim before it syncs without the exception silently vanishing.

---

### 4.5 Group expansion must be frontier-aware, not `members(group)`

**Decision:** `expand(v)` resolves group references **at *v*'s pinned frontier, under the view's
pinned resolution state `R`**, using `is_member_at(author, group, after)` /
`access_at_position(after, group, member)`
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
group entries this is *g* replays, and `members(group)` remains correct for *current-state*
questions (e.g. "who is in this group right now"), which is the only thing the spec uses it for.

**The cache key is `(group, frontier, R)`, not `(group, frontier)`** — and an earlier draft of this
section got that wrong. A frontier alone is *not* enough to make a group expansion permanent,
because the replay is not a pure function of the group's causal past: `resolve` drops fork-excluded
membership actions before they reach the layer, replacing their role with `Role::Other`
([`event_store/resolve.rs:82,124-133`](../../pkgs/id/src/event_store/resolve.rs)). So a membership
grant that a later fork proof invalidates changes the expansion of an *unchanged* group at an
*unchanged* frontier. Caching by `(group, frontier)` would serve pre-proof membership to a
projection evaluating under the newer `R`, while a cold evaluator would compute a different total
from the same named inputs — the exact reproducibility failure §3.4 and §D4 exist to prevent.

Adding `R` to the key keeps the useful property and states its real precondition: within one
resolution state an expansion is immutable and cacheable forever; across resolution states it must
be recomputed. The number of live `R` values is small (it changes only when a fork proof arrives),
so the cost is a bounded multiple of the replay cost, paid only when the store's resolution state
actually moves.

### 4.6 Live validity is a separate, explicitly-named query

**Decision:** the pinned evaluation of §4.1 and a *live* validity check are two different operations
with two different names, and `Projection` says which one produced it.

- `project(view, R)` — §4.1. `R` is the resolution state **as of the view's saved frontier**, so
  the total is reproducible. This is what a stored, comparable result means.
- `validity_now(event_id) -> Applies | Forked | Unknown` — evaluates `applied` against the **current**
  resolution state, and returns a per-event answer rather than a total.

Reasons: `applied` is not a fixed property of an event. `refresh` rebuilds `APPLIED` against **every**
fork proof the store knows ([`event_store/ingest.rs:890-937`](../../pkgs/id/src/event_store/ingest.rs)),
and an event at or above a fork point never applies
([`event_store/resolve.rs:7-8`](../../pkgs/id/src/event_store/resolve.rs)). So a fork proof that
arrives *after* a view was computed flips `applied` for claims the view already counted. Two
consequences, and conflating them was round-4 finding 16:

1. **The saved total must not change.** If it did, §3.4's provenance would name inputs that no longer
   reproduce the output, which is the failure provenance exists to prevent. `R` is therefore part of
   the input tuple and §3.4's provenance carries its identifier.
2. **The reader must still be able to learn that a counted claim is now forked.** Silently serving a
   stale number would be the "projection lie" §3.2 forbids; recomputing the number silently would
   break comparability. The answer is to keep the total pinned and expose the divergence as its own
   query: the UI can mark "3 of these 40 are no longer valid" and the owner can decide to advance the
   view's frontier, which recomputes under a new `R` and produces a *new* provenance — an explicit
   act, not a silent rewrite.

`Unknown` is a third answer, not a synonym for `Forked`: for a claim whose resolution cannot be
determined from the local store (a proof not yet synced), reporting `Forked` would let a missing
sync masquerade as a proven conflict. §1.2 is explicit that `applied` separates *present* from *in
force*; `Unknown` is the honest answer for "present, and neither yet".

The two answer spaces stay deliberately distinct. `WhyNotCounted` (§4.3) explains why a claim never
entered a total; `validity_now` reports on a claim that *may already be* in a total. Merging them
would put a "counted, but no longer valid" case into an enum whose members all mean "not counted",
which is exactly the counted/uncounted blurring §4.3 exists to prevent.

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

### D7 — Counted is hot/permanent; uncounted is cold with published expiry, unless pinned, counted, or a served view's input

**Decision:** the interface is a `Tier` decision per object, evaluated by any server:

```rust
enum Tier { Hot, Cold { expires_at: RetentionWindow } }
// Hot  if counted by any view this server serves, or pinned by its owner,
//      or named by a view exception,
//      or it is a view-configuration input of a view this server serves
//      (the view config itself, and every writer-set or rescind version that
//       view's projection reads) — see the second constraint below.
// Cold otherwise.
```

Reasons: #2030's storage-exhaustion answer is exactly this, and it must be an *interface* here
because the numbers are not this spec's to set. Three constraints from the code:

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
- **A view's own inputs are retention roots, and counting alone does not make them one** (round-8
  finding 27). The obvious rule — "hot iff counted by some served view" — protects the wrong side
  of the projection. A tracker view with `kind_scope = didit.record` counts *records*; its
  `idset.writer` version is not a `didit.record`, so it is not counted by that view and, absent an
  owner pin, falls to `Cold`. The counted records stay hot and their policy input expires, which is
  worse than either alone: the store keeps a total it can no longer re-derive, and §4.1 declares
  the projection **undefined** once `S.set_seq` is gone. The reference-holding rule above does not
  rescue it either, because the record's `after`/`backlink` point backwards to earlier events and
  cannot causally reference an admission published later — `prune_behind_cut` protects explicit
  roots and their ancestors, not later policy inputs. So: **the configuration of every view this
  server serves, and every writer-set/rescind version its projection reads** (the applied prefix
  up to `S.set_seq` for `S.set_id`, plus any group events the pinned frontier needs, §4.5), **are
  retention roots in their own right, independent of whether they count.** They are `protected`
  roots passed to the same `prune_behind_cut` contract, not a new mechanism.

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
counted/uncounted dimension **and the configuration-input roots of the third constraint** to
`cut`/`protected`. Recorded here so the implementation does not rebuild what exists.

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
| 1 | **Montana stranger** claims they walked my dog | Signed claim is `applied` (content always applies, `resolve.rs:1-20`) and stored; `counted()` is false because no version in the set's applied prefix ever admitted them, so `admitted_at` and `retro` are both false (§4.1) — *not* because they are absent from the current snapshot, which was finding 3. Discovery still shows the claim where its author is otherwise known (§5.1); a view that neither admits nor names them does not retrieve it at all, and that is deliberate (§5.3). Owner may ignore, `view.exception` it, or add the writer (§4.4). | **✅ wholly answered** |
| 2 | **Sybil flood** — 10k keys flood claims about my dog | Metrics: none count (§4.1) — no version admitted those authors, so they are not even candidates. Compute: projection is author-indexed, so cost ∝ writer-set rows, not flood volume (§4.2) — true for keyed/group sets; a `Public` set is the stated exception. Readability: bounded, cursor-paginated discovery with an honest `total_authors`; single-key floods collapse to one cluster, **multi-key floods do not** (§5.1). Storage: cold tier + #2032 quotas (§6). | **⚠️ metrics + compute answered; discovery readability only *bounded*, not solved — multi-key flood retained as an open risk routed to #2032/#2033** |
| 3 | **Storage exhaustion** against a relay | Validity is not a storage entitlement: `Tier::Cold { expires_at }` (§6) plus #2032's per-IP/per-ID/per-topic MB windows. Reference-holding rule (§6) prevents expiry stranding hot dependents. | **⚠️ holds only with #2032 budgets in place — this is the model's real weak point, stated not buried** |
| 4 | **Confusion attack** — spam crafted to look counted | `Projection.counted` and `.uncounted` are different types (§4.3); counted status is computed from the set, never read from the claim; every uncounted hit renders author + `WhyNotCounted`. | **✅ if the type split and the render rule hold** |
| 5 | **Writer-set poisoning** — I'm talked into adding an attacker | Set changes are signed `idset.writer` versions (§D1/D2), so the version history is a reviewable list ("your set changed 3× this week"); `rescind` exists (§3.3); per-entry scoping (`Entry::Group{role_min, ..}`, §D3) bounds blast radius. | **⚠️ accepted residual: the model makes an owner decision visible and reversible; it cannot make it wisely** |
| 6 | **Group-reference capture** — a group admin adds a spammer | `Excludes` entries win over group admissions without leaving the group (§D6); group expansion is one level (§D4) so capture does not cascade; the set version names the frontier it was expanded at (§D4), so a later add is a *new version*, not a silent change. | **✅ answered — this is the mechanism the issue's point 1 asks for** |
| 7 | **Removed writer disputes history** — "my past walks must still count" / "must be erased" | Claims written while the author was admitted stay counted via clause 4's `admitted_at` (§4.1), so past walks keep counting; claims written **after** removal do not, so removal is prospective; `rescind` (§3.3) is the owner's explicit erasure, itself a visible version; the walker's stored claims remain their evidence because removal deletes nothing (`resolve.rs:1-20`, "content is kept on removal"). | **✅ both directions answered without silent rewrite — the interval form is what makes both true at once** |

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
8. **OQ-8 — How does `include_history` apply to a group entry with mixed intent?** §3.1 makes the
   flag per entry (§D3), and a `Group` entry expands to many authors at a pinned frontier, so the
   flag cannot distinguish "import Alice's backlog, not Bob's" when both arrive through the same
   group. Two entries with different `role_min` work only if the group separates them by role;
   otherwise the owner must split the group or accept all-or-nothing. Whether writer sets should
   instead carry per-*author* overrides for group-expanded members — which reintroduces exactly the
   key list a group entry exists to avoid — is a modelling choice that belongs with the view
   ownership questions. **→ #2028.**

---

## 9. Implementation routing (file as their own issues — nothing built here)

Sequenced so no slice enables a counting path before the set object and its versioning exist.
Each is independently reviewable and small enough to land alone.

1. **`idset` schema types + canonical body encoding + `chain::sign`/`decode` round-trip tests.**
   `WriterSet`, `Entry` (all three shapes, each carrying `include_history`, §D3), `Excludes`,
   `Rescind`, with golden-header tests in the style of `chain.rs:733-852`. Library only; no
   projection yet. A decode test asserts an entry written before the flag existed decodes as
   `include_history: false` — the prospective default — rather than failing.
2. **`expand(set_version) -> BTreeSet<AuthorKey>`**, plus **`expand_retro(set_version)`** — the
   same expansion narrowed to entries carrying `include_history` (§3.1), because §4.1's `retro` is
   per entry and one flagged friend must not import another's backlog. Frontier-aware via
   `is_member_at`/`access_at_position` (**not** `members(group)`, §4.5), one-level flattening and
   `role_min` filtering (§D3/D4). Property tests: expansion is order-independent, frontier-pinned,
   **unchanged by a later group-membership event** (the reproducibility guarantee),
   `expand_retro(v) ⊆ expand(v)`, and — the one finding 23 added — **a fork proof that invalidates a
   group grant changes the expansion, and the cache does not serve the stale one** (cache key is
   `(group, frontier, R)`, §4.5).
3. **`AuthorEvents` index** — kind-generic `(author, seq) → (event_id, kind, subject?)`, persisted
   and incremental, built by ranging the `events` table per writer (§4.2). Tests, keyed to §4.2's
   three queries:
   - a view query touches **only** the historical writer-set author ranges, the retro-admitting
     versions' past claims and the named exception IDs (the cost claim, not just the answer);
   - **non-tag kinds are indexed** — a `didit.record` must come back, which is where `TagIndex` fails;
   - a **removed** author's earlier claims are still retrieved (their range stays in the historical
     union), which is retrieval's half of §3.2 — the predicate cannot preserve them if they never
     reach it.
4. **`Projection` + `WhyNotCounted` + `UncountedHit` types and the `counted`/`uncounted` split**
   (§4.1–4.3), plus the separate `validity_now` query (§4.6). Tests below are keyed to specific
   defects review rounds actually found — clauses 1–5 of §4.1 and the retrieval rules of §4.2:
   - a stranger's `applied` claim is `uncounted`;
   - a claim counted under set v1 **stays** counted under v2 after its author is removed;
   - a **new** claim from a removed author, written after v2, is **not** counted (the interval
     semantics — clause 4's `admitted_at`);
   - a claim that **predates every set version** is uncounted by default, and **counted** once a
     version admits its author with `include_history: true` (clause 4's `retro` — the branch whose
     absence made the flag unimplementable);
   - a claim outside `subject_scope` **or** outside `kind_scope` is uncounted for that view but
     **still visible on the uncounted side**, while the same author's in-scope claim counts
     (clause 5 — scope is the predicate's job, not the retrieval filter's; the kind half is what
     catches the double-filter of finding 19, since a retrieval-side kind filter would make this
     claim unretrievable and the test would fail on the missing row rather than on the reason);
   - a fork-invalidated claim is **not counted even when named in `view.exceptions`** (`applied` is
     the outermost gate), and it appears in **neither** collection — `validity_now` reports it
     (§4.3), so no enum has to say "uncounted because invalid";
   - re-evaluating the same `(view config, set version, frontier, resolution)` after new claims
     arrive returns the **identical** total (frontier closure);
   - a **later-arriving fork proof** leaves a saved `project(view, R)` total unchanged and flips
     `validity_now` for the affected events — the pinned/live split of §4.6, so neither the stale
     total nor the silent rewrite happens;
   - a named exception for a **stranger's** claim is retrieved by ID and counted, even though the
     stranger is absent from the writer set and from the author-range scan;
   - a **rescind** version erases the claims it saw and leaves later ones alone (§3.3), and — the
     regression finding 22 caught — **a claim written after a rescind is still counted for every
     writer the rescind did not target.** `seen_by` quantifies over `adm_versions`, so a rescind is
     never mistaken for the admission state in force, and its empty `expand` cannot reject the
     world's claims;
   - the **emergency stop takes two acts** (finding 24): `entries=[Public], excludes=[Public]` in v2
     leaves v1-admitted claims counted, and a `target: Public` rescind then erases them — the test
     asserts the intermediate total is *not* zero, because an earlier draft claimed it was;
   - a claim written **before its author's first admission**, by an author admitted later without
     `include_history`, is uncounted with `HistoryNotImported` (finding 25) — the shape that fits
     neither `AuthorRemoved` nor "predates every version";
   - a **forked set version** is not a version: a set chain forked at `set_seq` does not make
     `seen_by` ambiguous, and a never-applied `idset.writer` cannot admit an author (finding 17 —
     the set-side counterpart of the fork test above);
   - **two purpose-specific sets on one owner chain do not leak into each other** (finding 26): dog
     set seq 1 admits Alice, chat set seq 2 admits `Public`, dog set seq 3 removes Alice. A
     stranger's dog claim written after seeing seq 2 but not seq 3 stays **uncounted** in the dog
     view — the chat version is not a version *of the dog set*, so `seen_by` skips it and the
     in-force version is the dog set's seq 1, which never admitted the stranger. The negative
     control is the same fixture with the `set_id` filter removed: `seen_by` resolves to seq 2, the
     claim counts, and the test fails — so the filter is exercised rather than assumed. A chat-set
     `rescind` erases **no** dog claim.
5. **`discover()` bounded and cursor-paginated, grouped by author, with viewer-local petname
   labels** (§5). Two flood tests, because they differ: 10k claims from one key → one cluster;
   **10k distinct keys → 10k clusters, one bounded page, honest `total_authors`** (the one that
   fails today).
6. **`Tier` decision function** (§D7) with the reference-holding rule (§6), built on
   `prune_behind_cut`'s existing "protect dependencies and active backlinks" contract
   ([`event_store/mod.rs:527-545`](../../pkgs/id/src/event_store/mod.rs)) — give it a
   counted/uncounted policy and a caller, rather than writing new pruning — and wired to *no*
   scheduler: the scheduler and all window values come from #2032/#2033. Tests:
   - a counted record is `Hot`; an uncounted stranger's claim is `Cold`;
   - **a view's policy inputs outlive its records' coldness** (finding 27): with
     `kind_scope = didit.record`, the view's `idset.writer` version is not counted by that view and
     has no owner pin, yet is `Hot` because it is a configuration input; pruning with a cut that
     would drop it either keeps it or refuses, and `project(view, R)` stays defined afterwards. The
     negative control is the same fixture with the root rule removed — `project` becomes undefined
     and the test fails on that, so the rule is exercised rather than assumed.
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
| 3 | Projection tested membership only in the *current* set snapshot, so removing an author un-counted their whole history — contradicting §3.2 and abuse case 7. | **Fixed, and it was the most serious.** §4.1's predicate became admission-history-aware (`∨ admitted(view, claim)` over the set's version prefix, in the round-1 form), with a test for it in slice 4. Rounds 2–4 refined that into `seen_by`/`admitted_at`; the requirement it introduced — retrieval must reach removed authors' claims — was only completed in round 4 (finding 13). |
| 4 | Group expansion prescribed `Members::members(group)`, a **current-state** query, so a later group change would alter an unchanged set version's expansion. | **Fixed.** New §4.5 requires frontier-aware `is_member_at`/`access_at_position`, states the replay cost honestly, and keeps `members(group)` for current-state questions only. |
| 5 | Author-grouping was claimed to answer a 10k-**key** flood, but 10k distinct keys yield 10k clusters. | **Fixed by narrowing the claim.** §5.1 now scopes grouping to single-key floods, specifies bounded cursor pagination with an honest `total_authors`, and **retains the multi-key flood as an explicitly unsolved risk** routed to OQ-7 / #2032-#2033. The abuse-case table's row 2 verdict was downgraded accordingly. |
| 6 | `TagIndex` covers tags only, so the did-it tracker's `didit.record` events would project to nothing. | **Fixed.** §4.2 now specifies a kind-generic `AuthorEvents` index built by ranging `events` per author, with `TagIndex` demoted to a tag-specific adapter on top; slice 3's test asserts a `didit.record` comes back. |

**Review round 2** (Codex, on the round-1 fix commit) found three defects **introduced or left by
the round-1 fix**, all accepted and fixed. Recorded because they are instructive: the round-1
patch to §4.1 traded one bug for another.

The round 2 and round 3 rows below describe the predicate **as it stood in that round** (`v_at`,
`∨ admitted(view, claim)`). Round 4 replaced §4.1's formulation wholesale with `admitted_at`/`retro`
rather than patching it again; the history is kept because it is why the final form is a single
statement instead of a stack of clauses.

| # | Finding | Disposition |
|---|---|---|
| 7 | The round-1 `∃ v. author ∈ expand(v)` form admitted a removed author's **new** claims indefinitely — one old admission kept granting future counting, contradicting §3.2. | **Fixed.** §4.1 was rewritten to resolve `v_at(view, claim)` — the set version in force **at the claim's own causal position** — instead of existential quantification over all versions. That got both directions right: old claims stay, new ones stop. Round 4 restated the same intent as `admitted_at`. |
| 8 | `∨ claim ∈ exceptions` sat **outside** the `applied` gate, so an exception could count a fork-invalidated claim. | **Fixed.** `applied` is now the outermost conjunct; exceptions can lift the *writer-set* requirement for one claim, never store-wide validity. |
| 9 | The predicate had no claim-frontier restriction, so re-evaluating a saved `(set version, frontier)` after new claims arrived returned a different total. | **Fixed.** Added `past(view.frontier, claim)`. Pinning the set version fixes membership only; §3.4's `frontier` must also bound the claim set or the pair is not reproducible. §3.4 now says so explicitly. |

**Review round 3** (Codex, on the round-2 fix commit) found three more, all accepted and fixed.
That the spec took three rounds is the honest record of what a projection predicate costs.

| # | Finding | Disposition |
|---|---|---|
| 10 | `v_at` tested `claim ∈ causal_past(v)` — the causal relation in the **wrong direction** — so an ordinary prospective claim (written *after* the admitting version, therefore not in its past) resolved to `v_at = undefined` and was reported uncounted. | **Fixed.** Inverted to `v ∈ causal_past(claim)`: the set version the author had already seen when writing. §4.1 also explained the two edge cases this distinguishes — `v_at` undefined = predates every set, uncounted by default; defined = that version's expansion decides. Round 4's `admitted_at` keeps the inversion and adds an explicit `retro` branch, so "predates every set but is covered by `include_history`" is a counted case rather than an undefined one. |
| 11 | Exceptions are deliberately for **strangers**, but candidate retrieval was restricted to writer-set author ranges — so the excepted claim was never fetched and the exception could never fire. | **Fixed.** §4.2 now specifies two retrieval paths: author-range scan over the expanded set **plus** a direct `event_by_id` lookup per `view.exceptions` entry (O(log n) each, bounded by the owner's list). The cost test was restated to permit them rather than forbid the feature. |
| 12 | Provenance named only *(set version, frontier)*, so two views differing in `subject_scope`/`kind_scope`/`exceptions` could produce different totals under identical provenance. | **Fixed.** §3.4's provenance became `(view_id, config_seq, set_id, set_seq, set_head, frontier)` in the round-3 fix, making a projection a pure function of *(view config, set version, frontier)*. Round 4 added `resolution` (finding 16) and round 5 established that `view.exception` must advance `config_seq` or the `C` half of the pin is asserted rather than enforced. |

**Review round 4** (Codex, on the round-3 fix commit) found that **retrieval and the predicate had
drifted apart in three places, and that one input was missing from provenance entirely**. All four
accepted. This is the round that ended the clause-by-clause patching: §4.1 is now stated once, in
full, over a named input tuple.

| # | Finding | Disposition |
|---|---|---|
| 13 | Candidate retrieval scanned only the **current** expansion, so a removed author's earlier claims — which §3.2 and `admitted_at` say stay counted — were never fetched, and the predicate could not preserve what it never saw. | **Fixed.** §4.2's source 1 is now the **union of `expand(v)` over `v.seq ≤ S.set_seq`**, i.e. every author *historically* admitted. Slice 3 gained a test that a removed author's earlier claims are still retrieved. |
| 14 | The predicate had **no retrospective-admission branch at all**, so `include_history: true` (§3.1) could not work: a claim predating every set version resolved to "no admission in force" and stayed uncounted no matter what the set said. | **Fixed.** §4.1 now has two explicit paths, `admitted_at` (prospective, at the version the author had seen) and `retro` (a version with `include_history` whose causal past contains the claim and whose expansion admits the author). §4.2 source 2 makes those claims reachable. Slice 4 tests both directions. |
| 15 | **`subject_scope` was never applied.** It was in the view configuration from the first draft, but only `kind_scope` filtered anything, at retrieval — so a view scoped to one dog counted the other dog's walks from the same author. | **Fixed.** §4.1's clause 5 `in_scope` applies **both** scopes to every candidate. Round 5 found the half-fix left `kind_scope` applied *twice* (index and predicate), which made `OutOfScope(kind)` unreachable and hid filtered claims from the uncounted side; §4.2 now applies neither scope at retrieval. |
| 16 | **Resolution state was not in the provenance tuple.** `APPLIED` is rebuilt against every known fork proof on each `refresh` ([`event_store/ingest.rs:890-937`](../../pkgs/id/src/event_store/ingest.rs)), so a fork proof arriving later flips `applied` and silently changes a saved total while `(view config, set version, frontier)` still names the same inputs. | **Fixed.** `resolution` is now an input to `counted` (§4.1) and part of §3.4's provenance, and the new §4.6 separates the pinned `project(view, R)` total from a live `validity_now(event_id)` query, so neither a stale number nor a silent rewrite is the outcome. |

Round 2 is also why §4.1's predicate is presented with a numbered justification per clause rather
than as bare pseudocode, and why slice 4's tests are keyed to specific past defects rather than
to the happy path. Round 4 is why the predicate is now one statement over an explicit input tuple
`(C, S, F, R)`: three rounds of local patches each left retrieval, or provenance, or one scope,
behind the part that had just been fixed.

**Round 5** was a self-review of the round-4 rewrite against the substrate, done before pushing it,
and it found five defects — three of them in code (`chains`/`forks`) the earlier rounds had not
consulted. All accepted and fixed.

| # | Finding | Disposition |
|---|---|---|
| 17 | `seen_by` took `max` over **every** `idset.writer` event in the store. A set chain that forked at a `set_seq` gives two versions that `seq`, so `max` was ambiguous and §4.1 stopped being a function — and a forked, never-applied set version could still admit an author or satisfy `retro`, contradicting clause 1's own fork discipline. | **Fixed.** `versions(S)` is now explicitly the owner's **applied prefix under `R`**, which `chains`/`forks` already make linear ([`event_store/mod.rs:90-91`](../../pkgs/id/src/event_store/mod.rs)). `max` is a function again, and the set's history is gated the same way claims are. |
| 18 | Clause 3's `rescinded(S, c)` was **used but never defined**: §3.3 gave prose and a `through` field but no semantics, so the predicate rested on an undefined term. Relatedly, a rescind is "a new set version", so it would have entered `versions(S)` where `expand(_)` is meaningless for it. | **Fixed.** §3.3 now defines `rescinded` formally — target admits the author, the claim is in the rescind's causal past, `through` bounds it, `from_version` gates it — and states that a rescind carries no entries, so `expand(r)`/`expand_retro(r)` are empty and only clause 3 reads it. |
| 19 | `kind_scope` was applied **twice** (index filter and `in_scope`), so `OutOfScope(kind)` was unreachable and every claim the owner filtered by kind vanished from the uncounted side — the "uncounted must not be a bin" failure. | **Fixed.** §4.2 applies **neither** scope at retrieval; the index records `kind` so the predicate can test it cheaply. Round 4's finding 15 was half a fix. |
| 20 | Two `WhyNotCounted` variants were unreachable: `AuthorNotInSet` (such an author is in no historical range, so their claims are never candidates) and `BackupNotImported` as worded. | **Fixed.** `AuthorNotInSet` removed; the clause-4 reasons that *are* reachable — `AuthorRemoved` and `HistoryNotImported` — are worded to their actual conditions (round 7 renamed the second, finding 25). An arm a test can never reach is a defect in a closed enum. |
| 21 | §4.4 claimed an excepted claim "still appears on the uncounted side for any other view that did not name it". Under §4.2 that is false: a stranger's claim is fetched only by an author range or that view's own exception list. | **Fixed by narrowing.** §4.4 now states the true reach — other views that *admit* the author still show it uncounted, and a third party who neither admits nor excepts it never sees it, which is §5.3's whole point rather than a gap. |

Two smaller corrections rode along: §7 row 1 still explained case 1 with the current-snapshot test
finding 3 had declared a defect (now stated as "no version in the applied prefix admitted them"), and
§4.2's unconditional cost claim is now qualified for `Entry::Public`, which admits every author and
so makes source 1 store-shaped.

Round 5 is also why `versions(S)` is spelled out in the pseudocode rather than left as
`versions(set_id)`: the set's own fork discipline is as load-bearing as the claim's, and three
rounds of review had only ever examined the latter.

**Round 6** (Codex, on the round-5 fix commit) found two P2s, and **both were defects round 5
introduced** — which is the clearest evidence in this document that the clause-patching style was
the problem, since even a rewrite leaves a seam. Both accepted and fixed.

| # | Finding | Disposition |
|---|---|---|
| 22 | Defining `expand(r)` as empty for a rescind (§3.3) while `seen_by` quantified over **all** versions meant a claim written after a rescind selected that rescind as the version in force — which admits nobody — so `admitted_at` rejected **every** admitted writer's claims until the owner published another `idset.writer`. A rescind would have been a global outage of the metric, not a targeted erasure. | **Fixed.** §4.1 now separates `versions(S)` (all applied set versions) from **`adm_versions(S)`** (`idset.writer` only), and `seen_by`/`retro` quantify over the latter. A rescind acts through clause 3 alone, which is what §3.3 always intended. Slice 4 gained the regression test. |
| 23 | `expand(v)` was cached by `(group, frontier)`, but a group's expansion is **not** a pure function of its causal past: `resolve` drops fork-excluded membership actions before the layer sees them, replacing the role with `Role::Other` ([`event_store/resolve.rs:82,124-133`](../../pkgs/id/src/event_store/resolve.rs)). A later fork proof could therefore change an unchanged group at an unchanged frontier, and the cache would serve pre-proof membership to a projection under a newer `R`. | **Fixed.** §4.5 requires replay **under `R`** and gives the cache key as `(group, frontier, R)`. Immutability within one resolution state is kept; across states it must recompute. The number of live `R` values is small, so this is a bounded multiple of the replay cost. |

Round 6 is why the doc no longer says a frontier is sufficient to make anything permanent: §D4, §4.1
and §4.5 each pin a frontier, and only the pair *(frontier, `R`)* makes a replay reproducible.

**Round 7** (Codex, on the round-6 fix commit) found two more, both consequences of the
historical-admission predicate rather than of the round-6 fixes. All accepted.

| # | Finding | Disposition |
|---|---|---|
| 24 | §D6 called `entries=[Public], excludes=[Public]` the emergency "stop counting everything" switch, but §4.1's `seen_by` resolves each claim against the latest admitting version in **its own** causal past — so claims Alice wrote while v1 admitted her keep resolving to v1 under an empty v2. The switch stopped *prospective* admission and left existing totals populated. | **Fixed by narrowing the claim, not the mechanism.** §D6 now says it admits nobody from that version on, and names the two-act stop: the empty version **plus** an `idset.rescind` with `target: Public` (§3.3). Keeping the acts separate is deliberate — "stop admitting" and "erase what was admitted" stay independently auditable, which is §0's fifth principle applied to the emergency case. |
| 25 | `BackupNotImported` as worded ("predates every version") did not cover a claim written while v0 admitted nobody, by an author admitted later **without** `include_history`: v0 never removed them, and the claim need not predate every version, so no `WhyNotCounted` variant fit and an implementation could not construct the hit without misreporting why. | **Fixed.** Renamed `HistoryNotImported` and defined by the *cause* — the author's history was not imported — so it covers both shapes. `expand(v)` is now also defined once in §4.1 (entries minus excludes), since §D6, `rescind`'s target test and this variant all need the same expression. |

Round 7 is why the variants are named by cause rather than by instance: `HistoryNotImported` covers
two distinct histories that a reviewer will construct separately, and naming it after either one
leaves the other unrepresentable.

**Round 8** (Codex, on the round-7 fix commit) found two more P2s, and again both live in seams the
earlier rounds left open. Accepted.

| # | Finding | Disposition |
|---|---|---|
| 26 | `versions(S)` was "the owner's `idset.writer` applied prefix" with **no `set_id` filter**, and `adm_versions` narrowed only by kind. OQ-2 explicitly permits several purpose-specific sets on one owner chain, so the prefix interleaves them: dog set seq 1 admits Alice, chat set seq 2 admits `Public`, dog set seq 3 — a stranger's dog claim observing seq 2 but not seq 3 resolved `seen_by` to the *chat* version and wrongly counted. The same hole let a chat `rescind` erase dog claims. | **Fixed at the definition, not at a call site.** `versions(S)` is now *"applied prefix under `R` **and** `body(v).set_id = S.set_id`"* (§4.1), so both `adm_versions` and the `rescind` quantifier in §3.3 inherit the filter. §D2's "contiguous prefix" became "subsequence", which is what it always was once one owner can publish two sets. Slice 4 gained the two-set leak test with its negative control. |
| 27 | §D7's `Hot` rule kept only what is *counted*, pinned, or named by an exception. A view's `idset.writer` event is not counted under its own `kind_scope` (a `didit.record` view does not count an `idset.writer`), so absent an owner pin it goes cold while its counted records stay hot — and the record cannot causally reference a *later* admission, so the "referenced by a hot object" rule does not save it. Expiry could delete a served view's own input and make its projection undefined. | **Fixed.** §D7 now makes **every served view's configuration and the writer-set/rescind versions its projection reads** retention roots independent of whether they count, expressed as extra `protected` roots on `prune_behind_cut` rather than a new mechanism. Slice 6 gained the fixture whose negative control is the projection going undefined. |

Round 8 is why "counted" cannot be the only definition of hot: countedness is a property *of the
view*, and a view's inputs are not claims it counts. The retention model needs `roots = counted ∪
pinned ∪ excepted ∪ config inputs`, and the fourth term is the one that only appears when you try
to re-derive a served number after pruning.

Three citation errors found in self-review were also corrected (`chain.rs` decoder limits are at
`:41-42`; the `TagIndex` rebuild note is `:11-12,16`; `TagIndex.by_author` is `:46`). All internal
links were checked programmatically after every round and resolve.

**A note on what eight rounds actually shows.** Every round found a real defect, and rounds 2–8 each
found at least one that an earlier round had *introduced*: an interval predicate patched into a
snapshot, an exception placed outside a gate, a scope applied twice, a `max` over a non-linear
sequence, a rescind mistaken for an admission state, a cache keyed on a frontier without the
resolution state, a version history that was the owner's rather than the set's, and a hot-set rule
that excluded the view's own inputs. That is the argument for the shape this document settled on —
one statement over a fully-named input tuple, with retrieval specified to match it — rather than for
confidence that the current text is defect-free. A reviewer should treat §4.1 as a claim to attack,
not as settled.

Five places are the most likely to still be wrong, and naming them is more useful than a claim of
completeness:

1. **`retro` against a set that later revokes the flag** (§3.1). The import is monotone by design and
   only `rescind` retracts it. That is a deliberate choice, but it means "I removed `include_history`"
   does not do what an owner may expect, and the only affordance is a second signed act.
2. **The cost bound for a `Public` set** (§4.2), where the complexity claim is explicitly qualified
   rather than proved, because `Entry::Public` admits every author and collapses source 1 into a
   store-shaped scan.
3. **The interaction between "in force at the claim" and "what the owner says now"** (§3.2, §3.3,
   §4.1, §D6). Every round from 2 onward produced a finding in this seam: an interval predicate, a
   removal that did not stop new claims, a rescind mistaken for an admission, and an emergency stop
   that stopped admitting without erasing. The rule is consistent — the claim is judged by the state
   *it* saw, and changing the past takes an explicit `rescind` — but that rule has more visible
   consequences than any other in the document, and each one was found by a reviewer rather than by
   the author.
4. **The resolution-state plumbing** (§4.5, §4.6, §D4). `R` has to reach more places than any other
   input — the predicate, group replay, its cache key and the provenance — and rounds 5 and 6 each
   found a place it had not reached. Any remaining input evaluated without `R` is a reproducibility
   bug of the same shape as findings 16 and 23.
5. **The retention root set** (§D7, §6). Counting is a property of *this view*, so "hot iff counted"
   can never protect the inputs a view is defined over; round 8 found that and the fix is stated as
   an open shape (`counted ∪ pinned ∪ excepted ∪ config inputs`) rather than as an implemented
   predicate. Where the root set is computed — per server, per served view, or from a durable index
   of served views — is not specified, and a server that loses that index loses the guarantee.

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
