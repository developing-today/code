# D4 slice 4: merge on contact

Part of #13889 (D4, sneakernet bundles). Builds on slice 1 (the format),
slice 3 (import order and creator binding) and slice 5 (carriers). Slice 2
(bundle grants) is still open on Drew's bearer versus recipient-bound
decision; this slice does not depend on its answer.

Acceptance in #13889: "a diverged-hosts fixture merges to the same state via
bundle as via network sync, or the differences are named and justified."

Reference: `merge_reference.py` beside this file, stdlib only, loads the
slice 1, 3 and 5 references. `python3 -I merge_reference.py` runs the
self-test.

## 1. What this slice decides

- **One admission function.** A host admits an object only as a pair: the
  object and the envelope that placed it. Sync and bundles both end in the
  same function. Only what reaches the function differs.
- **Kind 4, the envelope.** Slice 1 said merge needed no manifest slot,
  because the contents are plain content-addressed objects. That was wrong.
  An object alone does not say which area it lives in or who was allowed to
  put it there, so a host cannot place it. Slice 4 adds manifest kind 4 for
  the envelope. A slice-1 reader rejects it as `unknown_kind`, which is the
  right failure for an older reader.
- **Derived state comes only from the store.** Supersede chains, forks,
  reply order, nesting and skipped verbs are computed from the set of
  admitted pairs. Arrival order therefore cannot change them.
- **Three named differences** between a bundle and sync (§6). Everything
  else is equal, and the reference checks it.

## 2. The envelope

In composition A the envelope is the p2panda operation that carried the
object: a signed header with the author's key, the payload's BLAKE3 hash,
and the author's Meadowcap capability in a header extension. The reference
models it as canonical JSON:

```text
envelope = {area, author, cap, payload, timestamp, sig}
cap      = {area, from, grantee, moderate, until, verbs, sig}
```

- `payload` is the object's digest, which is also its D3 object ID and its
  slice-1 manifest digest.
- `sig` on the envelope is the author's. `sig` on the capability is the
  broker's.
- The envelope's own manifest digest is BLAKE3 of the envelope bytes, like
  every other entry.

**Pairing rule.** An envelope pairs with the object whose digest equals its
`payload`. A bundle may carry an object without its envelope (an old slice-1
bundle, or a scoped one that left envelopes out), or an envelope without its
object.

- An object with no envelope is kept **unplaced**: stored, in no view, not
  re-exported. The next sync or bundle that brings its envelope places it.
- An envelope with no object does nothing. It does not create a placeholder.

## 3. Admission order

Each step either settles the pair or passes it on. The first four steps are
the same for every path.

| Step | Check                                                                                                                                                         | Outcome if it fails                                  |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| 1    | the host pruned this object                                                                                                                                   | `pruned`: dropped, pruning is final                  |
| 2    | the host already holds it                                                                                                                                     | `have`: nothing to do                                |
| 3    | payload matches the object; author signed the envelope; broker signed the capability; object author = envelope author = grantee; area, time and verb in grant | `unauthorised`: dropped; an unplaced object waits on |
| 4    | the area is one the host subscribes to                                                                                                                        | `not_subscribed`                                     |
| 5    | the verb is one the host subscribes to (known verbs only)                                                                                                     | `verb_not_subscribed`                                |
| 6    | the host holds a read grant for the area                                                                                                                      | sync: `not_sent`. bundle: `held_no_read_grant`       |
| 7    | —                                                                                                                                                             | `admitted`                                           |

Notes on the order:

- A valid envelope settles an unplaced object whatever steps 4 to 7 decide. A
  forged one (step 3) does not, so a forger cannot cancel the real envelope
  that arrives later.
- Step 5 applies only to verbs the host knows. An unknown verb (the fixture's
  `cheer`, written by a newer client and granted by a newer broker) passes,
  is kept and is skipped in every view, as D3 says.
- The time check is the grant's window at the write's own timestamp, not the
  import time. An old bundle of valid writes still imports after the grant
  has lapsed; a write made outside its grant never does.

## 4. What the views compute

All from the admitted store:

- **Supersede.** Valid when source and target share an author, or the
  capability on the supersede link carries `moderate` for the target's area.
  Any other supersede is kept and ignored. A supersede whose source or target
  has not arrived is pending until both have.
- **Forks.** When one object is superseded twice, both heads stay. The
  display shows the lowest object ID, as D5 decided.
- **Replies.** Ordered by causal depth from the root (via the reply link's
  `previous`), then claimed creation time, then object ID. Causal depth comes
  first, so a slow clock cannot put a reply before the message it answers.
- **Nesting** and **dangling targets.** A link whose target has not arrived
  stays in the store and shows nothing for that target. The view picks it up
  the moment the target is admitted, by either path. This is how slice 1's
  "dangling, resolve later" resolves (main record §6).

## 5. The fixture and the result

Two hosts share a graph (a board, a thread root, a reply, an old note), then
work offline:

- **Ana's host:** a reply (her phone clock runs slow), an edit of the root by
  supersede, a nest of the root into the board, and a prune of the old note.
  Ana subscribes to `garden/` and `ben/` but holds a read grant for `garden/`
  only, and does not subscribe to `quote`.
