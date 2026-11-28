# Iroh in the wild: real projects and the community home

Companion research for the capability-auth + p2p-transport investigation
([developing-today/code#13779](https://github.com/developing-today/code/issues/13779))
and the fleet transport design
([#13752](https://github.com/developing-today/code/issues/13752)).
Researched 2026-11-23 from primary sources (project repos, docs, engineering
blogs, the public GitHub Discussions board). Community material is
**paraphrased, never quoted**.

---

# Job 1 — Real projects built on iroh

## 1.1 Delta Chat — the flagship production deployment

Delta Chat is a messenger that runs on top of the federated e-mail system
instead of a proprietary server network. Its clients (desktop, Android, iOS)
are built on a shared Rust core (`deltachat` / chatmail-core); the core speaks
ordinary SMTP/IMAP to mail servers and layers end-to-end encryption
(Autocrypt/OpenPGP) and chat semantics on top. Because plain e-mail servers
are slow and metadata-hungry for chat, the project promotes **Chatmail
servers**: special-purpose, hardened mail servers optimised for chat —
instant account creation by scanning a QR code, no name/e-mail/phone number
required, fast push, strict rate limits, and automatic deletion of messages
after a short retention window. Anyone can run a Chatmail server, and the
network is federated like e-mail itself. The core library's own README
summarises the iroh relationship in one line: it provides _ephemeral
peer-to-peer networking using Iroh, for multi-device setup and webxdc
realtime data_
([chatmail-core](https://rs.delta.chat/src/deltachat/peer_channels.rs.html)).

Iroh appears in Delta Chat in two shipped features, and pointedly _not_ in a
third:

### (a) Second-device setup / backup transfer — iroh's origin story

The first iroh integration, and by n0's own account the use case that caused
modern iroh to exist: while n0 was still building an IPFS-compatible system,
a side project implemented _transfer of Delta Chat chat history when
provisioning a new device_; that lightweight QUIC + BLAKE3-verified-streaming
library, built from scratch for Delta Chat, is what became iroh, and it was
deployed in a real app used by thousands of people across Windows, Linux,
iOS and Android before iroh 1.0 was ever framed as a general toolkit
([n0, "The road to iroh 1.0"](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/the-road-to-iroh-1-0/page.mdx)).

In Delta Chat's core this is the `imex/transfer.rs` module
([source](https://rs.delta.chat/src/deltachat/imex/transfer.rs.html)):

- Custom protocol with ALPN **`/deltachat/backup`** directly on an iroh
  `Endpoint` — not iroh-blobs; the backup (SQLite database + blob directory)
  is streamed as a tar archive over a bidirectional QUIC stream.
- Bootstrap is a **QR code** carrying the provider's peer ID plus an
  authentication token, so both sides authenticate: the getter cannot be
  tricked into pulling from an impersonator, and the provider will not serve
  the backup to an impersonated getter.
- The getter opens the stream, sends the token, receives the total size,
  streams the tar, and acknowledges with a single byte; the provider then
  closes the endpoint. I/O on the sending device is stopped first so the
  database and blobs are consistent.

### (b) Webxdc realtime channels — iroh-gossip

The second, larger integration shipped in **Delta Chat 1.48 (November 2024)**, after almost two years of collaboration with the iroh team. Webxdc
is Delta Chat's in-chat mini-app format (small web apps sent as message
attachments and run inside chats). 1.48 added a `joinRealtimeChannel()` API
that gives those apps an ephemeral, direct peer-to-peer channel alongside the
store-and-forward e-mail channel
([Delta Chat, "Delta Chat introduces realtime Peer-to-Peer networking", 2024-11-20](https://delta.chat/en/2024-11-20-webxdc-realtime);
[announcement post, 2024-11-17](https://delta.chat/en/2024-11-17-webxdc-realtime)).

How it works, per Delta Chat's announcement and the core's
`peer_channels.rs`
([source](https://rs.delta.chat/src/deltachat/peer_channels.rs.html)):

- **Protocol: iroh-gossip.** The core holds an iroh `Router` plus a `Gossip`
  handle; each webxdc app instance gets a gossip topic — a random 32-byte
  `TopicId` generated on the sender's device.
- **Bootstrap rides the e-mail federation, not a DHT.** The topic ID travels
  in an encrypted message header (`IrohGossipTopic`) with the app message
  itself. When an app calls `joinRealtimeChannel().setListener()` or
  `.send()`, the device lazily starts the P2P machinery and posts a normal
  (encrypted) chat message containing its `IrohNodeAddr` (relay URL + public
  key — deliberately _not_ its direct IP addresses, since mail providers
  persist messages). Peers store that address scoped to the app instance and
  join the gossip swarm. Delta Chat's announcement stresses that no global
  DHT lookup is involved: an end-to-end encrypted system message carrying an
  iroh ticket bootstraps the ephemeral P2P network.
- **Relays come from the Chatmail infrastructure.** Connections fall back to
  iroh relays typically hosted on the same Chatmail servers that carry the
  accounts, providing STUN/TURN-like functions for NAT traversal and hole
  punching; direct QUIC paths are used when punching succeeds.
- **Ephemeral identities.** The P2P identity is generated fresh — Delta Chat
  uses ephemeral cryptographic identities for the realtime sessions rather
  than the account's long-term keys — and QUIC/TLS gives forward secrecy on
  the channel.
- The feature was opt-in/experimental for roughly half a year before being
  enabled by default in 1.48, with a joint focus on reliability across all
  platforms including mobile. Delta Chat's team reported the realtime API
  being easier to use than the browser WebRTC API and faster in some
  situations in preliminary measurements.
- Example apps shipped with it include a collaborative pixel canvas, a
  realtime collaborative editor with live cursors, and an ephemeral
  live-chat with typing indicators — all hybrid: durable updates via
  `webxdc.sendUpdate` over e-mail, ephemeral traffic over the iroh-gossip
  channel.

### (c) Calls — deliberately _not_ iroh

Audio/video calls are the natural third candidate, and they do **not** use
iroh. Delta Chat's calls are peer-to-peer **WebRTC**, signalled via regular
Delta Chat messages, with ICE servers from the Chatmail infrastructure; the
project's March 2026 release post describes native background-capable calls
on Android/iOS on exactly that basis
([Delta Chat, 2026-03-31 release post](https://delta.chat/en/2026-03-31-zero)).
An iroh-based A/V tech demo existed earlier (noted in
[this third-party deep-dive](https://blog.feld.me/posts/2025/03/deltachat-is-actually-good-though/)),
but the shipped calls feature is WebRTC. Independent Chatmail relay
implementations bundle the supporting cast accordingly — e.g. Madmail lists
"TURN / Iroh" relay functions alongside SMTP/IMAP in one binary
([madmail.chat](https://madmail.chat/)).

**Why Delta Chat matters for this estate:** it is the strongest evidence
that iroh survives contact with a mass-market, mobile-first, adversarial-NAT
user base — and its architecture is instructive: iroh is used _only_ where
direct connections beat store-and-forward (device provisioning, ephemeral
realtime), bootstrapped through the existing federated channel (encrypted
mail messages carrying tickets/addresses) rather than through any global
discovery system, with relays folded into the servers users already have.

## 1.2 Pubky — the pkarr sibling (not an iroh transport user)

Pubky, from Synonym, is a key-based web protocol and social ecosystem:
identity is an Ed25519 keypair, users keep data on self-chosen _homeservers_,
and **PKARR (Public Key Addressable Resource Records)** — signed DNS-like
records published to the BitTorrent Mainline DHT — makes each public key a
sovereign, resolvable "public key domain" pointing at wherever the user's
data currently lives. The flagship Pubky app (beta, launched at BTC Prague
in 2025) is a social network built on that stack, with the Pubky Ring mobile
app holding keys and authorising sessions
([Pubky launch post](https://medium.com/pubky/opening-pubky-4bdca7dc4c77);
[Pubky Directory](https://pubky.tech/)).

The iroh connection is at the **discovery layer, not the transport layer**:
iroh's address-lookup design adopted the pkarr format — n0 runs a pkarr
relay format for its DNS discovery service and gives users the option of
publishing/resolving endpoint records directly on the Mainline DHT for a
fully decentralised lookup, and an n0-built Cloudflare Worker implementing
the pkarr relay format is catalogued in the Pubky directory. Pubky itself
does not run on iroh connections; it is the flagship application of the
pkarr ecosystem that iroh's discovery composes with. For this estate that
distinction matters: pkarr is a _shared rendezvous format_ between the two
projects, and Pubky is the proof that pkarr works at ecosystem scale.

## 1.3 OpenArchive _Save_ — iroh for storage/replication (beta, real-world)

OpenArchive's **Save** app (Secure, Archive, Verify, Encrypt) lets
journalists and human-rights documenters archive mobile media verifiably.
Its **DWeb Storage** backend — built with Hypha Worker Co-operative, funded
by the Filecoin Foundation for the Decentralized Web, in beta — combines
**Veilid for encrypted, anonymous peer connections and group metadata (its
DHT) with iroh for data storage, retrieval, replication and hash-verified
integrity**. Users form groups, share media into repositories, advertise
their Veilid tunnel, and download blobs from whichever group member has
them using iroh's replication protocol, verifying by blob hash. OpenArchive
chose iroh because Veilid lacked robust data storage/verification, and
cites iroh's maintained Rust libraries as what made iOS/Android development
tractable
([OpenArchive, "How We Created the First Decentralized Storage Backend for Mobile Media"](https://www.open-archive.org/news/dweb-save-integration);
backend repo under [github.com/openarchive](https://github.com/openarchive),
`save-dweb-backend`, Rust). This is the same Veilid+iroh composition our
veilid note examines — with the roles split exactly as that note predicted:
Veilid carries privacy/routing, iroh carries bytes.
Scale label: **beta, deployed with at-risk user communities**.

## 1.4 n0's own apps: Sendme and Dumbpipe

- **Sendme** — file/directory transfer by ticket: `sendme send <path>`
  prints a ticket, `sendme receive <ticket>` fetches it with BLAKE3
  (bao) verified streaming over iroh-blobs. Shipped, packaged (including in
  Termux), and used daily by the n0 team itself; it has spawned a family of
  clones and GUI front-ends — alt-sendme (Tauri desktop), ARK Drop Desktop
  (QR-code send/receive), DataBeam, sendme-egui, iroh-send — catalogued in
  [awesome-iroh](https://github.com/n0-computer/awesome-iroh). Scale:
  **production tool, small scope**.
- **Dumbpipe** — the unix-pipe counterpart: arbitrary stdin/stdout streams
  and TCP/unix-socket forwarding between devices addressed by endpoint ID,
  no reachable IP or port-forwarding needed ([dumbpipe.dev, via Termux package record](https://github.com/termux/termux-packages/issues/30605)).
  It directly inspired pai-sho (multi-port forwarding with reconnects),
  iroh-ssh and do-ssh (SSH over iroh) and an Android SSH client. Scale:
  **production tool, small scope** — and architecturally the closest public
  relative of this estate's own route/exec work: direct connections by
  endpoint ID as the whole product.
- **Swarmie** — n0's proof-of-concept sendme extension adding global content
  discovery via the Mainline DHT; explicitly _very experimental_, included
  here as the boundary marker between the shipped tools and the experiments
  ([repo](https://github.com/n0-computer/swarmie)).

## 1.5 The rest of the shipped field

From n0's curated [awesome-iroh](https://github.com/n0-computer/awesome-iroh)
list and project sources, with honest scale labels:

**Decentralised AI compute (production-leaning, the largest-scale non-chat users):**

- **Prime Intellect** — decentralised AI compute infrastructure; uses iroh
  for peer connectivity between distributed training/inference nodes.
- **Psyche** — open infrastructure for decentralised model training
  (disbanded-comm style coordination over the internet); iroh carries the
  peer traffic.
- **Tandemn** — "iroh for tensors": inference across heterogeneous GPUs.

**Collaboration / dev tools (shipped tools, small teams):**

- **Teamtype** — editor-agnostic realtime collaborative editing of local
  text files.
- **p2pmux** — terminal multiplexer where every pane is a PTY on its
  owner's own machine (also shown in GitHub Discussions, Aug 2026).
- **Zeco** — share zellij sessions over the internet via iroh; **do-ssh /
  iroh-ssh** — SSH without IPs; **Codux** — connected terminal for AI
  coding agents with iroh as the encrypted P2P control plane across
  desktop/mobile/headless hosts; **cross.stream** — local-first event
  streaming with P2P sharing; **Rayfish** — mesh VPN by cryptographic
  identity; **Wispers Access** — share a web app with invitees without
  publishing it, iroh as default transport, native phone + desktop apps.

**File sharing / backup (shipped, mostly small):**

- **Dropwire** — E2EE resumable file transfer, no account, no server;
  **fsend** — webapp + CLI with resumable transfers; **Chorrent** —
  BitTorrent-style swarm transfer pulling verified pieces from every
  sharer; **Backup Buddies** — P2P backups with local encryption and
  restore versions; **P-Pass** — phones back photos up to the home
  computer, no cloud; **Strada** — file transfer/management product.

**Games (real, niche):**

- **Bones** — a Rust game engine networked with iroh; **Fish Folk: Jumpy**
  — a shipped 2D shooter built on Bones; **Godot Iroh** — P2P multiplayer
  extension for the Godot engine; **iroh-lan** — LAN-party-over-internet
  tooling.

**Social / local-first (beta to production):**

- **Delta Chat** (above); **Dash Chat** (darksoil studio) — secure chat
  designed for internet shutdowns; **Rostra** — a friends-to-friends social
  network (via Radicle); **Weird** — a local-first personal-web app from
  the Leaf/Earthstar orbit; **Oku** — a web browser emphasising local-first
  storage; **Obsiroh** — Obsidian sync over iroh; **Kith** — serverless
  E2EE sync of memory/tabs/files across one's own devices, also exposed to
  AI over MCP.

**Frameworks naming iroh as substrate:** **p2panda** is listed in
awesome-iroh's Frameworks section — consistent with the p2panda note in
this investigation: p2panda's networking layer is an iroh application.

---

# Job 2 — The community home

## 2.1 Where the iroh community actually talks

Drew's recollection of "a discussion board somewhere, not Zulip, maybe
Slack" resolves to **two main venues plus satellites**:

1. **The Iroh Discord — the primary social venue.** This is the "Slack-like"
   place Drew was remembering. n0's own materials route people there
   constantly: the iroh README carries a Discord badge
   ([invite](https://discord.com/invite/DpmJgtU7cW)), the iroh-ffi project
   directs blocked users to it ([an older invite link](https://discord.gg/B4pzE3usDC)
   circulates in repo text), the contributing guide sends people there for
   mentoring through bugs and features
   ([CONTRIBUTING.md](https://raw.githubusercontent.com/n0-computer/iroh/main/CONTRIBUTING.md)),
   and the canonical short link is **https://iroh.computer/discord**.
   **Access reality:** Discord requires a Discord account and joining the
   server; the channels are _not publicly readable_ from the open web.
   This lane did not join or read it — nothing behind that login is
   summarised here. If Drew wants the live conversation (help channels,
   project showcases, maintainer chatter), joining via
   https://iroh.computer/discord is the step only he can take.
2. **GitHub Discussions on n0-computer/iroh — the public board.**
   [github.com/n0-computer/iroh/discussions](https://github.com/n0-computer/iroh/discussions)
   is fully world-readable without an account, with categories
   **Announcements, Q&A, Ideas, Show and tell, General**. Release
   announcements are posted here by the maintainers (v1.0.2 and v1.0.3 in
   July 2026, v1.1.0 in August, v1.3.0 in September, v1.4.0 and v1.4.1 in
   October 2026 — a fast post-1.0 cadence), and the contributing guide
   names Discussions as the place for concept questions and help. Volume
   is modest — this is the considered, on-the-record layer, not the
   firehose.
3. **Satellites:** a semi-official **Matrix room** and Mastodon account
   (@n0iroh) are linked from
   [awesome-iroh](https://github.com/n0-computer/awesome-iroh)'s Follow
   section, alongside the official blog, @iroh_n0 on X, and the number 0
   YouTube channel. Announcements flow through the blog + Discussions +
   socials; day-to-day developer socialising is Discord.

**Bottom line:** the substantive _public_ record is GitHub Discussions +
the blog; the substantive _social_ record is the login-gated Discord.

## 2.2 Recent important conversations (public board, paraphrased)

All items below are from the public
[GitHub Discussions board](https://github.com/n0-computer/iroh/discussions),
paraphrased in substance — follow the links to read the originals.

- **A feature is being worked on that will have a major impact: post-quantum
  endpoint identity is now an open design conversation.**
  A September 2026 Ideas thread argues that because endpoint IDs are
  Ed25519 keys, a future fault-tolerant quantum computer could forge
  identities, and that migration across a deployed identity system takes
  years — so the proposal is to make iroh crypto-agile and define a
  post-quantum migration path _now_ while keeping Ed25519 today. The
  thread drew sustained maintainer engagement (8 comments).
  [Discussion #4532](https://github.com/n0-computer/iroh/discussions/4532)
- **A decision was made about how WebRTC support will happen: as an
  external transport crate, not an in-tree feature.**
  A March 2026 General thread started by a maintainer lays out that the
  team wants WebRTC as a custom transport (the API for custom transports
  landed in v0.97) but cannot prioritise building it before roughly
  mid-2026; maintainers then steered the work to a standalone crate built
  on the str0m WebRTC implementation, following the pattern of the
  existing Tor and Nym transport crates. A community contributor has
  since reported a working implementation covering native-to-native,
  browser-to-browser and browser-to-native connections.
  [Discussion #4024](https://github.com/n0-computer/iroh/discussions/4024)
- **Someone showed a project: FlipFlop — persistent tunnels between peers**
  for sharing files, text and songs and running microapps, posted to Show
  and tell on 2026-11-21 — the freshest community project on the board at
  research time.
  [Discussions board, Show and tell](https://github.com/n0-computer/iroh/discussions)
- **This was recently discussed and is an open architectural question:
  using relays for coordination only, never for application data**
  (Ideas, Nov 2026) — and a sibling July 2026 thread asks for the related
  ability to restrict n0's relays to hole-punching only, with subsequent
  traffic barred from the relay. Relay-traffic minimisation is a live
  design pressure from the community.
  [Discussions board, Ideas](https://github.com/n0-computer/iroh/discussions)
- **Someone showed a project: Sqwakie Direct** (Show and tell, Oct 2026) —
  a signed team log for AI agents communicating over iroh QUIC; and
  **Tincan** (Oct 2026), peer-to-peer voice and text chat in the terminal.
  [Discussions board, Show and tell](https://github.com/n0-computer/iroh/discussions)
- **Hardware-backed endpoint identity is being explored**: an April 2026
  Ideas thread seeks architectural guidance on keeping endpoint secret
  keys in TPM 2.0 / Secure Enclave hardware (9 comments, maintainers and
  the post-quantum proposer participating) — identity-key custody is an
  active community concern, parallel to this estate's CA work.
  [Discussions board, Ideas](https://github.com/n0-computer/iroh/discussions)
- **Group discovery is an unsolved want**: an October 2026 Ideas thread
  asks how to find the endpoint IDs of nodes belonging to a "group" —
  there is no built-in group-membership discovery primitive, which is
  precisely the layer this estate's auth investigation is about.
  [Discussions board, Ideas](https://github.com/n0-computer/iroh/discussions)
- **Authorisation approaches remain an open question on the board**: the
  long-running "Approaches for adding authorization?" General thread
  (started Feb 2025, 16 comments) is the board's main public record of
  users working out how to layer authorisation over iroh's
  authenticate-by-key transport — iroh itself ships no authz model.
  [Discussions board, General](https://github.com/n0-computer/iroh/discussions)

## 2.3 What is _not_ publicly visible

The Discord's content (help channels, working discussion, any
decision-making that happens socially) is not readable without joining,
and this research deliberately did not join under anyone's identity. The
public surfaces above — Discussions, the blog, release notes, awesome-iroh
— are a faithful record of _what shipped and what was proposed_, but if
Drew wants the community's live pulse, the Discord at
https://iroh.computer/discord is the room, and joining it is a one-click
step from his own account.
