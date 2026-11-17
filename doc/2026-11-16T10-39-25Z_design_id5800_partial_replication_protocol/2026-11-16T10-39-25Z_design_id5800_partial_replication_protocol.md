# Partial replication protocol — subset snapshot, scoped updates, existence privacy, dangling references

Design record for issue #5800 (developing-today/code).
Date: 2026-11-16. Status: Decision (recorded by queue task 16841, lane any-d).
Parent: #5672 (closed). Programme: epic #2056. Fixed inputs: #5799 grants-model
decision record (2026-11-05, task 10716), #5678 read-authority order (via #5799).

This record settles the protocol mechanics #5800 owes: how a granted subset
moves from an issuer to a peer — the subset snapshot, the scoped update
stream, the existence-privacy mechanics, and what happens to references that
point outside the slice. Implementation timing is a separate decision (the
issue notes it may be months out); this record is the design only.

## Fixed inputs (not re-decided here)

From #5672: selective sharing exists so a server shares _subsets_ instead of
all-or-nothing replication; the existence-privacy rule (a share must not leak
what it does not include); the receiving side may cap, pause, or drop a
share; revocation cannot un-copy plaintext a peer already holds.

From #5799 (grants model): grant units are the individual object and the
stable collection; operations are `read`, `subscribe` (requires `read`),
`write`, `append`, each explicit; durations are `standing`, `expires_at`,
and `snapshot_once` (one initial snapshot, no subscription or later
re-fetch; retries belong to that same transfer); revocation stops future
replication at the next authorization check; delegation is attenuated and
off by default. #5799 invariant 6 binds this protocol directly:
unauthorized and nonexistent resources are indistinguishable in
protocol-visible results, and partial replication exposes no
withheld-object inventory. #5799 also assigns the dangling-reference
treatment and the exact snapshot/update protocol to this record.

## Decisions

### 1. Subset snapshot: grant-evaluated, self-describing, scope-only

A snapshot is the set of objects the grant covers **at the snapshot cut**,
evaluated by the #5678/#5799 resolver at cut time — not a database dump
filtered afterwards. The bundle is self-describing and contains only:

- the grant id it answers and the issuer's snapshot epoch (a monotonic
  per-issuer sequence number stamped at the cut);
- a manifest listing **only the included objects** (id + content hash), and
  the objects themselves;
- for each object, its references rewritten per §4 below.

Later additions to a granted collection are **not** in the snapshot: they
reach the peer only through scoped updates (§2), and only when the grant
includes `subscribe`. A `snapshot_once` grant's snapshot is the whole
transfer, per #5799. Transfer is chunked with per-chunk acknowledgement
keyed by (grant, epoch, chunk), so an interrupted snapshot resumes the
same transfer instead of restarting or re-cutting at a newer epoch.

### 2. Scoped updates: a per-grant stream, epoch-ordered

For grants with `subscribe`, updates flow as a stream keyed by
(grant id, epoch):

- each update carries the object id, the operation (`upsert` / `delete`),
  and its epoch; the peer applies updates in epoch order;
- an update is emitted **only** for an object inside the grant's scope at
  emission time, re-evaluated per update — scope is not frozen at the
  snapshot cut;
- a **delete** inside scope is a real tombstone: the peer holds the object
  legitimately and must learn it is gone;
- when an object **leaves scope** without being deleted (revocation,
  expiry, removal from the collection, policy change), the peer receives a
  scope-exit marker for that object — and nothing else: the marker does
  not say why, because the reason can itself describe withheld state;
- grant termination (revocation, expiry, peer drop) ends the stream; a
  peer that detects an epoch gap resynchronises with a **fresh snapshot
  of the current grant scope** — never by requesting ranges, which would
  let it probe outside the slice.

Write-back under `write`/`append` grants travels the same stream in
reverse and is authorized per update by the same grant; conflict policy
for concurrent writes belongs to the data layer (#2048), not this
protocol — the protocol's obligation is only that nothing moves outside
the grant's operation set.

### 3. Existence-privacy mechanics

The #5672/#5799 rule is enforced by construction, not by filtering a
wider feed:

- **Scope-only content.** The snapshot manifest and the update stream
  contain granted objects and nothing else — no withheld ids anywhere in
  either, including in error paths and resync.
- **No totals.** Neither the snapshot nor the stream carries counts of
  the source collection or database, "N of M" progress, or
  withheld-object statistics. Transfer progress, if shown, is computed
  from included bytes/objects only.
- **Indistinguishable absence.** Absent and existing-but-ungranted
  produce the same protocol-visible result everywhere (queries, lookups,
  resync responses), per #5799 invariant 6.
- **Opaque placeholders.** Reference rewriting (§4) never embeds the
  target's identity.
- **Side channels named, not promised.** Timing and total-size
  correlation are not closed by this protocol in v1; batching/padding is
  an implementation choice, and no invariant below claims otherwise.

### 4. Dangling references: redacted placeholder by default

When an included object references an object outside the slice, the
reference is delivered as a **redacted placeholder**: the referencing
field is present, carrying an opaque token and no target data. The token
is a per-grant keyed hash of the target id, so:

- two fields in the same slice pointing at the same ungranted object
  render as the _same_ placeholder (intra-slice linkability is kept —
  the peer can tell its slice references one shared external thing);
- the token reveals neither the target's id nor anything correlatable
  across grants (each grant salts differently), so placeholders cannot
  be used to enumerate or cross-reference withheld objects.

Per data type, exactly two deviations are permitted, both explicit:

- **Omission** — the field is dropped instead of placeholdered — only
  where the data type's schema marks the reference optional; recorded
  per data type, never improvised per object.
- **Closure** — the referenced object is pulled into the slice — never
  implicit: only when the grant itself carries an explicit closure flag
  set at grant creation (off by default, recorded with the grant). This
  composes with #5799 invariant 3 (a collection grant cannot escape its
  scope): closure is the grant's own stated scope, not the protocol
  widening it.

