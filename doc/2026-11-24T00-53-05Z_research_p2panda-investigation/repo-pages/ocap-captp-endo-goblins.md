# The object-capability network family — CapTP, OCapN, Endo, Spritely Goblins, Agoric

Full lane notes: research lane OCAP for
[#13779](https://github.com/developing-today/code/issues/13779), written
2026-11-23 from project specs, docs, repos, and release notes.

## Verdict in one sentence

This family is the most completely realised design for _live, delegated
authority between mutually suspicious programs_ that exists anywhere —
and it is a **distributed-objects** system, not a data replication system.
It has no answer to "how do two offline nodes converge on shared state,"
because that is not the problem it solves. For this estate (Rust /
SQLite-Turso / Cloudflare / iroh) it is a poor base and a rich idea-mine;
the borrow list is at the end of this page.

## What each piece is — five names, five layers

The five names are not five alternatives. They are one lineage at five
different layers, and most confusion about this family comes from treating
them as competitors.

### CapTP — the protocol (the idea layer)

CapTP, the Capability Transport Protocol, is an abstract protocol for
message passing between objects on different machines in which **holding
a reference _is_ the authority to invoke**: an object can use another
object (capability) if and only if it possesses a reference to it
([OCapN CapTP draft specification](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)).
Its defining features:

- **Distributed object references.** Within a pairwise session, objects
  are exported/imported by position; a message can carry references as
  arguments, so the reference graph and the access graph are the same
  graph.
- **Promises and promise pipelining.** A message send creates a promise
  for the result; further messages can be sent _to the promise_ before it
  resolves, collapsing what would be N round trips into one message
  flight. This comes straight from Mark S. Miller's dissertation work on
  the E language.
- **Distributed (acyclic) garbage collection.** Peers cooperatively
  inform each other when a cross-session reference is no longer needed;
  acyclic-only, because full distributed cyclic GC requires special hooks
  into the garbage collector, whereas distributed acyclic GC merely
  requires weakrefs, weakmaps, and finalizers.
- **Third-party handoffs.** When a message in session A↔B carries a
  reference to an object that lives in session A↔C, a handoff protocol
  (roles: Gifter, Receiver, Exporter) lets B connect _directly_ to C with
  the same authority, in a secure fashion even if the messages are viewed
  by a malicious actor — introductions without proxying and without
  trusting the introducer with the traffic afterwards.
- **Error propagation.** Broken promises propagate across the network;
  partition breaks promises, which is how failure is represented.

Lineage: CapTP was first openly implemented in the **E programming
language**; it has been implemented many times since, incompatibly — in E,
in Cap'n Proto, in Agoric's implementation, and in Spritely's Goblins —
and OCapN exists precisely because those implementations could not talk
to each other.

### OCapN — the interop standardisation effort (the spec layer)

OCapN (Object Capability Network) is the pre-standardisation effort to
make CapTP implementations interoperable: a suite of draft specifications
— **CapTP**, **Netlayers**, **Locators**, plus a data Model and Notation
doc — with Syrup as the wire serialisation for messages crossing a CapTP
boundary.

- **Status, in its own words: "OCapN is still pre-specification."** The
  plan is to first get Agoric's and Spritely's implementations
  interoperable, _then_ consider Cap'n Proto, "before beginning any
  discussion of standardization" (no IETF/W3C submission yet). The CapTP
  draft itself warns it is "likely to undergo significant change".
- **Participants:** Spritely Institute (primary driver, spec editor),
  Agoric, Cap'n Proto (advisory), MetaMask (review).
- **Netlayers** are OCapN's transport abstraction: CapTP runs over any
  netlayer providing a bidirectional, ordered, secure channel; the draft
  names Tor onion services as the specified one, with TCP/TLS in
  development at draft time and libp2p / IBC / I2P named as possible.
- **Locators** define the `ocapn://` URI structure: a peer designator
  (netlayer-specific, self-authenticating), a transport name, routing
  hints, and for objects a _swiss number_ — an unguessable random nonce
  assigned at registration, which is itself the capability.
- There is a shared **test suite** that implementations run against a
  reference sturdyref-exporting peer — the mechanism by which interop is
  actually checked.

### Endo — the JavaScript ocap platform (the runtime layer)

Endo (by Agoric, Apache-2.0) is a framework for ocap programming in
JavaScript: tools for _confinement_, _communication_, and _concurrency_
([endojs.org](https://endojs.org/getting-started/)). Its parts:

- **SES / Hardened JavaScript** — a shim that locks down JS intrinsics
  and provides Compartments, so mutually suspicious code can share a
  process. This is the piece with real production mileage: Agoric and
  MetaMask rely on Hardened JavaScript and the SES shim for sandboxing
  third-party plugins, and Salesforce's Lightning framework is a SES user.
- **Eventual send (`E()`)** — the programming surface that stretches async
  method invocation over processes and networks via handled promises.
- **`@endo/captp`** — Agoric's _production_ CapTP, deliberately "a minimal
  CapTP implementation": JSON-able messages over a connection object you
  supply yourself; it is transport-agnostic because it doesn't do
  transport at all.
- **`@endo/ocapn`** — the _new_ package implementing OCapN CapTP proper
  (Syrup encoding, GC operations, swiss numbers), with netlayers for
  WebSocket and a TCP testing netlayer; released at 1.1.0 and synced
  against the OCapN test suite.
- **The Pet Daemon** — a persistent host for managing guest programs in
  hardened JavaScript worker processes, communicating with its controller
  in CapTP over a Unix domain socket / named pipe. On top of it sits the
  **petname system**: each agent keeps its own namespace of
  human-meaningful names for capabilities; you grant, inspect, and revoke
  by name rather than by hash or API key. Revocation is mechanical, not
  advisory: deleting every petname that retains a value collects its
  underlying _formula_, cancels the live value, and severs the CapTP
  connections and workers still holding it. Endo is actively pitching
  this daemon as the containment layer for AI-written code, with
  capability requests mediated by the user.

### Spritely Goblins — the distributed object environment (the other runtime)

Goblins is Spritely Institute's "distributed object programming
environment" (Apache-2.0), implemented twice: **Guile Scheme** (the lead
implementation) and **Racket Scheme** (network-compatible,
feature-lagging). Current release: **v0.18.0** (April 2026). Spritely's
own maturity label for Goblins is "Working Beta".

What Goblins adds on top of CapTP:

- **Vats and transactional turns.** Objects live in vats (units of
  synchrony); each turn is a cheap local transaction that rolls back on
  error, which also enables its time-travel debugging.
- **The most complete CapTP in the family** — the OCapN README states
  Spritely "has the most pieces of CapTP (including distributed acyclic
  GC, handoffs) and the start of the netlayer abstractions and URI
  concepts."
- **Aurie / the persistence system.** Persistent vats serialise their
  object graph (in Syrup) to a store and restore it later, with
  upgrade/migration support; v0.17.0 added **Bloblin**, a streaming
  binary-log store writing compressed deltas per vat churn, replacing
  whole-graph rewrites. v0.18.0 added "sleepy actors," a caching layer
  that lets persisted actors be evicted and rehydrated on demand.
- **Sealers/unsealers** — the standard ocap pattern for tamper-evident
  envelopes and rights amplification, used throughout the actor library.
- **Netlayers shipped in Guile Goblins:** Tor onion services; TCP+TLS;
  Prelay (relay, see below); libp2p (added v0.14.0, Sept 2024); WebSocket.
  No dedicated WebRTC netlayer has shipped.
- **Hoot** compiles Guile (and thus Goblins) to WebAssembly; Goblins has
  run in the browser since v0.15.0.

### Agoric — the chain (the application/platform layer)

Agoric is a Cosmos-SDK blockchain whose smart contracts are Hardened
JavaScript objects, built on **SwingSet**, a vat kernel: vats communicate
only asynchronously, vat state is orthogonally persistent, and a special
_comms vat_ speaks CapTP to the outside world — to other SwingSets, to
solo machines, and over **IBC** to other chains, so that a blockchain can
be treated as an ordinary machine supporting distributed objects on the
network. On top sit **ERTP** (transferable rights as objects) and **Zoe**
(offer safety for contracts) — the family's worked example of ocap
_economics_.

For a non-chain user, Agoric-the-chain is mostly irrelevant; what
transfers is (a) Endo/SES, which Agoric maintains and which runs in
production at MetaMask scale, (b) the SwingSet demonstration that CapTP
semantics survive even a store-and-forward, consensus-ordered "netlayer"
(transactions as transport), and (c) ERTP/Zoe's patterns for
rights-as-objects. Agoric's engineering talent is historically central to
CapTP — the OCapN README notes Agoric "has most of the engineering talent
who have implemented CapTP historically."

## The conceptual core: distributed objects ≠ data replication

This is the contrast the whole investigation turns on.

**The ocap model answers:** _"Program A holds authority over a live
service run by program B. How does A invoke it, delegate part of that
authority to C, attenuate it, or take it back — with the guarantees
enforced by the reference graph itself rather than by an ACL checked at a
boundary?"_ State lives **in exactly one place**: inside the object, on
its home machine, behind its methods. Other parties hold _presences_
(remote references), never copies. Consistency is therefore trivial —
there is one object, one turn order, one truth — and "conflict
resolution" is a category that does not exist. What exists instead is
_liveness_: if the home machine is offline or the session breaks, every
promise to it breaks.

**The log/CRDT model (p2panda, Willow, Hypercore, …) answers the
opposite question:** _"Many nodes each hold a copy of shared data and
edit it while disconnected. How do the copies converge, and who was
allowed to write?"_ Authority there is about _past writes_, and the
system's centre of gravity is durable, replicated, mergeable state.

**Where live references win:**

- **Real-time authority and immediate revocation.** Revoking in CapTP is
  a local act with immediate effect: interpose a forwarding "caretaker"
  object and flip it, or (Endo daemon) delete the petname and the formula
  collection cancels the value and severs the connection. No revocation
  list has to propagate; there is nothing to propagate _to_.
- **Attenuation is just programming.** A restricted facet is an ordinary
  object that forwards a subset of methods. It cannot be bypassed,
  because there is no other path to the underlying object. Token systems
  attenuate _claims_; ocap attenuates _behaviour_, including stateful,
  rate-limiting, revocable behaviour.
- **Delegation without identity infrastructure.** A third-party handoff
  introduces two peers directly, with certificates that keep the handoff
  secure even when CapTP messages are not kept secret. No directory, no
  token exchange, no shared namespace is needed.
- **Latency shape.** Promise pipelining removes round trips from chained
  calls, the dominant cost in chatty distributed-object workloads.
- **Confused-deputy resistance by construction.** Because designation and
  authority travel together (the reference names _and_ empowers), the
  whole class of "tricked into using its own authority on the wrong
  object" bugs that ACL systems suffer is designed out.

**Where live references lose:**

- **Offline-first anything.** A capability to an offline object is a
  promise that breaks. There is no store-and-forward in the core model,
  no operation log to replay, no merge. (Agoric's chain use is the
  exception that proves the rule: it works because the _chain itself_ is
  the always-on home of the objects.)
- **Durable shared state.** Nothing in CapTP replicates data. Goblins'
  answer is orthogonal persistence of _one node's own_ object graph — a
  checkpoint log, not replication; two nodes never hold convergent copies
  of the same object.
- **Many-reader / public-data workloads.** Content distribution, feeds,
  shared documents, audit logs — the bread and butter of the log
  families — are simply not the ocap shape.
- **Availability coupling.** Every holder's liveness depends on the
  object's home being up and reachable. NATs, sleep, and churn are
  protocol-level problems, where a sync system treats them as background
  conditions.

**Neither model subsumes the other.** A composed estate — replicated log
for shared durable state, live references for control-plane authority —
is not a compromise; it is each model doing the job it is shaped for.

## Identity & key management

- **Peer identity is a keypair, scoped by netlayer.** In OCapN, a
  locator's designator is "self-authenticating identity (usually a public
  key)". Concretely: on the Tor netlayer the designator _is_ the onion
  address (an Ed25519 public key); on TCP+TLS, sturdyrefs use a SHA-256D
  hash of the TLS certificate as the peer identifier.
