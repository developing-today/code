# Deep dive: Veilid, and IPFS / libp2p in general

Lane VEILID-IPFS for the capability-auth + p2p-transport investigation
(developing-today/code#13779). Companion to the survey entries in
[capnp-and-survey.md](capnp-and-survey.md) §2.4 (Veilid) and §2.6 (libp2p),
which scored both as _competing_ with iroh for this estate. Drew asked for
the full treatment, noting Veilid "is supported to some degree" and
IPFS/libp2p "just in general". Researched 2026-11-23 from primary sources;
every load-bearing claim is linked.

**Headline:** the survey's shape survives the deep dive, with one correction
of emphasis and one major new fact. The correction: Veilid is not only a
competitor — there is a shipped, in-beta-production composition
(OpenArchive's Save) that uses **Veilid for private routing and iroh for
data**, glued by hand, which is the concrete form a "complement" verdict
would take here. The new fact: **the IPFS centre of gravity lost its
maintainers in 2026** — Kubo v0.43 is the last release from the Shipyard
team after Protocol Labs declined to renew funding — while libp2p, the
family's other half, remains broadly maintained and production-critical
elsewhere. "IPFS/libp2p" is now two different risk profiles wearing one name.

---

# Part 1 — Veilid

## 1.1 What it is

Veilid is an open-source, peer-to-peer, **mobile-first application
framework**: a Rust core library (`veilid-core`) that an application embeds
to become a node on the Veilid network, plus a headless server
(`veilid-server`), a CLI, and Flutter/Python/WASM bindings. It was launched
at DEF CON 31 (August 2023) by the Cult of the Dead Cow — presented by
Katelyn "medus4" Bowden and Christien "DilDog" Rioux — explicitly framed as
taking Tor's privacy intent and IPFS's p2p structure and rebuilding both for
app developers: no exit nodes, every node equal, no servers, no accounts
([The Register's launch coverage](https://www.theregister.com/special-features/2023/08/12/cult-of-the-dead-cow-unveils-veilid-peer-to-peer-project/830646),
[Veilid FAQ](https://veilid.com/faq/)). It is stewarded by the **Veilid
Foundation Inc.**, a non-profit that takes tax-deductible donations, seeks
no venture capital, and has no cryptocurrency or blockchain
([veilid.com FAQ](https://veilid.com/faq/)). Source is MPL-2.0 and lives on
GitLab, not GitHub ([launch coverage](https://lemmy.world/comment/2354594)).

The design centre is **metadata privacy for consumer apps**, not
infrastructure coordination. That sentence explains almost every strength
and limit below.

## 1.2 Identity and cryptography

- **Node identity is a bare Ed25519 public key** (the NodeId) in a 256-bit
  keyspace; every node is addressed by its key, and changing cryptosystem
  changes node IDs because IDs are derived from the keys
  ([developer book, print edition](https://veilid-developer-book-8b0039.gitlab.io/print.html),
  [DHT overview notes](https://gist.github.com/radicalkjax/a2a91906ffc7ccf34d3f008b5b634710)).
- The one shipped cryptosystem, **VLD0**, is deliberately conservative:
  Ed25519 for authentication/signing, X25519 for key exchange,
  XChaCha20-Poly1305 AEAD, BLAKE3 for hashing, Argon2 for key derivation —
  "established algorithms since the project didn't want to risk introducing
  weaknesses" ([DEF CON 31 internals transcript](https://media.defcon.org/DEF%20CON%2031/DEF%20CON%2031%20video%20and%20slides/DEF%20CON%2031%20video%20and%20slides%20captions/The%20Internals%20of%20Veilid%2C%20a%20New%20Decentralized%20Application%20Framework%20-%20Christien%20-%20DilDog%20-%20Rioux%20and%20Katelyn%20-%20Medus4%20-%20Bowden.txt),
  [The Register](https://www.theregister.com/special-features/2023/08/12/cult-of-the-dead-cow-unveils-veilid-peer-to-peer-project/830646)).
- Crypto is **versioned and upgradable by design**: VLD0 nodes keep working
  alongside a future VLD1, which the project says is coming to address
  post-quantum concerns
  ([ComfyCon 2025 slides](https://app.yjmbo.net/veilid-comfycon2025/index.html)).
  There is no PQ in the shipped suite today.
- Connections are authenticated, timestamped, signed, and end-to-end
  encrypted; files Veilid writes locally are encrypted, and an encrypted
  table-store API is offered to apps
  ([The Register](https://www.theregister.com/special-features/2023/08/12/cult-of-the-dead-cow-unveils-veilid-peer-to-peer-project/830646)).
- The internal RPC wire format is **Cap'n Proto** (`veilid_capnp` appears
  throughout the RPC coders) — a cross-link to the capnp lane: Veilid is a
  production user of capnp _serialization_, though not of capnp RPC's
  capability model
  ([veilid-core source, signal_info.rs](https://docs.rs/veilid-core/latest/src/veilid_core/rpc_processor/coders/signal_info.rs.html)).

There is **no bridge to any external PKI**: no certificates, no CA, no
names. Identity is key possession, full stop — the fleet CA / IdP CA would
have to be layered on top by us, exactly as with raw iroh.

## 1.3 The capability story — stated precisely

Veilid marketing-adjacent writing sometimes lets "capability" float. In the
codebase the word means something specific and much smaller:

- A **`VeilidCapability` is a four-character node feature advertisement**
  (FourCC) in a node's signed NodeInfo: `ROUT` (relays safety/private
  routes), `SGNL` (makes reverse connections on request), `RLAY` (relays
  for NAT'd peers), `DIAL` (validates dial info), `DHTV` (participates in
  the DHT), `DHTW` (performs DHT watches for others), `APPM` (accepts app
  messages/calls). Two more exist only as unimplemented feature flags:
  `TUNL` (tunnelling — "Not currently implemented") and `BLOC` (block store
  — "Not currently implemented")
  ([developer book, print edition](https://veilid-developer-book-8b0039.gitlab.io/print.html),
  [veilid-core API docs](https://docs.rs/veilid-core/latest/veilid_core/)).
  These are protocol-feature flags for routing decisions, **not**
  authorization capabilities: they are not issued, delegated, attenuated,
  scoped, or revoked.
- The actual authorization surface is exactly two mechanisms:
  1. **DHT record schemas** (§1.5): the record key commits to a schema
     naming which keypairs may write which subkey ranges. Write authority =
     holding a named secret key. It is fixed at record creation forever
     (the schema digest is inside the record key, so the schema is
     immutable), with no delegation chains, no expiry, and no revocation —
     a compromised writer key is write-authorized until the record is
     abandoned.
  2. **Private-route blobs as bearer <redacted> (§1.4): whoever holds a
     route blob can address messages to that route. Possession is the whole
     credential — transferable by copying, revocable only by the route's
     owner tearing the route down and publishing a fresh blob.
- There is **no group or membership abstraction** in the framework. Apps
  build groups out of shared DHT records and shared secrets — OpenArchive's
  Save backend does exactly this: a group is a DHT record whose owner
  keypair and a ChaCha20Poly1305 shared secret are handed to every member,
  and joining means receiving those secrets out of band
  ([Save DWeb backend privacy overview](https://hackmd.io/@s74XZjUBQDuPPS04AgPvow/HkacodHgyx)).

Verdict on this axis: **Veilid has capability-_shaped_ addressing (route
blobs) and key-scoped write slots (schemas), but no capability system** in
the sense the rest of this investigation uses the word — nothing comparable
to Meadowcap areas, Keyline delegation, UCAN/Biscuit tokens, or capnp
facets. Auth would be entirely ours to build, same as on raw iroh.

## 1.4 Transport, routing, and NAT traversal

- Transports are **UDP, TCP, WebSocket, and WebSocket Secure** — notably
  **no QUIC**
  ([developer book, print edition](https://veilid-developer-book-8b0039.gitlab.io/print.html)).
- Two routing privacy mechanisms, which are the product's reason to exist:
  - **Safety routes** protect the _sender_: a routing context flag (on by
    default) that makes Veilid build a multi-hop route through other nodes
    for outbound operations, with tunable hop count, stability, and
    sequencing preferences
    ([docs.rs RoutingContext](https://docs.rs/veilid-core/latest/veilid_core/struct.RoutingContext.html)).
  - **Private routes** protect the _receiver_: the app allocates a route,
    receives an opaque **route blob**, publishes the blob somewhere (usually
    a DHT record); a sender imports the blob to obtain a RouteId and can
    then AppMessage/AppCall the route's owner **without learning its NodeId
    or IP address**. Routes churn as the network moves; the owner must
    watch for `RouteChanged` events and re-publish fresh blobs
    ([developer book, Private Routing](https://veilid.gitlab.io/developer-book/concepts/private-routing.html)).
- Live messaging is small and RPC-shaped: `app_call` / `app_message` carry
  **at most 32,768 bytes** per message
  ([docs.rs RoutingContext](https://docs.rs/veilid-core/latest/veilid_core/struct.RoutingContext.html)).
  Bulk transfer is the app's problem (or, in Save's case, iroh-blobs').
- NAT traversal is a named subsystem, **VICE** (Veilid Internet
  Connectivity Establishment): STUN-like public-address detection performed
  _in-network_ (no specialised STUN servers), then signalling through each
  node's chosen inbound relays to negotiate **reverse connections and UDP
  hole punching**, falling back to TURN-like **inbound relaying** through
  the same peer nodes when direct establishment fails. Nodes classify their
  own NAT situation into dial-info classes (Direct, Mapped, FullConeNAT,
  Blocked, AddressRestrictedNAT, PortRestrictedNAT) and pre-validate dial
  info ahead of connection time
  ([developer book, NAT Traversal](https://veilid.gitlab.io/developer-book/concepts/nat_traversal.html),
  [dial_info_class.rs source](https://docs.rs/veilid-core/latest/src/veilid_core/network_manager/types/dial_info_class.rs.html)).
  Relays are ordinary volunteer nodes (`RLAY`/`SGNL` capabilities), not
  operated infrastructure — an altruistic-relay model, with the bootstrap
  nodes as the only foundation-run rendezvous.
- The privacy routing is a **latency and throughput tax by construction**:
  default-routed traffic takes extra hops through strangers' nodes. A
  third-party 2026 evaluation of Veilid as an agent-network transport
  concluded the privacy properties are compelling but deferred adoption,
  citing API instability, latency overhead, and DHT limits
  ([subspace-transceiver evaluation](https://github.com/banebot/subspace-transceiver/blob/HEAD/specs/planning-artifacts/veilid-evaluation.md)).

## 1.5 The DHT — Veilid's real data primitive

The DHT is "the central asynchronous data communication service" and the
discoverability layer everything else hangs off
([developer book, Distributed Hash Tables](https://veilid.gitlab.io/developer-book/concepts/dht.html)):

- **Kademlia-shaped**: XOR distance between record keys and node identities;
  a record is stored on the nearest node plus the nearest four neighbours
  (replication factor ~5); lookups are an internal "fanout" walk
  ([developer book, DHT](https://veilid.gitlab.io/developer-book/concepts/dht.html)).
- A **record** = a schema + up to a schema-defined set of numbered
  **subkeys**, total size (schema + all values) **≤ 1 MiB**. The record key
  is a hash of the schema digest and the owner's public key — you cannot
  choose a record key, and the schema can never change
  ([developer book, DHT](https://veilid.gitlab.io/developer-book/concepts/dht.html)).
- **Conflict model is per-subkey last-writer-wins by sequence number**:
  every subkey value is signed by its writer and carries a sequence number;
  a storage node accepts a write only if its sequence exceeds the one held.
  Two schemas exist: `DFLT` (owner writes all subkeys) and `SMPL` (owner
  plus named writers, each with an _exclusive_ subkey range) — i.e.
  multi-writer records are made safe by **partitioning**, never by merging.
  There is no CRDT, no log, no history: a subkey is a single-writer register
  ([developer book, DHT](https://veilid.gitlab.io/developer-book/concepts/dht.html)).
- One sharp primitive worth naming: `min_seqnum` lets a writer **burn a
  subkey's remaining write budget**, up to writing at the maximum sequence
  number, which closes the subkey to all future writes from anyone,
  permanently — a one-shot commitment slot ("a cast vote, a published
  commitment, a 'this log is frozen' marker")
  ([developer book, DHT](https://veilid.gitlab.io/developer-book/concepts/dht.html)).
- **Watches**: a node can watch subkey ranges and receive `ValueChanged`
  notifications; nodes with `DHTW` can watch on behalf of others; watches
  are leased and renewed by the watcher (the 0.5.8 storage manager runs an
  outbound watch manager with renewal timings)
  ([docs.rs RoutingContext](https://docs.rs/veilid-core/latest/veilid_core/struct.RoutingContext.html),
  [veilid-core 0.5.8 storage_manager source](https://docs.rs/crate/veilid-core/latest/source/src/storage_manager/mod.rs)).
- **Offline behaviour**: DHT writes may be deferred when the node is
  offline (`AllowOffline`, default true) and flushed later; records are
  "rehydrated" back into the DHT after absence. The store-and-forward
  pattern is therefore _write it into a DHT record the recipient will read
  later_ — which is precisely how VeilidChat delivers messages to offline
  contacts. There is no mailbox/queue protocol beyond that
  ([veilid-core API docs](https://docs.rs/veilid-core/latest/veilid_core/),
  [developer book, DHT](https://veilid.gitlab.io/developer-book/concepts/dht.html)).
- **Encryption of values**: historically the app's job; **v0.5.0 (Dec 2025)
  added "automatic default record encryption"** and DHT transactions
  ([Veilid's release announcement](https://bsky.app/profile/did:plc:3ik6gfryhy76repwatdkpzq3)).
- **Local storage**: the node's own record/table stores are an encrypted
  KV — **SQLite on native, IndexedDB on WASM** — plus a ProtectedStore over
  OS keychains for secrets
  ([ComfyCon 2025 slides](https://app.yjmbo.net/veilid-comfycon2025/index.html)).
  Note the shape: Veilid _contains_ SQLite, but as its own sealed silo —
  our Turso/libsql stores neither feed it nor read from it.

## 1.6 Maturity — what "supported to some degree" concretely means

- **Release cadence is real and current**: `veilid-core` 0.5.8 shipped
  2026-11-02; the 0.5 line began 2025-12-22 (0.5.0/0.5.1) after a steady
  0.4.x series through 2025 (0.4.1 Oct 2024 → 0.4.8 Aug 2025). Releases ship
  simultaneously as crates, PyPI packages, and Debian/RPM packages, with
  weekly builds for the apps
  ([docs.rs veilid-core versions](https://docs.rs/crate/veilid-core/latest),
  [Veilid beta page](https://veilid.com/chat/beta/)).
- **Still pre-1.0, and the project says so by its versioning**: minor
  releases carry breaking API changes (the 0.5.0 release notes headline API
  changes and new DHT features). The project has publicly pointed at a
  **1.0 "next August"** in its 0.5.x announcements; as of this writing the
  current line is 0.5.8, so 1.0 has not landed
  ([release announcement thread](https://bsky.app/profile/did:plc:3ik6gfryhy76repwatdkpzq3),
  [docs.rs versions](https://docs.rs/crate/veilid-core/latest)).
- **Flagship app is a beta tech demo**: VeilidChat is 1-to-1, text-only,
  "currently in early beta", distributed via TestFlight / Play beta and
  weekly builds — three years after launch it remains the reference demo,
  not a production messenger
  ([veilid.com beta page](https://veilid.com/chat/beta/),
  [AlternativeTo listing](https://alternativeto.net/browse/all/?mf=encrypted-chat,end-to-end-encryption&p=3)).
- **One genuine third-party production-ish deployment exists**:
  OpenArchive's **Save** app (human-rights media archiving) runs a DWeb
  storage backend **in beta production** that uses **Veilid for encrypted,
  onion-routed peer connections and iroh for blob storage, replication and
  verification**, with custom glue architecture written by Hypha Worker
  Co-operative to bridge the two; their 2025 white paper documents a
  protocol comparison that chose exactly this pair
  ([OpenArchive, Decentralization as Human Rights Defense](https://www.open-archive.org/news/decentralization-as-human-rights-defense-in-an-ai-shaped-world),
  [OpenArchive/Hypha white paper](https://www.open-archive.org/pdf/Mobile_DWeb_Archiving.pdf)).
- **Network size is unpublished and evidently small**: the project
  publishes no node statistics; independent 2026 assessments describe a
  "tiny ecosystem" with "no large-scale third-party deployment data", in
  contrast to libp2p's and iroh's six-figure networks
  ([OCM mesh research, 2026-05](https://github.com/opencircuitdev/opencircuitmodel/blob/HEAD/docs/superpowers/research/2026-05-08-mesh-distributed-compute.md)).
- **Documentation is self-described as unfinished**: the developer book is
  "a work in progress" and the internals book is still "forthcoming"
  ([developer book introduction](https://veilid.gitlab.io/developer-book/)).
- Governance/support shape: a small foundation team (the two DEF CON
  presenters remain the public faces), donation-funded, community on
  Discord. Serious, credible, long-lived people — and a bus factor and
  funding base far thinner than libp2p's or even p2panda's grant stack
  ([veilid.com FAQ](https://veilid.com/faq/)).

## 1.7 What Veilid pointedly does NOT do

No QUIC. No bulk/streaming data plane (32 KB app messages; the block store
capability is unimplemented). No sync semantics beyond single-writer
registers. No groups. No capability delegation/attenuation/revocation. No
identity beyond key possession. No Cloudflare story at all (a Veilid node
is a long-lived UDP/TCP peer; nothing runs in Workers — the WASM build is a
browser _client_ of the Veilid network, not an edge component). No
performance story for infrastructure traffic — privacy hops are the point.

---

# Part 2 — IPFS / libp2p in general

## 2.1 The family's current shape

IPFS is a **content-addressed block distribution system**: data is chunked
into a Merkle-DAG, addressed by CID (multihash digest + codec), announced
via provider records on the **Amino DHT** (the public Kademlia network),
fetched with **Bitswap**, and named mutably — barely — by **IPNS**, signed
sequence-numbered records that point a key at a CID. HTTP gateways make the
whole thing readable by browsers. The reference implementation is **Kubo**
(Go), the shared library stack is **Boxo**, the JS implementation is
**Helia**, and the production gateway implementation is **Rainbow**
([Kubo v0.43.1 release](https://github.com/ipfs/kubo/releases/tag/v0.43.1)).

libp2p, spun out of IPFS, is the **modular networking suite** underneath:
multiaddr addressing, PeerId identity (a hash of the public key; Ed25519
keys inline), transports (TCP, QUIC-v1, WebSocket, WebRTC, WebTransport),
channel security (Noise as the baseline, or TLS 1.3 with a key-binding
certificate extension), yamux multiplexing, and a protocol shelf — identify,
ping, fetch, **kad-dht**, **gossipsub** (v1.0–1.3, with peer scoring from
v1.1), rendezvous, **AutoNAT**, **Circuit Relay v2**, and **DCUtR** hole
punching
([libp2p spec index, as summarised from the specs repo](https://github.com/ahmed-abdul-rahman/de-chat/blob/HEAD/.cursor/skills/libp2p-spec/SKILL.md),
[rust-libp2p on lib.rs](https://lib.rs/crates/libp2p)).
Crucially, NAT traversal in libp2p is **assembled, not included**: AutoNAT
to learn your reachability, a relay reservation to be contactable, DCUtR to
upgrade relayed connections to direct ones — three protocols the operator
must each wire and tune
([libp2p spec index](https://github.com/ahmed-abdul-rahman/de-chat/blob/HEAD/.cursor/skills/libp2p-spec/SKILL.md)).

## 2.2 The 2026 stewardship break (the family's headline fact)

- **Kubo v0.43 (and the v0.43.1 fix release, Sep 2026) is the last Kubo
  from the Shipyard team.** Protocol Labs declined to renew Shipyard's
  funding; the team's IPFS work **ended 2026-09-30**. Shipyard's
  announcement lists **Kubo, Helia, Boxo, Rainbow, IPFS Desktop, IPFS
  Companion** and others as losing dedicated maintainers, and the public
  gateways and bootstrap infrastructure it operated as winding down;
  Protocol Labs describes a transition to lighter-weight stewardship via
  foundation grants to independent maintainers
  ([operator notes on the 0.43.1 release and the Shipyard announcement](https://www.tismtek.com/articles/kubo-0-43-1-bootstrap-off-switch.html),
  [IPFS Desktop releases, carrying the same notice](https://github.com/ipfs/ipfs-desktop/releases)).
- The hosted layer around IPFS had already been dying in sequence:
  **Cloudflare's public gateway stopped serving IPFS on 2024-08-14**
  (traffic handed to the foundation gateway — which is part of what is now
  winding down), nft.storage Classic ended uploads in 2024, Estuary shut
  down in 2023, and **Infura's IPFS service shut down in August 2026**
  ([gateway/pinning graveyard](https://wiki.bitsy.services/wiki/cs/ipfs/pinning/providers/),
  [Cloudflare's transition post](https://static.obstracts.com/feed/aee1ee27-a4ba-526d-97e3-c955ffc172d9/posts/a28f50b6-0933-5061-9e42-f32bcbef4ddc/a28f50b6-0933-5061-9e42-f32bcbef4ddc_cloudflares-public-ipfs-gateways.pdf)).
- **libp2p is not in the same position.** Its maintenance is distributed
  across Protocol Labs (go-libp2p, v0.47 in Jan 2026) and the production
  client teams that depend on it; rust-libp2p shipped facade **0.57.0 in
  Sep 2026** (0.57.1 followed in October), draws ~718k downloads/month, and
  is used by ~757 crates
  ([lib.rs, libp2p](https://lib.rs/crates/libp2p),
  [libp2p.io projects feed](https://www.libp2p.io/projects/)).
  Any sentence about "IPFS maintenance" that lumps libp2p in with Kubo is
  now wrong in both directions.

## 2.3 Where the public network is healthy, and where it is not

Healthy:

- **The protocol engineering finally landed.** The provider-record system
  was IPFS's best-documented scaling failure: records expire after 48h,
  Kubo reprovides every 22h, and the legacy provider needed a ~15-second
  DHT walk _per CID_, so pinsets beyond roughly 5–10k CIDs silently stopped
  being findable while still pinned locally
  ([content routing notes](https://wiki.bitsy.services/wiki/cs/ipfs/content-routing/),
  [Provide Sweep write-up](https://guillaume.michel.id/posts/dht-provide-sweep/)).
  **Provide Sweep** (default since Kubo 0.39) batches announcements by
  keyspace region — **97% fewer DHT lookups** — and Optimistic Provide cut
  publication time ~10×; the DHT bottleneck for self-hosters is largely
  engineered away
  ([Kubo v0.39.0 release](https://github.com/ipfs/kubo/releases/tag/v0.39.0),
  [IPFS newsletter](https://newsletter.ipfs.tech/archive/u1f333-ipfs-newsletter-mainnet-improvements-web/)).
  The bitter footnote is that this landed in the final year of the team
  that built it.
- **libp2p's production base is the largest in p2p.** Ethereum's consensus
  layer runs gossipsub in every client implementation; rust-libp2p's named
  users include Lighthouse (Ethereum consensus), Forest (Filecoin in Rust),
  fuel-core, and Espresso's HotShot
  ([lib.rs, libp2p](https://lib.rs/crates/libp2p),
  [ethereum.org on rust-libp2p](https://ethereum.org/developers/tools/rust-libp2p/)).
  Even there, gravity has limits: Parity built **litep2p**, a leaner
  libp2p-compatible stack (v0.13.x, actively released), rather than keep
  carrying rust-libp2p inside Polkadot — the ecosystem's own heavyweight
  user concluded the full stack was more than it wanted
  ([libp2p.io projects feed](https://www.libp2p.io/projects/)).

Struggling:

- **Retrievability and replication are thin.** A 2025 measurement study
  (three years of traffic, 20B+ messages) found only **2.71% of files
  replicated more than 5 times**, and **5% of peers hosting over 80% of
  content**, with centralisation _growing_ over the three years
  ([WWW '25, Centralization in the Decentralized Web](http://tddg.github.io/assets/pdf/www25-ipfs-dedup.pdf)).
  Content on IPFS persists because someone specific pins it, not because
  "the network" keeps it.
- **The maintainers themselves centralised the routing.** The NSDI '24
  paper by IPFS's own team documents the three hybrid components they had
  to build — IPNI indexers, Hydra booster DHT nodes, HTTP gateways —
  because pure-DHT lookup latency and publication cost (measured at over a
  minute per publication in the worst cases) didn't work at scale
  ([The Eternal Tussle, NSDI '24](https://tysong.github.io/files/NSDI24.pdf)).
- **Hosting concentration is a live security property**: a 2026 routing
  study found providers concentrated in a few clouds/ASes such that a
  single malicious AS could block >75% of content from 57% of requesters
  ([Routing Centralization and Censorship in IPFS](https://securalab.org/publications/pdfs/SSS26routing.pdf)).
- **Mutable state was never really solved**: IPNS gives one signed pointer
  per key with sequence numbers — a register, not a data model. Everything
  else (logs, CRDTs, databases) is an application-layer construction on top
  of immutable blocks, which is why every serious IPFS app ends up building
  its own sync layer anyway.

## 2.4 Auth and capability story

There essentially isn't one, and the family is honest about that by
omission. A PeerId proves key possession inside the Noise/TLS handshake;
provider and IPNS records are signed by their keys; kad-DHT record
validation covers IPNS and public-key records. Beyond that: **no
delegation, no attenuation, no groups, no revocation, no access control on
content** — a CID is a bearer address readable by anyone who learns it, and
confidentiality means encrypt-before-you-add with key distribution as your
own problem. (It is not an accident that the most serious "IPFS storage
with authorization" system in production, Storacha, bolts UCAN on top —
see the UCAN material in [capnp-and-survey.md](capnp-and-survey.md).)
libp2p's one access-control-shaped feature, the private-network PSK
(`pnet`), is a single shared-secret fence for a whole swarm — the opposite
of per-principal capability discipline.

## 2.5 Why iroh left — the primary-source version

iroh _began_ as a Rust implementation of IPFS with working interop. The
team's own retrospective gives three concrete reasons for the break
([The road to iroh 1.0](https://aggr.aymericbeaumet.com/items/lobste-rs/2026-10-16-the-road-to-iroh-1-0/)):

1. **A narrow real use case was unbuildable on the stack.** Provisioning a
   new DeltaChat device meant moving one large resumable blob under hard
   disk/network constraints; the IPFS-based implementation couldn't hit
   acceptable network performance and dragged in "a very large dependency
   tree". The from-scratch QUIC + BLAKE3 side project outperformed the main
   project on the thing that mattered.
2. **rust-libp2p ergonomics.** With two days to add networking to a demo,
   "adding networking via rust-libp2p seemed impossible"; quinn (QUIC)
   was, in their words, "an absolute joy". The replacements were
   one-for-one simplifications: **multiaddrs → an Ed25519 public key;
   multihashes → BLAKE3; many transports → just QUIC.**
3. **Scope philosophy.** Per the n0 CTO, quoted in interview: p2p projects
   default to "boil-the-ocean" — a DHT, transports, pubsub, RPC — and "no
   one wants the nginx team to ship postgres". iroh deliberately kept only
   the transport layer and pushed sync/blobs/gossip up into composable
   application protocols
   ([LambdaClass interview with the iroh team](https://blog.lambdaclass.com/the-wisdom-of-iroh/)).

Independent adopters echo the same trade from the other side: a 2025
production postmortem chose iroh over libp2p for a gossip layer citing
exactly the "**configuration tax**" — every libp2p deployment is a snowflake
(yamux vs mplex, Noise vs TLS, mDNS vs seeds, gossipsub versions), while
iroh ships one opinionated NAT-traversal story
([iroh in production: encrypted-note gossip](https://blog.skill-issue.dev/blog/vanta_iroh_gossip_in_production/)).
And the bet has since hardened: **iroh 1.0 shipped 2026-06-15** with
wire-protocol stability commitments, and n0 reports >200M endpoints created
on its public relays in the preceding 30 days
([TechTimes on the iroh 1.0 release](https://www.techtimes.com/articles/318490/20260616/peer-peer-library-iroh-10-ships-dial-devices-key-not-ip-address.htm)).

What libp2p/IPFS would still give this estate that iroh does not:

- **Joinable public networks.** Ethereum, Filecoin, and the IPFS content
  network exist _now_, with strangers in them. iroh gives you a superb
  dial tone and no one to call but your own endpoints.
- **Content distribution at public scale with integrity built in**: a CID
  published once is fetchable from any provider, deduplicated network-wide,
  and readable through any HTTP gateway — a distribution property no
  key-addressed transport provides.
- **Browser reach**: WebRTC/WebTransport transports and verified-fetch in
  the browser are libp2p/IPFS home turf.
- **gossipsub with a decade of adversarial tuning** (peer scoring, mesh
  maintenance) for open-membership broadcast — a different beast from
  iroh-gossip's smaller-scale topic swarms.

What it costs: the assembled NAT stack, the configuration surface, the
dependency weight, an identity format (PeerId/multiaddr) that would sit
_beside_ our Ed25519/iroh identity rather than being it, and — for the IPFS
half — adopting infrastructure whose core team was disbanded this year.

---

# Part 3 — Estate fit, axis by axis

| Axis                       | Veilid                                                                                                                                                                      | IPFS / libp2p                                                                                                                                                               |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Identity & keys            | Bare Ed25519 NodeId; versioned cryptosystems (VLD0); no names, no CA bridge                                                                                                 | PeerId = key hash (Ed25519 inline); IPNS keys for naming; no CA bridge                                                                                                      |
| Capability model           | **None.** FourCC feature flags misnamed "capabilities"; DHT schema writer slots (immutable, key-possession); route blobs as bearer tokens                                   | **None.** Key possession in Noise/TLS; PSK swarm fence; UCAN etc. must be bolted on (Storacha precedent)                                                                    |
| Groups                     | None — apps fake it with shared records + shared secrets                                                                                                                    | None — swarms/topics only                                                                                                                                                   |
| Encryption                 | Strong in transit (VLD0 AEAD, signed envelopes) + IP-hiding routes; at-rest local stores encrypted; DHT values app-encrypted, default encryption since 0.5.0; **no PQ yet** | In transit via Noise/TLS; content is plaintext-by-default, encrypt-it-yourself; integrity (hash addressing) is the real guarantee                                           |
| Transport & NAT            | Own stack (UDP/TCP/WS/WSS, **no QUIC**); VICE: in-network STUN-like detection, signalled reverse-connect + hole punch, peer relays; privacy hops add latency                | Broadest transport menu incl. browsers; NAT = AutoNAT + Relay v2 + DCUtR, assembled and tuned by the operator                                                               |
| Sync semantics             | Per-subkey single-writer LWW registers, ≤1 MiB/record, watches; no log, no CRDT, no merge                                                                                   | Immutable DAG + provider records; IPNS single-pointer register; all mutable sync is app-built                                                                               |
| Offline behaviour          | DHT store-and-forward is the native pattern; deferred offline writes; record rehydration                                                                                    | Fetch-when-connected; pinning for persistence; nothing is delivered to an offline peer                                                                                      |
| Storage fit (SQLite/Turso) | Contains its own encrypted SQLite silo (native); Turso neither feeds nor reads it                                                                                           | Kubo uses flatfs + LevelDB/Pebble metadata stores; libp2p itself is storage-agnostic; no Turso story                                                                        |
| Rust maturity              | veilid-core 0.5.8 (2026-11), steady cadence, **pre-1.0 with breaking minors**                                                                                               | rust-libp2p 0.57.x, ~718k dl/mo, production users; heavyweight, also pre-1.0 by version, stable in practice                                                                 |
| Cloudflare fit             | None — long-lived p2p node; WASM build is a browser client only                                                                                                             | Poor and worsening: Cloudflare exited the gateway business (2024); Workers cannot run Kubo/libp2p natively; R2 already does the useful part (artifact storage) without IPFS |
| iroh relationship          | **Competitor as transport; proven complement as privacy layer** (Save runs Veilid routing + iroh data)                                                                      | **Competitor libp2p rejected by iroh's own team on record; IPFS is a different category (distribution network, not transport)**                                             |
| Spec / interop             | Single implementation, single network; spec is the code + developer book                                                                                                    | The interop champion: multi-language specs, cross-implementation test suites, existing public networks                                                                      |
| Project health             | Non-profit foundation, donation-funded, small team, steady releases, beta flagship, one real third-party deployment                                                         | Split: Kubo/Helia/Boxo/gateways **lost their team Sep 2026**, stewardship transition unproven; libp2p healthy via PL + production client teams                              |
| Operational complexity     | One embedded library + bootstrap dependence; but a second identity space, second NAT stack, second network to monitor                                                       | Kubo is a daemon ecosystem of its own; libp2p is a configuration discipline of its own; both duplicate machinery iroh already runs here                                     |

## 3.1 Could Veilid's DHT be a discovery/rendezvous layer under our protocols?

Technically yes, and it is the only Veilid-shaped idea that survives
contact with the estate: a seat publishes a small DHT record — "endpoint
hints for key X, current private-route blob, expiry" — and peers watch it.
That is, in fact, exactly the role the DHT plays inside Veilid itself
(route blobs are distributed through it) and in Save's group discovery
([developer book, Private Routing](https://veilid.gitlab.io/developer-book/concepts/private-routing.html),
[Save backend overview](https://hackmd.io/@s74XZjUBQDuPPS04AgPvow/HkacodHgyx)).

But priced honestly for _this_ estate: it means embedding a second,
pre-1.0 p2p stack (second identity space, second NAT traversal, second
network whose bootstrap and relay capacity are donated by strangers) to
obtain a rendezvous function that the #13752 design already gets from
iroh's own discovery (pkarr/DNS + relays we operate) and that the
Cloudflare doors already provide with accounts attached. The one thing it
adds that nothing else in our stack has is **IP-address privacy between
peers** — and the estate's stated hard requirements (no secret material in
logs, capability discipline, self-hosting) never ask for peer-IP anonymity;
our seats are our own machines, not dissident handsets. If a future
requirement does ask for metadata-private rendezvous, the Save composition
is the proven template and Veilid becomes a _sidecar_, adopted for that
requirement — not a substrate.

## 3.2 Could IPFS be just a distribution channel for artefacts?

Yes, and this is the only IPFS-shaped idea that survives: publish fleet
release binaries, book sites, or the nix cache as CIDs; anyone (including a
browser via a gateway) can fetch with integrity verification. But measured
against what the estate already runs — R2 buckets, GitHub releases, a
Cloudflare-served nix cache — IPFS adds a _public mirror_, not a missing
capability, and it adds it at the price of running Kubo, maintaining pin
discipline (the replication data says unpinned-by-others content is
effectively single-homed), and depending on gateway infrastructure whose
operator just wound down. As an optional public mirror for release
artefacts it is harmless and occasionally useful; as infrastructure it
would be adopting the family's weakest limb at its weakest moment.

## 3.3 What adopting either as the base would displace

- **Veilid as base** displaces iroh _and_ p2panda-net while providing
  neither QUIC performance nor sync semantics — the estate would rebuild
  logs/CRDTs on 1 MiB LWW registers and rebuild auth from nothing, inside a
  pre-1.0 API, to gain privacy properties it did not ask for.
- **libp2p as base** displaces iroh and reverses the #13752 design on the
  strength of interop with networks (Ethereum/Filecoin/IPFS) the estate
  has no requirement to join, at the documented configuration and
  dependency cost that made iroh's team and at least one other production
  adopter walk away.
- **IPFS as base** is a category error for fleet operations: it is a
  distribution network for immutable content, and the fleet's centre of
  gravity is mutable, authorized, small-group state.

---

# Verdicts

**Veilid — complement, narrowly; not an alternate base.**
Real advantages: the only framework in the investigation whose _product_
is metadata privacy (safety/private routes are genuinely well-shaped, and
route blobs are a clean rendezvous primitive); a conservative, versioned
cryptosystem; a Kademlia DHT with watches and an elegant one-shot-subkey
primitive; a Rust core with a steady release cadence and a non-profit
steward; and one real composition precedent (Save) proving it layers under
an iroh data plane. Blockers for this estate: pre-1.0 with breaking minors
and 1.0 still not landed; no capability system (the name notwithstanding),
no groups, no sync semantics, 32 KB messages, no QUIC, no Cloudflare
surface; a small, unpublished-size network and a thin funding/team base;
documentation still in progress. "Supported to some degree" resolves to:
_actively maintained by a small foundation, packaged everywhere, one beta
flagship app, one beta-production third-party deployment_ — supported
enough to adopt as a sidecar for a privacy requirement, not enough to bet
the fleet's transport on.

**IPFS — neither as base; optional public mirror only.**
Its real advantage is unique and should be stated plainly: it is the only
_existing public content-addressed distribution network_ — publish once,
fetch from strangers, verify by hash, read from a browser. Nothing else in
this investigation, iroh included, offers that. Its blockers are equally
plain: no auth model, no mutable-state model, persistence-by-pinning
discipline, gateway dependence for web reach, and a stewardship vacuum at
its centre as of late 2026 (Kubo/Helia/Boxo/Rainbow team disbanded
2026-09-30; gateway/bootstrap wind-down; replacement stewardship
unproven). Adopting IPFS as estate infrastructure this year means adopting
an ecosystem mid-transition for a distribution property the estate mostly
already has via R2/GitHub.

**libp2p — neither as base; keep as the interop escape hatch (survey
verdict stands, strengthened).**
It is the healthiest limb of its family: multi-language specs, the largest
production base in p2p (Ethereum consensus, Filecoin), a mature Rust
implementation with current releases. It would earn its place the day the
estate must _join_ an existing libp2p network or ship browser-native peers
at scale. Until then it duplicates — at higher configuration and dependency
cost, per its own alumni's published reasoning — the transport job iroh
already does here with the identity format the estate has standardized on.
Note for the record that even its heaviest historical Rust user, Polkadot,
moved to the leaner litep2p rather than deepen the commitment.
