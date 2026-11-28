# Addendum: transport broker boundary for the ID relay/proxy protocol

- **Date:** 2026-11-09 (UTC)
- **Status:** Dated clarification to the original decision record; it does not replace that record.
- **Issue:** [#5813](https://github.com/developing-today/code/issues/5813)
- **Original decision:** [ID relay and proxy protocol](./2026-11-08T11-08-53Z_design_id5813_relay_proxy_protocol.md)
- **Scope:** Clarify the relationship to #5671/#5791 and record the current integration status. No implementation or wire-format decisions are added.

## Transport broker boundary

The transport fallback contract in #5671, refined by #5791, defines its broker/relay as a dumb authenticated pipe for opaque end-to-end-encrypted traffic. It does not provide ID lookup, presence publication, or application routing.

The ID-aware lookup/home-relay service chosen in the original #5813 decision is a **distinct application-level role**. It may be reached over a #5791 transport, and an operator may host both roles, but they remain separate protocol layers: the transport broker forwards opaque bytes; the #5813 service handles only the minimum routable/control metadata allowed by #5795. It must not terminate application payloads or hold content keys. Co-hosting remains subject to #5795 authorization and #5815 security-binding review.

This distinction does not change #5671's broker shape, #5791's fallback order, or the original #5813 choice of volatile live forwarding, independent multi-home leases, and no relay message retention. Durable catch-up remains the #2036 event-graph sync path.

## Integration status snapshot

GitHub state checked **2026-11-09 00:06 UTC**:

- #5674 remains open; its shared wire and adapter decisions are not settled by this addendum.
- #5808 is open/blocked; #5809 and #5810 are open/parked. Their schema, negotiation, and framing decisions remain delegated to those issues.
- #5795 is open/blocked; this addendum does not define broker authorization or metadata policy.
- #5815 is open/in review; its security-binding decision remains subsequent.

These are point-in-time integration states, not additional protocol decisions. No #2036 decision is changed. If a later parent or sibling decision is incompatible with #5813, record and reconcile that conflict before implementation.