- **Sessions have their own keys, separate from peer identity.** The
  OCapN CapTP draft requires each side to create a _per-session_ Ed25519
  keypair at session start, used to sign handoff structures. Two identity
  tiers: long-lived netlayer identity (how you _reach_ me) and ephemeral
  session identity (how handoffs _authenticate_).
- **Objects have no identity separate from their references.** A
  registered object gets a swiss number — a large random nonce — inside
  its sturdyref; knowing the sturdyref _is_ the capability to enliven it.
  There is no global object namespace and no lookup without a reference.
- **Humans name things locally.** Petnames are per-user, non-global names
  bound to sturdyrefs/references — human-meaningful _and_ secure because
  they are local mappings onto unguessable identifiers, never a global
  registry.
- **Key management story is thin.** Keys are generated by the netlayer or
  by Tor; there is no PKI, no rotation protocol, no recovery story, no
  multi-device identity in the specs. Groups, as an identity concept,
  **do not exist** in this family.

## Capability model — delegation, attenuation, revocation, groups

- **Delegation** is reference-passing in a message, plus third-party
  handoffs when the reference crosses sessions. Delegation is exact: you
  can only ever give what you hold, and giving a facet gives only the
  facet.
- **Attenuation** is object composition: membranes, caretakers, read-only
  facets, sealers. It is Turing-complete policy (stateful, contextual,
  revocable), not a caveat list. This is the single largest
  capability-model gap between this family and every token system in the
  investigation.
