# Decision: framing and fragmentation across ID carrier classes

- **Date:** 2026-11-09
- **Status:** decision record; no implementation is included.
- **Issue:** [#5810](https://github.com/developing-today/code/issues/5810), “id: design: framing & fragmentation per carrier class”.
- **Parent:** [#5674](https://github.com/developing-today/code/issues/5674), “Wire format & transport adapters”. This record settles only the framing/fragmentation slice assigned to #5810. It does not change a parent decision.
- **Programme:** [#2056](https://github.com/developing-today/code/issues/2056).
- **Baseline:** `origin/main` at `c20d5c13fa`, checked 2026-11-09. The parent and this issue are open; no earlier #5810 decision record or implementation supersedes this one.

## Decision summary

The adapter boundary carries **one canonical serialized postcard frame**. It adds only the framing required by the carrier and removes that framing before handing the frame to ID. A frame is never re-encoded as JSON/CBOR or given carrier-specific message semantics.

| Carrier class | Framing and fragmentation decision | Delivery/order exposed to ID |
| --- | --- | --- |
| **Stream** | Use one length-prefixed frame per canonical postcard envelope, as #5674 requires. The stream adapter reads the prefix and exactly that many bytes; it may read/write incrementally. It does not split a frame into application fragments. The prefix encoding and byte-level envelope remain the shared wire contract owned by #5674/#5808. | The carrier supplies a reliable ordered byte stream. The adapter restores frame boundaries; a clean EOF between frames is close, while EOF inside a prefix or frame is truncation and the partial frame is discarded. |
| **Datagram** | Send a whole canonical frame in one datagram when it fits the carrier's current negotiated/safe datagram payload. If it does not fit, use the **existing checked-chunk/reassembly representation** referenced by #2036; do not rely on IP fragmentation and do not invent a datagram-specific chunk wire format. Reassemble the original frame bytes before postcard decoding. | Datagram frames and chunks may be lost, duplicated, or reordered. Chunk arrival order is irrelevant to reassembly; completed logical frames are exposed in completion order, which may differ from send order. Retry/reliability policy remains with the parent messaging/transport contract. |
| **Store-and-forward** | Store a whole canonical frame as one opaque object when it fits. For a constrained object/record limit, use the same checked-chunk representation as the datagram case; persist each chunk under its existing transfer identity and assemble the original bytes before decoding. | Accept chunks in any arrival order and make exact duplicate chunks idempotent. A logical frame is made available only after complete validation. Incomplete assemblies expire with their carrier object/retention policy and are never emitted as partial messages. Delivery receipts and retention periods remain outside #5810. |

## 1. Parent boundary and ownership

This record consumes the direction named in #5674 — adapters carry the same
ID payload unchanged and do not change ID semantics — and fills in the
per-carrier framing/assembly behavior #5674 requests (length-prefixed stream
framing, datagram framing, chunking/reassembly for store-and-forward). The
#5674 envelope fields, message-size rules, and fragmentation ownership in its
body are still proposed scope, not a frozen fragmentation contract: #5674
remains open and its triage names the canonical-envelope boundary,
message-size/extension rules, and framing ownership as decisions still to
record. This child proposes the carrier mapping against that pending contract;
it does not freeze the envelope itself.

The existing #2036 design contract remains in force: the shared **frame** cap
is **1 MiB**; do not add a new wire-level chunking protocol; larger framed
objects use the checked-chunk/reassembly pattern already used for welcomes;
and large media is represented by content-addressed blob references. This is
a per-frame cap, **not** a maximum for a logical message or object composed of
multiple frames/chunks. #5674 still owns the unresolved canonical-message and
extension-size rules, including how those limits relate to this frame cap.
#5810 does not raise the frame cap, settle a larger message limit, authorize
large inline media, or replace the checked-chunk pattern.

The final byte layout, chunk metadata encoding, `fragmented` envelope-flag
semantics, and schema evolution rules are owned by #5674/#5808/#5809. This
record does not mutate that proposed flag or treat a checked-chunk carrier
record as a separately decodable postcard envelope: the adapter reassembles
the canonical frame before postcard decoding.

**Frozen parent decisions changed: none.** #5674 is still open, so this record
does not claim its currently proposed envelope or fragmentation ownership is
already frozen. It records the child decision for the carrier mapping; the
parent must incorporate it into its final wire contract. If that contract
cannot express this shape using the existing checked-chunk representation,
the conflict must be resolved in #5674/#5808 before implementation; implementers
must not quietly add another format.

## 2. Normative invariants

1. **Canonical bytes:** after carrier framing is removed and any checked chunks
   are assembled, the bytes handed to postcard decoding are exactly the
   original canonical frame bytes. A carrier cannot rewrite, normalize, or
   selectively omit fields.
2. **One frame, one decode:** no partial, truncated, oversized, conflicting,
   or digest-invalid assembly is passed to the postcard decoder or application.
   Decode only after the advertised total length is within the 1 MiB frame cap
   and the checked-chunk validator succeeds. The cap applies to a frame, not
   to an entire logical message/object whose chunking is owned by #5674.
3. **Bounded resources:** prefix parsing and assembly validate lengths before
   allocation. Every adapter caps frame size at 1 MiB and caps aggregate
   in-progress assembly state by its configured resource budget. Exhausting a
   cap refuses/drops the candidate; it never causes unbounded buffering.
4. **Chunk identity and consistency:** chunks belong to one transfer according
   to the shared checked-chunk metadata. Exact duplicates are idempotent.
   Datagram/store-and-forward arrival may be out of order: buffer by
   checked-chunk identity and present chunks to the existing validator in
   canonical chunk order. Conflicting metadata, overlapping/inconsistent
   content, invalid chunk order, or a final length or digest mismatch
   invalidates the whole assembly. No bytes from an invalid assembly are
   delivered.
5. **Order is not inferred from arrival:** stream byte order is the stream
   transport's property. Datagram and store-and-forward chunk arrival order is
   unspecified; receivers buffer by checked-chunk identity and reconstruct in
   canonical chunk order. Distinct logical frames may complete out of send
   order; ordering between them remains governed by the parent message/session
   contract and its sequence/causal fields.
6. **No accidental reliability promise:** datagram loss, duplicate delivery,
   and store-and-forward retry do not create a new exactly-once guarantee.
   Reassembly is atomic at the frame boundary; message acknowledgement,
   retransmission, deduplication across complete frames, and application
   effects remain the parent protocol's responsibility.
7. **Expiry is terminal for an assembly:** stream truncation discards the
   partial frame. Datagram assembly state is bounded and expires after the
   adapter's configured reassembly lifetime. Store-and-forward incomplete
   state cannot outlive the carrier object's retention/expiry. Expiry discards
   the partial frame and surfaces a transport-level incomplete/gap diagnostic;
   it never fabricates a complete frame.
8. **Datagram size is carrier-specific:** the sender obeys the transport's
   negotiated maximum and current safe payload size. It must not assume a
   universal Internet MTU or emit an IP-fragmented packet to carry an ID frame.
9. **Security stays layered:** chunking and framing provide neither identity
   nor authorization. Existing signature, encryption, replay, and downgrade
   checks still apply to the fully reconstructed canonical frame; transport
   security does not replace ID identity.

### Carrier-specific invariants

| Class | Invariants |
| --- | --- |
| **Stream** | Prefix length is validated before allocation; the prefix delimits exactly one canonical frame; clean EOF is accepted only between frames; EOF inside a prefix/frame discards that partial frame. |
| **Datagram** | Every sent datagram obeys the carrier's negotiated/safe payload limit; oversize frames use the shared checked chunks rather than IP fragmentation; chunks may arrive lost, duplicated, or out of order; the adapter emits a frame only after complete validation and does not impose cross-frame order. |
| **Store-and-forward** | Stored records are opaque canonical frames or identified checked chunks; duplicates are idempotent and arrival order is irrelevant; no frame is exposed before full validation; carrier expiry discards incomplete assembly without partial delivery. |

## 3. Why this shape

- **TCP-like streams are byte streams, not message-boundary transports.** A
  length prefix is necessary to recover consecutive postcard frame boundaries;
  the prefix is framing, not a second serialization of the payload. Incremental
  I/O avoids requiring one contiguous read, while the pre-allocation length
  check enforces the shared cap.
- **Datagrams have independent packet limits and may be unreliable.** A frame
  that fits is one datagram; one that does not fit uses the parent's checked
  chunks, not IP fragmentation. RFC 8085 §3.2 recommends avoiding IP
  fragmentation because losing one fragment loses the original packet and
  middleboxes may drop fragments. QUIC DATAGRAM similarly has a negotiated
  maximum and cannot fragment a DATAGRAM frame (RFC 9221 §§3, 5).
- **Store-and-forward is not an ordered stream.** Persisted chunks may arrive
  or be replayed out of order. Index-based checked assembly makes this harmless
  without promising message order or exactly-once application delivery.
- **A single checked-chunk mechanism keeps adapters interoperable.** Making
  each adapter invent its own chunk headers, digest rules, and duplicate
  behavior would make the same logical ID frame mean different things on each
  carrier and would contradict the #2036 no-new-chunking decision.

## 4. Rejected alternatives

| Alternative | Reason rejected |
| --- | --- |
| Treat each stream read/write as one message | Stream APIs can split or coalesce writes; this loses frame boundaries. Use the parent's length-prefixed framing. |
| Require every datagram to contain an entire frame and reject all larger-than-path frames | A valid frame under the shared 1 MiB cap can exceed a carrier's negotiated datagram size. This would make the adapter unusable for those frames instead of using the already-approved checked-chunk path. |
| Depend on IP fragmentation for oversized datagrams | Loss of one IP fragment loses the whole packet, and fragments are filtered by some middleboxes; it is not an application framing contract. |
| Define a different fragment format for every adapter or put a second serialized envelope around each payload | Creates incompatible wire variants and re-decoding paths, contrary to “same payload unchanged” and the #2036 checked-chunk decision. |
| Require datagram and store-and-forward to preserve arrival order or provide exactly-once effects | Those guarantees are not inherent in the carriers and belong to the parent messaging/session semantics. The adapter assembles a frame atomically and exposes parent sequence metadata unchanged. |
| Re-encode for constrained carriers (for example, JSON or base64) | Changes the canonical bytes, adds expansion and another parser, and breaks byte-for-byte interoperability. |

## 5. Required conformance coverage

The conformance work in #5814 should include, against the same canonical frame
oracle:

- stream prefix split across reads, multiple frames coalesced in one read,
  clean EOF at a boundary, and truncation mid-prefix/mid-frame;
- datagram exactly at the negotiated payload limit, one byte above it,
  shuffled/duplicated/lost chunks, timeout, and proof no IP-fragmentation path
  is required;
- store-and-forward out-of-order insertion, exact duplicate insertion,
  conflicting duplicate, object expiry during assembly, and complete delivery
  only after length/digest verification;
- shared 1 MiB acceptance boundary and rejection above it, with adversarial
  lengths checked before allocation;
- byte-for-byte equality of the reconstructed frame with the canonical input,
  including unknown extension data that the adapter must preserve opaquely.

The oracle is equality with the original canonical frame bytes plus the
checked-chunk validator's invariants; the test suite must not treat a successful
round trip through one adapter's encoder as its own oracle.

## 6. Scope and follow-ons

This is one decision spanning the three carrier classes; it does not split
further. Existing sibling issues own adjacent boundaries: #5808 owns the exact
postcard schema and golden vectors; #5809 owns version negotiation and
forward-compatibility; #5813 owns relay/proxy behavior and relay queue/retention
policy (distinct from the store-and-forward carrier class here); #5814 owns the
cross-adapter conformance harness; and #5815 owns security binding across
carriers. Their current PRs are proposals, not merged decisions. No new issue
is needed. Implementation timing remains separate, as #5810 states.

## References

- #5674 — [wire format & transport adapters](https://github.com/developing-today/code/issues/5674)
- #2036 — [`doc/2026-11-01_design_id2036.md`](2026-11-01_design_id2036.md), especially §4.5 and §5 criterion 10
- #5808 — [postcard wire schema v1](https://github.com/developing-today/code/issues/5808)
- #5809 — [versioning, negotiation & forward compatibility](https://github.com/developing-today/code/issues/5809)
- #5813 — [relay & proxy protocol](https://github.com/developing-today/code/issues/5813)
- #5814 — [adapter conformance harness](https://github.com/developing-today/code/issues/5814)
- #5815 — [security binding across adapters and relays](https://github.com/developing-today/code/issues/5815)
- [RFC 9293 §2.2](https://www.rfc-editor.org/rfc/rfc9293#section-2.2) — TCP provides a reliable, in-order byte stream.
- [RFC 8085 §3.2](https://www.rfc-editor.org/rfc/rfc8085#section-3.2) — UDP message-size and IP-fragmentation guidance.
- [RFC 9221 §§3, 5](https://www.rfc-editor.org/rfc/rfc9221#section-3) — negotiated QUIC DATAGRAM maximum and no fragmentation.