If a placeholder's target later enters scope (grant widened, object
added to the collection), the next scoped update for the referencing
object resolves the placeholder to the real reference; the placeholder
token lets the peer match the resolution to what it already holds.

### 5. Receiving side

Per #5672, the peer may cap accepted share size/count, pause a share, or
drop it. A drop deletes the local replica by the peer's own choice and
reads to the issuer as subscription termination, not an error. Caps are
enforced at snapshot-chunk and update boundaries: hitting a cap pauses
the transfer with the (grant, epoch) position retained, so resumption
does not re-cut the snapshot.

## Alternatives rejected

- **Full snapshot + peer-side filtering.** The peer receives everything
  and discards the ungranted — existence privacy is violated outright,
  before any filtering happens. The protocol exists to make this
  unnecessary.
- **Closure by default.** Auto-including referenced objects widens every
  grant beyond its stated scope and contradicts #5799 invariant 3; it
  also makes share size unpredictable for the receiving side's caps.
- **Identity-revealing placeholders** (embedding the target id so the
  peer can "fetch it later if granted"). Leaks the existence and
  identity of withheld objects — precisely the enumeration #5799
  invariant 6 forbids — and invites probing.
- **Omitting all dangling fields.** Silently lossy: the peer cannot
  distinguish "no reference" from "reference withheld", and slices stop
  being schema-consistent. Placeholders keep the distinction visible
  without leaking the target.
- **Issuer-wide change feed, filtered at the peer.** Same leak as a full
  snapshot, stretched over time: the feed's shape (rates, ids, delete
  patterns) describes the withheld set.
- **Counts/totals for progress and planning.** "3 of 41 objects" tells
  the peer the collection holds 41; the withheld set's size is withheld
  information.

## Invariants implementation must hold

1. Every byte of a snapshot or update is authorized by the grant
   (subject, scope, operation, lifetime) at the moment it is emitted,
   evaluated by the #5678/#5799 resolver — there is no second,
   protocol-private authority path.
2. Snapshot membership is the resolver's answer at the cut; stream
   membership is the resolver's answer at each emission.
3. No protocol message — manifest, update, error, resync, progress —
   contains a withheld object's id, count, name, metadata, or a token
   correlatable to it across grants.
4. Absent and ungranted are indistinguishable in every protocol-visible
   result.
5. Deletes of held objects are delivered as tombstones; scope exits are
   delivered as reason-free markers; the two are never conflated.
6. Revoked or expired grants emit nothing further, including resync
   responses and placeholder resolutions.
7. `snapshot_once` delivers exactly one snapshot cut; retries resume
   that cut's chunks and never re-cut.
8. Dangling references are placeholders by default; omission only per
   data-type schema; closure only under an explicit grant flag.
9. A peer-side cap or drop never obliges the issuer to reveal more to
   "complete" a transfer; a paused transfer resumes from its retained
   (grant, epoch) position.

## Divergence check against the parents

None. Nothing here changes a #5672 or #5799 decision: the grants model,
the authority order, the revocation boundary, and the existence-privacy
rule are consumed as fixed. The epoch/scope-exit mechanics, the
per-grant placeholder token, and the chunk-resume rule are new detail
inside the space #5799 explicitly left to this record ("the treatment of
dangling references/redacted placeholders and the exact
partial-snapshot/update protocol remain #5800's decision").

## Splits and follow-ons

No further splits are filed: the three questions the issue names are
settled above, and the neighbouring designs already exist as their own
issues (encrypted shares #5801, public-share UX #5802). Implementation —
when its timing is decided — will touch the grants resolver surface, the
capability scopes in `pkgs/id/crates/id-caps`, and the ID relay transport
(#2036); none exists as replication code today, so this record is the
contract that work builds against.
