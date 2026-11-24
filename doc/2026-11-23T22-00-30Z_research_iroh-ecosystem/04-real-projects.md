# Real projects

*Part of the [iroh ecosystem research record](2026-11-23T22-00-30Z_research_iroh-ecosystem.md).
Scale labels are honest: production means real users depend on it;
beta means deployed with real communities but not finished; small
scope means a finished tool doing one thing.*

## Delta Chat — the flagship production deployment

Delta Chat is a messenger that runs on top of the federated e-mail
system instead of a proprietary server network. Its clients (desktop,
Android, iOS) are built on a shared Rust core that speaks ordinary
SMTP/IMAP to mail servers and layers end-to-end encryption
(Autocrypt/OpenPGP) and chat semantics on top. Because plain e-mail
servers are slow for chat, the project promotes **Chatmail servers**:
special-purpose, hardened mail servers optimised for chat — instant
account creation by QR code, no name/e-mail/phone number required,
fast push, strict rate limits, automatic message deletion after a
short retention window. Anyone can run one; the network is federated
like e-mail itself.

Iroh appears in Delta Chat in two shipped features — and pointedly
*not* in a third.

### (a) Second-device setup and backup transfer — iroh's origin story

The first iroh integration, and by n0's own account the use case that
caused modern iroh to exist: while n0 was still building an
IPFS-compatible system, a side project implemented transfer of Delta
Chat chat history when provisioning a new device. That lightweight
QUIC + BLAKE3-verified-streaming library, built from scratch for Delta
Chat, is what became iroh — deployed in a real app used by thousands
of people across Windows, Linux, iOS and Android before iroh 1.0 was
ever framed as a general toolkit
([n0, "The road to iroh 1.0"](https://www.iroh.computer/blog/the-road-to-iroh-1-0)).

In the core, the backup (SQLite database plus blob directory) is
streamed as a tar archive over a bidirectional QUIC stream, under a
custom protocol (ALPN `/deltachat/backup`) directly on an iroh
endpoint — not iroh-blobs. Bootstrap is a **QR code** carrying the
provider's peer ID plus an authentication token, so both sides
authenticate: the getter cannot be tricked into pulling from an
impersonator, and the provider will not serve the backup to an
impersonated getter. The getter sends the token, receives the total
size, streams the tar, and acknowledges with a single byte.

### (b) Webxdc realtime channels — iroh-gossip

The second, larger integration shipped in **Delta Chat 1.48 (November
2024)**, after almost two years of collaboration with the iroh team.
Webxdc is Delta Chat's in-chat mini-app format — small web apps sent
as message attachments and run inside chats. 1.48 added a
`joinRealtimeChannel()` API giving those apps an ephemeral, direct
peer-to-peer channel alongside the store-and-forward e-mail channel
([Delta Chat announcement, 2024-11-20](https://delta.chat/en/2024-11-20-webxdc-realtime)).

How it works:

- **Protocol: iroh-gossip.** Each webxdc app instance gets a gossip
  topic — a random 32-byte topic ID generated on the sender's device.
- **Bootstrap rides the e-mail federation, not a DHT.** The topic ID
  travels in an encrypted message header with the app message itself.
  When an app joins or sends, the device posts a normal encrypted chat
  message containing its node address (relay URL + public key —
  deliberately *not* its direct IP addresses, since mail providers
  persist messages). Peers store that address scoped to the app
  instance and join the gossip swarm. No global DHT lookup is
  involved: an end-to-end encrypted system message carrying an iroh
  ticket bootstraps the ephemeral peer-to-peer network.
- **Relays come from the Chatmail infrastructure** — typically hosted
  on the same Chatmail servers that carry the accounts — providing
  NAT traversal and hole punching; direct QUIC paths are used when
  punching succeeds.
- **Ephemeral identities.** The peer-to-peer identity is generated
  fresh for the realtime sessions rather than reusing the account's
  long-term keys, and QUIC/TLS gives forward secrecy on the channel.
- The feature was opt-in for roughly half a year before being enabled
  by default in 1.48. Example apps shipped with it include a
  collaborative pixel canvas, a realtime collaborative editor with
  live cursors, and an ephemeral live chat with typing indicators —
  all hybrid: durable updates over e-mail, ephemeral traffic over the
  gossip channel.

### (c) Calls — deliberately *not* iroh

Audio/video calls are the natural third candidate, and they do **not**
use iroh. Delta Chat's calls are peer-to-peer **WebRTC**, signalled
via regular Delta Chat messages, with ICE servers from the Chatmail
infrastructure; the project's March 2026 release post describes
native background-capable calls on Android/iOS on exactly that basis
([Delta Chat, 2026-03-31](https://delta.chat/en/2026-03-31-zero)).
An iroh-based A/V tech demo existed earlier, but the shipped calls
feature is WebRTC.

**Why Delta Chat matters for this estate:** it is the strongest
evidence that iroh survives contact with a mass-market, mobile-first,
adversarial-NAT user base — and its architecture is instructive. Iroh
is used *only* where direct connections beat store-and-forward
(device provisioning, ephemeral realtime), bootstrapped through the
existing federated channel (encrypted mail messages carrying
tickets/addresses) rather than through any global discovery system,
with relays folded into the servers users already have.

## Pubky — the pkarr sibling (not an iroh transport user)

Pubky's connection to iroh is at the **discovery layer, not the
transport layer**: iroh's address lookup adopted the pkarr format,
while Pubky itself does not run on iroh connections. It is the
flagship application of the pkarr ecosystem that iroh's discovery
composes with — see the [pkarr page](03-pkarr-and-discovery.md).

## OpenArchive Save — Veilid for privacy, iroh for bytes (beta)

OpenArchive's **Save** app lets journalists and human-rights
documenters archive mobile media verifiably. Its DWeb Storage backend
— built with Hypha Worker Co-operative, funded by the Filecoin
Foundation for the Decentralized Web, in beta — combines **Veilid for
encrypted, anonymous peer connections and group metadata (its DHT)
with iroh for data storage, retrieval, replication and hash-verified
integrity**. Users form groups, share media into repositories,
advertise their Veilid tunnel, and download blobs from whichever
group member has them using iroh's replication protocol, verifying by
blob hash. OpenArchive chose iroh because Veilid lacked robust data
storage/verification, and cites iroh's maintained Rust libraries as
what made iOS/Android development tractable
([OpenArchive write-up](https://www.open-archive.org/news/dweb-save-integration)).
Scale: **beta, deployed with at-risk user communities**. The role
split — Veilid carries privacy/routing, iroh carries bytes — is
exactly the composition our own Veilid research predicted.

## n0's own apps: sendme and dumbpipe

- **Sendme** — file/directory transfer by ticket, with BLAKE3-verified
  streaming over iroh-blobs. Shipped, packaged (including in Termux),
  used daily by the n0 team itself, and it has spawned a family of
  clones and GUI front-ends — alt-sendme (Tauri desktop), ARK Drop
  Desktop (QR-code send/receive), DataBeam, sendme-egui, iroh-send —
  catalogued in [awesome-iroh](https://github.com/n0-computer/awesome-iroh).
  Scale: **production tool, small scope**.
- **Dumbpipe** — the unix-pipe counterpart: arbitrary stdin/stdout
  streams and TCP/unix-socket forwarding between devices addressed by
  endpoint ID, no reachable IP or port-forwarding needed. It directly
  inspired pai-sho (multi-port forwarding with reconnects), iroh-ssh
  and do-ssh (SSH over iroh), and an Android SSH client. Scale:
  **production tool, small scope** — and architecturally the closest
  public relative of this estate's own route/exec work: direct
  connections by endpoint ID as the whole product.
- **Swarmie** — n0's proof-of-concept sendme extension adding global
  content discovery via the Mainline DHT; explicitly very
  experimental, included here as the boundary marker between the
  shipped tools and the experiments.

## The rest of the shipped field

From n0's curated [awesome-iroh](https://github.com/n0-computer/awesome-iroh)
list and project sources:

**Decentralised AI compute (production-leaning; the largest-scale
non-chat users):**

- **Prime Intellect** — decentralised AI compute infrastructure; uses
  iroh for peer connectivity between distributed training/inference
  nodes.
- **Psyche** — open infrastructure for decentralised model training;
  iroh carries the peer traffic.
- **Tandemn** — inference across heterogeneous GPUs.

**Collaboration and dev tools (shipped, small teams):**

- **Teamtype** — editor-agnostic realtime collaborative editing of
  local text files.
- **p2pmux** — terminal multiplexer where every pane is a PTY on its
  owner's own machine.
- **Zeco** — share zellij sessions over the internet via iroh;
  **do-ssh / iroh-ssh** — SSH without IPs; **Codux** — connected
  terminal for AI coding agents with iroh as the encrypted P2P
  control plane; **cross.stream** — local-first event streaming with
  P2P sharing; **Rayfish** — mesh VPN by cryptographic identity;
  **Wispers Access** — share a web app with invitees without
  publishing it, iroh as default transport.

**File sharing and backup (shipped, mostly small):**

- **Dropwire** — E2EE resumable file transfer, no account, no server;
  **fsend** — webapp + CLI with resumable transfers; **Chorrent** —
  BitTorrent-style swarm transfer pulling verified pieces from every
  sharer; **Backup Buddies** — P2P backups with local encryption and
  restore versions; **P-Pass** — phones back photos up to the home
  computer, no cloud; **Strada** — file transfer/management product
  (see also its production numbers on the
  [showcases page](06-talks-releases-showcases.md)).

**Games (real, niche):**

- **Bones** — a Rust game engine networked with iroh; **Fish Folk:
  Jumpy** — a shipped 2D shooter built on Bones; **Godot Iroh** — P2P
  multiplayer extension for the Godot engine; **iroh-lan** —
  LAN-party-over-internet tooling.

**Social and local-first (beta to production):**

- **Delta Chat** (above); **Dash Chat** — secure chat designed for
  internet shutdowns; **Rostra** — a friends-to-friends social
  network; **Weird** — a local-first personal-web app from the
  Leaf/Earthstar orbit; **Oku** — a web browser emphasising
  local-first storage; **Obsiroh** — Obsidian sync over iroh;
  **Kith** — serverless E2EE sync of memory/tabs/files across one's
  own devices, also exposed to AI over MCP.

**Frameworks naming iroh as substrate:** **p2panda** is listed in
awesome-iroh's Frameworks section — consistent with the p2panda
research in this investigation: p2panda's networking layer is an iroh
application.