- **Ben's host:** a reply, a photo with an attach link, a quote, an unknown
  `cheer` link, a mention of an absent object, a moderator's tombstone that
  supersedes the root (a fork with Ana's edit), a supersede of the root by
  Cat who did not write it, and a private note in `ben/notes`.

They merge twice, once by network sync both ways and once by one full bundle
each way through slice 3's import. The self-test checks:

- Both paths give identical views on both hosts.
- The fork has two heads, Ana's edit and the tombstone, and shows the lower
  ID.
- Cat's supersede is ignored. `cheer` is kept and skipped. The quote is
  declined by Ana's host only. The pruned note does not come back to Ana and
  Ben keeps it.
- Reply order is Ben's first reply, then Ana's, then Ben's second, though
  Ana's clock claims hers came first.
- Parts in reverse order, a bundle imported twice, and sync in reverse order
  all give the same result.
- Nine forged pairs are refused by both paths, each failing exactly one
  check, with a correctly signed control admitted.

Output: `ok: diverged hosts (9 and 17 objects) merge to the same views by
sync and by bundle (19 objects on ana's host), one named difference (an
unreadable area is held), order and repeats change nothing, a scoped bundle
misses exactly 6 objects until sync, envelope-less objects stay unplaced
until their envelopes arrive, 9 admission refusals on both paths`.

Mutation testing: 24 single changes to the admission function and the views
(each check removed, each boundary moved, each ordering key dropped) all make
the self-test fail.

## 6. Named differences

1. **An area the receiver cannot read is held, not absent.** Sync never sends
   an area the receiver has no read grant for. A bundle has already handed
   the bytes over, so the host keeps them as `held_no_read_grant`, in no view.
   In the fixture this is Ben's private note on Ana's host. Justified:
   discarding bytes a person carried by hand loses them for good, and slice
   2's bundle grants are exactly the mechanism that will release them. Until
   slice 2 lands, a held pair is never shown or re-exported. Under a bearer
   grant whoever holds the stick could read the note anyway; under a
   recipient-bound grant only Ana could. That is slice 2's question, and this
   rule is the same under both answers.
2. **A scoped bundle misses what its scope left out.** A bundle of one
   thread misses the six objects outside it (the photo and its attach, the
   `cheer`, the mention, Cat's post and its supersede). The next sync closes
   the gap exactly. Justified: scope is the creator's choice, and slice 1
   rejected closing over every link target (main record §6).
3. **An envelope-less bundle places nothing.** A slice-1 bundle without kind
   4 entries leaves its objects unplaced. They stay out of every view until
   an envelope arrives, then place exactly as if they had come together.
   Justified: an object without its envelope has no proven area or author
   grant, so showing it would let anyone put words in a shared space.

## 7. Assumptions to check at implementation

- **The p2panda payload hash is the D3 object ID.** Both are BLAKE3-256 of
  the object bytes in the references. If the implementation wraps the object
  before hashing, the pairing rule needs the wrapper's digest instead.
- **One creating operation per object.** The reference pairs an object with
  one envelope. If the same bytes are written twice into two areas, the store
  key must become (object, area) rather than object.
- **Log gaps.** p2panda logs are single-writer with contiguous sequence
  numbers, and LogSync compares log heights. A scoped bundle can carry an
  author's operations 5 and 9 without 6 to 8. Whether the operation store
  accepts gaps (headers do support pruning) has to be checked. If it does
  not, the export must include each author's log from the receiver's known
  height, or scoped bundles must leave envelopes out.

## 8. Cost on a QR code

The reference envelope for one chat message is 710 bytes beside a 172-byte
message, mostly the capability. A real p2panda header in CBOR is smaller, but
the capability still dominates. With slice 3's 65-byte keyset reference:

| Carrier       | Chat messages with envelopes | Without envelopes |
| ------------- | ---------------------------- | ----------------- |
| QR page code  | 2                            | 8                 |
| QR small code | 0                            | 3                 |

A small code can carry messages only unplaced, with their envelopes on a
following code. Removable media and email are unaffected.

## 9. Alternatives weighed

- **Objects only, area from the manifest scope.** Smaller, but a scope is the
  creator's claim, not the author's grant. Anyone could carry a message into
  an area its author never wrote to. Rejected.
- **Drop unreadable areas on bundle import, like sync.** Simpler and gives
  one fewer difference, but loses hand-carried data the receiver may soon be
  granted. Rejected in favour of holding.
- **Order replies by claimed time.** Matches what people expect most of the
  time, but one wrong clock reorders a thread differently on hosts that have
  different parts. Causal depth first fixes that.
- **Reject envelope-less bundles.** Strict, but it throws away every slice-1
  bundle already made. Keeping the objects unplaced costs nothing in the
  views.

## 10. Honest limits

- The reference is a model of composition A, not a p2panda client. It has no
  sequence numbers, backlinks or CBOR, so the log-gap question in §7 is open.
- Capability revocation is out of scope here. A revoked grant still admits
  writes made inside its window. Revocation belongs to the broker and keyset
  work.
- Held pairs wait on slice 2. Their release path is designed there, not
  here.
- The fixture has two hosts. Three-way merges follow from the store being a
  set, but the self-test does not run one.
