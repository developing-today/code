# Decision: postcard wire schema v1 — envelope, discriminators, and golden vectors

- **Date:** 2026-11-16
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5808](https://github.com/developing-today/code/issues/5808), "id: design: postcard wire schema v1 (from #5674)". Queue task 12892.
- **Parent design:** [#5674](https://github.com/developing-today/code/issues/5674) — wire format & transport adapters (postcard schema, non-UDP carriers, relays/proxies).
- **Ordering basis (named draft):** the parent #5674 decision record has not landed yet. Following the precedent set by the sibling records for #5809 (versioning/negotiation, merged) and #5810 (framing/fragmentation, merged), this record is decided explicitly against a named draft of the parent: **the #5674 issue body as checked fresh on 2026-11-16, in particular its §1 (canonical postcard wire schema: envelope fields, serialization rules, no silent coercion) and its §2 (versioning and forward-compatibility rules)**. §1 below lists exactly what is taken from that draft as given. If the final #5674 record changes any of those givens, the parent wins and this record must be revised to match — that precedence is stated here so no parent decision is silently forked (see §8, and the cross-reference note posted on issue #5674 alongside this record).
- **Programme:** epic [#2056](https://github.com/developing-today/code/issues/2056). Protocol ancestor: #2036 (ID messaging/relay protocol v1), which the parent extends and this record does not re-decide.
- **Sibling records this record must agree with:** #5809 (version shape, negotiation, cross-version behaviour — its D1–D4 are inputs here, not re-decided), #5810 (framing/fragmentation per carrier class — it explicitly leaves "the final byte layout, the `fragmented` envelope-flag semantics, and schema evolution rules" to #5674/#5808/#5809; this record settles the byte layout and the flag semantics), #5813 (relay & proxy protocol), #5815 (security binding).
- **Evidence baseline:** `origin/main` at `963cd2377e`, worktree `~/git/fleet/queue-12892-5808-postcard-wire-schema`, 2026-11-16. Verified at that baseline: issue #5808 is OPEN with no held / user/owned / do-not-merge / do-not-auto-edit labels; issue #5674 is OPEN; no wire-schema v1 record exists in `doc/`; and the live postcard practice this record ratifies is in `pkgs/id/crates/id-sync/src/protocol.rs` (meta protocol `/iroh-meta/3`), `pkgs/id/crates/id-sync/src/log_sync.rs`, and `pkgs/id/crates/id-core/src/discovery.rs` (`PeerAnnouncement`).

## Decision summary

| Question | Decision |
| --- | --- |
| Encoding | postcard (the `postcard` crate, v1) over serde types, exactly as the live `pkgs/id` practice — varint integers, no field names, no padding, no alignment (D3) |
| Frame shape | One canonical frame = the postcard encoding of one `Envelope` struct: a fixed-order header followed by an opaque payload (D1). Carrier framing (#5810) wraps this frame; it is never re-encoded per carrier |
| Envelope fields | In order: schema major, schema minor, message-type discriminator, flags, sender, optional recipient, stream id, sequence, timestamp, extension/TLV list, payload (D1) |
| Discriminators | One byte-range varint namespace for the parent §1 message family: 0 capability negotiation, 1 handshake/identity, 2 direct message, 3 relayed message, 4 control/keepalive, 5 error, 6 diagnostic/trace. Append-only, never reused, pinned by test (D2) |
| Flags | Bit 0 compressed, bit 1 encrypted, bit 2 fragmented (semantics per D4 — a provenance marker for #5810 checked-chunk reassembly, never a decoder instruction). Bits 3–7 reserved: zero on send, ignored on receipt within a major |
| Payload length | No separate length field inside the envelope: postcard length-prefixes the payload itself, and the outer carrier framing (#5810) owns frame delimiting. A second in-envelope length would be a redundant value that can disagree with the first (D1, §7) |
| Size limit | The encoded frame is capped at **1 MiB** total — the #2036 frame cap, the #5810 per-frame cap, and the meta protocol's `MAX_REQUEST_BYTES` all agree. Oversize is a defined error, never truncation (D5) |
| Decode rule | Exact decode: a frame that does not consume every byte is an error (trailing bytes, padding, and concatenated frames are rejected), ratifying `decode_meta_exact` in the meta protocol (D3) |
| Golden vectors | Four normative byte-for-byte vectors in D6, one per shape class (minimal, addressed+encrypted, relayed+extension, error). Implementations pin them in a test the way `wire_format_is_pinned` pins the meta protocol's discriminants (D6) |
| Existing protocols | `/iroh-meta/3` and the other live postcard protocols keep their current ALPN-governed bare-enum formats. This schema governs the #2036/#5674 messaging envelope; adopting it inside an existing protocol is that protocol's own major/ALPN event, never a silent reinterpretation (D3, §8) |

## 1. What the parent draft already fixed (not re-decided here)

From the #5674 issue body (the named draft above), taken as given:

1. **One canonical postcard wire schema** for every ID message type carried by #2036, with an envelope carrying: schema major/minor, a message-type discriminator, sender/recipient IDs, stream/sequence identifiers, timestamp/clock fields, payload length, flags (compressed, encrypted, fragmented), and an extension/TLV area.
2. **Serialization rules** are part of the contract: field ordering, varint encoding, optional-field encoding, maximum message size, and fragmentation/reassembly at carriers with a smaller MTU.
3. **No silent coercion:** unknown discriminators, truncated frames, and oversized payloads have defined error behaviour.
4. **Versioning:** (major, minor) in the envelope, negotiation at session start, additive minors, major bumps for breaking changes — settled in detail by #5809, whose D1–D4 this record consumes unchanged.
5. **Golden vectors** are published byte-for-byte per message type at each supported schema version.
6. **Adapters carry the same payloads unchanged** (#5810's canonical-bytes invariant); relays see only the envelope fields the relay model exposes (#5813/#5815).

This record settles the byte-level detail inside that frame: the exact field order and encodings, the discriminator assignments, the flag semantics #5810 left open, and the v1.0 golden vectors. Where given 1 lists an in-envelope "payload length", this record records a deliberate, named deviation — see D1 and §8.

## D1 — The envelope: field order is the wire format

A v1 frame is the postcard encoding of exactly one value of this struct, fields in this order (postcard encodes struct fields in declaration order, with no names, tags, or padding):

| # | Field | Type on the wire | Encoding |
| --- | --- | --- | --- |
| 1 | `major` | `u16` | postcard varint |
| 2 | `minor` | `u16` | postcard varint |
| 3 | `kind` | discriminator | postcard varint (D2 table) |
| 4 | `flags` | `u8` bitfield | one raw byte (D4) |
| 5 | `sender` | `[u8; 32]` | 32 raw bytes (the sender's endpoint ID) |
| 6 | `recipient` | `Option<[u8; 32]>` | `0x00` = none (broadcast / relay-routed), `0x01` + 32 raw bytes = addressed |
| 7 | `stream_id` | `u64` | postcard varint; 0 = no stream (single-frame exchange) |
| 8 | `sequence` | `u64` | postcard varint; per-stream sequence, starting at 0 |
| 9 | `timestamp_ms` | `u64` | postcard varint; Unix epoch milliseconds, 0 = the sender asserts no clock (clockless/store-and-forward origin) |
| 10 | `extensions` | list of `(u16, bytes)` | varint count, then per entry: varint type, varint length, raw bytes (the TLV area) |
| 11 | `payload` | bytes | varint length + raw bytes; opaque to the envelope layer |

Rules that follow from the shape:

- **The header is not self-delimiting apart from the payload, and does not need to be:** a decoder reads fields 1–11 in order and has consumed exactly the frame. There is no header-length field; adding or reordering a header field is a major change (#5809 D1), because every reader decodes positionally.
- **No in-envelope payload length (deviation from draft given 1, named):** field 11 is already length-prefixed by postcard, and the carrier framing (#5810) independently delimits the frame on stream carriers. The draft's "payload length" is therefore carried — by the payload's own prefix — rather than duplicated as a header field that could disagree with it. §8 records this as this record's one deliberate change to a draft given.
- **IDs are fixed 32-byte arrays, never length-prefixed strings:** endpoint IDs are ed25519 public keys throughout `pkgs/id`; a variable-length ID field would admit non-canonical encodings of the same identity.
- **Extensions are the only extensibility point inside a major.** An extension type is a `u16`: 0x0000 is reserved (never valid), 0x0001–0x7FFF are assigned by decision records in this series, 0x8000–0xFFFF are experimental and must never be required to process a frame. Unknown extensions are preserved and forwarded per #5809 D3; they are never an error.
- **The payload is typed only by `kind`.** Each discriminator's payload schema is a postcard serde type defined by that message type's own record/implementation; this record fixes the envelope they all share, not their interiors. The one exception is `error` (D2), whose first payload byte is fixed here so the oldest peer can always say what went wrong (#5809 invariant 6).

## D2 — Discriminators: one namespace, append-only

| Value | Kind | Payload, in brief |
| --- | --- | --- |
| 0 | Capability negotiation | Version ranges and adapter capabilities, per #5809 D2 |
| 1 | Handshake / identity | Identity proof and the negotiation transcript confirmation (#5809 D2, #5815) |
| 2 | Direct message | End-to-end payload for the addressed recipient |
| 3 | Relayed message | Same payload shape as direct, carried for relay routing (#5813) |
| 4 | Control / keepalive | Session control: keepalive, close, backpressure signals |
| 5 | Error | First byte: error code (0x01 unsupported-version, 0x02 unknown-type, 0x03 truncated-frame, 0x04 oversized-frame, 0x05 no-common-version); remainder is a human-readable UTF-8 detail string, length-bounded by the frame cap |
| 6 | Diagnostic / trace | Opt-in, authenticated path diagnostics (parent §7); never sent unsolicited |

- **Append-only, never reused, never reordered** — the meta protocol's compatibility rule verbatim ("postcard encodes enum variants by position. Within a protocol version, new variants may only be *appended*"), applied to this namespace. Values 7–127 are reserved for assignment by later decision records in this series; 128–255 are experimental and must never appear in a settled session. A retired kind keeps its value forever.
- **Unknown discriminator behaviour is #5809 D3, unchanged:** an endpoint answers with the error kind (code 0x02); a relay forwards the frame opaquely, byte-for-byte. No peer coerces an unknown kind into a known one.
- **Discriminants are pinned by test** in any implementation, in the shape of the meta protocol's `wire_format_is_pinned` test: one assertion per assigned value, so an accidental reorder fails CI instead of corrupting the wire.

## D3 — Serialization rules (ratifying live practice)

1. **postcard v1, serde-derived, nothing else.** The same encoding the meta protocol, log-sync, and peer announcements already use (`postcard::to_allocvec` / `take_from_bytes`). Integers are postcard varints; `bool` is one byte (0x00/0x01); `Option` is a 0x00/0x01 prefix; sequences and byte strings are varint-length-prefixed; fixed arrays are raw bytes with no prefix. There are no field names, no alignment, and no padding on the wire.
2. **Exact decode.** Decoding a frame must consume every byte. Trailing bytes are an error, never ignored — ratifying `decode_meta_exact` ("Accepting a valid postcard prefix would let a padded or concatenated stream be interpreted as a different, complete message"). One frame per carrier unit after #5810 reassembly; concatenation is a framing-layer concern, never something a decoder tolerates inside a frame.
3. **Deterministic encoding.** One value has exactly one valid encoding: varints are minimal (postcard's encoder already guarantees this; a decoder must reject a non-minimal varint rather than accept a second spelling of the same frame — relays compare and forward bytes, and two spellings of one frame would defeat duplicate detection at the carrier layer).
4. **No silent coercion, mechanically.** A truncated frame, a varint that overruns the buffer, a length prefix larger than the remaining bytes, or a payload length that overruns the frame cap are all decode errors (error codes 0x03/0x04 where a reply is possible), never partial values.
5. **Existing protocols are untouched.** The meta protocol's messages are bare postcard enums with no envelope header, versioned by ALPN (`/iroh-meta/3`; its own rule: any non-append change requires a new ALPN). That remains correct for it. This envelope governs the #2036 messaging/relay traffic the parent designs; if an existing protocol adopts the envelope, it does so at its own next ALPN/major, with vectors published for both forms during the transition (#5809 D4).

## D4 — Flags semantics

| Bit | Name | Meaning |
| --- | --- | --- |
| 0 | `compressed` | The payload bytes are compressed (algorithm is a capability-negotiation item, #5809 D2 — not an envelope field) |
| 1 | `encrypted` | The payload is end-to-end encrypted; envelope fields remain visible to relays exactly as #5813/#5815 specify |
| 2 | `fragmented` | **Provenance only:** the logical payload exceeded one carrier frame and was carried as #5810 checked chunks, reassembled to the original bytes *before* postcard decoding. By the time any decoder sees this frame, the payload is whole. The flag records that chunking happened (for diagnostics and size accounting); it is never an instruction to reassemble, and a receiver must not reject or re-chunk a frame because of it |
| 3–7 | reserved | Zero on send. A receiver ignores them within a major; assigning one is an additive minor act (#5809 D1) recorded in this series |

Flags describe the payload as carried, never the envelope: no flag changes how fields 1–11 are decoded, so a relay that understands no flags can still route, forward, and preserve the frame.

## D5 — Size limits

- **Frame cap: 1 MiB (1,048,576 bytes) for the whole encoded frame**, header included. This is the one number three sources already agree on: the #2036 frame cap (cited by #5810), #5810's per-frame cap, and the meta protocol's `MAX_REQUEST_BYTES`. This record fixes it as the envelope-layer rule so no fourth number appears.
- The payload maximum is therefore 1 MiB minus the header actually encoded; senders compute it from the encoded frame, not from a nominal payload allowance.
- A frame over the cap is refused with the oversized-frame error where a reply is possible; it is never truncated, split at the envelope layer (splitting is the carrier's checked-chunk mechanism, #5810), or partially decoded.
- Length prefixes are validated against the cap *before* allocation (the #5810 bounded-resources invariant applies at this layer too).

## D6 — Golden vectors (schema v1.0, normative)

Computed from D1–D4 (all varint values below 128 encode as one byte; IDs are repeated single bytes so the vectors stay readable). An implementation of this schema must reproduce these bytes exactly, in a pinned test, before it may claim v1.0 support (#5809 D4: the vectors are the conformance contract).

**Vector A — capability negotiation, minimal frame** (major 1, minor 0, kind 0, flags 0x00, sender = 0x00×32, no recipient, stream 0, sequence 0, timestamp 0, no extensions, empty payload; 42 bytes):

```text
010000000000000000000000000000000000000000000000000000000000000000000000000000000000
```

**Vector B — direct message, encrypted, payload `hi`** (major 1, minor 0, kind 2, flags 0x02, sender = 0x11×32, recipient = 0x22×32, stream 1, sequence 0, timestamp 0, no extensions, payload = `68 69`; 76 bytes):

```text
01000202111111111111111111111111111111111111111111111111111111111111111101222222222222222222222222222222222222222222222222222222222222222201000000026869
```

**Vector C — relayed message, fragmented provenance, one extension** (major 1, minor 1, kind 3, flags 0x04, sender = 0xAA×32, recipient = 0xBB×32, stream 9, sequence 42, timestamp 0, extensions = [(type 0x0001, bytes `DE AD`)], payload = `00 FF`; 80 bytes):

```text
01010304aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa01bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb092a00010102dead0200ff
```

**Vector D — error, unsupported-version** (major 1, minor 0, kind 5, flags 0x00, sender = 0x77×32, no recipient, stream 3, sequence 3, timestamp 0, no extensions, payload = `01` + empty detail; 43 bytes):

```text
01000500777777777777777777777777777777777777777777777777777777777777777700030300000101
```

**Negative vectors (must be decode errors, never values):**

- Vector B with its last byte removed — truncated frame (error 0x03 where a reply is possible).
- Vector A with one `00` byte appended — trailing bytes after an exact decode.
- Vector A with the kind byte (byte 3) replaced by `7F` — unassigned discriminator: unknown-type error (0x02) at an endpoint, opaque forwarding at a relay.
- Any frame whose payload length prefix exceeds the remaining bytes, or whose total length exceeds 1 MiB — oversized/invalid (0x04).

New message types and new schema minors add vectors; existing vectors never change within a major. A vector change is how a breaking change announces itself — it requires the major bump #5809 D1 defines.

## 5. Invariants implementation must hold

1. **One value, one encoding, one frame.** Deterministic postcard bytes, exactly consumed on decode; no padding, no trailing bytes, no second spelling.
2. **Field order is frozen within a major.** Fields 1–11 appear in D1's order in every frame of the major; extensibility lives in the extension list and in additive minors, never in reordering.
3. **Discriminators are append-only.** Assigned values never change meaning, are never reused, and are pinned by a discriminant test (D2).
4. **The envelope is relay-legible, the payload is not relay-owned.** A relay reads fields 1–10 to route and forwards the frame byte-for-byte, payload and unknown extensions untouched (#5809 D3, #5810 invariant 1, #5813).
5. **Nothing is silently dropped or coerced.** Every malformed frame ends as a defined error (D2 codes), an opaque forward, or a recorded undeliverable — the #5809 invariant 1 applied at the byte layer.
6. **The cap is checked before allocation.** No length prefix — payload, extension, or count — causes allocation beyond the 1 MiB frame cap (D5).
7. **Flags never change decoding.** Bits 3–7 and the `fragmented` provenance bit cannot alter how a frame is parsed (D4); a frame is decodable by a reader that implements no flags at all.

## 6. Alternatives rejected (summary)

- **A self-describing encoding (JSON / CBOR with field names)** — larger, admits multiple encodings of one value, and abandons the repo's working postcard practice for no property the envelope needs; the schema is fixed by this record, not discovered from the bytes.
- **Protocol Buffers / a schema-IDL wire format** — a second toolchain and a tag-number discipline that duplicates what the discriminator table and append-only rule already give postcard; also changes every existing `pkgs/id` protocol's encoding family.
- **A fixed-width binary header** (fixed 8-byte integers, fixed timestamp width) — simpler to eyeball, but wastes bytes on every frame of a store-and-forward-friendly protocol and freezes field widths a varint never needs to revisit; postcard practice in this repo is varint throughout.
- **An in-envelope payload-length header field** (the draft given, as literally read) — duplicates the payload's own postcard length prefix and the carrier's frame delimiting (#5810), creating a value that can disagree with both; recorded as a named deviation in D1/§8 instead of silently kept or silently dropped.
- **Per-message-type envelopes** (each kind defines its own header) — negotiation, relay routing, and version handling would be re-implemented per kind; the parent's single-envelope direction is the point of the ticket.
- **Length-prefixed or string IDs** — admits non-canonical spellings of one identity; IDs in this system are 32-byte keys (D1).

## 7. Relation to the parent, stated explicitly (no silent fork)

- This record **changes one draft given, in the open:** the parent draft's §1 lists "payload length" among the envelope fields; D1 carries the length in the payload's own postcard prefix instead of as a separate header field, for the disagreement reason stated there. Everything else in §1's field list is carried verbatim, in the draft's order.
- This record **settles two items the siblings left to it:** the final byte layout (left open by #5810) and the `fragmented` flag semantics (left open by #5810). It consumes #5809's versioning decisions unchanged and adds no negotiation mechanism of its own.
- **Precedence:** until the parent #5674 decision record lands, this record stands on the named draft in the header. When the parent record lands, any conflict between it and this record — including the payload-length deviation above — is resolved in the parent's favour, and this record is revised in a follow-up change. A note linking this record and naming the deviation has been posted on issue #5674 alongside it.
- No other parent decision is changed, narrowed, or extended by this record.

## What this record does not do

No code, no changes to any live protocol (in particular `/iroh-meta/3` keeps its current format and ALPN), no per-kind payload schemas beyond the error code table, and no implementation timing (a separate later decision, per issue #5808). Implementation work (the envelope type, the pinned-vector tests, per-kind payload records) is follow-on work to be filed from this record; issue #5808 stays open until this record is merged, per its acceptance.
