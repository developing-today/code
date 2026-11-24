# The protocol family

*Part of the [iroh ecosystem research record](2026-11-23T22-00-30Z_research_iroh-ecosystem.md).*

Everything here lives in the [n0-computer GitHub org](https://github.com/n0-computer)
and is listed on the [protocol registry page](https://www.iroh.computer/proto)
where noted. Since the 0.28–0.29 releases these are independent
crates/repos that an application adds explicitly — iroh core no longer
bundles or privileges them
(["Let them have crates"](https://www.iroh.computer/blog/iroh-0-28-let-them-have-crates),
["net is the new iroh"](https://www.iroh.computer/blog/iroh-0-29-net-is-the-new-iroh)).

## First-party protocols and tools

| Project | What it does | Maturity / latest |
|---|---|---|
| **iroh-blobs** | Content-addressed blob transfer: BLAKE3-verified streaming, resumable downloads, multi-provider download, persistent and in-memory stores, hash-and-format tickets (ALPN `/iroh-bytes/4`) | 0.103.0 shipped aligned with iroh 1.0 (2026-06-15); the most production-hardened of the family |
| **iroh-gossip** | Topic-based broadcast overlay: HyParView partial-view membership plus a PlumTree eager/lazy-push broadcast tree (ALPN `/iroh-gossip/1`) | 0.101.0 (2026-06-15), 0.102.0 (2026-11-03), **0.103.0 (2026-11-16)**; explicitly on "the road to 1.0" |
| **iroh-docs** | Multi-writer, eventually consistent key-value sync: entries keyed by (namespace, author, key), values stored as blobs, replicas reconcile via range-fingerprint set reconciliation (ALPN `/iroh-sync/1`) | 0.101.0 (2026-06-15); the least-frequently released of the three core protocols |
| **iroh-relay** | The relay server (and client) implementation; WebSocket-based relay protocol after the pre-1.0 overhaul | Ships inside the iroh workspace and versions with core (1.5.0); this is the code n0 runs for the public relays |
| **iroh-willow** | Implementation of the [Willow protocol](https://willowprotocol.org) (data model + Meadowcap + Confidential Sync) using iroh for transport and iroh-blobs for payloads | **Experimental / stale — see below.** Crate 0.28.0 carries a "not released yet" notice on docs.rs |
| **iroh-roq** | RTP over QUIC as an iroh protocol — packetises RTP media over QUIC streams and datagrams | Registry-listed; the media building block under the `callme` demo |
| **iroh-live** | Live audio/video streaming over iroh using Media over QUIC: publish/subscribe broadcasts and 1:1 "call" sugar | Active n0 repo; the current home of n0's calls line of work — the earlier calls experiments were rebuilt as MoQ-based live streaming |
| **callme** | Demo app for iroh-roq: Opus voice calls, with CLI and desktop/Android clients (ALPN `/callme/rtc/0`) | n0 demo; shows the roq stack end to end |
| **iroh-content-discovery** | A tracker protocol so blob providers and consumers can find each other for iroh-blobs content | Registry-listed protocol |
| **iroh-ping** | Minimal ping protocol for iroh endpoints — the canonical "smallest real protocol" example | Registry-listed; example-grade by design |
| **dumbpipe** | "Netcat over iroh": pipes stdin/stdout between endpoints by ticket; also TCP and unix-socket forwarding, and stable listeners via generated tickets | Standalone n0 tool, packaged widely (Homebrew, Nix, Termux in progress); versions in lockstep with sendme |
| **sendme** | File/directory transfer CLI on iroh-blobs: send a path, print a ticket; receive by ticket with BLAKE3-verified, resumable streaming | **0.37.1** current; on Homebrew/Nix/Arch/Scoop/winget; the family's flagship end-user tool ([sendme page](https://www.iroh.computer/sendme)) |
| **iroh-doctor** | Standalone connectivity diagnostics CLI | Released as its own tool per the roadmap |
| **iroh-dns-server** | The server behind `dns.iroh.link`: a pkarr relay plus DNS resolver (see the [pkarr page](03-pkarr-and-discovery.md)) | Ships in the iroh workspace; self-hostable |
| **iroh-ffi** | Official bindings: Python, Node.js, Swift, Kotlin | Rebuilt on the stable 1.0 API and officially supported |
| **noq** | n0's QUIC implementation (quinn fork): multipath, address discovery, NAT traversal | Separate n0 repo; versions with iroh (noq 1.5.0) |
| **net-tools / netwatch / portmapper** | Cross-platform networking utilities factored out of iroh (netcheck, port mapping, interface watching) | Separate n0 repos/crates |

## iroh-willow: state, and why it is stale

The verdict, confirmed by this census and unchanged from the parent
programme's Willow research:

- docs.rs still serves crate **0.28.0** with the notice that the work
  is not released yet
  ([docs.rs](https://docs.rs/iroh-willow/latest/iroh_willow/)).
- Its dependencies are not crates.io releases but git patch-overrides
  to n0-computer forks of the Willow crates, plus a feature branch of
  iroh-blobs — the signature of a co-development snapshot, not a
  maintained release line.
- The version number is keyed to the *old* iroh numbering: there is no
  iroh-willow release aligned with iroh 0.9x, the 1.0 release
  candidates, or 1.x, while blobs, gossip and docs all made that
  migration.
- Independent corroboration: a third-party stack that vendored the
  iroh tree in October 2026 lists iroh-willow in its own inventory as
  a broken build with no consumers.

**Why:** n0's protocol effort went into stabilising core (1.0), then
gossip's road to 1.0 and irpc 1.0 (below); Willow's own sync spec
(Confidential Sync) is paused upstream, so the implementation has no
stable target to converge on. Treat iroh-willow as a research
artefact: the design — Willow data model plus Meadowcap over iroh
transport, payloads in iroh-blobs — is proven in code, but adopting it
means forking and porting it to iroh 1.x yourself.

## The RPC layer: irpc 1.0

**irpc** is n0's request-response RPC framework built *for* iroh
protocols — blobs, gossip and docs internals run on it.
[**irpc 1.0 shipped 2026-11-10**](https://www.iroh.computer/blog/irpc-1-0)
("Stable RPC for the iroh ecosystem"), making it a stable public
building block for anyone writing an iroh protocol. Historically there
were several community `iroh-rpc` crates; irpc is the survivor and the
one with a stability commitment. Anyone designing a new iroh protocol
today should start from irpc 1.0 rather than a community crate.

## Third-party protocols and libraries

- **Subduction / Keyhive (Ink & Switch).** `subduction_iroh`
  implements Subduction's transport trait over iroh, with its own
  signed handshake (sender binding, timestamp, challenge, endpoint-ID
  continuity) *above* iroh's TLS layer, because Subduction's threat
  model does not delegate identity to the transport
  ([PR #63](https://github.com/inkandswitch/subduction/pull/63)).
  Subduction was **upgraded to iroh v1.0** in mid-2026, and Teamtype
  made the same jump — both recorded in Automerge's July 2026
  ecosystem post ([Automerge blog](https://automerge.org/blog/2026-july/)).
  Keyhive integrates through the `subduction_keyhive` crate family, so
  the Keyline capability design now has a shipping iroh transport.
- **go-iroh** — an independent **Go implementation** of iroh. Notable
  because it is a from-scratch port rather than an FFI binding, and
  evidence the 1.0 wire spec is stable enough to reimplement
  ([awesome-iroh](https://github.com/n0-computer/awesome-iroh)).
- **Community protocols and tools**, from
  [awesome-iroh](https://github.com/n0-computer/awesome-iroh) (~414
  stars; pull-request activity through November 2026):

  | Project | What | Notes |
  |---|---|---|
  | **iroh-ssh** | SSH over iroh | The "just give me a shell" niche |
  | **pai-sho** | Multi-port forwarding over iroh with reconnects | The persistent-forwards niche dumbpipe does not cover |
  | **ha-iroh** | Home Assistant integration | Iroh as remote access for Home Assistant |
  | **Godot iroh** | Iroh bindings/protocol for the Godot engine | Listed on n0's protocol registry page |
  | **Bones** | Game engine with iroh networking | Games vertical |
  | **Pigg** | Raspberry Pi GPIO over the network, iroh transport | Embedded/IoT vertical |
  | **iroh-esp32-example** | Iroh on ESP32 (n0) | Embedded proof-point for custom transports |
  | **Oku** | Distributed web browser on iroh | Browser vertical |
  | **Codux / Datum / Edky** | Dev tools / data platforms on iroh | Listed in awesome-iroh |
  | **Prime Intellect, Psyche, Tandemn** | Distributed AI training/inference on iroh | The AI/ML cluster; largest-scale non-n0 usage signal |

**Maturity read.** The community layer is broad but shallow — many
single-purpose tools and integrations, few with release trains. The
centre of gravity is still n0's own family plus two production
ecosystems: Delta Chat (gossip) and the Automerge/Keyhive stack
(custom transport). The registry page's short list — Blobs, Gossip,
Documents, roq, Godot, Content Discovery, Ping — is n0's curated view
of what counts as a "protocol"
([protocol registry](https://www.iroh.computer/proto)).
