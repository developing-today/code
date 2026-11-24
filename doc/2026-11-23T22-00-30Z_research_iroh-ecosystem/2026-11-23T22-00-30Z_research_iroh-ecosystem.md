# Iroh ecosystem — research record

_Compiled 2026-11-23 for the capability-auth + p2p-transport investigation
([#13779](https://github.com/developing-today/code/issues/13779)) and the
fleet transport design
([#13752](https://github.com/developing-today/code/issues/13752)). Sources:
n0-computer repos and blog, crates.io, project repos, the public GitHub
Discussions board, conference archives. Community material is paraphrased,
never quoted._

## The short version

Iroh — n0's Rust library for dialling cryptographic keys instead of IP
addresses — shipped its stable **1.0 on 2026-06-15** and is now at **1.5.0**
(2026-11-19), on a frozen wire protocol with official Python, Node.js,
Swift and Kotlin bindings. Around the core sits a first-party protocol
family (blobs, gossip, docs, plus media and tooling), the **pkarr**
discovery layer it shares with the Pubky ecosystem, and a growing set of
real production users — Delta Chat is the flagship, with decentralised-AI,
payments, film-production and human-rights deployments behind it. The
stabilisation queue is visible and deliberate: **irpc 1.0 shipped
2026-11-10, iroh-gossip is on an announced road to 1.0, iroh-docs lags,
and iroh-willow is stale** — a research artefact built on iroh 0.34-era
APIs, not a maintained release line.

## Pages in this record

- [Overview and state of iroh](01-overview-and-state-of-iroh.md) — what
  1.0 actually froze, the architecture as it now stands (identity, noq,
  relays, address lookup, custom transports), the post-1.0 release train,
  and what is announced as upcoming.
- [The protocol family](02-protocol-family.md) — every first-party
  `iroh-*` project with its job, maturity and latest release, the
  iroh-willow staleness verdict in full, and the third-party protocol and
  library layer (irpc, go-iroh, community tools).
- [pkarr and discovery](03-pkarr-and-discovery.md) — how a signed DNS
  packet on the BitTorrent Mainline DHT became both a tiny
  self-certifying message channel and iroh's global bootstrap, the two
  codebases and two relay sets to keep apart, and Pubky as the flagship
  consumer.
- [Real projects](04-real-projects.md) — who actually runs iroh in
  production and how: Delta Chat in depth (device setup, webxdc realtime
  channels, and why its calls are deliberately _not_ iroh), OpenArchive
  Save's Veilid+iroh composition, n0's own sendme/dumbpipe, and the rest
  of the shipped field with honest scale labels.
- [Community and where discussion lives](05-community-and-where-discussion-lives.md)
  — the Discord is the social home (login-gated), GitHub Discussions is
  the public record; recent important conversations paraphrased: the
  post-quantum identity design thread, the WebRTC-as-external-transport
  decision, group discovery and authorisation as open wants.
- [Talks, releases and showcases](06-talks-releases-showcases.md) — the
  show-and-tell index: 9 talks/videos, 8 written announcements, 6 demos,
  from Local-First Conf and FOSDEM to Strada's 150 TB month and Block's
  compute sharing.
- [Implications for the fleet](07-implications-for-the-fleet.md) — what
  this means for our own programme: the transport bet in #13752 is
  confirmed by a frozen wire protocol; gossip's road to 1.0 and
  `subduction_iroh` on iroh 1.0 are the near-term moving parts; the
  community's open wants (group discovery, authorisation, blinded
  lookup) are exactly the layers our auth investigation says we must
  compose ourselves.

## Headline findings

1. **Core is done being designed.** A v1 endpoint talks to any other v1
   endpoint regardless of minor version or language; wire-breaking
   changes now require a major release
   ([1.0 announcement](https://www.iroh.computer/blog/v1)).
2. **The supported toolkit for new protocol work is blobs + gossip +
   irpc.** Blobs is the hardened data plane, gossip is next to
   stabilise, irpc 1.0 is the stable RPC layer both are built on;
   docs is the laggard and willow is stalled
   ([protocol census detail](02-protocol-family.md)).
3. **pkarr is load-bearing twice over** — iroh's global bootstrap and
   Pubky's identity layer — but the two deployments use different
   relays run by different parties; do not conflate them
   ([pkarr page](03-pkarr-and-discovery.md)).
4. **Discovery is a design choice, not a default.** Delta Chat, the
   largest production user, ignores global discovery entirely and
   bootstraps from its own encrypted channel with tickets — a pattern
   directly relevant to any estate system that already has an
   authenticated channel ([real projects](04-real-projects.md)).
5. **Iroh ships authentication by key and no authorisation model at
   all.** The community's own board treats group discovery and
   authorisation as open questions — the gap our #13779 compositions
   (Meadowcap-as-spec, Keyline model, Biscuit + capnp for live
   sessions) exist to fill
   ([implications](07-implications-for-the-fleet.md)).
