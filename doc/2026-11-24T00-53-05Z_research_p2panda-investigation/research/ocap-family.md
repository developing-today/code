# The object-capability network family: CapTP, OCapN, Endo, Spritely Goblins, Agoric

Research lane OCAP, for the auth + transport investigation (tracking issue
developing-today/code#13779). Written 2026-11-23 from primary sources —
project specs, docs, repos, and release notes; every factual claim is linked.
Secondary summaries are flagged where used.

**One-sentence verdict:** this family is the most completely realised design
for *live, delegated authority between mutually suspicious programs* that
exists anywhere — and it is a **distributed-objects** system, not a data
replication system. It has no answer to "how do two offline nodes converge on
shared state," because that is not the problem it solves. For this estate
(Rust / SQLite-Turso / Cloudflare / iroh) it is a poor base and a rich
idea-mine; §8 lists exactly what to borrow.

---

## 1. What each piece IS, and which layer it sits at

The five names are not five alternatives. They are one lineage at five
different layers, and most confusion about this family comes from treating
them as competitors.

### 1.1 CapTP — the protocol (the idea layer)

CapTP, the Capability Transport Protocol, is an abstract protocol for
message passing between objects on different machines in which **holding a
reference *is* the authority to invoke**: "an object can use another object
(capability) if and only if it possesses a reference to it — if you don't
have it, you can't use it"
([OCapN CapTP draft specification](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
Its defining features, per the same draft:

- **Distributed object references.** Within a pairwise session, objects are
  exported/imported by position; a message can carry references as arguments,
  so the reference graph and the access graph are the same graph.
- **Promises and promise pipelining.** A message send creates a promise for
  the result; further messages can be sent *to the promise* before it
  resolves and are delivered to its eventual resolution, collapsing what
  would be N round trips into one message flight (the draft's car-factory
  example: 6 sequential steps become 4 messages sent without waiting)
  ([CapTP draft, Promises](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
  This comes straight from Mark S. Miller's dissertation work on the E
  language, which the OCapN README cites as the explanation of pipelining's
  value ([ocapn/ocapn README](https://github.com/ocapn/ocapn)).
- **Distributed (acyclic) garbage collection.** Peers cooperatively inform
  each other when a cross-session reference is no longer needed
  (`op:gc-exports` / `op:gc-answers`); acyclic-only because full distributed
  cyclic GC "requires special hooks into the garbage collector, whereas
  distributed acyclic GC merely requires weakrefs, weakmaps, and finalizers"
  ([ocapn/ocapn README](https://github.com/ocapn/ocapn)).
- **Third-party handoffs.** When a message in session A↔B carries a reference
  to an object that lives in session A↔C, a handoff protocol (roles: Gifter,
  Receiver, Exporter) lets B connect *directly* to C with the same authority,
  "in a secure fashion, even if the messages are viewed by a malicious actor"
  — i.e. introductions without proxying and without trusting the introducer
  with the traffic afterwards
  ([CapTP draft, Third party handoffs](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
- **Error propagation.** Broken promises propagate across the network;
  partition breaks promises, which is how failure is represented.

Lineage: CapTP was first openly implemented in the **E programming language**
(itself a continuation of Electric Communities Habitat); "CapTP is something
which has been implemented many times" since, incompatibly — in E, in Cap'n
Proto, in Agoric's implementation, and in Spritely's Goblins — and OCapN
exists precisely because those implementations could not talk to each other
([ocapn/ocapn README](https://github.com/ocapn/ocapn)).

### 1.2 OCapN — the interop standardisation effort (the spec layer)

OCapN (Object Capability Network) is the pre-standardisation effort to make
CapTP implementations interoperable. It is a suite of draft specifications —
**CapTP**, **Netlayers**, **Locators**, plus a data Model and Notation doc —
in the [ocapn/ocapn](https://github.com/ocapn/ocapn/tree/main/draft-specifications)
repo, with Syrup as the wire serialisation for messages crossing a CapTP
boundary ([CapTP draft, Additional Documents](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).

- **Status, in its own words: "OCapN is still pre-specification."** The plan
  is to first get Agoric's and Spritely's implementations interoperable,
  *then* consider Cap'n Proto, "before beginning any discussion of
  standardization" (i.e. no IETF/W3C submission yet)
  ([ocapn/ocapn README](https://github.com/ocapn/ocapn)). The CapTP draft
  itself warns it is "likely to undergo significant change" and is based on
  Spritely Goblins' implementation as the starting point
  ([CapTP draft, header](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
- **Participants:** Spritely Institute (primary driver, spec editor),
  Agoric, Cap'n Proto (advisory), MetaMask (review)
  ([search-surfaced DeepWiki summary of ocapn meeting minutes, 2023-04-25](https://deepwiki.com/ocapn/ocapn/9.1-participating-organizations) — secondary).
- **Netlayers** are OCapN's transport abstraction: CapTP runs over any
  netlayer providing a bidirectional, ordered, secure channel; the draft
  names Tor onion services as the specified one, with TCP/TLS in development
  at draft time and libp2p / IBC / I2P named as possible
  ([Netlayers draft, via DeepWiki](https://deepwiki.com/ocapn/ocapn/6.1-netlayers-specification) — secondary summary of the draft).
- **Locators** define the `ocapn://` URI structure for machines and objects:
  a peer designator (netlayer-specific, self-authenticating), a transport
  name, routing hints, and for objects a *swiss number* — an unguessable
  random nonce assigned at registration, which is itself the capability
  ([Goblins manual, Using the CapTP API](https://files.spritely.institute/docs/guile-goblins/latest/Using-the-CapTP-API.html);
  locator components as described in
  [Spritely, End-to-end encrypted relays for OCapN](https://www.spritely.institute/news/end-to-end-encrypted-relays-for-ocapn.html)).
- There is a shared **test suite** ([ocapn/ocapn-test-suite](https://github.com/ocapn/ocapn-test-suite))
  that implementations run against a reference sturdyref-exporting peer —
  the mechanism by which interop is actually checked (§5).

### 1.3 Endo — the JavaScript ocap platform (the runtime layer)

Endo (by Agoric, Apache-2.0) is a framework for ocap programming in
JavaScript: "tools for *confinement*, *communication*, and *concurrency*"
([Endo getting started](https://endojs.org/getting-started/)). Its parts:

- **SES / Hardened JavaScript** — a shim that locks down JS intrinsics and
  provides Compartments, so mutually suspicious code can share a process.
  This is the piece with real production mileage: "Agoric and MetaMask rely
  on Hardened JavaScript and the SES shim" for sandboxing third-party
  plugins, and Salesforce's Lightning framework is a SES user
  ([Endo getting started](https://endojs.org/getting-started/);
  [Agoric platform paper](https://papers.agoric.com/documentation/platform/)).
- **Eventual send (`E()`)** — the programming surface that stretches async
  method invocation over processes and networks via handled promises
  ([Endo docs, Distributed Programming](https://docs.endojs.org/documents/get-started.html)).
- **`@endo/captp`** — Agoric's *production* CapTP, deliberately "a minimal
  CapTP implementation": JSON-able messages over a connection object you
  supply yourself; it is transport-agnostic because it doesn't do transport
  at all ([@endo/captp README](https://raw.githubusercontent.com/endojs/endo/master/packages/captp/README.md)).
  Historically it lacked the full OCapN feature set — the 2020 interop
  discussion lists distributed GC emission and handoff tables as things
  *both* sides still needed to build
  ([Agoric/agoric-sdk#1827](https://github.com/Agoric/agoric-sdk/issues/1827)).
- **`@endo/ocapn`** — the *new* package implementing OCapN CapTP proper
  (Syrup encoding, GC operations, swiss numbers), with netlayers for
  WebSocket and a TCP testing netlayer; released at 1.1.0 and synced against
  the OCapN test suite
  ([Endo release PR #3209, @endo/ocapn 1.1.0 notes](https://github.com/endojs/endo/pull/3209)).
- **The Pet Daemon (`@endo/daemon`, `@endo/cli`)** — "a persistent host for
  managing guest programs in hardened JavaScript worker processes,"
  communicating with its controller in CapTP over netstring envelopes on a
  Unix domain socket / named pipe, with per-user storage
  ([@endo/daemon README](https://raw.githubusercontent.com/endojs/endo/master/packages/daemon/README.md)).
  On top of it sits the **petname system**: each agent keeps its own
  namespace of human-meaningful names for capabilities ("@documents/work",
  "@payment/groceries"); you grant, inspect, and revoke by name rather than
  by hash or API key
  ([Endo, Review: Sandboxes Won't Save You From OpenClaw](https://endojs.org/review-sandboxes-wont-save-you-from-openclaw/);
  [DCF, Containing AI Agents: The Endo Familiar Demo](https://dcfoundation.io/containing-ai-agents-the-endo-familiar-demo/)).
  Revocation is mechanical, not advisory: a daemon "Reaper" change makes
  deleting every petname that retains a value collect its underlying
  *formula*, cancel the live value, and sever the CapTP connections and
  workers still holding it
  ([endojs/endo PR #3087](https://github.com/endojs/endo/pull/3087)).
  Endo is actively pitching this daemon as the containment layer for
  AI-written code, with capability requests mediated by the user
  ([endojs/endo#3019](https://github.com/endojs/endo/issues/3019)).

### 1.4 Spritely Goblins — the distributed object environment (the other runtime)

Goblins is Spritely Institute's "distributed object programming environment"
(Apache-2.0), implemented twice: **Guile Scheme** (the lead implementation;
"most new features are aimed at Guile Goblins") and **Racket Scheme**
(network-compatible, feature-lagging)
([Goblins v0.12.0 release post](https://community.spritely.institute/t/blog-post-spritely-goblins-v0-12-0-released-two-new-netlayers-join-the-ocapn-family-and-more/396)).
Current release: **v0.18.0** (April 2026)
([Spritely news](https://www.spritely.institute/news/spritely-goblins-v0-18-0-sleepy-actors.html)).
Spritely's own maturity label for Goblins is "Working Beta"
([spritelyproject.org](https://spritelyproject.org/) — search-surfaced).

What Goblins adds on top of CapTP:

- **Vats and transactional turns.** Objects live in vats (units of
  synchrony); each turn is a cheap local transaction that rolls back on
  error, which is also what enables its time-travel debugging (secondary
  summary of the Spritely whitepaper:
  [The Heart of Spritely](https://agent-comms.anuna.io/papers/ocap/the-heart-of-spritely---distributed-objects-and-capability-security/)).
- **The most complete CapTP in the family** — the OCapN README states
  Spritely "has the most pieces of CapTP (including distributed acyclic GC,
  handoffs) and the start of the netlayer abstractions and URI concepts"
  ([ocapn/ocapn README](https://github.com/ocapn/ocapn)).
- **Aurie / the persistence system.** Persistent vats serialise their object
  graph (in Syrup) to a store and restore it later, with upgrade/migration
  support; v0.17.0 added **Bloblin**, a streaming binary-log store writing
  compressed deltas per vat churn ("many thousands of deltas per second"),
  replacing whole-graph rewrites
  ([Goblins v0.17.0 release post](https://www.spritely.institute/news/spritely-goblins-v0-17-0-persistence-is-better-than-ever.html);
  [Goblins manual, Persistence stores](https://spritely.institute/files/docs/guile-goblins/latest/Persistence-stores.html)).
  v0.18.0 added "sleepy actors," a caching layer that lets persisted actors
  be evicted and rehydrated on demand
  ([Goblins v0.18.0 release post](https://www.spritely.institute/news/spritely-goblins-v0-18-0-sleepy-actors.html)).
- **Sealers/unsealers** — the standard ocap pattern for tamper-evident
  envelopes and rights amplification, used throughout the actor library
  ([Goblins garden tutorial, community post](https://community.spritely.institute/t/growing-a-networked-garden-with-spritely-goblins/195)).
- **Netlayers shipped in Guile Goblins:** Tor onion services; TCP+TLS;
  Prelay (relay, see §4); libp2p (added v0.14.0, Sept 2024 —
  [release announcement](https://www.mail-archive.com/guile-user@gnu.org/msg15207.html));
  WebSocket (exists — v0.18.0 fixed a signing-oracle vulnerability in its
  designator authentication
  ([v0.18.0 release post](https://www.spritely.institute/news/spritely-goblins-v0-18-0-sleepy-actors.html))).
  No dedicated WebRTC netlayer has shipped; WebRTC appears only as community
  discussion alongside the libp2p work
  ([community forum index](https://community.spritely.institute/c/architecture/goblins/48) — search-surfaced).
- **Hoot** compiles Guile (and thus Goblins) to WebAssembly; Goblins has run
  in the browser since v0.15.0, and OCapN-over-Hoot issues were fixed in
  v0.16.1 ([Spritely news archive](https://www.spritely.institute/news/)).

### 1.5 Agoric — the chain (the application/platform layer)

Agoric is a Cosmos-SDK blockchain whose smart contracts are Hardened
JavaScript objects, built on **SwingSet**, a vat kernel: a vat is "a unit of
synchrony," vats communicate only asynchronously, vat state is orthogonally
persistent, and a special *comms vat* speaks CapTP to the outside world —
to other SwingSets, to solo machines (ag-solo), and over **IBC** to other
chains, so that a blockchain can be treated as "an ordinary machine
supporting distributed objects on the network"
([Agoric platform paper](https://papers.agoric.com/documentation/platform/);
[ocapn/ocapn README](https://github.com/ocapn/ocapn);
[Agoric/agoric-sdk#1827](https://github.com/Agoric/agoric-sdk/issues/1827)).
On top sit **ERTP** (transferable rights as objects) and **Zoe** (offer
safety for contracts) — the family's worked example of ocap *economics*.

For a non-chain user, Agoric-the-chain is mostly irrelevant; what transfers
is (a) Endo/SES, which Agoric maintains and which runs in production at
MetaMask scale, (b) the SwingSet demonstration that CapTP semantics survive
even a store-and-forward, consensus-ordered "netlayer" (transactions as
transport), and (c) ERTP/Zoe's patterns for rights-as-objects. Agoric's own
engineering talent is historically central to CapTP — the OCapN README notes
Agoric "has most of the engineering talent who have implemented CapTP
historically" ([ocapn/ocapn README](https://github.com/ocapn/ocapn)).

---

## 2. The conceptual core: distributed objects ≠ data replication

This is the contrast the whole investigation turns on, so it is worth being
precise.

**The ocap model answers:** *"Program A holds authority over a live service
run by program B. How does A invoke it, delegate part of that authority to
C, attenuate it, or take it back — with the guarantees enforced by the
reference graph itself rather than by an ACL checked at a boundary?"*
State lives **in exactly one place**: inside the object, on its home
machine, behind its methods. Other parties hold *presences* (remote
references), never copies. Consistency is therefore trivial — there is one
object, one turn order, one truth — and "conflict resolution" is a category
that does not exist. What exists instead is *liveness*: if the home machine
is offline or the session breaks, every promise to it breaks, and the
authority is simply unavailable until a sturdyref is re-enlivened over a new
session.

**The log/CRDT model (p2panda, Willow, Hypercore, …) answers the opposite
question:** *"Many nodes each hold a copy of shared data and edit it while
disconnected. How do the copies converge, and who was allowed to write?"*
Authority there is about *past writes* (signed operations, capability
tokens checked at ingest), and the system's centre of gravity is durable,
replicated, mergeable state.

Consequences, stated as wins and losses for the ocap side:

**Where live references win:**
- **Real-time authority and immediate revocation.** Revoking in CapTP is a
  local act with immediate effect: interpose a forwarding "caretaker" object
  and flip it, or (Endo daemon) delete the petname and the formula
  collection cancels the value and severs the connection
  ([endojs/endo PR #3087](https://github.com/endojs/endo/pull/3087)).
  No revocation list has to propagate; there is nothing to propagate *to* —
  every call goes to the live object. Compare any token/log system, where a
  revoked credential remains valid everywhere until expiry or until a CRL /
  epoch update reaches every verifier.
- **Attenuation is just programming.** A restricted facet is an ordinary
  object that forwards a subset of methods (read-only directory, spend-limit
  payment source). It cannot be bypassed, because there is no other path to
  the underlying object — authority only exists as references. Token systems
  (UCAN/Biscuit/macaroons) attenuate *claims*; ocap attenuates *behaviour*,
  including stateful, rate-limiting, revocable behaviour.
- **Delegation without identity infrastructure.** A third-party handoff
  introduces two peers directly, with certificates that keep the handoff
  secure "even when CapTP messages are not kept secret"
  ([CapTP draft](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
  No directory, no token exchange, no shared namespace is needed for A to
  give C exactly the reference A holds.
- **Latency shape.** Promise pipelining removes round trips from chained
  calls, which is the dominant cost in chatty distributed-object workloads
  ([CapTP draft, Promises](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
- **Confused-deputy resistance by construction.** Because designation and
  authority travel together (the reference names *and* empowers), the whole
  class of "tricked into using its own authority on the wrong object" bugs
  that ACL systems suffer is designed out — this is the family's founding
  argument, restated in the Spritely whitepaper (secondary summary:
  [The Heart of Spritely](https://agent-comms.anuna.io/papers/ocap/the-heart-of-spritely---distributed-objects-and-capability-security/)).

**Where live references lose:**
- **Offline-first anything.** A capability to an offline object is a promise
  that breaks. There is no store-and-forward in the core model, no
  operation log to replay, no merge. (Agoric's chain use is the exception
  that proves the rule: it works because the *chain itself* is the always-on
  home of the objects, with consensus as the netlayer.)
- **Durable shared state.** Nothing in CapTP replicates data. Goblins'
  answer is orthogonal persistence of *one node's own* object graph
  (Aurie/Bloblin) — a checkpoint log, not replication; two nodes never hold
  convergent copies of the same object
  ([Goblins v0.17.0](https://www.spritely.institute/news/spritely-goblins-v0-17-0-persistence-is-better-than-ever.html)).
- **Many-reader / public-data workloads.** Content distribution, feeds,
  shared documents, audit logs — the bread and butter of the log families —
  are simply not the ocap shape; you'd end up building a replication system
  *inside* objects, badly.
- **Availability coupling.** Every holder's liveness depends on the object's
  home being up and reachable. NATs, sleep, and churn are protocol-level
  problems (hence the relay work in §4), where a sync system treats them as
  background conditions.

**Neither model subsumes the other.** A composed estate — replicated log for
shared durable state, live references for control-plane authority — is not a
compromise; it is each model doing the job it is shaped for. This matters
for §7.

---

## 3. Axes: identity & key management

- **Peer identity is a keypair, scoped by netlayer.** In OCapN, a locator's
  designator is "self-authenticating identity (usually a public key)"
  ([Spritely, E2EE relays](https://www.spritely.institute/news/end-to-end-encrypted-relays-for-ocapn.html)).
  Concretely: on the Tor netlayer the designator *is* the onion address
  (an ed25519 public key); on TCP+TLS, "sturdyrefs for this netlayer use a
  SHA-256D hash of the TLS certificate as the peer identifier," with address
  hints carried alongside
  ([Goblins manual, TCP+TLS](https://files.spritely.institute/docs/guile-goblins/latest/TCP-_002b-TLS.html)).
- **Sessions have their own keys, separate from peer identity.** The OCapN
  CapTP draft requires each side to create a *per-session* Ed25519 keypair
  at `op:start-session`, used to sign handoff structures; a peer's public
  identifier within the session is the double-SHA256 of its session key
  ([CapTP draft, Cryptography](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
  So there are two identity tiers: long-lived netlayer identity (how you
  *reach* me) and ephemeral session identity (how handoffs *authenticate*).
- **Objects have no identity separate from their references.** A registered
  object gets a swiss number — a large random nonce — inside its sturdyref;
  knowing the sturdyref *is* the capability to enliven it
  ([Goblins manual, CapTP API](https://files.spritely.institute/docs/guile-goblins/latest/Using-the-CapTP-API.html)).
  There is no global object namespace and no lookup without a reference.
- **Humans name things locally.** Petnames (Endo daemon; Spritely's Brux
  project) are per-user, non-global names bound to sturdyrefs/references —
  Zooko's-triangle-aware: human-meaningful *and* secure because they are
  local mappings onto unguessable identifiers, never a global registry
  ([kriskowal.com, Endo](https://kriskowal.com/endo/);
  [Endo review](https://endojs.org/review-sandboxes-wont-save-you-from-openclaw/)).
- **Key management story is thin.** Keys are generated by the netlayer
  (self-signed TLS certs auto-generated in Goblins' TCP+TLS netlayer) or by
  Tor; there is no PKI, no rotation protocol, no recovery story, no
  multi-device identity in the specs. Persistence of netlayer secrets is
  handled by the vat persistence system ("Goblins now handles storing and
  providing the secrets… a netlayer might need to persist")
  ([v0.14.0 announcement](https://www.mail-archive.com/guile-user@gnu.org/msg15207.html)).
  Groups, as an identity concept, **do not exist** in this family — see §4.

## 4. Axes: capability model — delegation, attenuation, revocation, groups

- **Delegation** is reference-passing in a message, plus third-party
  handoffs when the reference crosses sessions (§1.1). Delegation is exact:
  you can only ever give what you hold, and giving a facet gives only the
  facet.
- **Attenuation** is object composition: membranes, caretakers, read-only
  facets, sealers. It is Turing-complete policy (stateful, contextual,
  revocable), not a caveat list. This is the single largest capability-model
  gap between this family and every token system in the investigation.
- **Revocation** has three mechanisms, all live: (1) don't hand out the
  object, hand out a forwarder you control, then disable the forwarder;
  (2) session severance — `op:abort` ends a session and breaks its promises
  ([CapTP draft](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md));
  (3) retention-based — Endo's daemon collects a value's formula when its
  last petname goes, cancelling the value and disconnecting retainers
  ([endojs/endo PR #3087](https://github.com/endojs/endo/pull/3087)).
  There is deliberately **no expiry** in the core model: a reference is
  valid until revoked or its object dies. (Token-style expiry can be built
  as a facet; the reverse — live revocation on top of bearer tokens — is
  the thing that cannot be retrofitted.)
- **Groups: nothing.** No group abstraction, no roles, no membership
  protocol exists in CapTP/OCapN/Endo/Goblins. A "group" would be an
  ordinary object someone writes (e.g. an object holding references to
  members' objects and forwarding). Anyone whose auth problem is *primarily*
  group membership over shared data (the p2panda-auth / Keyhive shape of
  problem) gets no leverage here.

## 5. Axes: encryption, transport & NAT traversal, spec/interop status

**Encryption.** CapTP itself does no encryption; confidentiality and
channel authentication are *netlayer* requirements (a netlayer must provide
a secure, ordered, bidirectional channel where only the two participants
can insert messages —
[Netlayers draft, via DeepWiki](https://deepwiki.com/ocapn/ocapn/6.1-netlayers-specification), secondary).
In practice: Tor netlayer inherits Tor's encryption and anonymity; TCP+TLS
uses TLS with self-signed certs whose hash is the peer identity (so the
channel is authenticated by construction, no CA)
([Goblins manual, TCP+TLS](https://files.spritely.institute/docs/guile-goblins/latest/TCP-_002b-TLS.html)).
Handoff security does *not* depend on channel secrecy — handoff certificates
are signed with session keys so they stay safe "even when CapTP messages are
not kept secret"
([CapTP draft](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
There is **no end-to-end encryption of object state at rest** in this
family (that is what Spritely's separate Porta/Bella work and the other
families' document encryption address), and no group encryption at all.

**Transport & NAT.** The honest summary: this is the family's weakest axis
relative to the iroh/libp2p world.

- Tor onion services are the flagship netlayer precisely because onion
  services solve NAT by rendezvous — at the cost of high latency; the
  Goblins manual itself steers NAT'd TCP+TLS users to Tor, noting TCP+TLS
  has "lack of both 'store and forward' and NAT hole punching support" and
  otherwise needs manual port forwarding
  ([Goblins manual, TCP+TLS](https://files.spritely.institute/docs/guile-goblins/latest/TCP-_002b-TLS.html)).
- **Prelay** (v0.12.0) is a federated relay netlayer layered over other
  netlayers — but v0 forwards messages **in cleartext through the relay**,
  "unauthenticated, which means the relay has the power to both snoop on and
  modify messages"; it was named *pre*-relay to discourage reliance on it
  ([Goblins manual, Prelay](https://files.spritely.institute/docs/guile-goblins/latest/Prelay.html)).
- The fix is recent and not yet in a numbered release as of writing: an
  NLnet-funded **Noise-protocol netlayer** that composes with Prelay to make
  a federated, end-to-end-encrypted relay with perfect forward secrecy,
  announced October 2026 for "the next release of Goblins," with an OCapN
  spec for the relay to follow, and already dogfooded in a shipped p2p game
  (Polydraw, Autumn Lisp Game Jam 2026)
  ([Spritely, E2EE relays](https://www.spritely.institute/news/end-to-end-encrypted-relays-for-ocapn.html);
  [Spritely news index](https://spritely.institute/)).
- A **libp2p netlayer** exists in Guile Goblins since v0.14.0
  ([announcement](https://www.mail-archive.com/guile-user@gnu.org/msg15207.html)),
  which in principle inherits libp2p's hole punching and relay circuits,
  but it is the least exercised of the shipped netlayers (community
  questions about its status recur —
  [community forum](https://community.spritely.institute/c/architecture/goblins/48), search-surfaced).
- Endo's side is transport-minimal: `@endo/captp` runs over whatever
  message channel you hand it (WebSocket, MessagePort, …)
  ([Endo docs](https://docs.endojs.org/documents/get-started.html));
  `@endo/ocapn` ships WebSocket and TCP-testing netlayers
  ([Endo release PR #3209](https://github.com/endojs/endo/pull/3209)).
  There is no NAT story in Endo at all — the Familiar demo reaches peers
  through a WebSocket *relay* on the invite-accept model
  ([DCF, Endo Familiar demo](https://dcfoundation.io/containing-ai-agents-the-endo-familiar-demo/)).

**Spec & interop status — the question the brief asks directly: has
Goblins↔Endo OCapN interop actually been demonstrated?** Yes — but recently,
narrowly, and against a moving spec:

- 2020–2023: interop was aspirational. The Agoric↔Spritely discussion
  enumerates real divergences (serialisation: Goblins' Syrup vs Agoric's
  JSON marshal/SmallCaps; session-break semantics; handoff tables and GC
  unimplemented on the Agoric side)
  ([Agoric/agoric-sdk#1827](https://github.com/Agoric/agoric-sdk/issues/1827)).
- 2023: the demonstrated interop was *intra-family* — Guile Goblins ↔
  Racket Goblins exchanging messages over OCapN, proven with a ported chat
  app (Milestone 1 of the NLnet standardisation grant)
  ([Spritely, Introducing OCapN](https://spritely.institute/news/introducing-ocapn-interoperable-capabilities-over-the-network.html)).
- 2026: **Endo's `@endo/ocapn` runs a CI interop test against Guile
  Goblins** — a bilateral chatroom (goblin-chat) exercised in CI via
  `guix shell`, plus the shared Python test suite pinned to a Feb 2026
  commit; the package shipped at 1.1.0 with the WebSocket netlayer "used
  for interop with Guile-Goblins peers"
  ([Endo release PR #3209](https://github.com/endojs/endo/pull/3209)).
  So: two independent implementations, one application shape (chat),
  CI-checked. That is a genuine milestone and also the *entirety* of
  cross-vendor OCapN interop today.
- The spec still churns underneath implementers: Goblins v0.18.0 (April
  2026) bumped the CapTP protocol version — dropping `op:deliver-only`,
  pluralising the GC ops — and states plainly that "applications built with
  an earlier release of Goblins are incompatible with the OCapN shipped in
  this release"
  ([Goblins v0.18.0](https://www.spritely.institute/news/spritely-goblins-v0-18-0-sleepy-actors.html)).
  The FOSDEM 2026 talk likewise describes OCapN as "pre-standardization"
  with implementations in Racket, Guile, JS, and Dart
  ([FOSDEM 2026 slides](https://archive.fosdem.org/2026/events/attachments/QPUXCQ-ocapn_the_secure_decentralized_protocol_of_the_future/slides/267538/presentat_e7u2q2b.pdf)).

## 6. Axes: sync semantics, storage fit, languages, project health

- **Sync/replication semantics:** none. Sessions are live, ordered,
  pairwise; a disconnected session's ephemeral references are severed and
  its promises break (the semantics of re-establishing them via sturdyrefs
  was flagged as an unsettled design issue by both camps in
  [Agoric/agoric-sdk#1827](https://github.com/Agoric/agoric-sdk/issues/1827)).
  Offline behaviour is "unavailable," by design.
- **Storage fit (SQLite/Turso):** none natively. Goblins persistence is its
  own Syrup/Bloblin file stores (a binary delta log of the object graph);
  Endo's daemon keeps per-user formula/petname stores. Nothing in the
  family speaks SQL, and nothing would gain from Turso — persistence here
  is *object-graph checkpointing*, not tables or replicated logs
  ([Goblins manual, Persistence stores](https://spritely.institute/files/docs/guile-goblins/latest/Persistence-stores.html)).
- **Languages:** JavaScript (Endo/Agoric), Guile Scheme and Racket Scheme
  (Goblins), plus a Dart implementation reported in the FOSDEM talk
  ([slides](https://archive.fosdem.org/2026/events/attachments/QPUXCQ-ocapn_the_secure_decentralized_protocol_of_the_future/slides/267538/presentat_e7u2q2b.pdf)).
- **Rust presence: effectively zero.** The only Rust OCapN implementation
  is [cmars/ocapn-rs](https://codeberg.org/cmars/ocapn-rs) — a single-author
  learning project, self-described as "Stage 0 in progress," whose stated
  goals are understanding and eventual interop with Goblins. There is no
  Rust CapTP in the OCapN sense, no crate, no maintainer community. (The
  capability RPC that *does* exist in mature Rust form is Cap'n Proto's —
  `capnp-rpc`, a Level-1 implementation
  ([docs.rs](https://docs.rs/crate/capnp-rpc/latest)) — but that is the
  capnproto family's divergent CapTP descendant, not OCapN; see the sibling
  note.)
- **Cloudflare fit:** poor for the protocol, incidental for the runtime.
  Workers are JS/Wasm isolates: SES/Endo code can run there in principle
  and `@endo/ocapn`'s WebSocket netlayer matches Workers' WebSocket
  support, but there is no Workers-targeted CapTP anything, no Durable
  Objects integration, and the daemon/Goblins models assume long-lived
  processes with local storage — the opposite of the Workers shape. Nobody
  in this family is building for Cloudflare.
- **iroh fit:** none, and mildly competing. A netlayer over iroh is
  *conceivable* (iroh gives an authenticated, encrypted, NAT-traversing
  byte stream keyed by ed25519 NodeId — which maps cleanly onto OCapN's
  "designator = public key" locator field), but no such netlayer exists,
  and the family's own trajectory (Tor, Prelay+Noise, libp2p) is building
  its own answer to the same problem iroh already solves better.
- **Project health:** Spritely Institute is a small 501(c)(3) shipping
  steadily on NLnet grant funding (Goblins 0.18.0 April 2026, Hoot 0.10.0
  October 2026, active news cadence —
  [spritely.institute](https://spritely.institute/)); OCapN standardisation
  itself is NLnet-funded
  ([ocapn/ocapn README](https://github.com/ocapn/ocapn)). Endo is maintained
  by Agoric with MetaMask as a production SES consumer. All Apache-2.0.
  One operational note for this estate specifically: the OCapN project
  prohibits AI-generated contributions to its specs, test suite, and
  official tooling by policy
  ([ocapn/ocapn README](https://github.com/ocapn/ocapn)) — upstream
  engagement from an agent-driven fleet would have to be human-authored.
- **Operational complexity:** high for a non-Lisp/non-JS shop. Running this
  family means running a vat host (Guile or Node), a persistence store,
  netlayer infrastructure (Tor or relays), and a pre-1.0 protocol whose
  wire version has already broken compatibility once in 2026.
- **What it pointedly does NOT do:** replicate data; work offline; manage
  groups or membership; provide a global namespace or discovery; encrypt
  content at rest; run in Rust; target Cloudflare; promise a stable wire
  protocol yet.

## 7. Fit for this estate, honestly

The estate's auth/transport problems, from the master prompt, are: agent
seats on laptops/sandboxes/Actions runners needing to reach each other and
the broker; delegation and revocation of authority between agents; group-ish
access to shared replicated state (queue, atlas data); all in Rust, over
SQLite/Turso, with iroh as the emerging transport and Cloudflare doors at
the edge.

Against that shape, OCapN-as-base fails on nearly every load-bearing axis:
no Rust implementation beyond a stage-0 learning project; no replication
for the queue/atlas state that is the estate's centre of gravity; no group
model for seat/team access; NAT traversal a generation behind iroh (Tor
latency, a cleartext relay being replaced only now, an unexercised libp2p
netlayer); a wire protocol that broke compatibility within the last year;
and no Cloudflare story. Adopting it as the foundation would mean porting
CapTP to Rust *and* building the missing data layer *and* the missing
group layer — i.e. funding the family's unfinished decade with this
estate's own roadmap.

Where the fit is genuinely good is narrower and real: **the fleet's
control plane is a distributed-objects-shaped problem.** A seat holding a
live, attenuated, revocable reference to "the broker's dispatch facet for
repo X" — rather than a bearer token plus an ACL check — is exactly the
CapTP shape, and the estate already feels the token model's limits (walls,
scopes, broker-mediated everything). If the estate ever builds a live
agent-to-agent RPC layer (as opposed to queue-mediated work), CapTP's
semantics — not necessarily its wire protocol — are the right blueprint,
and Cap'n Proto's Rust RPC is the pragmatic vehicle for those semantics
(sibling lane's subject).

## 8. What to borrow even if we build elsewhere

1. **Revocation by interposition as a first-class pattern.** Never hand a
   raw authority to a delegate; hand a forwarder/caretaker object whose
   forwarding can be switched off. This works in any RPC system (including
   a Rust one over iroh) and gives immediate, propagation-free revocation —
   the property token systems structurally lack. (Pattern evidence:
   [Endo daemon Reaper](https://github.com/endojs/endo/pull/3087) shows the
   retention-based variant: authority dies when its last name does.)
2. **Petnames for agent-facing authority.** Whatever the estate's capability
   format becomes, the *human/agent-facing* layer should be local,
   meaningful names bound to unguessable identifiers — never raw hashes or
   tokens in prompts, logs, or UIs. Endo's pet daemon is the existence
   proof that this UX works for capabilities specifically
   ([Endo review](https://endojs.org/review-sandboxes-wont-save-you-from-openclaw/)).
   This aligns directly with the estate's existing "secrets by name only"
   discipline.
3. **Third-party handoff thinking for introductions.** When A introduces B
   to C, design for B↔C direct connection with the introduction
   cryptographically bound (gift certificates signed by session keys), not
   permanent proxying through A and not "send C's token to B." The CapTP
   handoff roles (Gifter/Receiver/Exporter) are the cleanest vocabulary
   available for this
   ([CapTP draft](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
   Directly relevant to the coordinator-handoff design in the fleet
   actions-exec work.
4. **Promise pipelining as an API-design rule.** Any chatty cross-machine
   API the estate designs should let callers chain operations on
   not-yet-resolved results in one flight. Even without CapTP, "send the
   plan, not the steps" is the lesson
   ([CapTP draft, Promises](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
5. **The netlayer contract as a transport-abstraction checklist.**
   OCapN's requirement set — bidirectional, ordered, session-secure,
   at-most-once-or-die, designator = self-authenticating key, hints
   separate from identity — is a sharp checklist for evaluating the
   estate's own route-form/transport candidates (iroh satisfies it almost
   by definition)
   ([Netlayers draft, via DeepWiki](https://deepwiki.com/ocapn/ocapn/6.1-netlayers-specification), secondary;
   [E2EE relays post](https://www.spritely.institute/news/end-to-end-encrypted-relays-for-ocapn.html)).
6. **SES/Hardened-JS thinking for the JS surfaces that already exist.**
   The estate runs JS at the edge (Workers, the route proxy). Where
   third-party or agent-written JS executes, Endo's confinement model —
   not its network protocol — is the mature, production-proven piece
   (MetaMask), and Endo is explicitly building bridges to AI-agent tooling
   ([endojs/endo#3019](https://github.com/endojs/endo/issues/3019)).
7. **Formula/retention GC as a lifecycle metaphor for seats and sessions.**
   Endo's daemon models every value as a *formula* whose liveness is its
   reachability from named roots; collection cancels dependents and severs
   connections. The fleet's seat/session/lease lifecycle (claims, leases,
   emergency extension) is the same shape; the vocabulary transfers even
   though the code cannot
   ([endojs/endo PR #3087](https://github.com/endojs/endo/pull/3087)).

## 9. Open questions / what would change this assessment

- A Rust OCapN implementation reaching test-suite parity (today: stage 0,
  one author — [ocapn-rs](https://codeberg.org/cmars/ocapn-rs)).
- The Noise E2EE relay landing in a numbered Goblins release *and* being
  adopted by a second implementation (Endo) — that would make OCapN's NAT
  story merely "young" instead of "behind."
- OCapN freezing a 1.0 wire version. The 2026 incompatible bump
  ([v0.18.0 notes](https://www.spritely.institute/news/spritely-goblins-v0-18-0-sleepy-actors.html))
  is the current evidence against betting a foundation on it.
- Any group-membership or replicated-state work appearing in the family —
  none is on any public roadmap found; Spritely's adjacent pieces
  (Porta/Bella storage, Brux petnames) are separate projects, not a sync
  layer.
