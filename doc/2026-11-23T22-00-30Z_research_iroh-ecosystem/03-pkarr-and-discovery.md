# pkarr and discovery

*Part of the [iroh ecosystem research record](2026-11-23T22-00-30Z_research_iroh-ecosystem.md).*

## What pkarr is

**Pkarr — Public-Key Addressable Resource Records.** An Ed25519
keypair signs a small DNS packet (at most 1000 bytes) and publishes it
as a mutable item on the **BitTorrent Mainline DHT** (BEP-44). Anyone
can resolve the packet by public key — directly from the DHT or
through an HTTP relay — and verify the signature themselves. The
public key thereby becomes a sovereign, self-issued top-level domain
([pkarr README](https://raw.githubusercontent.com/dignifiedquire/pkarr/main/README.md)).

Mechanics that matter:

- The DHT is ephemeral: records drop after a few hours, so publishers
  (or their friends and services) **republish periodically**. TTLs are
  clamped (minimum 5 minutes, maximum 24 hours) and relays cache
  aggressively.
- **Relays** are trivial HTTP PUT/GET proxies in front of the DHT.
  They are untrusted by design: clients verify signatures end to end.
- There are **two codebases to keep apart**: the upstream
  [pubky/pkarr](https://github.com/pubky/pkarr) (Synonym/Pubky; the
  crate is now at **5.0.0**) and **n0's fork** (maintained by an n0
  engineer), which is the line iroh's address lookup was built
  against.
- pkarr 5.x also carries an "endpoints" spec feature for
  service-endpoint records.

## Role one: a transport for small signed messages

A pkarr packet is just signed DNS records under a key — nothing in the
format restricts it to addresses. In practice the ecosystem uses it as
a tiny, globally resolvable, self-certifying data channel:

- **Pubky** publishes each user's homeserver pointer as a record in
  their packet; resolving a person *is* resolving their pkarr packet.
- **pkdns** publishes whole DNS zone files as pkarr packets, making
  public keys function as resolvable domains for ordinary DNS
  clients.
- **Iroh** puts exactly one thing in the packet today: the endpoint's
  home relay URL (and optionally direct addresses).

The constraints are the design: at most 1000 bytes, hours-scale
ephemerality without republishing, minute-scale TTLs. It is a
rendezvous/config channel, not a data plane — precisely the shape of a
channel for small signed payloads (pointers, invitations, session
setup) that any third party can carry without being trusted.

## Role two: the initial discovery and relay bootstrap

This is the role iroh productionised. The problem: an `EndpointId` is
all you have; you need a *dialable* address, and the endpoint's IP
changes constantly.

- The endpoint signs a pkarr packet containing its **relay URL** and
  publishes it via HTTP PUT to a pkarr relay — by default **n0's own**
  `dns.iroh.link`, served by `iroh-dns-server`, which doubles as a DNS
  resolver and deliberately does **not** touch the Mainline DHT: a
  central-but-untrusted convenience.
- A dialer resolves the ID either from that relay over HTTP or through
  plain DNS — a TXT lookup under `dns.iroh.link` — so even non-pkarr
  software can bootstrap an iroh connection
  ([address lookup docs](https://github.com/n0-computer/docs.iroh.computer/blob/HEAD/concepts/address-lookup.mdx)).
- Defaults in code: packet TTL 30 seconds, republish every 5 minutes,
  relay-only address filter by default.
- For deployments that want the fully decentralised path, the DHT
  address-lookup variant publishes and resolves through pkarr relays
  **and** the Mainline DHT directly.

So pkarr is the bridge between "a key" and "a first connection": the
relay named in the packet then provides the always-on path and
hole-punch coordination until a direct path exists. Every other
discovery mechanism iroh ships (mDNS, static) is local or manual;
pkarr/DNS is the only global one on by default.

## The rest of the address-lookup family

- **mDNS** — local networks, including full local-first operation with
  no internet at all.
- **Static/memory lookup** — tests and pre-configured peers.
- **Blinded address lookup** — n0 published design work in October
  2026 on hiding lookup metadata, so that resolving an endpoint does
  not leak who is looking up whom. This is an active workstream, and
  it matters to us: see the
  [implications page](07-implications-for-the-fleet.md).

## Pubky: the flagship consumer

Pkarr's home ecosystem is **Pubky** (by Synonym): a key-oriented,
user-controlled web built from **Pubky Core** (pkarr identity/routing
plus homeservers that store and serve a user's data) and the **Pubky
App** (social feeds, tagging, web-of-trust curation), plus **Pubky
Ring**, the mobile key/session manager. Pubky launched publicly in
beta at BTC Prague (August 2025), positioned as a strict upgrade to
Nostr ([launch post](https://medium.com/pubky/opening-pubky-4bdca7dc4c77)).
Its SDK exposes signers, sessions, homeserver storage and pkdns
publish/resolve as one facade, and its release train is what pulled
pkarr from 3.x to 5.x.

Pubky matters to this census for two reasons: it is the proof that
pkarr works as consumer infrastructure at product scale, and it is
the reason the upstream crate is actively maintained independently of
n0's fork.

**One warning that runs through this whole record:** the two relay
sets are different infrastructure run by different parties. The pkarr
crate's default relays (`relay.pkarr.org`, `pkarr.pubky.org`) are
operated by the pkarr/Pubky side; iroh's default is n0's
`dns.iroh.link`. Same format, different operators — do not conflate
them in a design.
