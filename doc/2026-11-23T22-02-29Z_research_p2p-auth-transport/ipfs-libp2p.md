# IPFS / libp2p — judged as a base: neither, and the stewardship story is why

Full lane notes: Part 2 of research lane VEILID-IPFS for
[#13779](https://github.com/developing-today/code/issues/13779), from
project announcements, implementation repos, crates.io, and measurement
literature, read 2026-11-23. The Veilid half of the same lane has its
[own page](veilid.md).

**Judgment up front: neither IPFS nor libp2p as a base for this estate.**
libp2p itself is healthy software; IPFS-the-ecosystem just went through
a stewardship collapse that is the single best cautionary data point in
this whole investigation about building on other people's p2p
infrastructure; and both were designed for a problem — a public,
permissionless, content-addressed web — that is adjacent to, but not,
this estate's problem (named seats, capability auth, private sync).
iroh exists precisely because its authors (several of them libp2p/IPFS
veterans) concluded the general-purpose stack's generality was the cost.

## The stewardship shock, stated plainly

**Kubo v0.43 is the last release from Interplanetary Shipyard.** The
Shipyard — the organisation that maintained the core IPFS implementations
after Protocol Labs spun them out — announced the end of its IPFS work
effective **2026-09-30**, after Protocol Labs declined to renew funding.
The affected surface is the centre of the ecosystem: **Kubo** (the
reference node), **Helia** (the JS implementation), **Boxo** (the shared
Go library), **Rainbow** (the gateway implementation), plus Desktop and
Companion. These projects did not get a new steward with the
announcement; they lost their dedicated maintainers.

The surrounding infrastructure had already been retreating:

- **Cloudflare's public IPFS gateway** — for years the default "just
  fetch it via https" path — exited on **2024-08-14**.
- **Infura's IPFS service** ended in **August 2026**.
- Public bootstrap and gateway capacity is in wind-down alongside the
  implementations.

None of this means IPFS content disappears (content addressing is
content addressing; anyone can run the code). It means: the *maintained
centre* that an adopter in 2023 would have relied on — reference node,
JS stack, gateways, hosted pinning — has dissolved within one funding
decision. For an estate choosing infrastructure to build a decade on,
this is the data point: **the risk was never the protocol's elegance; it
was a single funder's renewal.** (Contrast, within this same
investigation: p2panda's grant-to-grant smallness is at least *visible*
and plural; libp2p's maintenance is anchored by infrastructure users —
below.)

## libp2p itself: healthy, and still not the answer

libp2p the library is in a different position from IPFS the product:

- **rust-libp2p** is actively released (facade crate 0.57.x across
  September–October 2026), with roughly **718k downloads/month** on
  crates.io, and production users whose incentives are structural —
  Ethereum consensus clients (Lighthouse), Filecoin (Forest), Substrate.
  It will be maintained because consensus networks run on it.
- Even so, commitment has limits: **Parity built litep2p** — a ground-up
  alternative implementation — rather than deepen its investment in
  rust-libp2p for Substrate's future. When your largest users hedge by
  rewriting, the library is a commons, not a platform.

The technical judgement matches the organisational one. libp2p is a
*menu*: multiaddr addressing, multihash/multicodec registries, a dozen
transports, pluggable muxers, Kademlia, gossipsub, relay, AutoNAT,
DCUtR hole-punching — assembled per deployment. That assembly is the
famous tax: NAT traversal in libp2p is *assembled* (AutoNAT to learn
your situation + Circuit Relay v2 for fallback + DCUtR to upgrade to
direct), provider/DHT behaviour varies by configuration, and two
libp2p applications interoperate only after agreeing on most of the
menu. **iroh is the deliberate refusal of that menu**: one addressing
scheme (an Ed25519 key), one hash (BLAKE3), one transport family
(QUIC, direct-first with relay fallback), shipped as decisions rather
than options — and iroh 1.0 shipped in June 2026 with that posture
intact. This estate already made that bet; nothing in the libp2p
landscape argues for unmaking it.

## IPFS the protocol, on the axes that matter here

- **Content addressing is genuinely good** — CIDs, Merkle-DAGs, CAR
  files are the best-developed vocabulary in the field for "name data by
  its hash and verify it anywhere," and pieces of that vocabulary are
  already inside this programme (UCAN's CIDs are IPFS CIDs; p2panda
  interops with the same hashing world). Adopting the *vocabulary* is
  free and already done; it does not require adopting the network.
- **Discovery/replication economics are the measured weak point.** DHT
  provider records expire at **48 hours** (content must be perpetually
  re-announced by *someone*), and measurement studies of the public
  network find only **2.71% of files replicated more than 5 times** and
  **5% of peers hosting over 80% of content** — the public network is, in
  practice, a small set of pinning services and gateways wearing a p2p
  protocol. The one bright spot: **Provide Sweep** (default since Kubo
  0.39) substantially fixed provider-record scaling, with a claimed 97%
  reduction in DHT lookups for large providers — evidence the design
  *can* be repaired, arriving in the same season its maintainers left.
- **IPNS** (mutable naming) is a single signed pointer register: one key,
  one current value, sequence-numbered. Fine as a "latest version"
  pointer; it is not a sync protocol and has no multi-writer story.
- **No capability/auth model.** IPFS has no authorisation layer beyond
  key possession (IPNS keys, private-network pre-shared keys as a
  network-admission blunt instrument). Everything this investigation
  exists to decide — delegation, attenuation, groups, revocation — would
  still be entirely to build. Bitswap/gossipsub are distribution
  mechanisms for public data; private, authorised replication is not
  what they are for.

## What remains defensible

Exactly one use survives the judgement: **IPFS as an optional public
artefact-distribution mirror.** If the estate ever wants release
artefacts, public datasets, or documentation mirrored in a
content-addressed public network (fetchable by CID, verifiable offline,
hostable by anyone), publishing *to* IPFS is a reasonable distribution
decision — with eyes open about pinning (you are your own pinning
service now; the hosted ones left) and about stewardship (pin the Kubo
version you test against; the maintenance centre is gone). That is a
publishing choice about public artefacts, **not an architecture
adoption**: no seat identity, no auth, no private sync should ever be
designed on top of it.

## The lesson the lane actually files

The IPFS outcome is the investigation's cleanest evidence for a rule the
estate should keep: **when evaluating p2p infrastructure, weight
stewardship structure over protocol merit.** Ask who maintains it, paid
by whom, with what succession if the funder walks — before asking how
elegant the DHT is. Applied across this bundle, that rule is part of why
the synthesis prefers composing boring, separately-maintained layers
(iroh, p2panda, Biscuit, SQLite) over adopting any single totalising
p2p platform.
