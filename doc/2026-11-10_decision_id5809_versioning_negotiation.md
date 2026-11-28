# Decision: versioning, negotiation & forward-compatibility policy for the ID wire format

- **Date:** 2026-11-10
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5809](https://github.com/developing-today/code/issues/5809), "id: design: versioning, negotiation & forward-compatibility policy (from #5674)". Queue task 12799.
- **Parent design:** [#5674](https://github.com/developing-today/code/issues/5674) — wire format & transport adapters (postcard schema, non-UDP carriers, relays/proxies).
- **Ordering basis (named draft):** the parent #5674 decision record has not landed yet. Per the task 12799 acceptance, this record is therefore decided explicitly against a named draft of the parent: **the #5674 issue body as checked fresh on 2026-11-10, in particular its §1 (envelope fields: schema major/minor, discriminator, extension/TLV area) and its §2 (versioning and forward-compatibility rules)**. §1 below lists exactly what is taken from that draft as given. If the final #5674 record changes any of those givens, the parent wins and this record must be revised to match — that precedence is stated here so no parent decision is silently forked (see §7, and the cross-reference note posted on issue #5674 alongside this record).
- **Programme:** epic [#2056](https://github.com/developing-today/code/issues/2056). Protocol ancestor: #2036 (ID messaging/relay protocol v1), which the parent extends and this record does not re-decide.
- **Evidence baseline:** `origin/main` at `af20f775f3`, worktree `~/git/fleet/12799`, 2026-11-10. Verified at that baseline: issue #5809 is OPEN with no hard-stop labels and no decision record in its comments or in `doc/`; issue #5674 is OPEN, its follow-on design tickets #5808–#5815 are still open, and no final parent decision record has landed (triage verdict on issue #5674, task 12244, 2026-11-08).

## Decision summary

| Question | Decision |
| --- | --- |
| Version shape | A wire version is a **(major, minor)** pair carried in the envelope (the parent draft's schema major/minor fields). Minor changes are additive-only; any breaking change is a new major (D1) |
| Negotiation | At session start, inside the parent draft's capability-negotiation exchange: peers advertise a supported **range per major line** plus adapter capabilities; the session settles on the **highest mutually supported (major, minor)**; no overlap is a defined error, never a silent unversioned fallback (D2) |
| Downgrade protection | The negotiated result is bound into session establishment: the advertised ranges and the settled version are part of the handshake transcript both peers confirm, so a relay or proxy cannot silently force the lowest common version (D2) |
| v1 peer, v2 frame | Different major: the peer **must not parse the payload, must not silently drop the frame**. A destination peer answers with the defined unsupported-version error; a relaying peer forwards the frame **opaquely, byte-for-byte** (D3) |
| Same major, newer minor | Parse the known fields, ignore additive optional fields, and **preserve unknown extension/TLV data** — in memory, in storage, and byte-for-byte when relaying (D3) |
| Never silently dropped | Unknown extension data (when relaying), unknown discriminators (defined error instead), truncated/oversized frames (defined error instead), and frames of an unsupported major (error or opaque forward, per role) — see the invariant list, §5 |
| Deprecation | A field or type is deprecated by marking it in a minor, accepted in both forms for at least one full major cycle, and removed only in a major bump with a dual-stack transition window; golden vectors pin every supported version (D4) |

## 1. What the parent draft already fixed (not re-decided here)

From the #5674 issue body (the named draft above), taken as given:

1. **The envelope carries schema major/minor**, a message-type discriminator, sender/recipient IDs, stream/sequence identifiers, timestamp/clock fields, payload length, flags, and an extension/TLV area.
2. **The message-type family** includes capability negotiation, handshake/identity, direct and relayed messages, control/keepalive, and error types.
3. **No silent coercion:** unknown discriminators, truncated frames, and oversized payloads have defined error behaviour.
4. **Negotiation happens at connection/session start**, peers advertise their supported schema range and adapter capabilities, and the session settles on the highest mutually supported version.
5. **The forward-compat sketch:** additive optional fields are ignorable by older readers; readers must preserve/forward unknown extension data where relaying; breaking changes require a major bump and a dual-stack transition window.
6. **Golden vectors** are published per message type per supported schema version, and a deprecation policy with conformance tests exists.

This record settles the policy detail inside that frame: the exact negotiation semantics, the role-dependent behaviour on a version mismatch, the never-drop invariants, and the deprecation mechanics. It does not change any item above; where D2 adds transcript binding, that addition is flagged explicitly in §7 rather than folded in silently.

## D1 — Version shape: (major, minor), additive minors only

- Every frame's envelope carries exactly one wire version: the (major, minor) pair of the schema that produced it. There is no per-field or per-message-type versioning in v1 of this policy; the envelope version governs the whole frame.
- **Minor bumps are additive-only:** a newer minor may add optional fields, add extension/TLV entries, and add new message types (see D3 for how older peers treat those). It may never change the meaning, encoding, ordering, or required-ness of anything an older minor defined, and never remove a field or type.
- **Anything else is a major bump:** removing or reinterpreting a field or discriminator, changing varint/optional encoding of an existing field, tightening a size limit so previously legal frames become illegal, or changing error behaviour for an existing condition.
- Version numbers are compared as integer pairs, highest first by major then minor. There are no version strings, no ranges in frames (ranges exist only in negotiation advertisements, D2), and no "compatible-with" hints beyond the pair itself.

## D2 — Negotiation: advertise ranges, settle highest mutual, bind the result

- Negotiation runs once per session, at session start, as part of the parent draft's capability-negotiation exchange — before any application frame is sent. Adapter capabilities are negotiated in the same exchange but are **not** part of the wire version: the same (major, minor) payloads travel unchanged over every adapter (parent rule), so an adapter change never forces a version change and a version is never negotiated per-adapter.
- Each peer advertises, **per major line it supports**, the highest minor it implements (support for a major line implies support for every minor of that line from 0 up to the advertised minor, because minors are additive-only per D1).
- **Settlement:** the session version is the highest (major, minor) both peers support: take the highest major present in both advertisements, then the lower of the two advertised minors on that line. Both peers compute the same result from the same two advertisements; there is no proposer/acceptor asymmetry to disagree about.
- **No overlap is a defined error, not a fallback.** If the advertisements share no major line, the session fails with the unsupported-version error (D3) naming each side's advertised ranges. Peers never fall back to an unversioned or assumed-v1 mode: a silent downgrade to guessed framing is exactly the failure this policy exists to prevent.
- **Transcript binding (downgrade protection):** both advertisements and the settled version are included in the handshake transcript that session establishment confirms (the parent draft's handshake/identity step). If a relay, proxy, or middlebox rewrites an advertisement in transit, the transcripts disagree and establishment fails. A peer that detects a settled version lower than the transcript supports must abort, not proceed at the lower version.
- **Renegotiation** does not exist mid-session in this policy. A peer that gains a new version (upgrade while running) opens a new session; in-flight sessions keep their settled version to their end. This keeps the version of every frame in a session explainable by exactly one negotiation.

## D3 — Cross-version behaviour: what a v1 peer must do with a v2 frame

Behaviour is decided by role and by the kind of mismatch. "Drop" appears nowhere in this section as a silent action.

- **Same major, newer minor (e.g. a 1.3 peer receiving a 1.5 frame):** parse all fields the reader's minor defines; skip additive optional fields and unknown extension/TLV entries by length; **preserve** unknown extension/TLV data attached to the frame — in storage and, when relaying, byte-for-byte in the forwarded frame. Ignoring a field for local processing is permitted; destroying it for downstream peers is not.
- **Different major, destination peer (e.g. a v1-only peer receiving a v2 frame addressed to it):** the peer must not attempt to parse the payload, must not acknowledge the frame as processed, and must answer with the defined **unsupported-version error**, which names the receiver's advertised ranges and the frame's version. If the carrier cannot return an error (store-and-forward), the frame is held as undeliverable with that reason recorded — surfaced to the operator/sender path, never deleted silently.
- **Different major, relaying peer:** a relay or proxy that does not terminate the payload **forwards the frame opaquely, byte-for-byte**, regardless of major. Version enforcement is an endpoint duty; a relay that dropped frames of majors it does not implement would make every major roll-out depend on the slowest relay upgrade, which inverts the parent's relay model. (A relay still enforces envelope-independent limits — frame size, rate, authorisation to relay at all — under its own policy.)
- **New message type within a known major:** an unknown discriminator is the parent draft's defined-error case, not a drop: endpoints return the unknown-type error; relays forward opaquely. A peer must never treat an unknown type as a known one (no silent coercion, parent given 3).
- **Sender duty:** a peer sends only frames at or below the session's settled version. Sending above the settled version is a protocol violation by the sender, and the receiver's behaviour above (error, not parse) is the specified response — receivers are never expected to "do their best" with a higher version.

## D4 — Deprecation and transition mechanics

- A field, extension, or message type is deprecated in three steps: **(1) marked** deprecated in a minor release (senders stop emitting it; receivers keep accepting it); **(2) dual acceptance** for at least one full major cycle — both the deprecated form and its replacement are accepted, and golden vectors cover both; **(3) removal only in a major bump**, whose transition window is dual-stack: implementations of the new major must still *receive* the previous major for the window's duration, per the parent draft's dual-stack rule.
- Deprecation is announced in the capability exchange only insofar as version advertisements already express it (a peer that has removed a form advertises a range that no longer includes it); there is no separate deprecation signalling channel.
- **Golden vectors are the conformance contract:** every supported (major, minor) publishes byte-for-byte vectors for each message type, including vectors exercising unknown-extension preservation (D3) and each defined error. An implementation that fails a vector of a version it advertises does not support that version, whatever its advertisement says.

## 5. Invariants implementation must hold

1. **Nothing is silently dropped.** Every frame a peer or relay handles ends in exactly one of: processed, forwarded byte-for-byte, answered with a defined error, or held as undeliverable with the reason recorded. There is no code path whose outcome is quiet deletion.
2. **Unknown extension data survives transit.** Within a major, unknown TLV/extension content is preserved by readers, storage, and relays; stripping it is a conformance failure even when local processing is unaffected.
3. **No parsing above your major.** A peer never best-effort parses a frame of a major it does not implement; the choices are the unsupported-version error (endpoint) or opaque forwarding (relay).
4. **One version per session, bound to the transcript.** The settled version is computed by the D2 rule from both advertisements, confirmed in session establishment, and constant for the session's life; a transcript mismatch aborts establishment.
5. **Minors are additive, period.** A change that would break invariant-compatible reading by an older minor is a major bump by definition (D1), and conformance vectors must include an older-minor reader case that would catch it.
6. **Errors are typed and versioned.** The unsupported-version, unknown-type, truncated-frame, and oversized-frame errors are part of the schema from the first version that can emit them, so the oldest peer in a mismatch can still *say* what went wrong in a frame the other side can parse (the error type lives at the lowest major both sides could ever share: the envelope header stays parseable by any peer that can read a version field at all).
7. **Relays are version-transparent.** Relaying never requires implementing the payload's major; relay policy limits are version-independent.

## 6. Alternatives rejected (summary)

- **Negotiation-free tolerant reading** (every peer just does its best with any frame) — gives no defined behaviour on a major mismatch, makes silent data loss an implementation choice, and cannot express "this session is v2". Rejected by D2/D3.
- **Lockstep upgrades only** (no cross-version interop; everyone upgrades together) — unworkable across relays, store-and-forward carriers, and independently operated peers; also contradicts the parent draft's forward-compat sketch (given 5).
- **Per-message-type versions** — a negotiation and conformance matrix per type for no benefit the envelope pair lacks; D1 keeps one version per frame.
- **Semantic-version strings / named releases on the wire** — parsing and comparison ambiguity for zero extra expressiveness over two integers.
- **Silent drop of unknown or future frames** (the "ignore what you don't understand" reading) — destroys extension data for downstream peers and hides roll-out failures until data is unrecoverable; the single behaviour this ticket was filed to forbid.
- **Relay-side version enforcement** (relays reject majors they don't implement) — makes global roll-out wait on every relay; rejected by D3's opaque-forwarding rule.
- **Mid-session renegotiation** — two versions per session makes every frame's governing negotiation ambiguous; new sessions are cheap and unambiguous (D2).

## 7. Relation to the parent, stated explicitly (no silent fork)

- This record **adds one mechanism** the parent draft's §2 sketch did not spell out: D2's transcript binding of the negotiation result. It is an addition within this ticket's assigned scope (negotiation policy), not a change to any parent given in §1: the parent's negotiation-at-session-start and highest-mutual rules are unchanged. A note linking this record and naming this addition has been posted on issue #5674 alongside it, so the parent's own record can adopt, amend, or reject it in the open.
- **Precedence:** until the parent #5674 decision record lands, this record stands on the named draft in the header. When the parent record lands, any conflict between it and this record is resolved in the parent's favour, and this record is revised in a follow-up change — conflicts are never resolved by quietly implementing against this record's version.
- No other parent decision is changed, narrowed, or extended by this record.

## What this record does not do

No code, no wire bytes beyond the field vocabulary above, no adapter behaviour, and no implementation timing (a separate later decision, per issue #5809). Implementation work (negotiation exchange, error types, conformance vectors, relay opaque-forwarding tests) is follow-on work to be filed from this record; issue #5809 stays open until this record is merged, per its acceptance.