- **Revocation** has three mechanisms, all live: (1) don't hand out the
  object, hand out a forwarder you control, then disable the forwarder;
  (2) session severance — aborting a session ends it and breaks its
  promises; (3) retention-based — Endo's daemon collects a value's
  formula when its last petname goes, cancelling the value and
  disconnecting retainers. There is deliberately **no expiry** in the
  core model: a reference is valid until revoked or its object dies.
  (Token-style expiry can be built as a facet; the reverse — live
  revocation on top of bearer tokens — is the thing that cannot be
  retrofitted.)
- **Groups: nothing.** No group abstraction, no roles, no membership
  protocol exists in CapTP/OCapN/Endo/Goblins. A "group" would be an
  ordinary object someone writes. Anyone whose auth problem is _primarily_
  group membership over shared data (the p2panda-auth / Keyhive shape of
  problem) gets no leverage here.

## Encryption, transport & NAT traversal, spec/interop status

**Encryption.** CapTP itself does no encryption; confidentiality and
channel authentication are _netlayer_ requirements (a netlayer must
provide a secure, ordered, bidirectional channel). In practice: the Tor
netlayer inherits Tor's encryption and anonymity; TCP+TLS uses TLS with
self-signed certs whose hash is the peer identity (so the channel is
authenticated by construction, no CA). Handoff security does _not_ depend
on channel secrecy — handoff certificates are signed with session keys.
There is **no end-to-end encryption of object state at rest** in this
family, and no group encryption at all.

