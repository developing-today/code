Status: design record for issue #9127, written 2026-11-17.
Program: #9106 (extensible primitives / federation / namespaces),
Drew directive 2026-11-12 18:29 CST, §5 (13-29). Parent epic: #9107
(Part A — ID core). This record decides the append-list user-attribute
protocol; it does not amend §5. Where this record and §5 disagree, §5
governs and this record is the defect.

## 1. What this decides, and what it does not

Decides: the submission protocol for append-list user attributes
(submit, verify, append, pending, approve), the ordering rule, the
OPENED state, the pending-queue semantics, and the chosen defaults
for the open questions in §8.

Does not decide: the triples store's physical encoding (sibling
#9125), versioning of list entries after append (sibling #9129 —
append-list entries are immutable once appended under this record;
any edit story is #9129's), true-delete of appended entries (sibling
#9130 — moderation removal is a tombstone under that design, never a
rewrite of the order), or the social reference shapes that consume
append-lists (sibling #9128 — feeds and bulletin-board comments are
clients of this protocol, not part of it).

## 2. Model

User attributes (Drew's term in §5 — distinct from Atlas metadata
and from content-location metadata) are objects attached to a
subject, addressed by path: the attribute `comments` on the subject
at `geordi.png` is the append-list at `geordi.png/comments`.

An **append-list attribute** is an attribute whose value is an
ordered list of submitted objects. Three properties define the type:

1. **Append-only by protocol.** The list changes only through the
   ops in §3. There is no set, insert-at, reorder, or edit op, for
   anyone — including the attribute's owner. (Owner moderation is
   #9130's tombstone, which hides an entry without renumbering the
   list.)
2. **Entries are self-contained signed objects.** An entry is a
   content object plus its content hash, signed by the submitter's
   key. The list stores references (hash + server sequence, §4);
   the object itself lives in the content plane like any other
   object.
3. **Submitters hold no list state.** A submitter proposes an entry;
   the server decides whether and where it lands. Submitter-chosen
   order, position, and timestamps carry no authority (§4).

## 3. Submission protocol

Ops (names are this record's; the wire encoding is the implementing
packet's call against the program's transport design):

- `append-submit(attribute, object, hash, signature)` — propose an
  entry. The server verifies, in this order, before anything is
  appended or queued:
  1. the hash matches the object bytes;
  2. the signature verifies against the submitter's key over the
     hash;
  3. the attribute exists and is an append-list;
  4. the attribute is OPENED (§5) — a submission to an unopened
     attribute is refused, not queued.
  A submission failing (1) or (2) is refused with a value-free
  error; nothing is stored. A submission passing verification is
  either appended (§4) or pended (§6), per the submitter's standing.
- `append-read(attribute, after-seq?, limit?)` — read entries in
  server order. Pending entries are never returned by `append-read`.
- `append-pending-read(attribute)` — the pending queue (§6), visible
  to the attribute's approvers; a submitter may read the state of
  their own submissions (pending / appended-at-seq / refused).
- `append-approve(attribute, submission)` /
  `append-reject(attribute, submission)` — approver ops (§6).
- `append-open(attribute)` / `append-close(attribute)` — the owner
  (or group admin, for group attributes) toggles OPENED (§5).

Submission is **idempotent per (attribute, hash, submitter)**: a
submitter re-submitting a hash they already appended gets its
existing sequence back; re-submitting one they have pending
returns its pending state. A submitter cannot append the same
object twice by retrying (a different submitter is a different
submission — §8.5).

## 4. Server order

The server assigns every appended entry a **central sequence
number** per attribute: monotonically increasing, gap-free for
appended entries, assigned at append time (for approved pending
entries: at approval time, not submission time). The list's order
is that sequence, and only that sequence.

