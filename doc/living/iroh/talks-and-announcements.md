---
created: 2026-11-23
last-modified: 2026-11-24
revision: 1
---

# Iroh — Talks, Videos, Releases & Announcements

*Lane IROH-MEDIA, companion to the iroh investigation (developing-today/code#13779). Researched 2026-11-23 from primary sources: n0's blog (iroh.computer/blog), FOSDEM and conference archives, YouTube, podcast archives, Delta Chat, Raspberry Pi, Pubky. Dates are as published by the venue or poster; where a date could not be verified it is marked as such rather than guessed. Drew's brief: "people showing off is good to bring to the forefront" — so demos and production show-and-tell get their own section.*

**Counts: 9 talks/videos · 8 written announcements/releases · 6 demos/showcases — 23 items.**

---

## (a) Talks & videos

### 1. "Iroh: syncing terabytes of data, peer-to-peer" — Local-First Conf 2026
- **Who:** Brendan O'Brien (b5, n0 co-founder; Rae McKelvey co-billed, recording has O'Brien speaking)
- **Venue/date:** Local-First Conf 2026, 2026-07-13
- **What's shown:** The iroh 1.0 story told as organisational lessons rather than a feature tour: how the project narrowed from an IPFS implementation to "just connections," what it took to get to a stable release, and the scale the public relay network now sees — roughly 30 billion observable relay connections in six months. Delta Chat is presented as the first production partner, and an experimental SSH-over-iroh is announced at the end.
- **Why it matters:** The best single talk for understanding *why* iroh 1.0 looks the way it does, and the clearest public evidence of production scale.
- **Link:** https://app-2026.localfirstconf.com/schedule/day-2/1330-iroh-syncing-terabytes-of-data-peer-to-peer

### 2. "iroh p2p connections" — FOSDEM 2026
- **Who:** Floris Bruynooghe (n0)
- **Venue/date:** FOSDEM 2026, Decentralized Internet and Privacy devroom, Brussels, Sun 2026-02-01
- **What's shown:** A tour of the post-1.0-era stack: dialling by EndpointId instead of IP, address lookup via DNS/pkarr/mDNS/DHT, iroh-blobs verified streaming, and iroh-gossip (HyParView/PlumTree) for pub/sub. Bruynooghe also gave a second talk the same weekend, "Building QUIC Multipath," in the Network track.
- **Why it matters:** The most current conference-level technical overview of the whole iroh protocol family in one slot, from the engineer who works on the transport core.
- **Link:** https://fosdem.org/2026/schedule/speaker/floris_bruynooghe/ (mirror with talk details: https://0x51.dev/fosdem/schedule/event/iroh-p2p-connections/)

### 3. "Building Peer-to-Peer QUIC" — FOSDEM 2025
- **Who:** Floris Bruynooghe (n0)
- **Venue/date:** FOSDEM 2025, Network track, Brussels, Sun 2025-02-02
- **What's shown:** How the pieces compose into a working p2p transport: relays for fallback, QUIC address discovery, multipath QUIC, the NAT-traversal drafts, and ed25519 endpoint identity — i.e. the engineering underneath hole punching, explained against the actual IETF drafts n0 was helping to push.
- **Why it matters:** The canonical deep-dive on iroh's transport design; the talk to watch before reading the noq announcement (below). The event page links the video recordings.
- **Link:** https://fosdem.org/2025/schedule/event/fosdem-2025-6053-building-peer-to-peer-quic/

### 4. "iroh: you can just connect things" — RustConf 2026 sponsored demo
- **Who:** Brendan O'Brien
- **Venue/date:** RustConf 2026, Montréal, Thu 2026-09-10 (10-minute demo slot)
- **What's shown:** A live demo of iroh connections switching transports underneath a running session — Bluetooth, LAN, Tor, IPv4, IPv6 and relays — with the application none the wiser. Short, visual, and aimed squarely at Rust developers who have never seen the library.
- **Why it matters:** The purest "show-off" item on this list: path-switching across wildly different networks is the demo that makes iroh's abstraction click.
- **Link:** https://rustconf2026.sched.com/event/2U3Ff/sponsored-demo-iroh-you-can-just-connect-things

### 5. Workshop: "iroh p2p chat over gossip" — 39C3
- **Who:** Hosted at the Delta Chat assembly, 39th Chaos Communication Congress (39C3), Dec 2025
- **What's shown:** A two-hour hands-on workshop (Day 2, 12:30–14:30): participants build a working group chat on raw iroh connections plus iroh-gossip, in the room, on congress network conditions.
- **Why it matters:** Evidence the stack is teachable in an afternoon to a hacker-congress audience — and a sign of how close the Delta Chat and n0 communities work together.
- **Link:** https://delta.chat/39c3.html