**Transport & NAT.** The honest summary: this is the family's weakest
axis relative to the iroh/libp2p world.

- Tor onion services are the flagship netlayer precisely because onion
  services solve NAT by rendezvous — at the cost of high latency; the
  Goblins manual itself steers NAT'd TCP+TLS users to Tor, noting TCP+TLS
  lacks both "store and forward" and NAT hole punching support.
- **Prelay** (v0.12.0) is a federated relay netlayer layered over other
  netlayers — but v0 forwards messages **in cleartext through the
  relay**, unauthenticated, which means the relay has the power to both
  snoop on and modify messages; it was named _pre_-relay to discourage
  reliance on it.
- The fix is recent and not yet in a numbered release as of writing: an
  NLnet-funded **Noise-protocol netlayer** that composes with Prelay to
  make a federated, end-to-end-encrypted relay with perfect forward
  secrecy, announced October 2026 for "the next release of Goblins,"
  with an OCapN spec for the relay to follow, and already dogfooded in a
  shipped peer-to-peer game.
- A **libp2p netlayer** exists in Guile Goblins since v0.14.0, which in
  principle inherits libp2p's hole punching and relay circuits, but it is
  the least exercised of the shipped netlayers.
- Endo's side is transport-minimal: its CapTP runs over whatever message
  channel you hand it (WebSocket, MessagePort, …). There is no NAT story
  in Endo at all.

