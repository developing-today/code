# Decision record: rendezvous & discovery for UDP-denied clients

- **Issue:** #5793 (follow-on design from #5671, decomposition item 3)
- **Parent:** #5671 — transport fallback for ID when UDP is denied
- **Programme:** epic #2056
- **Status:** DECIDED (task 12886, lane code-y, 2026-11-10)
- **Scope of this record:** signed transport advertisements and
  substrate selection only — how a client that cannot use UDP finds
  and authenticates the rendezvous point it can use.

## Parent decisions this record does NOT re-decide

From #5671, fixed before this ticket and unchanged by it:

1. Fallback order: direct UDP → relay-assisted UDP → TCP →
   HTTP/HTTPS proxy (CONNECT / WebSocket-style framing).
2. The broker/relay is a dumb pipe for end-to-end-encrypted traffic;
   it holds no session or content keys and never terminates ID
   payloads or SSH.
3. The rendezvous path must itself be reachable over plain HTTPS,
   or the fallback is circular.
4. Rendezvous records are signed and verifiable, carry TTLs, and
   clients fall back to last-known-good endpoints.
5. Broker use defaults to authenticated (allowlist / capability),
   never an open relay.

One parent **open question** is answered inside this record's scope
("what rendezvous substrate is authoritative when several
disagree?", #5671 open questions): see Decision 2. That is a
completion of the parent, recorded here explicitly — not a silent
fork. No other parent decision changes.

Relation to the 2026-03-22 peer-discovery design
(`doc/2026-03-22T00-00-00Z_feature_peer-discovery/`): that design
covers discovery on the UDP-capable public network (DHT/gossip).
This record covers the disjoint case where UDP is denied and only
HTTPS egress exists; the two compose — a client uses whichever
channel its network permits, and both authenticate the same server
identity (Decision 1).

## Decision 1 — the advertisement is a signed object; substrates are untrusted transports for it

A server publishes exactly one current **TransportAdvertisement**:

```text
TransportAdvertisement (v1)
  subject        the server's ID identity (its identity public key
                 fingerprint — the same identity a paired client
                 already pins; no new key type is introduced)
  transports[]   one entry per usable path:
                   kind      udp | relay-udp | tcp | http-connect |
                             websocket
                   endpoint  host:port or https URL
                   priority  lower = try earlier, consistent with
                             the parent's fallback order
                   broker    (relay kinds only) the broker's own
                             identity fingerprint, so the client
                             authenticates the broker from the ad,
                             not from the substrate
  issued_at      unix seconds
  expires_at     unix seconds; TTL ≤ 24 h, 1 h RECOMMENDED
  sequence       strictly increasing per subject (server persists
                 it beside the identity key)
  signature      Ed25519 by the subject identity key over the
                 domain-separated canonical encoding:
                 "id-transport-advertisement/v1" || canonical-json
                 (object keys sorted, UTF-8, no insignificant
                 whitespace)
```

Rules:

- **The client trusts the signature and its pinned subject
  identity — never the substrate.** Any substrate may serve any
  ad; a substrate can withhold or replay, it cannot forge.
- **Freshest valid wins.** Among signature-valid ads for the
  subject, the highest `sequence` wins; ties or regressions are
  discarded. An expired ad is used only as last-known-good
  (Decision 3) and is always flagged degraded.
- Ads name brokers by identity fingerprint (above), so broker
  authorization from #5671 §4 composes: the client knows which
  broker the *server* authorizes for it before connecting.

## Decision 2 — substrate selection: well-known HTTPS is authoritative, Workers is the hosted mirror, DNS is bootstrap-only

Substrates, in the order a client tries them for a subject it
cannot currently reach:

1. **Last-known-good cache** (Decision 3) — zero-network first try.
2. **Well-known HTTPS on a known host** —
   `https://<host>/.well-known/id-transport-advertisement?subject=<fingerprint>`
   for every host the client already knows for the subject (from
   pairing, config, or a previous ad). This is the **authoritative
   self-hosted substrate**: the server's own HTTPS endpoint serves
   its current signed ad.
3. **Hosted rendezvous (Cloudflare Workers)** — for servers that
   cannot serve HTTPS themselves (behind NAT, UDP-denied egress, or
   offline-intermittent): the server (or its broker, on its behalf)
   **pushes** each new signed ad to the Workers rendezvous, keyed by
   `H(subject fingerprint)`; clients fetch
   `https://<rendezvous-host>/v1/advertisement/<H(subject)>`. The
   Workers component stores and serves opaque signed blobs only —
   it holds no keys and cannot mint or alter ads (Decision 1), so
   hosting it is not a trust decision. This is the same Workers
   component the parent's decomposition item 4 designs; this record
   fixes only the interface it must serve for rendezvous (push of a
   signed blob, fetch by subject hash, TTL honouring `expires_at`).
4. **DNS (bootstrap hints only)** — a `_id.<host>` TXT record may
   name the hosts for (2) and (3). DNS is **never authoritative**:
   TXT size limits, resolver spoofing on hostile networks, and slow
   propagation make it unfit to carry or vouch for the ad itself;
   everything it points to is authenticated by Decision 1 anyway.

**Disagreement rule (the parent's open question, answered):** no
substrate outranks another. The signed object is the authority:
the valid ad with the highest sequence wins, whichever substrate
delivered it. A substrate returning an older-sequence ad is stale,
not conflicting, and the client re-checks the next substrate when
the freshest ad it holds is expired or its endpoints all fail.

**When each shape is required:**

- Self-hosted well-known HTTPS suffices when the server (or a
  broker it authorizes) has any publicly reachable HTTPS endpoint.
- The Workers rendezvous is required when the server has no
  reachable inbound path at all — precisely the NAT / UDP-denied
  servers this feature exists for. A deployment may run both; the
  disagreement rule makes that safe.
- A self-hosted `id` broker that serves ads must serve the same
  signed object byte-for-byte (parity with (2)/(3)); it is a mirror,
  not a fourth format.

## Decision 3 — freshness, replay, and last-known-good

- Clients cache the freshest valid ad per subject
  (last-known-good) and use it past `expires_at` only when no fresh
  ad can be fetched, marking the connection degraded and
  revalidating in the background.
- `sequence` is the replay defence: a replayed older ad never
  displaces a fresher cached one, even if unexpired.
- Clock skew: clients accept `issued_at` up to 5 minutes in the
  future; servers mint a new ad (sequence + 1) on any transport
  change and at least once per TTL.
- Privacy: rendezvous fetches go through the client's normal HTTPS
  egress (including its CONNECT proxy); the Workers lookup key is
  `H(subject fingerprint)`, so the rendezvous does not learn which
  named server a client seeks beyond the hash, and query logs at
  the rendezvous are disabled by default (parent §4 metadata
  minimization). Enumeration of the rendezvous yields signed public
  ads only — the same data the subject publishes deliberately.

## Alternatives considered and rejected

- **DNS as the authoritative substrate.** Rejected: TXT payload
  limits force fragmentation of a signed multi-transport record;
  hostile-network resolver spoofing is exactly the threat model;
  TTL/propagation lag fights the 1 h ad TTL. Demoted to bootstrap
  hints (Decision 2.4).
- **Workers-only rendezvous.** Rejected: sovereignty — a self-hosted
  deployment must not depend on a third-party hosted component to
  be reachable (parent §3 raises the same axis). Workers is one
  substrate, not the substrate.
- **Unsigned discovery (public DHT/gossip, mDNS) for this case.**
  Rejected: the 2026-03-22 discovery network is UDP-shaped and so
  is unreachable in exactly the networks this ticket covers
  (circular), and unsigned records cannot authenticate a rendezvous
  point. It remains the discovery path where UDP works.
- **Advertisement only inside the relay handshake.** Rejected as
  the sole mechanism: circular for a client that does not yet know
  any reachable relay. (An in-handshake refresh of the ad is a fine
  optimisation and is compatible with this record.)
- **Per-substrate record formats.** Rejected: one signed object,
  many dumb transports is what makes the disagreement rule trivial
  and the compromised-substrate analysis hold.

## Invariants implementation must hold

1. No client ever acts on an advertisement whose signature does not
   verify against the pinned subject identity.
2. A substrate (DNS, well-known host, Workers rendezvous, broker
   mirror) can cause at most staleness or denial — never a forged
   or altered transport set. Tests must demonstrate a hostile
   substrate failing closed.
3. Highest valid `sequence` wins across all substrates; an older
   ad never displaces a fresher cached one.
4. Expired ads are used only as explicitly-degraded last-known-good,
   with background revalidation.
5. Every rendezvous fetch is plain HTTPS (optionally via the
   client's CONNECT proxy); no step of discovery requires UDP.
6. Brokers are contacted only at identities the subject's ad names;
   broker authorization stays as #5671 §4 fixes it (authenticated
   by default, dumb pipe, no keys).
7. The rendezvous stores/serves opaque signed blobs only and holds
   no identity or session keys; compromising it yields metadata
   (subject hashes, timing) and denial, never content or forgery.
8. `sequence` is durably monotonic per subject: a server restart or
   key-rotation event (per the parent identity rules) must never
   reissue a lower sequence with different content.

## Implementation split (filed and linked, not done here)

One implementation issue follows this record: the advertisement
format + sign/verify + fetch/cache client in `pkgs/id`, and the
`.well-known` serving handler — with the hostile-substrate and
sequence tests invariant 2/3 require. The Workers rendezvous
component itself stays decomposition item 4's own ticket under
#5671; this record fixes only the blob interface it must serve
(Decision 2.3).

## What this record leaves open (deliberately)

- Broker protocol/framing, multiplexing, keepalive (item 2's
  ticket), Workers component internals/limits (item 4's), CONNECT
  proxy traversal details (item 6's), and latency budgets (item
  7's) — each has its own #5671 decomposition ticket.
- Whether in-handshake ad refresh is worth building (optimisation,
  not correctness).
