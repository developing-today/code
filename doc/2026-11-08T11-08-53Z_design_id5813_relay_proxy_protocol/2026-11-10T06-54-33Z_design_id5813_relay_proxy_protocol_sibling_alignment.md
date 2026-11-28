# Addendum: sibling contract alignment for the ID relay/proxy protocol

- **Date:** 2026-11-10 (UTC)
- **Status:** Dated clarification to the original decision record; it does not replace that record.
- **Issue:** [#5813](https://github.com/developing-today/code/issues/5813)
- **Original decision:** [ID relay and proxy protocol](./2026-11-08T11-08-53Z_design_id5813_relay_proxy_protocol.md)
- **Prior addendum:** [transport-boundary clarification](./2026-11-09T00-07-24Z_design_id5813_relay_proxy_protocol_transport_boundary_clarification.md)
- **Scope:** Re-check the sibling design state and record how the relay/proxy contract consumes the current schema, framing, and security-binding proposals. No protocol decision is changed.

## Sibling state re-check

GitHub state checked **2026-11-10 06:54 UTC**:

| Issue | State | Decision record |
| --- | --- | --- |
| #5674 (parent) | OPEN | no parent record landed; shared envelope/adapter choices still delegated |
| #5808 (postcard schema) | OPEN/blocked | none |
| #5809 (versioning/negotiation) | OPEN/parked | none |
| #5810 (framing/fragmentation) | OPEN/in review | proposal in PR #7980, not merged |
| #5795 (broker trust contract) | OPEN/blocked | none |
| #5815 (security binding) | OPEN/in review | proposal in PR #7863, not merged |

None of these is a landed decision. The original record's rule therefore still applies: relay control/data messages use the canonical envelope, negotiation, and framing selected by the parent and siblings, and this record allocates no discriminants, byte layout, or independent framing.

## Framing alignment (#5810 proposal)

The #5810 proposal in PR #7980 carries **one canonical serialized postcard frame** across carriers: length-prefixed stream frames, whole-frame-or-checked-chunk datagram assembly, and opaque-object-or-checked-chunk store-and-forward assembly. That is compatible with this record as written:

- A relay forwards the canonical frame bytes unchanged; it does not re-frame, re-encode, or reinterpret them, and it adds no relay-specific chunking. Carrier deframing and checked-chunk reassembly remain the adapter's responsibility under #5810, before any ID decode.
- The 1 MiB frame cap is shared and unchanged. A relay's byte-credit limits operate on frames and streams, not on a new message-size rule.
- #5810's **store-and-forward carrier class** and this record's **rejected durable relay retention** are different things: the carrier class stores canonical frames at an adapter's storage endpoint, while the relay service keeps zero payload retention and performs no offline mailbox, disk spool, or replay. PR #7980 states the same split and leaves relay queue/retention policy to #5813. No conflict.

If #5810's merged wording changes carrier framing in a way that cannot carry the relay's opaque frame-forwarding unit, reconcile that additively here or in #5810 before implementation.

## Security-binding alignment (#5815 proposal)

The #5815 proposal in PR #7863 binds session security to the ID layer (origin signature, handshake transcript, local policy floor), classifies downgrade as a security failure, and states that relays cannot re-originate traffic. This record's invariants already require the same outcomes: relay acceptance/forwarding receipts are hop-local, only the destination application acknowledges, and a relay never becomes message authority. The proposal adds binding mechanisms without changing the relay-service choices. #5815 remains the review owner for the security binding, and #5795 still owns broker authorization metadata; this record neither duplicates nor pre-empts either.

## Decision ledger

- **No #2036 decision is changed.** Envelope, validation-before-forward, hop/loop rules, durable graph, live-session behavior, and the 1 MiB frame cap remain authoritative.
- **No #5674 decision is changed.** The parent still owns the wire-format family, adapter model, resolver defaults, and the final size/extension rules.
- **No sibling decision is pre-empted.** #5808/#5809 have no record; #5810 and #5815 have open proposals only. Their outcomes bind the relay operations by reference, not by a rewritten copy of their contracts.
- **No trust policy is duplicated.** #5795 continues to own authorization, allowlists/capabilities, metadata minimization, and the compromised-broker ceiling.