**Spec & interop status — has Goblins↔Endo OCapN interop actually been
demonstrated?** Yes — but recently, narrowly, and against a moving spec:

- 2020–2023: interop was aspirational. The Agoric↔Spritely discussion
  enumerates real divergences (serialisation, session-break semantics,
  handoff tables and GC unimplemented on the Agoric side).
- 2023: the demonstrated interop was _intra-family_ — Guile Goblins ↔
  Racket Goblins exchanging messages over OCapN, proven with a ported
  chat app.
- 2026: **Endo's `@endo/ocapn` runs a CI interop test against Guile
  Goblins** — a bilateral chatroom exercised in CI, plus the shared test
  suite. So: two independent implementations, one application shape
  (chat), CI-checked. That is a genuine milestone and also the _entirety_
  of cross-vendor OCapN interop today.
- The spec still churns underneath implementers: Goblins v0.18.0 (April 2026) bumped the CapTP protocol version and states plainly that
  applications built with an earlier release are incompatible with the
  OCapN shipped in this release. The FOSDEM 2026 talk likewise describes
  OCapN as "pre-standardization" with implementations in Racket, Guile,
  JS, and Dart.

## Sync semantics, storage fit, languages, project health

- **Sync/replication semantics:** none. Sessions are live, ordered,
  pairwise; a disconnected session's ephemeral references are severed and
  its promises break. Offline behaviour is "unavailable," by design.
- **Storage fit (SQLite/Turso):** none natively. Goblins persistence is
  its own Syrup/Bloblin file stores; Endo's daemon keeps per-user
  formula/petname stores. Nothing in the family speaks SQL, and nothing
  would gain from Turso — persistence here is _object-graph
  checkpointing_, not tables or replicated logs.
- **Languages:** JavaScript (Endo/Agoric), Guile Scheme and Racket Scheme
  (Goblins), plus a Dart implementation reported in the FOSDEM talk.