Submitter timestamps are stored on the entry object as data and are
never an ordering input — two entries are ordered by sequence even
if their claimed timestamps say the opposite. Federation note: the
sequence is per-server; a list replicated across servers has one
server of record for ordering (the attribute's home server), and
other servers mirror its sequence. Choosing the home server is the
namespace design's call (#9116); this protocol only requires that
exactly one sequence authority exists per attribute at a time.

## 5. The OPENED state

OPENED is an **explicit per-attribute state**, set by the owner (or
group admin) with `append-open`. It is never implied by the
attribute existing, by the subject being public, or by group
membership.

Chosen default: a newly created append-list attribute is **not
OPENED**. Submissions to an unopened attribute are refused at step
(4) of §3 — they are not silently held — so a submitter learns the
list is closed from the refusal, and opening a list later does not
release a backlog of stale submissions.

`append-close` stops new submissions and new approvals; entries
already appended stay readable, and the pending queue is preserved
(approvers may still drain it if the attribute reopens; see §8.4).

## 6. The pending queue

Standing decides the path at submit time:

- **Members** of the attribute's group (for a personal attribute:
  the owner and keys the owner has designated as members) append
  directly, subject to §3 verification.
- **Non-group submitters** land in the attribute's **pending
  queue**, in submission order, after the same §3 verification.
  Verification is not deferred to approval time: an unverifiable
  submission never reaches the queue.

An approver — the attribute owner by default, or member/admin per
the attribute's group configuration — drains the queue with
`append-approve` (the entry appends at that moment, taking the next
server sequence, §4) or `append-reject` (the submission is dropped;
the refusal is recorded against the submission so the submitter's
own state read in §3 can report it). Approval order need not follow
queue order; the queue preserves submission order for display, but
an approver may act on any entry in it.

Pending entries are not list entries: they are invisible to
`append-read`, carry no sequence, and confer no position. Approval
is the append.

## 7. Invariants

1. **Nothing unverified is stored.** Hash and signature are checked
   before append and before queueing; there is no path on which an
   unverified object enters either structure.
2. **One order, server-assigned.** List order is the server
   sequence; no client input (timestamp, claimed position, retry)
   can move an entry.
3. **Submitters cannot set or edit the list.** The only submitter
   op is propose. Even the owner mutates the list only through
   approve/open/close — never by direct placement.
4. **Closed means closed.** An unopened or closed attribute accepts
   no submissions and no approvals; refusal is explicit, never a
   silent hold.
5. **Pending is not appended.** No read of the list can observe a
   pending entry, and no pending entry has a sequence number.

## 8. Open questions, with chosen defaults

1. **Pending-queue expiry.** Do pending submissions expire?
   *Default: no expiry in v1.* The queue is durable and
   approver-drained; expiry would silently drop verified
   submissions, and §5's directive does not ask for it. Revisit if
   queue growth becomes a measured problem.
2. **Sequence at approval time vs submission time.** *Default:
   approval time* (§4, §6). The sequence means "order in the
   list"; a submission order claim by a pending entry would let
   non-members reserve positions ahead of members.
3. **Who approves by default.** *Default: the attribute owner is
   the sole approver* unless the attribute's group configuration
   names member/admin approvers. Narrowest authority that satisfies
   §5's "per config".
4. **Pending queue on close.** *Default: preserved, not drained or
   dropped* (§5). Closing is a pause; reopening resumes the queue
   as it was. An owner who wants the queue emptied rejects entries
   explicitly.
5. **Duplicate object from two submitters.** The same content hash
   submitted by two different keys: *default: both may append* —
   idempotence in §3 is per (attribute, hash) only within one
   submission's retry story as recorded by the server; two distinct
   verified submissions of the same object are two entries, each
   attributed to its signer. (If a consumer wants dedupe, it is a
   view policy, e.g. in #9128 — not a protocol rule.)
6. **Rate/size limits.** *Default: none specified here.* Per-server
   operational limits are an implementation concern; the protocol
   only requires that a limit refusal is explicit and value-free
   like other refusals.

## 9. Implementation breakdown

Filed as implementation issues under #9127 (links recorded on the
issue):

1. **Server protocol**: the §3 ops, §4 sequence assignment, §5
   OPENED state, and §6 pending queue in `pkgs/id`, with the §7
   invariants as property tests (oracle: no sequence of ops can
   produce a list state the invariants forbid).
2. **Client submission path**: submit + own-submission state read
   (pending / appended-at-seq / refused) against the §3 protocol,
   including retry idempotence.