### 6. "Hands-on Introduction to Iroh" — Rawkode Live
- **Who:** Brendan O'Brien with host David (Rawkode)
- **Venue/date:** Rawkode Live / Rawkode Academy, 2026 (episode page does not carry a firm air date)
- **What's shown:** A live-coding walkthrough: spinning up endpoints, connecting peers, and building on iroh in real time with the usual live-stream detours into the state of p2p and Rust tooling. The full transcript is on the episode page.
- **Why it matters:** The most approachable on-ramp video for a developer audience — less polished than the conference talks, more honest about the developer experience.
- **Link:** https://rawkode.academy/watch/hands-on-introduction-to-iroh

### 7. devtools.fm podcast — "Brendan O'Brien — n0, Iroh and the Future of Peer to Peer"
- **Who:** Brendan O'Brien
- **Venue/date:** devtools.fm podcast, 2025-07-07
- **What's shown:** A long-form interview on pragmatic p2p: why iroh deliberately does *only* transport, what running it in production on hundreds of thousands of devices taught the team, and where the local-first ecosystem is heading.
- **Why it matters:** The best audio introduction to n0's philosophy — scope discipline ("just connections") in the founder's own words.
- **Link:** https://podcast.app/brendan-obrien-n0-iroh-and-the-future-of-peer-to-peer-e419358449

### 8. Fedimint 0.7 and iroh — Stephan Livera Podcast
- **Who:** Eric Sirion and Joschi (Fedimint)
- **Venue/date:** Stephan Livera Podcast, 2025-04-24
- **What's shown:** A *builder's* talk, not an n0 one: Fedimint 0.7 replaced domain names and port-forwarding for federation guardians with iroh networking, so a guardian node behind home NAT just works. The episode walks through what that changed for people actually running federations.
- **Why it matters:** Independent production adoption in the Bitcoin/federated-custody world — iroh solving the "my node is behind NAT" problem for non-technical operators.
- **Link:** https://www.murrayrudd.pro/one-click-federated-custody-with-fedimint-0-7/ (episode summary page)

### 9. n0's intro videos — "Introducing iroh" and "A tour of iroh"
- **Who:** n0 (YouTube channel)
- **Venue/date:** YouTube; early project videos from the pre-1.0 era (exact upload dates not verified)
- **What's shown:** "Introducing iroh" is the intro video linked directly from the iroh GitHub README — the project's canonical first impression, from its IPFS-research era. "A tour of iroh" is a longer walkthrough kept in n0's own iroh playlist and recommended in project-adjacent docs as "a good presentation."
- **Why it matters:** Useful as a baseline: comparing these against the 1.0 material shows how far the scope narrowed and the pitch sharpened.
- **Links:** https://www.youtube.com/watch?v=RwAt36Xe3UI_ · https://www.youtube.com/watch?v=AkHaIVuFHK4

---

## (b) Written announcements & releases

### 1. Iroh 1.0 — "Dial Keys, not IPs"
- **Who/where:** Brendan O'Brien (b5), n0 blog, 2026-06-15
- **What's announced:** The first stable release, after 65 pre-1.0 versions: a wire- and API-stability guarantee, official Python/Node/Swift/Kotlin bindings alongside Rust, and dialling by public key as the headline primitive. The post reports more than 200 million endpoints created on the public relays in the prior 30 days, and embeds a YouTube launch video.
- **Why it matters:** The milestone the whole 1.0 arc (roadmap post in Oct 2024, 0.35 LTS, canary series) was building to — the point at which building on iroh stopped meaning tracking a moving API.
- **Link:** https://www.iroh.computer/blog/v1

### 2. "The road to iroh 1.0"
- **Who/where:** Rüdiger Klaehn, n0 blog, 2026-07-09
- **What's announced:** A four-year retrospective: the IPFS-implementation origin, krakensync, Delta Chat's backup transfer as the first real use case, the February 2023 pivot away from IPFS compatibility, adopting Tailscale-style hole punching and pkarr for address lookup, the sendme/dumbpipe experiments, narrowing scope to "just connections," and the quinn fork that became noq.
- **Why it matters:** The single best history of the project — nearly every other item on this list is a footnote to a decision explained here.
- **Link:** https://www.iroh.computer/blog/the-road-to-iroh-1-0