- **Rust presence: effectively zero.** The only Rust OCapN implementation
  is cmars/ocapn-rs — a single-author learning project, self-described as
  "Stage 0 in progress," whose stated goals are understanding and
  eventual interop with Goblins. There is no Rust CapTP in the OCapN
  sense, no crate, no maintainer community. (The capability RPC that
  _does_ exist in mature Rust form is Cap'n Proto's — a Level-1
  implementation — but that is the capnproto family's divergent CapTP
  descendant, not OCapN; see the [Cap'n Proto page](capnp-capnweb.md).)
- **Cloudflare fit:** poor for the protocol, incidental for the runtime.
  Workers are JS/Wasm isolates: SES/Endo code can run there in principle,
  but there is no Workers-targeted CapTP anything, no Durable Objects
  integration, and the daemon/Goblins models assume long-lived processes
  with local storage — the opposite of the Workers shape.
- **iroh fit:** none, and mildly competing. A netlayer over iroh is
  _conceivable_ (iroh gives an authenticated, encrypted, NAT-traversing
  byte stream keyed by Ed25519 NodeId — which maps cleanly onto OCapN's
  "designator = public key" locator field), but no such netlayer exists,
  and the family's own trajectory (Tor, Prelay+Noise, libp2p) is building
  its own answer to the same problem iroh already solves better.
- **Project health:** Spritely Institute is a small 501(c)(3) shipping
  steadily on NLnet grant funding (Goblins 0.18.0 April 2026, Hoot 0.10.0
  October 2026); OCapN standardisation itself is NLnet-funded. Endo is
  maintained by Agoric with MetaMask as a production SES consumer. All
  Apache-2.0. One operational note for this estate specifically: the
  OCapN project prohibits AI-generated contributions to its specs, test
  suite, and official tooling by policy — upstream engagement from an
  agent-driven fleet would have to be human-authored.
- **Operational complexity:** high for a non-Lisp/non-JS shop. Running
  this family means running a vat host (Guile or Node), a persistence
  store, netlayer infrastructure (Tor or relays), and a pre-1.0 protocol
  whose wire version has already broken compatibility once in 2026.
- **What it pointedly does NOT do:** replicate data; work offline; manage
  groups or membership; provide a global namespace or discovery; encrypt
  content at rest; run in Rust; target Cloudflare; promise a stable wire
  protocol yet.

## Fit for this estate, honestly

The estate's auth/transport problems are: agent seats on
laptops/sandboxes/Actions runners needing to reach each other and the
broker; delegation and revocation of authority between agents; group-ish
access to shared replicated state; all in Rust, over SQLite/Turso, with
iroh as the emerging transport and Cloudflare doors at the edge.

Against that shape, OCapN-as-base fails on nearly every load-bearing
axis: no Rust implementation beyond a stage-0 learning project; no
replication for the queue/atlas state that is the estate's centre of
gravity; no group model; NAT traversal a generation behind iroh; a wire
protocol that broke compatibility within the last year; and no
Cloudflare story. Adopting it as the foundation would mean porting CapTP
to Rust _and_ building the missing data layer _and_ the missing group
layer.

Where the fit is genuinely good is narrower and real: **the fleet's
control plane is a distributed-objects-shaped problem.** A seat holding a
live, attenuated, revocable reference to "the broker's dispatch facet
for repo X" — rather than a bearer token plus an ACL check — is exactly
the CapTP shape. If the estate ever builds a live agent-to-agent RPC
layer (as opposed to queue-mediated work), CapTP's semantics — not
necessarily its wire protocol — are the right blueprint, and Cap'n
Proto's Rust RPC is the pragmatic vehicle for those semantics.

## What to borrow even if we build elsewhere

1. **Revocation by interposition as a first-class pattern.** Never hand a
   raw authority to a delegate; hand a forwarder/caretaker object whose
   forwarding can be switched off. This works in any RPC system
   (including a Rust one over iroh) and gives immediate,
   propagation-free revocation — the property token systems structurally
   lack.
2. **Petnames for agent-facing authority.** Whatever the estate's
   capability format becomes, the _human/agent-facing_ layer should be
   local, meaningful names bound to unguessable identifiers — never raw
   hashes or tokens in prompts, logs, or UIs. Endo's pet daemon is the
   existence proof that this UX works for capabilities specifically.
   This aligns directly with the estate's existing "secrets by name
   only" discipline.
3. **Third-party handoff thinking for introductions.** When A introduces
   B to C, design for B↔C direct connection with the introduction
   cryptographically bound (gift certificates signed by session keys),
   not permanent proxying through A and not "send C's token to B." The
   CapTP handoff roles (Gifter/Receiver/Exporter) are the cleanest
   vocabulary available for this. Directly relevant to the
   coordinator-handoff design in the fleet actions-exec work.
4. **Promise pipelining as an API-design rule.** Any chatty cross-machine
   API the estate designs should let callers chain operations on
   not-yet-resolved results in one flight. Even without CapTP, "send the
   plan, not the steps" is the lesson.
5. **The netlayer contract as a transport-abstraction checklist.**
   OCapN's requirement set — bidirectional, ordered, session-secure,
   at-most-once-or-die, designator = self-authenticating key, hints
   separate from identity — is a sharp checklist for evaluating the
   estate's own route-form/transport candidates (iroh satisfies it almost
   by definition).
6. **SES/Hardened-JS thinking for the JS surfaces that already exist.**
   The estate runs JS at the edge (Workers, the route proxy). Where
   third-party or agent-written JS executes, Endo's confinement model —
   not its network protocol — is the mature, production-proven piece
   (MetaMask), and Endo is explicitly building bridges to AI-agent
   tooling.
7. **Formula/retention GC as a lifecycle metaphor for seats and
   sessions.** Endo's daemon models every value as a _formula_ whose
   liveness is its reachability from named roots; collection cancels
   dependents and severs connections. The fleet's seat/session/lease
   lifecycle (claims, leases, emergency extension) is the same shape; the
   vocabulary transfers even though the code cannot.

## Open questions / what would change this assessment

- A Rust OCapN implementation reaching test-suite parity (today: stage
  0, one author).
- The Noise E2EE relay landing in a numbered Goblins release _and_ being
  adopted by a second implementation (Endo) — that would make OCapN's NAT
  story merely "young" instead of "behind."
- OCapN freezing a 1.0 wire version. The 2026 incompatible bump is the
  current evidence against betting a foundation on it.
- Any group-membership or replicated-state work appearing in the family —
  none is on any public roadmap found.
