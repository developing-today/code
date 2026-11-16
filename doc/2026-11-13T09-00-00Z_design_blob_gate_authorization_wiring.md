# Design: blob gate authorization wiring in `id serve` (#173)

Date: 2026-11-13. Status: implemented on the #173 branch (PR #7181).
Companion to the gate's own module record in `src/blob_gate.rs`; this record
covers the _wiring_ — where the gate's inputs come from, when they refresh,
and what happens when they are absent — which is the part that changes
production blob authorization.

## Context

`iroh-blobs` answers a `get` for any hash from any peer, and `observe` with
the blob's size and held ranges. The blob gate (`blob_gate::decide`) makes
both membership-checked: a fetch or observe is served only to a current
member of a world that references the hash. `id serve` is where that
decision meets the live store, and this record documents the wiring before
it is relied on.

## Decision

### Reference source: the event store's document references

The reference index (hash → worlds that name it) is built from the event
store's document references (`EventStore::document_references`): each
document event names a blob hash and, optionally, its group. The world key
is the hex of the group's create-event hash (`blob_gate::world_key`), the
same key the membership fold uses, so a document's group and the group's
membership agree by construction. A document with **no group names no
world**: its blob stays unreferenced and is refused to every remote peer.
Artifacts are not a source: they carry SHA-256 hashes, not BLAKE3 blob
hashes, and only WASM modules are pinned, without a record of their world.

### HashSeq inference: present-children-only, size-bounded

A referenced blob that parses as a `HashSeq` (a list of 32-byte hashes)
contributes its children to the referencing world, transitively — a member
fetching a collection needs its contents, not just the sequence blob.
Inference is conservative on both axes:

- **All children must already be in the store.** Arbitrary content whose
  length is a multiple of 32 would otherwise spray references onto hashes
  nobody stored.
- **The read is size-bounded.** Only a group-referenced blob is a
  candidate, and it is read into memory only when the store status reports
  it complete with a size that is a positive multiple of 32 and no larger
  than `MAX_HASHSEQ_BYTES` (1 MiB, 32768 children). The size is asked from
  the store, never learned by reading the content, so building the index
  costs memory proportional to candidate collections, not to the largest
  document the node holds.

### Membership source and refresh / revocation timing

Membership is the group log folded as one causal graph
(`blob_gate::fold_group_events`): partitioning the log per world would
drop the cross-group dependencies, vetoes, and removals the fold needs.
Any source that fails to read folds to empty for that source — the gate
fails closed and the next refresh retries.

The fold is not a startup snapshot. `RefreshingWorlds` swaps fresh folds in
as they are rebuilt; every decision reads the latest fold. A removed
member therefore loses fetch at the next refresh, not at the next restart,
and a newly referenced document becomes fetchable the same way.

### Local reads: the operator is trusted, by key

The gate is installed on the same blob protocol the local CLI and REPL
use. `LocalEvents::record` writes `group: None`, so documents published
locally (including editor saves) are referenced by no world, and plain
uploads through `commands/put.rs` create no document references at all —
a membership-only gate would deny the operator their own store the
moment the web UI is enabled.

The wiring therefore passes the node's **local readers** to the gate: the
node's own key and the data directory's client key — exactly the keys
`build_access_policy` already trusts automatically for writes. A local
reader may fetch and observe any blob in its own store
(`decide_with_local`). The trust is keyed, not positional: it follows
those two keys and no others, and a remote peer's request is still
decided by membership alone. If the client key cannot be loaded, local
readers degrade to the node's own key and a warning is logged.

### Ungated behavior: no event store, no gate — loudly

The event store is opened only with the web UI, so a non-web `serve` has
no document references to build an index from and runs **without** the
gate. That state is announced at startup with a warning naming #173
("blob reads are not membership-checked this run"), never silently.
Closing that gap means opening the event store without the web UI, which
is a separate behavior change and is deliberately not made here.

## Consequences

- Remote blob authorization is default-deny: unreferenced and
  not-a-member look identical, and observe needs the server policy to
  serve it for the referencing world on top of membership.
- Startup and refresh cost is bounded by candidate HashSeq collections
  (see the size bound), not by total store size.
- The operator's local workflows (CLI get/gethash, REPL fetches, editor
  saves) behave identically with the gate on or off.
