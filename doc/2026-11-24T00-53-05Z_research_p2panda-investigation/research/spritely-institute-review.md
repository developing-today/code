# Spritely Institute — full site review

Research lane SPRITELY-REVIEW, for the auth + transport investigation
(developing-today/code#13779). Written 2026-11-23 from the Institute's own
site ([spritely.institute](https://spritely.institute)) and the pages/repos
it links. Companion to `ocap-family.md` in this folder, which covered
Goblins/OCapN from specs and repos; this note covers the same ground from
Spritely's own presentation, plus the complete news archive and a
deep-dive on the end-to-end encrypted relay work (§6).

**Coverage statement.** News archive: all **61 articles** listed across
the seven archive pages ([news/](https://spritely.institute/news/) and
`index-1.html` … `index-6.html`), spanning **2023-01-30 to 2026-11-19**.
56 were read in full on their article pages; "Announcing Spritely Oaken"
was read via search-surfaced article text plus its NLnet project page;
four (Goblinville retrospective, Functional hash tables, Hoot 0.6.1,
Successful supporter drive) are summarised from the archive listing and
are marked † — the browser service rate-limited (HTTP 429) before their
article pages could be re-opened, and no detail beyond the listing is
claimed for them. Everything else on the site — home, about, project
pages, whitepaper list — was read directly.

---

## 1. The organisation

**What it is.** The Spritely Networked Communities Institute is a
501(c)(3) nonprofit research institution "built to develop new
technologies for the decentralized web"; all code is free/open-source and
all papers open access ([home](https://spritely.institute)). Its stated
purpose is "to advance networked user freedom" — communication systems
that respect agency and autonomy, and communities' rights to organise,
govern, and protect their members, framed as applying fundamental human
rights to networked systems
([about](https://spritely.institute/about/)).

**Lineage.** Spritely's founders co-authored
[ActivityPub](https://www.w3.org/TR/activitypub/), the W3C standard
behind the Fediverse. The Institute's thesis is that ActivityPub-style
federation leaves too much power and data with servers, and that object
capability security — the tradition running from E and Electric
Communities through Mark Miller's work — is the missing foundation. The
history section traces this directly: Christine Lemmer-Webber's
post-ActivityPub research led to Randy Farmer (Electric Communities /
Habitat lineage), and the two co-founded the Institute
([about](https://spritely.institute/about/)).

**Design values** (from the about page, paraphrased): secure
collaboration; "networks of consent" — capability security as consent
that is intentional, granted, contextual, accountable, and revocable,
explicitly rejecting both all-or-nothing trust and "zero trust" framing;
healthy self-governing communities; user agency; contextual
communication (an explicit rejection of the "global town square");
decentralised-by-default foundations; gatekeeper-free participation;
honesty about limitations ("we should not pretend we can prevent what we
cannot"); contribute to the commons; and "fun is a revolutionary act" —
adoptability through delight is treated as strategy, which explains the
game-jam programme in §3.

**People** ([about](https://spritely.institute/about/)):

- **Christine Lemmer-Webber** — Executive Director (from April 2024;
  co-founder, ActivityPub co-author, MediaGoblin co-founder).
- **David (Dave) Thompson** — Chief Technology Officer (from April 2024;
  leads engineering; Hoot's lead author by output).
- **Jessica Tallon** — Founding Technologist; lead Goblins developer and
  primary author of the OCapN specifications; ActivityPub co-editor.
- **Andy Wingo** (Igalia, contractor) — compiler specialist leading the
  Guile-on-Wasm work; also upstream Guile maintainer (Whippet GC).
- Wider team visible in article bylines: Juliana/Juli Sims (Shepherd
  port), Daphne Preston-Kendal (Oaken), tessa (developer relations),
  Amy Grinn/Pillow (community/comms).
- **Board**: Karen Sandler (Software Freedom Conservancy), Deb Nicholson
  (Python Software Foundation), Alex Handy (The MADE museum).
- **Randy Farmer** — co-founder; stepped down as Executive Director in
  April 2024, remaining a consultant
  ([org changes post](https://www.spritely.institute/news/organizational-changes.html)).

**Funding model.** Three legs, all visible on the site:

1. **NLnet / NGI Zero grants** — the technical programme's main funder.
   Grant-funded work announced on the site includes: OCapN
   standardisation + Guile/Racket interop (2023), the Shepherd×Goblins
   port (2024), Spritely Oaken (2025), and the E2EE relay + Guile Noise
   (2026). Adjacent NLnet grants to collaborators (Wingo's Whippet GC,
   Andrew Whatson's Pre-Scheme port) are celebrated in the same posts.
2. **Individual supporter drive** — Spritely's first, launched December
   2024, passed its **$80,000 goal in January 2025** with three weeks to
   spare and added a $40,000 stretch goal (a browser Goblins playground
   and "Cirkoban Deluxe")
   ([stretch-goal post](https://www.spritely.institute/news/2024-2025-supporter-drive-success.html)).
   News posts close with named supporter tiers (Diamond/Gold/Silver/
   Bronze) — the E2EE relay post lists dozens of individual names.
3. **Organisational partners/grants** — an organisational partners page
   is linked from About; its contents were **not verified** in this pass
   (browser rate limit).

**Community surfaces.** Developer community forum
([community.spritely.institute](https://community.spritely.institute),
opened to the public February 2024), IRC `#spritely` on libera.chat,
monthly-ish engineering office hours (from September 2024, Cal.com +
Jitsi), Fediverse (`@spritely@social.coop`) and Bluesky accounts, and a
twice-yearly Lisp Game Jam presence that functions as the project's
public demo engine.

---

## 2. Papers and whitepapers

Listed on the home page, all open access (HTML/PDF/org formats):

- **A Scheme Primer** — the on-ramp text for the Scheme language family,
  written for Spritely newcomers but general-purpose.
- **The Heart of Spritely: Distributed Objects and Capability Security**
  — the core technical paper: Goblins' toolbox for building p2p
  applications as naturally as local programs.
- **Petnames: A humane approach to secure, decentralized naming** — how
  to map human-readable local names onto cryptographic identifiers
  without a global registry.
- **Implementation of a Petnames System in an existing chat application**
  — the follow-on experience report.
- **Content Addressed Descriptors and Interfaces** — programs describing
  themselves to each other in a mutually suspicious network, using
  content addressing + a simple petname system.

All at [files.spritely.institute](https://spritely.institute) via the
home page's whitepaper section.

---

## 3. Projects

### 3.1 Goblins — the foundation
([project page](https://spritely.institute/goblins/))

Spritely's distributed object programming environment: local
transactions that roll back on error, asynchronous messaging that works
the same locally and over the network, automatic persistence, a
time-travelling debugger, distributed garbage collection, and promise
pipelining. Two implementations — **Guile** (primary since v0.10) and
**Racket** — both at **v0.18.0** per the project page, network-compatible
over OCapN. Code on Codeberg (`spritely/goblins`,
`spritely/racket-goblins`); manual published on the site. Spritely's own
maturity framing elsewhere is "working beta" (see `ocap-family.md`).

### 3.2 Hoot — Scheme to WebAssembly
([project page](https://spritely.institute/hoot/))

A Scheme-to-Wasm compiler backend for Guile plus a self-contained Wasm
toolchain (assembler, disassembler, linker, and a Wasm interpreter that
runs on the Guile VM) — no Emscripten/Binaryen dependency. Built so
Goblins can run in browsers. Latest release **0.10.0** (October 2026).
Supports R7RS-small and a subset of Guile; runtimes: Firefox 121+,
Chrome 119+, Safari 26+, Node 22+. Distributed via Guix, Debian
testing/unstable, and a Homebrew tap.

### 3.3 OCapN — the protocol suite
([ocapn.org](https://ocapn.org); linked from the home page)

The pre-standardisation effort to make CapTP implementations
interoperate: CapTP itself, third-party handoffs, the netlayer
abstraction, and the `ocapn://` locator URI structure. Participants per
ocapn.org: Agoric, Spritely, Cap'n Proto, and independent experts;
meetings monthly, work on GitHub (`ocapn/ocapn`). Implementations listed:
Goblins (Guile), Goblins (Racket), Endo (JavaScript), DObjects (Dart).
Spritely drafted the first spec versions from Goblins' implementation,
with Jessica Tallon as primary spec author. Status remains explicitly
**pre-specification** — no standards-body submission yet (see
`ocap-family.md` §1.2 for the spec-repo detail).

### 3.4 Oaken — capability-secure Scheme sublanguage (2025)
Announced May 2025, NLnet-funded, by Daphne Preston-Kendal
([announcement](https://spritely.institute/news/announcing-spritely-oaken.html);
[NLnet project page](https://nlnet.nl/project/SpritelyOaken/)). Oaken is
a new Scheme-family programming system for running **untrusted
third-party code** safely inside a host program: a library system with
"taming" as a first-class concept (a library can be closed over a
restricted version of, e.g., filesystem access that reaches only one
directory — the "powerbox pattern"), building on Scheme 48 and Jonathan
Rees's lambda-calculus security work (Rees advises). Aimed at safe mods,
macros, and plugins, and at composing with Goblins/Hoot. Early-stage;
initial version deliberately supports only basic tamings.

### 3.5 Shepherd × Goblins — capability-secure system layer
A port of GNU Shepherd (Guix's init/service manager) to Goblins actors,
NLnet-funded, announced May 2024 and updated September 2025. Goal:
services as object capabilities, unifying root and user Shepherds, and
eventually distributed Shepherds managing each other over OCapN (a
possible `guix deploy` agent). Status at the September 2025 update: core
actors rewritten with unit tests, public API preserved as a
compatibility layer, demo managing services in a Guix home container; a
handful of shell tests still failing and syslog a known gap — **not yet
PID 1 ready** ([update](https://www.spritely.institute/news/shepherd-goblins-update.html)).

### 3.6 Guile Noise — Noise Protocol library for Guile
Written for the E2EE relay work (§6.3): a general-purpose Noise
implementation for Guile/Hoot where none existed. All 12 fundamental
interactive handshake patterns; X25519, AES-GCM, SHA-256/SHA-512;
GnuTLS backend on the Guile VM, WebCrypto SubtleCrypto backend under
Hoot; validated against the Noise test vectors on both backends. Linked
from the relay article to Codeberg (exact repo path not re-verified in
this pass due to the browser rate limit).

### 3.7 Applications and demos built on the stack

- **Mandy** (January 2026) — a prototype implementing **ActivityPub on
  Goblins**: Goblins actors bridged to HTTP via Guile Fibers, activities
  unmarshalled into actors and delivered as ordinary Goblins messages.
  The argument: ActivityPub's actor/inbox/outbox shape maps naturally
  onto Goblins actors, opening a capability-secure path for Fediverse
  software.
- **Brassica Chat** (November 2025) — local-first group chat composing
  ocaps (online authority), actors, Byzantine-fault-tolerant
  operation-based CRDTs (signed DAG events, hybrid logical clocks) for
  offline sync, and zcaps for offline authorisation. Spritely's worked
  answer to "capabilities vs replicated data" — compose them.
- **GoblinShare** (November 2025) — secure p2p file sharing in ~250 lines
  over Goblins + a Guile port of **Magenc** (encrypted content-addressed
  storage where the magnet link itself is the capability), delivered
  over the Tor netlayer.
- **Polydraw** (November 2026) — peer-to-peer party drawing game
  (Autumn Lisp Game Jam 2026, 5th overall); the first application to
  exercise **third-party handoffs over the new E2EE relay netlayer**,
  and the relay's dogfood test.
- **Cirkoban** (June 2024) — Sokoban × Wireworld puzzle game (15 levels,
  2nd in Spring Lisp Game Jam 2024); the first publicly accessible
  Goblins-in-the-browser application, using transactional rollback as
  its undo mechanic.
- **Goblinville** † (June 2025) — Spring Lisp Game Jam 2025
  retrospective; a networked virtual-town jam project.
- **Fantasary** (June 2023) — prototype text virtual world; testbed for
  the TCP netlayer, petnames, and the "unum" pattern.
- **Wireworld** (June 2023) — WASM-4 cellular-automaton cartridge built
  with Hoot's low-level Wasm tooling.
- Plus jam entries **Strigoform** and **Cybersol** (December 2023).

---

## 4. The news archive — all 61 articles, individually

Newest first, as listed by the archive. Format: **title** — author,
date — what it announced/argued; why it matters. † = summarised from the
archive listing only (see coverage statement).

### 2026

1. **Lisp Game Jam devlog: Polydraw** — Thompson & Lemmer-Webber,
   2026-11-19 — Day-by-day build log of the p2p party game Polydraw
   (draw a prompt in triangles in 60 seconds; others guess), built from
   Player/Lobby/Game/Controller actors distributed over OCapN via the
   new E2EE relay. Records real bugs found under deadline: a "crossed
   hellos" race (promises on the losing connection never resolve;
   worked around with a timeout retry) and the promise-vs-reference
   `eq?` identity footgun. Ranked 5th overall. *Matters: first app to
   exercise third-party handoffs over the relay netlayer — the relay's
   public proof.*
   ([article](https://spritely.institute/news/lisp-game-jam-devlog-polydraw.html))
2. **Hoot 0.10.0 released!** — Thompson, 2026-10-28 — Adds R6RS library
   versioning and a large `(rnrs …)` surface, partial SRFI-13/60,
   `define-macro`, CLI validation and a Node exnref flag; promotes the
   game-jam template. *Matters: closes much of the R6RS/Guile
   compatibility gap for browser Scheme.*
   ([article](https://spritely.institute/news/hoot-0-10-0-released.html))
3. **End-to-end encrypted relays for OCapN** — Tallon, 2026-10-15 —
   Announces the NLnet-funded federated, stateless, E2EE relay that
   replaces prelay: a Noise meta-netlayer (XK, X25519/AES-GCM/SHA-256)
   composed over the relay netlayer, a new Guile Noise library, and an
   OCapN spec to follow. *Matters: the subject of §6 — Spritely's NAT
   answer becomes recommendable.*
   ([article](https://www.spritely.institute/news/end-to-end-encrypted-relays-for-ocapn.html))
4. **Hoot 0.9.0 released!** — Thompson, 2026-05-13 — Moves to standard
   Wasm exceptions and DWARF debug data (with a `hoot strip` for
   production), a DWARF parser in reflect.js, an exported `main` for
   non-JS runtimes, and a new `hoot compile` CLI replacing
   `guild compile-wasm`. *Matters: debugging and portability of Hoot
   binaries beyond JavaScript.*
   ([article](https://spritely.institute/news/hoot-0-9-0-released.html))
5. **Spritely Goblins v0.18.0: Sleepy actors!** — Thompson &
   Lemmer-Webber, 2026-04-21 — "Sleepy actors": persisted actors can be
   evicted to the store and woken on message receipt (sleep-every-turn
   and LRU strategies shipped). Also adopts the latest OCapN draft —
   `op:deliver-only` dropped, GC ops batched/pluralised, **protocol
   version bumped (incompatible)** — and fixes a multi-vat restore race
   and a WebSocket netlayer signing-oracle vulnerability. *Matters:
   large persistent actor graphs no longer need to be memory-resident;
   also the clearest evidence the wire protocol is still moving.*
   ([article](https://www.spritely.institute/news/spritely-goblins-v0-18-0-sleepy-actors.html))
6. **Hoot 0.8.0 released!** — Thompson, 2026-02-25 — A built-in
   `(hoot repl)` for live hacking in Wasm, a web REPL/server proxying to
   a browser tab, `hoot repl`/`hoot server` CLI commands, initial Emacs
   (Geiser) support, web request/response/socket modules, and extensive
   Wasm toolchain validation work. *Matters: REPL-driven development —
   Scheme's signature workflow — arrives for Wasm.*
   ([article](https://spritely.institute/news/hoot-0-8-0-released.html))
7. **Mandy: ActivityPub on Goblins** — Tallon, 2026-01-06 — The
   ActivityPub-on-Goblins prototype (§3.7): HTTP bridging via Fibers, a
   nonce registry for persistent object IDs, activities as Goblins
   messages. *Matters: a concrete bridge from Spritely's research back
   to the Fediverse its founders standardised.*
   ([article](https://spritely.institute/news/mandy-activitypub-on-goblins.html))

### 2025

8. **Composing capability security and conflict-free replicated data
   types** — Thompson, 2025-11-24 — Argues ocaps and CRDTs compose
   rather than compete, demonstrated by Brassica Chat (§3.7): ocaps for
   online access, BFT operation-based CRDTs for offline sync, zcaps for
   offline authorisation; contrasts capabilities with ACLs on
   granularity, delegation, and confused-deputy risk. *Matters:
   Spritely's position paper for local-first architecture — and the
   family's first real answer to the offline question.*
   ([article](https://spritely.institute/news/composing-capability-security-and-conflict-free-replicated-data-types.html))
9. **GoblinShare: Secure, Peer-to-Peer File-Sharing with Goblins** —
   Sims, 2025-11-18 — GoblinShare + the Guile Magenc port built in a
   week (§3.7); core logic ~250 lines. *Matters: the "look how little
   code secure p2p takes" demo.*
   ([article](https://spritely.institute/news/goblinshare-secure-peer-to-peer-file-sharing-with-goblins.html))
10. **Hoot 0.7.0 released!** — Thompson, 2025-10-22 — Guile source
    modules on the load path by default, SRFI-1 from Guile's pure
    Scheme, modules from JS typed arrays, explicit top-level imports;
    first Safari 26 compatibility. *Matters: more real Guile code runs
    under Hoot; WebKit unblocked.*
    ([article](https://spritely.institute/news/hoot-0-7-0-released.html))
11. **Spritely Goblins v0.17.0: Persistence is better than ever!** —
    Tallon, 2025-10-16 — **Bloblin**, a streaming persistence store:
    full graph once, then compressed Syrup deltas per churn (new file
    every 1,000 churns), "many thousands of deltas per second"; store
    migration tooling; new `^vector`/`^ring-buffer` actors. *Matters:
    removes the whole-graph-rewrite bottleneck that capped persistent
    vat scale.*
    ([article](https://www.spritely.institute/news/spritely-goblins-v0-17-0-persistence-is-better-than-ever.html))
12. **Shepherd × Goblins update** — Sims & Thompson, 2025-09-10 —
    Progress on the Shepherd port (§3.5): core actors rewritten and
    unit-tested, API kept as a compatibility layer, working demo in a
    Guix home container; 4–7 shell tests still failing, syslog the big
    gap. *Matters: capability security reaching the OS service layer,
    honestly reported as not-yet-PID-1.*
    ([article](https://www.spritely.institute/news/shepherd-goblins-update.html))
13. **Spritely Goblins v0.16.1 released!** — Tallon, 2025-09-03 — Patch
    fixing 0.16.0 issues that broke OCapN-over-Hoot, duplicate-connection
    bugs, and a vat event-log data-loss bug. *Matters: restores the
    browser networking path.*
    ([article](https://www.spritely.institute/news/spritely-goblins-v0-16-1-released.html))
14. **Spritely Goblins v0.16.0 released!** — Lemmer-Webber, 2025-08-07 —
    Two core speedups claimed at 10–20× each (compile-time `spawn`;
    cookie-comparison promises replacing sealer-based `bcom`), plus a
    new **Unix Domain Socket netlayer** using socket-passing to avoid
    ACL/confused-deputy risks for local IPC. *Matters: a step-change in
    Goblins performance, and OCapN as a local IPC mechanism.*
    ([article](https://www.spritely.institute/news/spritely-goblins-v0-16-0-released.html))
15. **Spritely presented spirited speeches spanning the planet** —
    Pillow, 2025-07-31 — Roundup of six months of talks (FOSDEM-heavy):
    fediverse futures, ocap security, Hoot web deployment, Shepherd,
    Spritely's vision. *Matters: the talks index for the programme.*
    ([article](https://www.spritely.institute/news/spritely-presented-spirited-speeches-spanning-the-planet.html))
16. † **Goblinville: A Spring Lisp Game Jam 2025 retrospective** —
    2025-06-04 (date from secondary listings) — Retrospective on
    Goblinville, Spritely's Spring Lisp Game Jam 2025 networked
    virtual-town project.
    ([article](https://spritely.institute/news/goblinville-a-spring-lisp-game-jam-2025-retrospective.html))
17. † **Functional hash tables explained** — 2025-05-21 — Explainer on
    the quest for a persistent/functional hash table for Goblins:
    association lists are immutable and O(1) to extend but O(n) to look
    up; the post works through what a functional hash table must
    provide. *Matters: the data-structure groundwork under Goblins'
    persistence and Hoot's hashtable work.*
    ([archive listing](https://www.spritely.institute/news/index-1.html))
18. † **Hoot 0.6.1 released!** — 2025-05-06 — Patch release following
    0.6.0; details not verified in this pass.
    ([archive listing](https://www.spritely.institute/news/index-1.html))
19. **Announcing Spritely Oaken** — Preston-Kendal, 2025-05-05 —
    Introduces Oaken (§3.4): an NLnet-funded secure Scheme sublanguage
    for running untrusted code, with library "taming" and the powerbox
    pattern, inspired by Rees's dissertation (Rees advising). Use cases
    named: safe spreadsheet macros, game mods, sandboxed Shepherd
    services. *Matters: Spritely's confinement layer — the missing
    piece between Goblins' network security and in-process plugin
    safety.*
    ([article](https://spritely.institute/news/announcing-spritely-oaken.html))
20. † **Successful supporter drive supplements Spritely's story** —
    2025-04-08 — Reports the success of Spritely's first supporter
    drive (see #24/#25 for the goal and launch); the archive excerpt
    stresses that donations are also individual endorsements of the
    mission. *Matters: evidence the individual-funding leg works.*
    ([archive listing](https://www.spritely.institute/news/index-1.html))
21. **Spritely Goblins v0.15.1 released!** — Thompson, 2025-03-26 —
    Patch (largely Tallon's work) fixing CapTP issues with ghashes/far
    references in persistent vats, GC thread-safety, severed-connection
    re-enlivening, prelay/WebSocket severance, and Hoot compilation;
    adds serialisable vectors/pairs over CapTP, `on-sever`, and a
    timers library. *Matters: stabilises the new browser/OCapN surface.*
    ([article](https://www.spritely.institute/news/spritely-goblins-v0-15-1-released.html))
22. **Spritely Goblins v0.15.0: Goblins in the browser!** —
    Lemmer-Webber & Grinn, 2025-01-24 — Goblins compiled by Hoot to
    Wasm from the same codebase as Guile, OCapN working in-browser
    (in-page 150-line goblin-chat demo), a new WebSocket netlayer
    bridged by prelay (noted as **not yet end-to-end encrypted** — the
    gap §6 closes), and 1.2–2× performance gains. *Matters: the
    Hoot bet pays off; Goblins reaches browser users.*
    ([article](https://www.spritely.institute/news/spritely-goblins-v0-15-0-goblins-in-the-browser.html))
23. **Guile Hoot 0.6.0 released!** — Thompson, 2025-01-22 —
    Reimplemented weak-key/weak-value hashtables and weak vectors;
    enough of Guile (psyntax expander, eval, environments, modules) to
    compile nearly all of Goblins; Safari/WebKit 18.2+ joins the
    supported list after WebKit shipped Wasm GC + tail calls.
    *Matters: removes the last browser blockers to Goblins-on-Hoot.*
    ([article](https://www.spritely.institute/news/guile-hoot-0-6-0-released.html))
24. **Spritely is going to Guix Days and FOSDEM** — Grinn, 2025-01-16 —
    Whole team to Brussels; two hours of talks across Guix Days and
    FOSDEM 2025 (fediverse, Shepherd, Hoot, Goblins). *Matters: the
    public debut of several programme threads at once.*
    ([article](https://www.spritely.institute/news/spritely-is-going-to-guix-days-and-fosdem.html))
25. **Supporter drive goal complete! Time for a stretch goal!** —
    Lemmer-Webber & Grinn, 2025-01-15 — The first supporter drive
    passed **$80,000 with three weeks to go**; stretch goal +$40,000
    for a browser Goblins playground and Cirkoban Deluxe. *Matters:
    community funding validated early and publicly.*
    ([article](https://www.spritely.institute/news/2024-2025-supporter-drive-success.html))

### 2024

26. **Spritely launches Supporter Drive** — Lemmer-Webber & Grinn,
    2024-12-04 — Launches the first supporter campaign (501(c)(3),
    tiered perks) with the full pitch: Goblins + OCapN + Hoot as new
    decentralised foundations rather than Web 2.0 replicas. *Matters:
    the individual-funding leg begins.*
    ([article](https://www.spritely.institute/news/spritely-launches-supporter-drive.html))
27. **Make a game with Hoot for the Autumn Lisp Game Jam!** — tessa,
    2024-10-22 — Jam invitation with Spritely's Hoot game template and
    forum support. *Matters: the jam pipeline that later produced
    Polydraw starts here.*
    ([article](https://www.spritely.institute/news/make-a-game-with-hoot-for-the-autumn-lisp-game-jam.html))
28. **Our first office hours: A recap (and how to join the next one)**
    — tessa, 2024-10-17 — Ten community members at the first session;
    office hours become monthly (last Wednesday, Cal.com + Jitsi).
    *Matters: a standing open support channel.*
    ([article](https://www.spritely.institute/news/first-office-hours-recap.html))
29. **Spritely went to DWeb Camp: 2024 Recap** — Thompson, 2024-09-26 —
    Talks on the Wasm/Hoot bet and keeping decentralisation fun; a
    "Spritely Arcade" demo table (with Endo JS) running Cirkoban and
    Strigoform. *Matters: games-as-demos validated with the dweb
    community.*
    ([article](https://www.spritely.institute/news/spritely-went-to-dweb-camp-2024-recap.html))
30. **Spritely Goblins v0.14.0: libp2p and Improved Persistence** —
    Tallon, 2024-09-19 — Adds a **libp2p netlayer** (TCP/QUIC + NAT
    hole punching via a companion Go libp2p daemon, mirroring how the
    onion netlayer uses Tor) and upgrades Aurie persistence:
    cross-vat far references, persistent netlayers, versioned
    migrations. *Matters: Goblins' first non-Tor NAT traversal, and
    persistence becomes practical across vats and firewalls.*
    ([article](https://www.spritely.institute/news/spritely-goblins-v0-14-0-libp2p-and-improved-persistence.html))
31. **Come work with us! Hiring a Technical Administrator** —
    Institute, 2024-09-12 — Organisational-capacity hire bridging
    technical and administrative work. *Matters: the nonprofit is
    professionalising behind the research.*
    ([article](https://www.spritely.institute/news/come-work-with-us-hiring-a-technical-administrator.html))
32. **Guile Hoot v0.5.0 released!** — Thompson, 2024-09-06 — A Scheme
    interpreter in `(hoot eval)`, a lightweight Fibers-style
    concurrency API on JS promises/delimited continuations, more Guile
    compatibility, backtraces with source locations. Safari still
    unsupported. *Matters: runtime evaluation + async + debuggability
    for real browser apps.*
    ([article](https://www.spritely.institute/news/guile-hoot-v0-5-0-released.html))
33. **Announcing Spritely Office Hours!** — tessa, 2024-09-05 — Trial
    of engineering office hours; support routing (bugs → Codeberg
    trackers, questions → forum first). *Matters: support
    infrastructure for a growing user base.*
    ([article](https://www.spritely.institute/news/announcing-spritely-office-hours.html))
34. **Spritely goes to DWeb Camp** — tessa, 2024-08-09 — Itinerary
    announcement for DWeb Camp 2024 (talks + Demo Night Market).
    *Matters: outreach to the decentralised-web community.*
    ([article](https://www.spritely.institute/news/spritely-goes-to-dweb-camp.html))
35. **Cirkoban: Sokoban meets cellular automata written in Scheme** —
    Thompson, 2024-06-03 — The Cirkoban story (§3.7): 15 levels,
    transactional rollback as undo, first publicly accessible
    Goblins-in-browser app, 2nd place in the Spring jam. *Matters: the
    first complete, playable proof of Hoot + Goblins on the web.*
    ([article](https://www.spritely.institute/news/cirkoban-sokoban-meets-cellular-automata-written-in-scheme.html))
36. **Make a game with Hoot for the Lisp Game Jam!** — Thompson,
    2024-05-15 — Spring jam invitation; 100+ registered participants;
    the Hoot game template introduced. *Matters: the jam as Hoot's
    stress-test and recruiting channel.*
    ([article](https://spritely.institute/news/make-a-game-with-hoot-for-the-lisp-game-jam.html))
37. **Distributed System Daemons: More Than a Twinkle in Goblins'
    Eye** — Sims, 2024-05-09 — Announces the NLnet-funded Shepherd port
    (§3.5), "the largest real-world deployment of Spritely technology
    so far," with the ambition of Guix as an object-capability OS and
    distributed Shepherds; also notes NLnet grants for Whippet and
    Pre-Scheme. (Slug retains its December-2023 drafting date.)
    *Matters: Goblins tested at the operating-system layer.*
    ([article](https://www.spritely.institute/news/spritely-nlnet-grants-december-2023.html))
38. **Guile Hoot v0.4.1 released!** — Thompson, 2024-04-30 —
    Maintenance release fixing 0.4.0 miscompilations (tailify pass),
    FFI and linker bugs; adds `(hoot web-server)` and compiler flags.
    *Matters: stabilises the compiler before the Spring jam.*
    ([article](https://www.spritely.institute/news/guile-hoot-v0-4-1-released.html))
39. **Organizational changes at the Spritely Institute** —
    Lemmer-Webber, 2024-04-25 — Randy Farmer steps down as Executive
    Director (remains consultant/co-founder); Lemmer-Webber becomes
    ED, Thompson CTO; full team listing. *Matters: the leadership
    handover that set the current structure.*
    ([article](https://www.spritely.institute/news/organizational-changes.html))
40. **Spritely Goblins v0.13.0: Object Persistence and Easier IO!** —
    Lemmer-Webber, 2024-04-23 — Introduces **Aurie**, Goblins'
    persistence system (version-tagged self-portraits via
    `define-actor`, transactional deltas), and the `^io` actor wrapping
    IO resources so programs avoid programming in raw Fibers.
    *Matters: long-lived Goblins worlds can survive restarts and
    evolve — the persistence line that Bloblin later completes.*
    ([article](https://www.spritely.institute/news/spritely-goblins-v0-13-0-object-persistence-and-easier-io.html))
41. **Guile Hoot v0.4.0 released!** — Thompson, 2024-04-09 —
    User-defined modules (R6RS `library` first), the standard library
    reorganised into importable modules, a project template, complex
    arithmetic, Wasm exceptions; 54 of 57 Scheme benchmarks running.
    *Matters: multi-file programs make Hoot usable for real projects.*
    ([article](https://www.spritely.institute/news/guile-hoot-v0-4-0-released.html))
42. **How to Draw an Owl with the Spritely Institute** — tessa,
    2024-04-01 — April Fools' two-step owl-drawing meme post.
    *Matters: culture/fun as deliberate community strategy.*
    ([article](https://www.spritely.institute/news/how-to-draw-an-owl-with-the-spritely-institute.html))
43. **LWN on Spritely** — Institute, 2024-02-29 — Pointer to LWN.net's
    piece on Spritely's long-term distributed-computing paradigm and
    OCapN's role. *Matters: mainstream free-software press attention.*
    ([article](https://www.spritely.institute/news/lwn-on-spritely.html))
44. **[Contract] Goblins (Scheme/Wasm) Integration Engineer** —
    Institute, 2024-02-26 — Remote 20 hrs/wk contract ad. *Matters:
    targeted investment in the Goblins×Hoot integration point.*
    ([article](https://www.spritely.institute/news/contract-goblins-scheme-wasm-integration-engineer.html))
45. **Developer Community Forum open for everyone!** — Farmer,
    2024-02-02 — The community forum goes public; support expectations
    set (bugs to trackers, questions to the forum). *Matters: Spritely's
    public, searchable community home.*
    ([article](https://www.spritely.institute/news/developer-community-forum-open-for-everyone.html))
46. **Guile Hoot v0.3.0 released!** — Thompson, 2024-01-30 — Partial
    module system (prelude split into importable libraries), hash
    tables incl. weak-key, debugging aids; over half the Scheme
    benchmarks passing; Firefox 121 enables Wasm tail calls.
    *Matters: the prerequisite step to user modules in 0.4.0.*
    ([article](https://www.spritely.institute/news/guile-hoot-v0-3-0-released.html))
47. **Meet us at GuixDays & FOSDEM!** — Farmer, 2024-01-23 — FOSDEM
    2024 invitation (Hoot and unified-vision talks); openly solicits
    partnership and joint-funding conversations. *Matters: Spritely
    positioning inside the Guix/Guile community.*
    ([article](https://www.spritely.institute/news/fosdem2024.html))
48. **Spritely Goblins v0.12.0 Released: Two new netlayers join the
    OCapN family and more!** — Tallon, 2024-01-22 — Introduces
    **prelay** (the federated relay, shipped explicitly unencrypted and
    unauthenticated — "the relay has the power to snoop and modify")
    and the **TCP+TLS netlayer** (self-signed certs, faster than Tor);
    CapTP hardening (crossed-hellos mitigation, handoff certificate
    verification, test-suite integration). *Matters: the starting point
    of the relay story §6 completes — and an unusually honest
    "don't trust this yet" release.*
    ([article](https://www.spritely.institute/news/spritely-goblins-v0-12-0-released-two-new-netlayers-join-the-ocapn-family-and-more.html))

### 2023

49. **[EFF] Spritely and Veilid: Exciting Projects Building the
    Peer-to-Peer Web** — Institute, 2023-12-15 — Pointer to the EFF's
    joint profile of Spritely and Veilid. *Matters: civil-society
    endorsement beyond the Lisp world (and a Spritely–Veilid pairing
    worth noting for our survey).*
    ([article](https://www.spritely.institute/news/eff-spritely-and-veilid-exciting-projects-building-the-peer-to-peer-web.html))
50. **Scheme in Scheme on Wasm in the browser** — Thompson, 2023-12-15
    — Building a browser REPL for Hoot from a metacircular Scheme
    interpreter (SICP-style) because Hoot didn't yet expose Guile's
    evaluator in Wasm. *Matters: capability-secure in-browser Scheme
    REPL; the path to try-it-live tooling.*
    ([article](https://www.spritely.institute/news/scheme-in-scheme-on-wasm-in-the-browser.html))
51. **Having a Hoot at the Lisp Game Jam: A Tale of Two Daves** —
    tessa, 2023-12-07 — Interview with David Wilson (Cybersol) and Dave
    Thompson (Strigoform) on building jam games with Hoot 0.1.0 and
    working around its limits. *Matters: the jam as real-world
    stress-test driving Hoot's FFI.*
    ([article](https://www.spritely.institute/news/having-a-hoot-at-the-lisp-game-jam-a-tale-of-two-daves.html))
52. **Building interactive web pages with Guile Hoot** — Thompson,
    2023-11-30 — Tutorial: with Wasm GC, Hoot-compiled Scheme calls the
    DOM directly via the new FFI; builds from hello-world to a full
    to-do app with a React-like virtual-DOM layer, almost entirely in
    Scheme. *Matters: proof Hoot builds real interactive front-ends.*
    ([article](https://www.spritely.institute/news/building-interactive-web-pages-with-guile-hoot.html))
53. **Guile Hoot v0.2.0 released!** — Thompson, 2023-11-30 —
    Near-complete R7RS-small, the foreign function interface,
    user-defined record types, a better tree-shaker; Chrome 119 ships
    Wasm GC + tail calls. *Matters: the FFI release — Hoot reaches the
    DOM and host APIs.*
    ([article](https://www.spritely.institute/news/guile-hoot-v0-2-0-released.html))
54. **Introducing OCapN, interoperable capabilities over the network**
    — Tallon, 2023-11-16 — The OCapN launch post: NLnet-funded
    standardisation with five milestones — Guile↔Racket interop done,
    pre-standardisation group formed, three draft specs (CapTP,
    Netlayers, Locators), an interop test suite (already passed by an
    independent Haskell implementation), and both Goblins
    implementations passing it; eventual IETF/W3C ambition stated.
    *Matters: Goblins' internal protocol becomes a multi-implementation
    effort — the founding document of the OCapN programme.*
    ([article](https://www.spritely.institute/news/introducing-ocapn-interoperable-capabilities-over-the-network.html))
55. **Guile Hoot v0.1.0 released!** — Thompson, 2023-10-16 — First
    official Hoot release: Guile's Scheme-to-Wasm backend plus the
    self-contained Scheme-written Wasm toolchain; kilobyte-scale
    binaries on host Wasm GC; explicitly no module system, records, or
    FFI yet. *Matters: a dynamic language becomes a first-class Wasm
    citizen.*
    ([article](https://www.spritely.institute/news/guile-hoot-v0-1-0-released.html))
56. **Scheme in the browser: A Hoot of a tale** — Thompson, 2023-10-10
    — First end-to-end Hoot proof: Wireworld reimplemented in ~70 lines
    of Scheme (28 KiB gzipped), and the design choices explained
    (Wasm GC over a shipped collector, tail calls, in-Guile Wasm
    interpreter for REPL-driven development). *Matters: the "it
    actually works" moment before 0.1.0.*
    ([article](https://www.spritely.institute/news/scheme-wireworld-in-browser.html))
57. **[Hiring] Open Source Developer Relations Manager** — Institute,
    2023-09-29 — DevRel hire to support the Goblins ecosystem; more
    roles promised as funding permits. *Matters: the shift from pure
    R&D toward community growth.*
    ([article](https://www.spritely.institute/news/hiring-open-source-developer-relations-manager.html))
58. **Lisp Game Jam – "Wireworld" – Hoot's low level Wasm tooling in
    action** — Lemmer-Webber, 2023-06-12 — A WASM-4 Wireworld cartridge
    built with Hoot's low-level assembly tools; argues Scheme
    generating Wasm beats hand-written WAT on readability, correctness,
    and iteration speed. *Matters: Hoot's low-level layers as a
    "programs that write programs" toolkit.*
    ([article](https://www.spritely.institute/news/hoot-wireworld-live-in-browser.html))
59. **Lisp Game Jam – "Fantasary" – prototyping an object world
    (& TCP netlayer)** — Thompson, 2023-06-07 — Fantasary (§3.7): a
    text virtual world built to test a prototype Simple TCP netlayer,
    petnames, and the unum pattern; candid about open problems — NAT
    traversal, 4,124-character sturdyref URIs from encoded certs, X.509
    as a poor fit. *Matters: the testbed for the TCP+TLS netlayer that
    shipped in 0.12.0.*
    ([article](https://www.spritely.institute/news/lisp-game-jam-fantasary-prototyping-an-object-world-tcp-netlayer.html))
60. **Directly compiling Scheme to WebAssembly: lambdas, recursion,
    iteration!** — Lemmer-Webber, 2023-05-30 — Three months after the
    Guile-on-Wasm announcement, Hoot compiles Scheme procedures
    directly to Wasm (factorial in 970 bytes; a tail-recursive loop
    benchmarked ~5× native Guile, with caveats stated); roadmap:
    R7RS-small, then Goblins in the browser. *Matters: first public
    evidence direct Scheme-to-Wasm could be tiny and fast.*
    ([article](https://www.spritely.institute/news/scheme-to-wasm-lambdas-recursion.html))
61. **Spritely Goblins v0.10, for Guile and Racket!** — Lemmer-Webber,
    2023-01-30 — Guile and Racket Goblins interoperate over OCapN for
    the first time (goblin-chat across both), and Guile becomes the
    primary implementation. *Matters: cross-implementation interop
    proves OCapN is a protocol, not an implementation detail — the
    archive's founding release.*
    ([article](https://spritely.institute/news/spritely-goblins-v0-10-for-guile-and-racket.html))

### Themes across the archive

- **Release trains (31 of 61 posts).** Goblins: 0.10 → 0.18.0 (9 release
  posts, roughly two per year, each with a named theme — persistence,
  netlayers, browser, performance, sleepy actors). Hoot: 0.1.0 → 0.10.0
  (13 release posts, accelerating to ~4/year). The two trains are
  visibly coupled: Hoot releases exist to unblock Goblins milestones
  (0.6.0 → Goblins-in-browser; 0.16.1 fixes OCapN-over-Hoot).
- **OCapN standardisation (a 2023 founding, a 2026 relay).** #61 (first
  interop) → #54 (the programme launch) → #48 (new netlayers) → #3
  (E2EE relay + spec). The through-line: every protocol claim is
  shipped in Goblins first, then specified for others.
- **The browser as the deployment target.** From #60's first compiled
  lambda to #22's Goblins-in-the-browser to #1's relay-served game,
  the archive's main plot is Scheme/Goblins escaping the desktop via
  Wasm — with the relay as the networking piece browsers forced.
- **Games as the demo strategy.** 11 posts are jam-related. This is
  deliberate ("fun is a revolutionary act"): every major capability
  (TCP netlayer, Hoot FFI, Goblins-in-browser, E2EE relay handoffs)
  got its first public exercise in a jam game.
- **Grants and community funding.** NLnet underwrites the protocol
  work (#54, #37, #19, #3); the supporter drive (#26, #25, #20) funds
  the commons. Funding announcements are treated as technical news,
  with amounts and goals published.
- **Institution-building.** Leadership handover (#39), forum (#45),
  office hours (#33, #28), hiring (#57, #44, #31), conference presence
  (#47, #24, #15) — roughly a fifth of the archive is the nonprofit
  building its own scaffolding, reported with unusual candour about
  gaps and failures (failing Shepherd tests, Safari gaps, prelay's
  insecurity).

---

## 5. Project health and contribution policy

**Cadence.** The archive itself is the health signal: 61 posts over 46
months with no silent year; Goblins releases roughly twice a year
(0.10 Jan 2023 → 0.18.0 Apr 2026), Hoot accelerating (0.1.0 Oct 2023 →
0.10.0 Oct 2026, four releases in 2026 alone counting 0.8–0.10). Work is
grant-paced: each NLnet grant maps to a visible deliverable chain
(standardisation → Shepherd → Oaken → relay), and the 2024–25 supporter
drive added a second, community leg ($80k goal publicly met). The team
is small (three named engineering leads plus a handful of staff and
contractors), which is both the strength (coherence) and the risk
(concentration: Tallon alone is Goblins lead *and* OCapN spec author).

**Contribution policy — the AI question.** Confirmed, with a scope
nuance. The **OCapN project's** README states the prohibition on
AI-generated contributions to its specifications, test suite, and
official tooling (recorded in `ocap-family.md` §6, from
[ocapn/ocapn](https://github.com/ocapn/ocapn)). That policy belongs to
the multi-party OCapN group, whose work happens on GitHub with monthly
meetings open to anyone ([ocapn.org](https://ocapn.org)). On the
Institute's own site, contribution routes are the Codeberg repos (issue
and PR links from the project pages) and the community forum; this pass
did **not** surface an equivalent AI-contribution statement on
spritely.institute itself — so: OCapN upstream = no-AI-contributions by
policy; Spritely's own repos = not verified either way from the site.

---

## 6. Deep dive: end-to-end encrypted relays for OCapN

Primary source: Tallon, "End-to-end encrypted relays for OCapN,"
2026-10-15
([article](https://www.spritely.institute/news/end-to-end-encrypted-relays-for-ocapn.html)),
plus the Goblins 0.12.0 prelay announcement (#48), the Goblins manual's
netlayer pages as summarised in `ocap-family.md`, and the Polydraw
devlog (#1) as the field report.

### 6.1 What Spritely means by it

A **relay** in Spritely's usage is a public-internet server that proxies
messages between two peers that cannot connect directly — because of
home NATs, carrier-grade NATs that defeat port forwarding and UPnP, or
browser sandboxes where listening is impossible. Hole punching is
acknowledged and dismissed as "far from guaranteed"; the relay is the
general-case fallback, and Spritely is blunt that relays are
"unsatisfying compared to making direct connections" but necessary.

The history matters. Goblins 0.12 (January 2024) shipped **prelay**, a
*federated* relay that was deliberately named "pre-" because, although
the peer↔relay legs were encrypted in transit, the relay could read and
modify the CapTP traffic passing through it — it was unauthenticated
and not end-to-end encrypted, and Spritely said so in the manual and
the release post. The October 2026 announcement is the fix: a relay
that is **federated, stateless, and end-to-end encrypted**, funded by
an NLnet grant, landing in the next Goblins release after 0.18.0, with
an OCapN specification so other implementations can build compatible
relays, relay test-suite support, and a Guix service for running one.

So "E2EE relays for OCap" means, precisely: **the CapTP session between
the two peers is encrypted and authenticated peer-to-peer (Noise), and
the relay — even relaying between two different relay servers — handles
only ciphertext it cannot read, modify undetectably, or replay.**

### 6.2 What the design provides

- **Reachability in the general case.** Any two peers with outbound
  internet can meet, regardless of NAT/CGNAT, and browser (Hoot) peers
  — which cannot listen at all — become full OCapN citizens. This is
  the property that made the relay "a critical part of the OCapN story"
  once Hoot existed.
- **Federation instead of a platform.** A relay server exposes two
  actors: a **Controller** (a client steers its own connections) and an
  **Endpoint** (named in the peer's locator, publicly enlivenable).
  Two peers homed on *different* relays still connect: the initiating
  relay enlivens the remote relay's Endpoint, the two relays spawn
  paired forwarding objects (`rarelay`/`rbrelay`), and the destination
  relay calls the destination peer's registered incoming object. No
  shared operator, no central directory — the relay-to-relay hop is
  itself just OCapN.
- **Statelessness as an anti-lock-in guarantee.** The relay stores no
  user data or configuration; its only job is forwarding live messages.
  Spritely draws the ActivityPub lesson explicitly: an instance holds
  your data and acts for you, so leaving is costly; a relay holds
  nothing, so switching relays means pointing your sturdyrefs at a new
  one. Relays are meant to be cheap and disposable — the grant includes
  a Guix service precisely so many small operators can run them on "the
  cheapest VPS or old computers."
- **End-to-end security properties.** Via Noise (below): confidentiality
  and integrity against the relay and any intermediary, **man-in-the-
  middle resistance** (peer authentication against the locator's
  designator key), **replay protection**, and **perfect forward
  secrecy** from per-session ephemeral keys.
- **Composability.** The encryption is not relay-specific: Noise is a
  *meta-netlayer* that wraps any other netlayer, so the same E2EE +
  mutual authentication can be layered over TCP+TLS or even Tor
  (which authenticates only the outbound direction to an onion service;
  Noise adds bidirectional authentication).

**What the relay can and cannot see.** It cannot read message contents,
cannot undetectably modify them, and cannot replay them. It necessarily
*can* see connection metadata: which Endpoint/designator is being
contacted, when, how much traffic, and the connecting peers' network
addresses on their home legs (and, in federation, the other relay).
Spritely claims **no anonymity property** for the relay — that is Tor's
job in their stack, and the relay is positioned as the low-latency,
low-trust complement to the onion netlayer, not a replacement for it.
It can also deny service (drop/delay), as any relay can; federation +
statelessness are the mitigation (use another relay), not a protocol
guarantee.

### 6.3 What Guile Noise is

**Guile Noise** is Spritely's from-scratch implementation of the
[Noise Protocol Framework](https://noiseprotocol.org) for Guile Scheme,
written by Jessica Tallon because no Guile implementation existed. (A
Dart implementation by Ridley of DObjects followed the same path for
the same reason — a sign of how the OCapN ecosystem grows: each
implementation writes its own Noise.)

- **What it implements.** The Noise state machines (CipherState,
  SymmetricState, HandshakeState) and **all 12 fundamental interactive
  handshake patterns**. Supported primitives: X25519 key agreement,
  AES-GCM ciphers, SHA-256 and SHA-512 hashes — chosen because they are
  available in both target environments: **GnuTLS** on the Guile VM and
  **WebCrypto SubtleCrypto** under Hoot/in the browser. The crypto
  backend is pluggable.
- **How OCapN uses it.** The relay/Noise netlayer uses the **XK**
  pattern: the responder's static public key is known to the initiator
  in advance — it *is* the designator in the peer's OCapN locator — and
  the initiator transmits its own static key during the handshake, so
  both sides end up mutually authenticated. XK's three-message
  handshake mixes ephemeral and static DH operations to produce two
  CipherStates (one per direction) with forward secrecy.
- **Verification/maturity.** The library is validated against the
  official Noise test vectors for every pattern/cipher it supports,
  with the vector runner executed on **both** backends (Guile VM and
  Hoot). It is new (announced October 2026, shipping with the relay in
  the next Goblins release) and general-purpose — usable independently
  of Goblins — but it has not yet had a numbered-release soak, and
  Spritely is candid that none of them are cryptographers: the design
  deliberately stands on Noise (the framework behind WireGuard and
  WhatsApp's transport) rather than inventing a protocol. Judge it as:
  a careful, test-vector-checked implementation of a well-reviewed
  framework, by a small team, at version-zero maturity.

### 6.4 How the relay actually works, end to end

1. Each peer registers an *incoming* object with its home relay's
   Controller. Its sturdyrefs name the relay's **Endpoint** object plus
   its Noise static key (the locator designator).
2. To connect, the initiator's CapTP enlivening asks its relay's
   Controller to open a connection to the target sturdyref. The
   Controller extracts the remote Endpoint, enlivens it (connecting
   relay-to-relay if the peers are on different relays), and spawns a
   forwarding object for the session.
3. The remote relay calls the target peer's registered incoming object,
   handing it the paired forwarding object; both sides now hold a
   message pipe through one or two relays.
4. Over that pipe, the peers run the **Noise XK handshake directly with
   each other** — the relay forwards handshake bytes it cannot usefully
   read (the responder's key is checked against the locator; the
   initiator's static key arrives encrypted under keys derived from
   the responder's). A **bootstrap inbox/outbox exchange** inside the
   Noise netlayer then binds the encrypted channel to CapTP: each side
   delivers its outbox to the bootstrap object named in the locator,
   receives the peer's outbox, and thereafter sends CapTP messages as
   Noise-encrypted payloads.
5. CapTP runs unchanged on top — including **third-party handoffs**:
   when A introduces B to C mid-session, B and C establish their own
   Noise session (over relays if needed) using handoff certificates
   signed with session keys, so the introduction is secure even though
   the original channel's messages transited relays. Polydraw (#1) is
   the first shipped application to do handoffs over this netlayer —
   and its devlog's "crossed hellos" bug (two simultaneous connections,
   one must win, promises on the loser hang) is the honest footnote:
   OCapN's **at-most-one-connection** invariant is load-bearing for
   object identity, and the relay path is where it gets exercised.

### 6.5 Comparisons: where each relay/rendezvous design puts trust

| System | What the relay/route is | Can it read content? | Metadata it learns | Who runs it / federation | Anonymity aim |
|---|---|---|---|---|---|
| **Spritely E2EE relay (2026)** | Federated, stateless OCapN forwarder; Noise XK between peers | No — ciphertext only; modification/replay prevented | Peer designators/endpoints contacted, timing, volume, client IPs on home legs | Anyone; relays interoperate via OCapN itself; Guix service planned | None claimed |
| **Spritely prelay (2024)** | Same federation shape, no E2EE | **Yes — could snoop and modify** (documented) | Everything above plus plaintext | Anyone | None |
| **iroh relay** | Home-relay + fallback relay for QUIC endpoints; hole punching attempted first, relay is fallback | No — payload is QUIC/TLS 1.3 between endpoints | EndpointIds of both sides, IPs, timing, volume | n0's public relays or self-run; per-endpoint home relay | None claimed (endpoint IDs are public keys, linkable) |
| **Tor onion services** | Rendezvous over 6-hop circuits via introduction/rendezvous points | No — onion encryption end to end | Rendezvous point sees a rendezvous happening, not the parties' identities (by design) | Volunteer relay network; rendezvous points are chosen relays | **Yes — core goal**, at high latency cost |
| **Veilid** | Safety routes (relay hops, DHT-mediated) and private routes (peer-provided route blobs) | No — route traffic is encrypted to the destination | Compiled-route relays see fragments; design minimises any single node's knowledge | Any node in the public DHT network | **Yes** — route privacy is the product |
| **Nostr relays** | Store-and-forward servers holding signed events | Public events: yes, plaintext by design; DMs: NIP-44 ciphertext, but relay stores them | Nearly everything: authors, recipients (gift-wrap mitigates), timing, social graph | Anyone; users multi-home across relays | No (pseudonymity only) |
| **Matrix federation** | Homeservers that host rooms and replicate room state | E2EE rooms: content no; but the server holds room state, membership, and metadata, and *is* the user's home | Extensive — it is the user's server, not a transit hop | Federation of homeservers; users are bound to their server's data custody | No |
| **TURN (WebRTC)** | Generic media/data relay with credentials | No for DTLS-SRTP media (E2E between peers) | Both IPs, ports, timing, volume; allocation credentials identify the user to the operator | Usually the app operator's own servers; not meaningfully federated | No |

Three contrasts Drew should take away:

1. **Spritely's relay is closest to iroh's**, in shape and in honesty:
   both are "dumb, replaceable pipes under endpoint-key cryptography,
   run by anyone, leaking connection metadata, claiming no anonymity."
   The differences are in what rides on top and how federation works —
   iroh relays are located via DNS/home-relay config and forward QUIC
   between EndpointIds, with hole punching as the preferred path and
   the relay as fallback; Spritely's relays are *addressed as OCapN
   objects* and federate by speaking OCapN to each other, and the
   relay is a first-class netlayer rather than a fallback (there is no
   hole-punching attempt in the Goblins relay path; the libp2p netlayer
   is where Goblins gets hole punching). Spritely also composes
   encryption as a separate meta-netlayer usable over *any* transport,
   where iroh bakes TLS into QUIC.
2. **Against Tor and Veilid, the Spritely relay is deliberately
   weaker on privacy and stronger on simplicity/latency.** Tor and
   Veilid spend hops and complexity to hide *who is talking to whom*
   from the infrastructure; Spritely's relay hides only *what is said*.
   In Spritely's own stack those are different netlayers for different
   threat models (onion for anonymity, relay for reachability), which
   is the right way to read the design: the relay does not compete with
   Tor, it complements it.
3. **Against Nostr/Matrix, the contrast is custody.** Nostr relays
   store your events; Matrix servers store your rooms and *are* your
   identity's home. Spritely's relay stores nothing and knows no
   accounts — the ActivityPub lesson applied to infrastructure. The
   cost is the mirror image: no store-and-forward, so an offline peer
   is simply unreachable until it reconnects (the same liveness limit
   as all of CapTP; see `ocap-family.md` §2).

### 6.6 Fit with the OCapN netlayer contract

The relay work is best understood as the netlayer contract being kept
under hostile conditions. The contract (Netlayers draft; see
`ocap-family.md` §5) requires a netlayer to provide a bidirectional,
ordered, secure channel between two designator-identified peers, with
at most one live connection per peer pair, so that CapTP above it can
assume authenticated, non-replayable delivery and stable object
identity. Prelay violated the "secure" clause in spirit (transport
encryption but a trusted middle); the Noise composition restores it
*without changing CapTP or the relay's federation machinery* — the
meta-netlayer pattern (a netlayer defined over another netlayer) is
doing the architectural work. Sturdyrefs survive unchanged: the
locator gains a Noise-key-bearing designator and a relay Endpoint, and
enlivening produces the same live references whether the path was
direct TCP+TLS, Tor, or two federated relays. Third-party handoffs —
the contract's hardest clause — are demonstrated working over the
relay by Polydraw, with the crossed-hellos race as the known wart:
when two peers connect simultaneously, one connection must be chosen
and the loser's promises currently hang rather than break cleanly;
Spritely shipped a timeout workaround in the game and flags a Goblins
fix as future work.

**Estate note (one paragraph).** For our own transport programme, the
transferable ideas are: (a) encryption as a *composable layer* keyed by
the locator's designator, separable from the forwarding mechanism;
(b) statelessness as an explicit anti-custody design goal for relay
infrastructure, learned from federation's mistakes; (c) relays
addressed and federated *in the same capability system they serve*,
rather than as external configured infrastructure; and (d) the honest
metadata posture — Spritely's relay, like iroh's, does not pretend to
anonymity, and says what the relay can see. The non-transferable part
is the substrate: this is a Scheme/Guile implementation of a
pre-standardisation protocol, at first-release maturity, with no Rust
presence (see `ocap-family.md` §7).
