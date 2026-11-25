# D1 slice 3: the failure and revocation matrix

Issue: #13886 (parent epic #13880). Builds on slice 1 (the grammar, §7) in this
folder, the decision record on #13886 (issuecomment-7157414471), R1 (#13884,
closed) and D3 (#13885, closed; §5 supersede chains and tombstones, §9 expiry
non-renewal).

Status: proposal. Slice 3 of 5. Slices 1, 2, 4 and 5 have landed; this slice
completes the set.

Executable reference: `failure_reference.py` in this folder
(`python3 failure_reference.py`, stdlib only). It models one resolution over a
synthetic object/caller state and prints:

```text
ok: 13 conformance cases, collapse holds for unauthorised and lapsed callers, detail only for authorised callers
```

## 1. The decision in one paragraph

A failed dereference has five causes worth naming — the object is unknown, the
object was pruned, the caller's grant has lapsed, the caller was never
authorised, or a pinned mutable head has gone stale — but a resolver may only
ever *reveal* a cause to a caller who currently holds access to the object.
Everyone else gets the one coalesced answer slice 1 §7 already promises:
`unavailable`. A URI names an object and never grants authority, and under the
non-renewal posture (composition A) a lapsed grant is not renewed by presenting
the URI again, with or without hints: a lapsed caller is an unauthorised
caller, immediately and indistinguishably.

## 2. The outcome vocabulary

A resolver reports one of four outcomes for a v2 graph URI. These are the only
outcomes; there is no per-cause error code on the wire.

| Outcome       | Meaning                                                                                                                                 |
| ------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `ok`          | The named object (or the current head) is returned and verified.                                                                        |
| `unavailable` | The coalesced answer. It asserts nothing about which cause produced it.                                                                 |
| `tombstone`   | The object was removed and a tombstone was retained (D3 §5: a tombstone-kind object linked by `supersede`, carrying reason and scope).   |
| `stale_head`  | `head/…/at/<cid>` only: the pinned head is readable but is no longer the current head. The `stale` state word of slice 5.                |

`tombstone` and `stale_head` are *detail*: they are only ever returned to a
caller with current access, under the rules of §3. For the `head/` form whose
current head is itself a tombstone, an authorised caller gets `tombstone`
(the head is "removed", per slice 1 §7 and D3 §5); the display word is
`removed` (slice 5).

## 3. The matrix: cause by caller

Caller states: **authorised** (holds current access to the object),
**unauthorised** (never held it), **lapsed** (held a grant that has lapsed
under non-renewal). Lapsed and unauthorised are the same column in every row —
that identity is a rule, not a coincidence (§4).

| Cause                                   | Authorised caller                              | Unauthorised or lapsed caller |
| --------------------------------------- | ---------------------------------------------- | ----------------------------- |
| Unknown object (never existed here)     | `unavailable`                                  | `unavailable`                 |
| Pruned object, tombstone retained       | `tombstone`                                    | `unavailable`                 |
| Pruned object, no tombstone retained    | `unavailable`                                  | `unavailable`                 |
| Grant lapsed (object itself unaffected) | — (the lapse *is* the caller state)           | `unavailable`                 |
| Never authorised (object exists)        | — (the caller state, not an object cause)      | `unavailable`                 |
| Stale mutable head (`head/…/at/<cid>`)  | `stale_head`                                   | `unavailable`                 |
| Current head / existing object          | `ok`                                           | `unavailable`                 |

Reading the matrix:

- **Unknown is `unavailable` even for an authorised caller.** A resolver
  cannot prove a negative — the object may exist on a carrier it has not
  synced — and there is no retained record to describe. Authorisation buys
  detail about records the resolver holds, not proof of non-existence.
- **Pruning detail is exactly the retained tombstone.** Where pruning kept a
  tombstone, an authorised caller learns that the object was removed, and
  the tombstone's reason and scope (D3 §5). Where pruning kept nothing, the
  outcome collapses to `unavailable` for everyone: a pruned object with no
  tombstone is indistinguishable from one that never existed, by design.
- **A lapse is per-caller, not per-object.** One caller's lapsed grant does
  not change any other caller's row: a caller authorised through another
  grant resolves the same object normally (`ok`, `tombstone` or
  `stale_head` as the object's own state dictates).
- **Staleness is not failure of the object.** `stale_head` says the *pin*
  moved, not that anything was removed. It is still detail: it confirms the
  chain exists and has advanced, so an unauthorised caller gets
  `unavailable`, never `stale_head`.

## 4. The revocation rules

1. **Naming is not authority.** A URI contains no capability (slice 1 §5
   enforces this with the closed query vocabulary), and presenting one —
   however the caller obtained it — grants nothing, renews nothing and
   changes no outcome in the matrix.
2. **Hints are not authority either.** A `via` hint may help a reader find a
   resolver (slice 4); it never changes object identity and never grants or
   renews access. The oracle checks the lapsed row with a URI and a hint
   both presented: still `unavailable`.
3. **Lapse revokes immediately.** Under the non-renewal posture, when a
   grant lapses the caller's dereference access ends at that moment. There
   is no grace window in which the old grant still resolves, and no
   different code for "you had this yesterday": lapse reads exactly as
   never having had access (slice 1 §7).
4. **No existence oracle.** For a caller without current access, unknown,
   pruned and unauthorised objects return the identical outcome with no
   distinguishing field. A URI therefore cannot be used to probe whether an
   object exists, existed, or was removed. Resolvers must not reintroduce
   the distinction through side channels they control (per-cause messages,
   per-cause sub-codes, or a member count of hidden objects).
5. **Detail follows current access only.** `tombstone` and `stale_head`
   require access *at resolution time*. Past access — including access the
   caller held when the object was pruned — does not qualify.
6. **Naming never pins.** Pruning may proceed while URIs to the object are
   outstanding (slice 1 §7, R1 row 7). The matrix, not retention, is how a
   holder of a URI learns what happened.

## 5. Collections (`set/`)

The matrix applies per member. A member the caller cannot read is omitted
from the result and is indistinguishable from a member that does not exist;
the resolver does not report how many members were hidden (rule 4). A member
that was pruned with a retained tombstone may appear as its tombstone only
to a caller authorised for that member. Collection traversal never follows
`supersede` (decision record; slice 2), so a set result never upgrades a
member into its successor.

## 6. Conformance

`failure_reference.py` encodes §3 as a single function over a synthetic
state (exists / pruned / tombstone retained / caller state / head stale) and
checks 13 named cases, including: unknown and pruned targets collapsing to
`unavailable` for unauthorised and lapsed callers; a retained tombstone and a
stale pin surfacing only for an authorised caller; and a lapsed grant
presented with both a URI and a hint still returning `unavailable`. It also
asserts the collapse property directly: for any caller without current
access, every object state returns the one identical outcome.

## 7. Consistency with R1 and D3

- R1 row 7 (prune fails): adopted as rule 6 — naming never pins — with the
  cause matrix R1 left open now filled by §3.
- R1 row 18 (no capability in the URI): adopted as rules 1–2; the failure
  matrix adds that *presenting* a URI is also not a renewal event.
- D3 §5: the tombstone outcome is D3's tombstone-kind object reached by
  `supersede`, surfaced as the head of a removed chain; removal stays a
  link, never a rewrite.
- D3 §9 / composition A: expiry non-renewal is rule 3 — lapse is immediate,
  silent and indistinguishable from never-authorised.
- The decision record's remaining points are untouched: no `id://any/`
  authority beyond slice 1's resolver-namespace reading, hints never change
  identity, and collection verb filters come from the D3 registry with
  `supersede` never traversed (§5).
