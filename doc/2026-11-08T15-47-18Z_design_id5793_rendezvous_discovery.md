# Design: signed rendezvous advertisements and discovery for ID

- Date: 2026-11-08
- Status: **decision record** — design only; no implementation or deployment.
- Issue: [#5793](https://github.com/developing-today/code/issues/5793),
  rendezvous and discovery for UDP-denied clients.
- Parent: [#5671](https://github.com/developing-today/code/issues/5671),
  whose fallback order, broker shapes, trust model, and E2E boundary this
  record preserves.
- Related decisions: [#5791](https://github.com/developing-today/code/issues/5791)
  (transport negotiation), [#5792](https://github.com/developing-today/code/issues/5792)
  (broker protocol), and [#5794](https://github.com/developing-today/code/issues/5794)
  (Workers rendezvous/relay).
- Evidence baseline: `origin/main` at `3762e65642`, 2026-11-08. The parent
  and #5793 were OPEN. #5791, #5794 and #5798 were CLOSED with decision
  records; #5792 was OPEN with its record posted; #5795–#5797 were OPEN.
  A fresh PR search for #5793 returned no PR.

## Decision

Use an **identity-signed transport advertisement** stored and distributed by
one or more HTTPS rendezvous directories. The identity key is authoritative
for the advertisement's contents; directories are untrusted mirrors, not
authorities. The default v1 directory is the HTTPS Workers directory chosen
by #5794. Operators may configure additional self-hosted directories using
the same record and API contract. Publishing to multiple directories sends
the same signed envelope to each.

This resolves #5671's open question about which rendezvous substrate is
authoritative when sources disagree: **the endpoint identity signer is the
authority; no directory, DNS answer, source priority, or response arrival
order overrides a valid signed revision.** A configured default directory
gives a client starting from an opaque ID a non-circular HTTPS lookup target.
Custom directory URLs come from local configuration or an already trusted
invitation/configuration; a client must not learn its first directory solely
from the advertisement it is trying to fetch.

DNS resolves the configured HTTPS directory hostname in the ordinary way,
but DNS records are not the v1 source of transport-advertisement content.
The existing Iroh/N0 address lookup remains responsible for native Iroh
address discovery; this record adds the ID-level advertisement needed for
broker and TCP/HTTPS fallback and does not replace or reconfigure that path.

## Advertisement and signature

V1 is one complete snapshot per ID identity, not a sequence of independently
merged endpoint updates. Its canonical payload contains:

```text
version: 1
subject: 32-byte ID Ed25519 public key
revision: monotonically increasing unsigned 64-bit integer
issued_at, expires_at: UTC timestamps
offers: ordered list of { transport, endpoint, priority, protocol,
                          optional broker_identity }
```

`transport` names one of the parent-defined tiers (direct UDP,
relay-assisted UDP, TCP, or HTTP/HTTPS proxy). `priority` orders endpoints
only within a tier; it cannot change the client's fallback order from #5671
or #5791. `protocol` identifies the endpoint framing/version needed to use
the offer. Broker identity is included when applicable. The advertisement
contains no capability tokens, proxy credentials, session keys, or payload
data. Reject payloads larger than 64 KiB or snapshots with more than 64
offers; individual endpoint strings are limited to 2 KiB.

The envelope carries the canonical payload and a strict Ed25519 signature
over `"id.transport-advertisement.v1\0" || payload`. The public key derived
from the signature must equal `subject`, and `subject` must equal the ID
requested by the client. Use deterministic CBOR for the signed bytes; the
HTTPS directory may wrap those bytes and the signature in a JSON/base64url
envelope, but it must not parse and reserialize the signed payload.

The single logical publisher for an ID identity persists and increments
`revision`; replicas mirror the exact envelope rather than signing separate
copies. Multiple publishers sharing one identity must coordinate one
revision allocator. If they cannot, they must not publish concurrently.
Withdrawals are signed higher-revision tombstones with an empty `offers`
list; directories retain the highest revision (including tombstones) so an
old cache cannot resurrect withdrawn endpoints.

V1 advertisements are valid for at most 24 hours; the initial implementation
default is a one-hour lifetime with renewal at or before half-life. Clients
allow at most five minutes of clock skew. Implementations may tighten these
limits but must not extend them without a protocol-version change. These
bounds limit replay from a provider that withholds newer data; they do not
make a directory prove that no newer revision exists.

## Lookup and conflict handling

1. The client queries its configured HTTPS directory set. It may use bounded
   parallel requests; #5797 owns the exact latency budget. Response order and
   provider preference affect latency only, never which signed data wins.
2. The client verifies the signature, domain separator, requested subject,
   version, revision, payload limits, and timestamp syntax. Signature-valid
   envelopes establish the revision floor even after expiry; an expired
   envelope is never dialed. HTTPS protects the directory connection, but
   signature verification—not TLS or DNS—is the content-authentication
   check.
3. Among signature-valid envelopes, the highest revision wins. Identical
   envelopes at the same revision are replicas. Two different, valid payloads
   at the same revision are signer equivocation: reject both as new data and
   report the conflict. The conflicting signatures still establish that
   revision as the minimum acceptable revision. A previously verified cached
   envelope may be used only if it is unexpired and its revision meets the
   client's persisted revision floor; otherwise return
   discovery-unavailable. If the highest revision is expired, do not fall
   back to a lower revision.
4. Clients persist the highest verified revision per subject. A lower
   revision is never accepted after a higher one has been seen, even if the
   higher revision is now expired. An expired highest revision is not dialed
   and cannot be rolled back to an older offer.
5. If all directories are unavailable, the client may use its highest
   previously verified envelope only while that envelope remains within its
   signed validity interval. With no valid cached envelope, return a
   discovery-unavailable error; do not misreport this as `peer_offline` or
   silently invent an endpoint.

A directory verifies the signature and subject before storing an envelope,
keeps the highest revision and tombstone, and serves the envelope unchanged.
It may withhold, delay, or replay a still-valid older envelope; it cannot
forge a new offer. The expiry bound and the client's persisted revision
floor limit that failure mode. A cold client with no revision floor cannot
know that a still-valid signed record is stale if every configured provider
withholds the newer revision; this availability/freshness limit is explicit
in v1 rather than hidden behind a claim of directory authority.

## Substrate choices

- **Default: HTTPS directory on the #5794 Workers component.** Its
  rendezvous role is reachable without UDP and can answer a client whose
  network permits plain HTTPS. It stores the same signed record described
  here; the provider is not trusted to create or alter it.
- **Sovereignty/availability option: self-hosted HTTPS directory.** It uses
  the same envelope and lookup semantics. Operators can configure more than
  one provider and publish identical bytes to each. The self-hosted service
  can share deployment with the `id` broker role, but broker authorization
  and frame semantics remain those decided in #5792.
- **DNS records are not the content authority.** DNS-only lookup adds
  resolver/propagation and record-size/update constraints. DNSSEC adds a
  separate zone trust root; it is not the ID authenticity check. DNS may
  locate a configured HTTPS provider, while the ID signature authenticates
  content.
  A future DNS/PKARR adapter is acceptable only if it transports the same
  signed envelope and obeys the revision rules.
- **A per-peer well-known HTTPS endpoint is not the default bootstrap.** It
  assumes a domain and reachable inbound HTTPS service for an identity that
  may be only a public key and whose host may itself be behind restricted
  egress. Such an endpoint can be an explicitly configured provider, not a
  prerequisite for discovery.
- **One globally authoritative directory is rejected.** It makes provider
  outage, censorship, or compromise a content-integrity and availability
  dependency. Signed snapshots allow independent mirrors without selecting
  one as semantic authority.
- **Provider-priority or newest-response-wins conflict resolution is
  rejected.** It makes results depend on network timing or a mutable source
  preference and permits a stale mirror to override a newer signed record.
  Revision, signature, and validity determine the result instead.

## Parent and sibling boundaries

No decision in #5671 is changed: its transport attempt order, broker/relay
shapes, trust properties, E2E payload boundary, and policy-enforced-egress
non-goal all remain binding. This record settles only the authority and
verification rules for the signed advertisements that #5671 requires.

The record complements #5791: advertisements say what endpoints exist;
client-local negotiation chooses among them in the parent's order. It
complements #5794: Workers is the default HTTPS directory, while the signed
record and self-hosted parity contract remain provider-neutral. #5792's
broker protocol and authorization are consumed, not re-decided. Exact
timeouts and user-visible latency targets remain with #5797.

## Invariants implementation must hold

1. Only the subject ID's Ed25519 key can authorize its advertisement; a
   provider, resolver, or broker cannot sign or modify offers.
2. The signed bytes are domain-separated and deterministic. The requested
   subject must match the signed subject before any endpoint is used.
3. The client applies #5671/#5791's global transport order; advertisement
   priority affects only endpoint order inside a tier.
4. All configured mirrors receive the same signed envelope. Providers do
   not invent revisions, merge fields, or replace tombstones.
5. The highest verified revision is monotonic per subject. Equal-revision
   disagreement fails closed and raises the revision floor; cached fallback
   must satisfy that floor as well as expiry. Expired or lower revisions never
   roll a client's known state back.
6. An offer is used only while its signed validity interval is current. A
   cached offer is fallback only while unexpired; no valid offer means
   discovery unavailable, not peer offline.
7. The advertisement contains no credentials or secret material. Providers
   log no payload; queries expose peer identity, client IP, timing, and
   volume as metadata and must be minimized per #5671/#5794.
8. Discovery itself is reachable over HTTPS without requiring the transport
   path it is meant to discover; endpoint selection cannot bypass an
   operator's transport policy.

## Splits, next action, and verification

No further design split is required: record authenticity, mirror semantics,
and conflict resolution form one decision. **Tracked next action:** when
implementation of #5671's transport-fallback v1 is prioritized, create one
implementation issue linked from #5793 and referencing this record, #5792,
and #5794. It must implement the wire envelope, publisher revision/tombstone
handling, configured HTTPS providers, rollback protection, and tests for
forgery, stale responses, equal-revision equivocation, expiry, and directory
outage. Implementation timing remains separate from this design.

This decision-record change has no code-test oracle. Verification was a
fresh review of #5671 and the states/records for #5791–#5798, a PR search for
#5793, and consistency checks against `doc/2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md`,
`doc/2026-11-05T15-10-00Z_design_cloudflare_workers_rendezvous_relay.md`,
`pkgs/id/crates/id-core/src/connectivity.rs`, and
`pkgs/id/src/directory_log.rs` at the evidence baseline above. No source or
runtime behavior changed; no code tests apply.
