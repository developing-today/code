# Physical erasure of redacted talk-page and changelog bodies (#14278)

Decision record for #14278, the follow-on that the #5834 talk-page and
changelog record deferred
([source record](../2026-11-23T21-10-00Z_design_id5834_talk_page_changelog/2026-11-23T21-10-00Z_design_id5834_talk_page_changelog.md),
landed by PR #13756, merge SHA `1d964139d48975bb860ca19752bd43399e0efd7f`).
Parent: #5677. Epic: #2056.

Status: proposed for review. It refines one invariant of the #5834 record
(invariant 4, below) before any schema ships; slice 1 (#14274) has not
landed, so the refinement costs nothing to adopt.

## Question

The #5834 record settles redaction as a tombstone: a `redact` changelog
event replaces a body, keeps the actor, the time, the removed body's hash
and the `prev_hash` chain. It leaves open:

1. Whether a redacted body must ever be physically destroyed, and on what
   triggers.
2. If so, what happens to the retained body hash, the per-object chain
   (invariant 3: end-to-end verification; invariant 4: a `redact` entry keeps
   the removed body's hash and actor), replicas and backups.
3. How that composes with #5677's takedown/erasure non-goal.

## Decision

**Yes: physical destruction of a redacted body is supported, for body bytes
only, on three triggers. Attribution and the chain are never destroyed.**

### 1. Triggers

| Trigger                                                             | Who may invoke                              | Effect                               |
| ------------------------------------------------------------------- | ------------------------------------------- | ------------------------------------ |
| Legal erasure (a verified legal obligation to delete personal data) | The operator of the store holding the bytes | `redact` then `erase`                |
| Author's own request for their own post body                        | The post's author (its signing identity)    | `redact` then `erase`                |
| Unlawful content (material the operator may not keep)               | The operator                                | `redact` then `erase`                |
| Community or object-owner takedown                                  | Per #5677 §7                                | `redact` only (tombstone, body kept) |

The object owner cannot erase someone else's post: sovereignty (#5677 §7)
covers forks and visibility, not destroying another author's words. A
community takedown hides the body and keeps it, so the decision stays
reviewable.

### 2. Commit to bodies with a salted commitment, not a bare hash

A bare `hash(body)` kept forever defeats erasure for short or guessable
bodies: anyone holding a candidate text can confirm it was the erased body.
So every post and changelog body is committed to as

```
body_commitment = H(domain_tag || salt || body)
```

with a fresh random 32-byte `salt` per body, stored **next to the body, not
in the chained entry**. The canonical entry (and therefore `hash` and the
next entry's `prev_hash`) covers `body_commitment` only.

Destroying a body means deleting the body bytes **and** its salt. The
commitment stays in the chain, so:

- **Invariant 3 holds.** The chain still verifies end to end: no entry's
  canonical bytes change.
- **Invariant 4 is refined, not broken.** A `redact` entry keeps the removed
  body's _commitment_ (which is what "the removed body's hash" now means) and
  the actor. After erasure, nobody, including the operator, can confirm a
  candidate body against it.

### 3. `erase` is its own attributed changelog event

Erasure is recorded, never silent. A new changelog `kind`:

- `erase`: `refs` names the `redact` entry it completes; it carries the
  erasing actor, the trigger reason code from the table above, and `at`.
- `erase` is valid only after a `redact` of the same body. A verifier rejects
  an `erase` with no prior `redact`.
- After `erase`, the body store returns "erased" for that commitment; a
  verifier treats an erased body as present-by-commitment, so verification
  does not need the bytes.

This keeps the #5834 append-only rules: nothing in the chain is updated or
deleted; the body store, which was never part of the chain, is where bytes
are removed.

### 4. Replicas and backups

- **Bodies live in a separate body store** keyed by `body_commitment`, apart
  from the changelog chain. Replication carries chain entries and bodies
  separately, so a replica that applies an `erase` deletes the body and salt
  and keeps every chain entry.
- **Backups use per-body encryption keys (crypto-shredding).** Each body is
  stored encrypted under its own key kept in the body store's key table.
  Erasure destroys the key with the salt. Old backups then hold ciphertext
  nobody can open, without rewriting backup archives.
- **Restore replays erasures.** A restore applies every `erase` event in the
  restored chain to the restored key table before the store serves reads, so
  a key restored from a backup older than the erasure is destroyed again.
- **Peers outside this operator's control** receive the `erase` event as a
  request. The operator can prove the request was sent, not that a foreign
  peer complied. Product disclosure must say so; it must not promise global
  deletion.

### 5. Composition with #5677

#5677 lists "deleting history" as a non-goal. This decision keeps it: the
record of _who_ did _what kind_ of thing _when_ is never deleted. What can be
destroyed is the content of one body, after it has been redacted, on one of
three named triggers, with the destruction itself recorded. Tombstoning
(#5834) stays the only community takedown tool.

## Alternatives rejected

- **Never destroy bodies.** Simplest and fully verifiable, but leaves no way
  to meet a legal erasure obligation or to stop holding unlawful material.
- **Rewrite the chain on erasure (recompute hashes from the erased entry
  on).** Breaks invariant 3 and makes every replica's chain disagree.
- **Keep a bare body hash.** Confirmable by guessing for short bodies, so the
  erasure would not erase.
- **Destroy whole entries, including actor and time.** Breaks attribution
  (#5834 rules 1–3) and #5677's history non-goal.

## Invariants implementation must hold

1. A chained entry commits to a body only through `body_commitment`; salt
   and bytes live outside the chain.
2. `erase` is accepted only after a `redact` of the same body, carries one
   actor and a reason code, and is itself chained.
3. After `erase`, neither the bytes nor the salt nor the body key exist in
   the live store, and a restore re-destroys them before serving.
4. The chain verifies end to end with any set of bodies erased.

## Implementation slices

Filed as their own issues when this record is accepted:

1. **Into #14274 (slice 1 of #5834, not yet landed):** salted
   `body_commitment` in the post and changelog record types, the separate
   body store keyed by commitment, and verifier support for an absent body.
2. **`erase` event:** the changelog kind, trigger reason codes, the
   redact-before-erase rule, and body plus salt destruction.
3. **Crypto-shredding and restore replay:** per-body keys in the body store,
   key destruction on `erase`, and erasure replay on restore.
4. **Federation propagation and disclosure:** sending `erase` to peers and the
   user-facing statement of what erasure can and cannot guarantee.