### 3. "Iroh language support"
- **Who/where:** n0 blog, 2026-06-18
- **What's announced:** First-party bindings for Swift, Kotlin, Python and JavaScript graduate to official status alongside the Rust core. The demo: an accelerometer app streaming phone tilt at ~60 Hz between peers, written against the mobile bindings.
- **Why it matters:** 1.0 made iroh a multi-language platform, not a Rust library — this is what put it within reach of the mobile apps (Delta Chat, Mesh LLM's planned app) that drive adoption.
- **Link:** https://www.iroh.computer/blog/iroh-language-support

### 4. "noq, noq, who's there?" — n0's own QUIC implementation
- **Who/where:** n0 blog, 2026-03-19
- **What's announced:** noq, n0's hard fork of the quinn QUIC library, as a project in its own right: QUIC Multipath, QUIC NAT Traversal and QUIC Address Discovery as first-class features. It had already been powering iroh since v0.96; this post makes the fork public and explains why upstream quinn couldn't carry the p2p requirements.
- **Why it matters:** iroh's transport advantage is now a standalone, reusable QUIC stack — relevant to anyone evaluating QUIC for p2p, not just iroh users.
- **Link:** https://www.iroh.computer/blog/noq-announcement

### 5. "irpc 1.0 — Stable RPC for the iroh ecosystem"
- **Who/where:** n0 blog, 2026-11-10
- **What's announced:** irpc reaches 1.0 with a stable wire format and cancellation-safe streaming RPC. Because iroh-blobs and iroh-gossip are both built on irpc, this is the keystone release that unblocks their own 1.0s (the gossip road-to-1.0 and blobs canary posts followed the same season).
- **Why it matters:** Shows the companion protocols converging on stability in a deliberate order — RPC layer first, data protocols next.
- **Link:** https://www.iroh.computer/blog/irpc-1-0

### 6. "Delta Chat introduces realtime Peer-to-Peer networking"
- **Who/where:** Delta Chat team, delta.chat blog, 2024-11-20
- **What's announced:** Delta Chat 1.48 ships iroh-based realtime networking on all platforms after almost two years of collaboration with n0: webxdc apps get a `joinRealtimeChannel()` API that spins up private, ephemeral, end-to-end-encrypted gossip networks between chat participants, bootstrapped by an Iroh Ticket inside an encrypted chat message, with relays hosted on chatmail servers. Delta Chat's holger krekel also presented the integration in his usable-security talk at Pass the SALT 2025.
- **Why it matters:** The first large-scale production deployment of iroh in a consumer app — the proof point every later adopter (Fedimint, Strada, Block) could point to.
- **Link:** https://delta.chat/en/2024-11-20-webxdc-realtime

### 7. "iroh comes to our RP2350 microcontroller" — Raspberry Pi
- **Who/where:** Raspberry Pi official news, 2026-11-06
- **What's announced:** iroh ported to the RP2350 microcontroller, demoed running on a Pimoroni Presto (over Wi-Fi) and a Pico 2, with Rüdiger Klaehn's engineering write-up linked and examples published in the iroh-pico-examples repo. n0's own embedded line continued with ESP32 posts the same year, including a smart-fan build.
- **Why it matters:** A microcontroller endpoint that can be dialled by public key from anywhere is a genuinely new shape for IoT — and Raspberry Pi itself chose to announce it.
- **Link:** https://www.raspberrypi.com/news/iroh-comes-to-our-rp2350-microcontroller/

### 8. "Pubky is Now Open!" — the pkarr public launch
- **Who/where:** Pubky / Synonym, Medium, public launch late 2024 (post since re-dated in Medium listings)
- **What's announced:** Pubky's public opening, built on **pkarr** (Public-Key Addressable Resource Records): signed DNS-like records published to the BitTorrent Mainline DHT, so a public key resolves to current endpoint addresses with no registrar or central directory.
- **Why it matters:** pkarr is the address-lookup layer iroh adopted (see "The road to iroh 1.0") — this is where that piece of the stack came from, and its launch is the moment key-addressable naming went public.
- **Link:** https://medium.com/pubky/opening-pubky-4bdca7dc4c77

*Release cadence note:* the 1.0 line kept moving after June — the n0 blog index lists iroh 1.4.2 (security fixes, 2026-11-18) and iroh 1.5.0 "Stream Affinity, Then Heads Down" (2026-11-19) as the latest release posts: https://www.iroh.computer/blog

---

## (c) Demos & showcases

### 1. Strada — direct file transfer for film & media production
- **Who/where:** Strada case study, n0 blog, 2026-11-02
- **What's shown:** A media-workflow company moving production files peer-to-peer with iroh instead of through central cloud storage. July 2026 numbers: 150 TB transferred in the month, 90.7% of it kept off central servers, across 12,000+ users including Fortune 500 teams.
- **Why it matters:** The hardest production numbers anyone has published for iroh — terabytes, real customers, and a cloud bill avoided. The showcase to cite when someone asks "but does it scale?"
- **Link:** https://www.iroh.computer/blog/strada-direct-file-transfer

### 2. Mesh LLM — distributed AI computing on iroh
- **Who/where:** Mesh LLM project, featured on n0 blog, 2026-07-11
- **What's shown:** Pooling GPUs across ordinary machines into one OpenAI-compatible API at `localhost:9337/v1`: requests run locally, route to a peer that has the model loaded, or split a too-big model layer-by-layer across several machines ("Skippy" pipeline mode, 235B models included). Every node is an iroh endpoint; gossip over QUIC handles discovery and routing.
- **Why it matters:** Distributed inference is the most ambitious "built on iroh" application so far — it treats authenticated p2p connections as the substrate for shared compute, not just file transfer.
- **Link:** https://www.iroh.computer/blog/mesh-llm

### 3. Buzz — Block shares spare compute over iroh
- **Who/where:** Block (Buzz + MeshLLM), featured on n0 blog, 2026-09-22
- **What's shown:** Block's agent workspace lets community members opt in to sharing spare GPU compute: MeshLLM is built in, machines advertise the models they can serve, and agents consume them through a local OpenAI-compatible endpoint — with discovery signed and membership-gated, and the inference path never touching a central server. An inter-relay mesh crate (one iroh endpoint per relay runtime, ALPN `buzz/mesh/1`) carries tunnel traffic and even huddle audio over QUIC datagrams.
- **Why it matters:** A Fortune-500-scale company building agent infrastructure on iroh in the open — and a worked example of iroh's identity model doubling as a compute-sharing trust boundary.
- **Link:** https://www.iroh.computer/blog/buzz-agent-workspaces

### 4. Paycode — iroh for payments at highway toll booths
- **Who/where:** Paycode (Mexico), featured on n0 blog, 2026-03-26
- **What's shown:** Payment terminals and point-of-sale systems at highway toll booths talking directly over iroh — no added servers — on gloriously legacy hardware: touch-based Windows 7 machines and dual-core CPUs, with iroh bundled inside a .NET 6 SDK and a Kotlin PoS app. QR codes carry iroh tickets to bootstrap connections; the PCI-compliant design keeps raw payment data on the terminal while iroh carries the encrypted command channel.
- **Why it matters:** The best counter-example to "p2p is a greenfield toy": production payments, strict compliance, ancient hardware, unreliable networks.
- **Link:** https://www.iroh.computer/blog/paycode

### 5. sendme & dumbpipe — the demo tools that grew an ecosystem
- **Who/where:** n0 (recounted in "The road to iroh 1.0," 2026-07-09)
- **What's shown:** n0's two minimal file-transfer/pipe demos became the ecosystem's seed crystals: sendme inspired a family of clones and descendants (Lightning P2P, alt-sendme, ARK Drop Desktop, DataBeam, sendme-egui, iroh-send), and dumbpipe inspired iroh-ssh and similar tools. Most are community-built GUIs and variants n0 never wrote.
- **Why it matters:** Organic cloning is the sincerest adoption signal — a whole shelf of "built with iroh" apps traces back to two weekend-demo-sized tools.
- **Link:** https://www.iroh.computer/blog/the-road-to-iroh-1-0

### 6. DashBeam — third-party product overview video
- **Who/where:** DashBeam (independent project), YouTube
- **What's shown:** A product overview video for a cross-platform p2p file-transfer app built on iroh — desktop, mobile, web and CLI — using mDNS for nearby devices and ticket/QR pairing for remote ones, with BLAKE3-verified chunks (the iroh-blobs model) under the hood.
- **Why it matters:** A polished, end-user-facing product demo from outside n0: what iroh looks like when it's invisible inside someone else's app.
- **Link:** https://www.youtube.com/watch?v=K6tHvd79cIw

---

## Notes on method

- Sources were checked directly on 2026-11-23 (blog index, FOSDEM event/speaker pages, conference schedules, podcast and episode pages, Delta Chat and Raspberry Pi posts).
- EuroRust was specifically checked: no iroh talk found in its archives — none is listed here, rather than padding the list.
- The iroh 1.0 launch video is embedded in the 1.0 announcement post (item b1) rather than linked separately, to avoid a duplicate entry.
- Dates after mid-2026 (Local-First Conf, RustConf, the 1.5.0 release) are as published by the venues and the n0 blog index at research time.


## Change log

- r1 (2026-11-24): Migrated from hatch-personal workspace/iroh-investigation/ under #13877; content unchanged.
