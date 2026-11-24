# Per-object talk page and changelog data model (#5834)

Decision record for #5834, the census child of #5677 (note and board
reference model, merge model, sovereignty rule, approval flow). It settles
the data model for per-object talk pages and changelogs: shape, attribution,
and append-only rules. It does not implement anything.

Status: proposed for review. Parent decisions are restated, not re-decided.
The one place this record touches a parent non-goal is called out in
[Parent reconciliation](#parent-reconciliation).

## Question

#5677 says every object has a talk page (wiki-style discussion, distinct from
content) and a changelog (every edit, fork, mark and approval, attributed and
timestamped). It does not fix what a talk-page post or a changelog entry is,
how each is attributed, or what "append-only" forbids. Implementation needs
those answers before any schema exists.

## Decision

**Talk page = an append-only thread of posts attached to an object id.**
Each post is an immutable record:

- `post_id` (content hash of the canonical record), `object_id`,
  `parent_post_id` (reply threading; null for a top-level post),
- `author` (the signing identity of the user or agent, never a display name),
  `created_at` (author's clock, plus the server receipt time),
- `body` (text, with optional references to notes and boards per #5677 §1),
- `supersedes` (optional, the `post_id` of an earlier post this one corrects).

A correction is a new post that `supersedes` the old one. Nothing is edited in
place. A display layer may collapse a superseded post, but the original stays
in the store and in the changelog.

**Changelog = an append-only, hash-chained event log per object.** Each entry:

- `seq`, `object_id`, `prev_hash` (hash of the previous entry for that object),
  `hash` (over the canonical entry, including `prev_hash`),
- `kind` one of `edit`, `fork`, `mark`, `merge_proposal`, `merge`, `approval`,
  `reject`, `session` (a multiplayer session that ended), `redact` (see below),
- `actor` (signing identity, required; a kind with no actor is invalid),
  `at` (author time and receipt time),
- `refs` (the edit, fork, post or session it concerns, by content hash),
- `summary` (short human text, optional).

The changelog records **semantic events**, not CRDT operations. A multiplayer
session writes one `session` entry at its end that names the op-log range it
covers. The fine-grained ops stay in the merge layer; the changelog stays
readable and small.

## Attribution rules

1. Every post and every changelog entry is signed by its author's identity
   key. Anonymous writes are not a shape the model can express.
2. An entry's `actor` is the identity that signed it. A proxy (a bot or an
   agent acting for a user) signs as itself and names the user it acts for in
   `refs`, so "user N edited this page" is recoverable.
3. Attribution cannot be changed or removed by anyone, including the object
   owner and the community. Sovereignty (#5677 §7) allows forks and visibility
   changes; it does not allow rewriting who did what.

## Append-only rules

1. No update or delete of a post or changelog entry. Corrections are new
   entries (`supersedes`, or a new changelog event).
2. Hashes chain per object. A gap or a changed hash is detectable on read, and
   a verifier can replay the chain from any retained checkpoint.
3. Visibility is the only gate on writing a talk-page post: whoever can see
   the object can post. Approval flow (#5677 §5) governs the object's effective
   version, not whether a post is allowed.
4. Spam and graffiti controls act on ranking and per-reader filters, never on
   the stored record (#5677 §7).

## Redaction (explicit, not silent)

#5677 names takedown and erasure as a non-goal that must be reconciled with
append-only history. This record resolves the reconciliation for talk pages
and changelogs as follows, and leaves full erasure to its own design:

- A `redact` changelog event replaces a post's or entry's **body** with a
  tombstone, keeps `actor`, `at`, `hash` of the removed body, and the
  `prev_hash` chain, and records the redacting actor and a reason code.
- The chain stays verifiable (the hash of the removed body is retained), so
  the record still shows that something was there, by whom, and when it was
  removed.
- Redaction does not remove attribution. It is the only removal, and it is
  itself an attributed, append-only event.

Whether a redacted body must be physically destroyed (for example for legal
erasure) is a separate decision. It is filed as a follow-on and is out of
scope here.

## Alternatives rejected

- **A mutable wiki page with revision history.** Revisions would be derived
  from a mutable page, so the page could be changed without a trace. Rejected:
  the history must be the source of truth.
- **A changelog table without a hash chain.** Simpler, but a database admin
  could edit history with no detectable trace. Rejected: tamper evidence is a
  requirement for attribution to mean anything across communities.
- **The CRDT op log as the changelog.** It is complete but unreadable, and it
  mixes merge mechanics with user-visible history. Rejected for the changelog,
  kept as the merge layer's own store; the changelog references it.
- **Talk posts as editable comments with a "last edited" stamp.** Rejected: an
  edit with no preserved prior body breaks the append-only promise.

## Invariants implementation must hold

1. A post and a changelog entry, once written, are never mutated.
2. Every changelog entry names exactly one actor, and that actor signed it.
3. The changelog chain per object verifies end to end from its first entry.
4. A `redact` entry keeps the removed body's hash and the actor; nothing about
   who acted is removed.
5. Visibility governs who may write a talk-page post; approval governs the
   effective version, and neither removes history.
6. Multiplayer sessions appear in the changelog as one `session` entry each,
   referencing the op-log range.

## Parent reconciliation

- No #5677 decision is changed: the note↔board model, the merge model, the
  sovereignty rule and the approval flow are used as stated.
- One **new** decision is recorded here, not silently forked from #5677: a
  `redact` event is the only removal from talk pages and changelogs. It goes
  beyond #5677's "deleting history" non-goal, which only forbids erasure. It
  is flagged for owner review and does not block the data model.

## Proposed slices (not yet filed)

Each slice is a child of #5834's decision, to be filed as an implementation
issue when the model is accepted:

1. Post and changelog record types, canonical hashing, and the chain verifier.
2. Talk-page read and write over the existing ID query surface, with
   `supersedes` display collapse.
3. Redaction event and tombstone rendering.
4. Session-end changelog entry linked to the multiplayer op-log range (after
   #5677 §3 lands).

Splitting further or filing these issues is left to the design PR's follow-up,
so this record stays a decision, not a backlog.
