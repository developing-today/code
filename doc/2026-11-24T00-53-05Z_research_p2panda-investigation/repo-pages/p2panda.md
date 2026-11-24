# p2panda — modular Rust stack for signed logs, sync, and group-encrypted spaces

Full lane notes: research lane PANDA for
[#13779](https://github.com/developing-today/code/issues/13779), from primary
sources (p2panda.org, docs.rs, the p2panda GitHub repo and CHANGELOG, the
crates.io API), read 2026-11-23.

## Two generations — do not confuse them

p2panda was redesigned from the ground up. The **old generation** — the
`p2panda-rs` client library (Bamboo append-only logs) plus the `aquadoggo`
reference node (GraphQL API, dynamic schemas, libp2p, SQLite/PostgreSQL) —
is deprecated and unmaintained; its repo was archived in January 2026
([lib.rs: aquadoggo](https://lib.rs/crates/aquadoggo),
[GitHub org](https://github.com/p2panda)). The **new generation** is a
modular workspace of Rust crates, first published 2024-12-06 and currently
at **0.8.1 (2026-11-13)**: no Bamboo, no GraphQL, no schema system, no
libp2p — logs are a new minimal operation format, networking is iroh,
storage is SQLite via SQLx
([crates.io: p2panda-core](https://crates.io/api/v1/crates/p2panda-core)).

Every crate README carries the same warning in substance: under active
development, APIs not yet considered stable for production use, stability
guarantees will improve with v1.0.0. There is no 1.0 yet.

## Core: operations and logs

- The atom is an **Operation**: a signed header (format version, author's
  Ed25519 verifying key, signature, payload size + BLAKE3 hash, sequence
  number, backlink to the author's previous operation, and a generic
  **extensions** slot) plus an optional body. The operation's identity is
  the BLAKE3 hash of its header bytes
  ([docs.rs: p2panda-core](https://docs.rs/p2panda-core/latest/p2panda_core/)).
- An author (one Ed25519 keypair) maintains one or many **single-writer
  append-only logs**; multi-writer collaboration is many logs read
  together. Logs are fork-tolerant (two operations at the same sequence
  number form a detectable fork, not corruption), bodies can be deleted
  independently of the log structure, and headers support pruning.
- Wire encoding is CBOR; payloads are arbitrary bytes. **There is no
  schema system** — "schema" means your own Rust types plus your own
  validation. Core is deliberately data-type-agnostic, "compatible with
  any application data and CRDT". System features (pruning, tombstones,
  causal ordering, group encryption, auth) ride as header extensions, and
  users can define their own.
- `p2panda-stream` is the processing pipeline: ingest (validation,
  out-of-order buffering), causal ordering, log pruning, and processors
  for groups and spaces.

## Identity & key management

Identity **is** an Ed25519 keypair; the verifying key is the author ID in
every operation header. There is no account layer, no naming system, and
no binding to external identity systems (no X.509, no DID, no OIDC). The
network identity is iroh's NodeId — also Ed25519, but a _separate_ key
from the author key. For encrypted spaces, members additionally publish
**key bundles** (identity + X3DH pre-keys) so others can invite them
asynchronously; 0.8.0 added cross-signing of identity keys and automatic
bundle renewal ([docs.rs: p2panda-spaces](https://docs.rs/p2panda-spaces/latest/p2panda_spaces/)).

Multi-device is modelled socially: each device has its own keypair, and a
**group whose members are your devices** acts as your collective identity.
What does not exist: author-key rotation, recovery, escrow, hardware-key
support, or any bridge to a CA/PKI. For this estate, p2panda identities
would live _alongside_ the fleet CA / IdP CA, not under them — nothing in
p2panda consumes certificates.

## Auth: what p2panda-auth actually is

Precision matters here: **p2panda-auth is not a capability system.** There
are no capability tokens, no delegation chains, no bearer credentials, no
caveats, no expiry. It is a **replicated group-membership state machine**
from which authorisation decisions are derived locally:

- A group is a DAG of signed **group operations** (create, add, remove,
  promote, demote), each naming its causal predecessors. Every peer that
  replicates the operations computes the same group state — it is a CRDT
  ([docs.rs: p2panda-auth](https://docs.rs/p2panda-auth/latest/p2panda_auth/)).
- Each member holds one of four strictly ordered **access levels** —
  `Pull < Read < Write < Manage`. Only `Manage` members may modify group
  state. `Pull` is a replicator tier: sync metadata, no read access, no
  encryption secrets.
- **Attenuation, of a sort:** each access level can carry
  application-defined **conditions** (e.g. a `Read` grant scoped to a path
  prefix). Conditions are opaque to the crate; the application evaluates
  them. This is the only fine-grained lever, and it is a policy hook, not
  an enforced caveat language.
- **Delegation:** there is no "delegate a subset of my rights" operation.
  The nearest mechanisms are a manager adding someone at a lower level,
  and **groups as members of groups** — authority can be delegated to a
  _collective_ whose own managers control its membership. Notably, the
  project's own high-level docs name Willow's **Meadowcap** and **UCAN**
  as external capability systems that can be integrated via header
  extensions — p2panda itself frames delegated capability auth as
  something you compose in, not something it provides
  ([docs.rs: p2panda](https://docs.rs/p2panda/latest/p2panda/)).
- **Revocation:** removal is a group operation, resolved under
  concurrency by the default `StrongRemove` resolver: concurrent actions
  by a member who was concurrently removed (or demoted from manager) are
  invalidated, transitively; mutual concurrent removals both apply; a
  remove-then-re-add keeps the member but still invalidates their
  concurrent actions. The `Resolver` trait is replaceable. Revocation is
  therefore _eventually consistent and retroactive over concurrent work_ —
  but data already replicated to (and decrypted by) a removed member is
  not clawed back; forward protection comes from encryption key rotation.
- **Expiry:** none. Membership does not time out.
- **Enforcement point:** each receiving peer evaluates authorisation when
  processing operations. There is no reference monitor and no server to
  appeal to — correct behaviour is a property of honest peers running the
  same validation. That fits a fleet of our own agents, but it is a
  different trust shape from a broker enforcing ACLs.

## Groups

Groups are first-class, replicated, and nestable. `p2panda-spaces` builds
on auth groups to provide groups reusable across multiple encryption
contexts ("spaces"), nested groups for multi-device profiles, private
space identifiers, and a single global auth context shared across an
application. Practical shape for this estate: a "fleet" group with Manage
= operator keys, Write = seats allowed to publish, Read = observers, Pull
= dumb relays — and per-seat device groups nested inside. That maps well.
What groups do not give you: scoped one-shot grants ("this runner may
read artifact X for 10 minutes") — that pattern needs conditions
gymnastics or an external capability layer.

## Encryption

Three distinct layers:

1. **In transit (pairwise):** iroh connections are QUIC with TLS 1.3
   using self-certified Ed25519 node keys. That protects a direct
   connection between two nodes, not data replicated through
   intermediaries.
2. **End-to-end group encryption (`p2panda-encryption`):** a Rust
   implementation of **DCGKA** (Decentralised Continuous Group Key
   Agreement), not MLS. Two-party key agreement bootstraps with Signal's
   X3DH and continues with HPKE. Two schemes on top: _Data Encryption_
   (the default, used by spaces) — a shared symmetric group key
   (XChaCha20-Poly1305) rotated when a member is removed; late joiners
   can decrypt history; forward secrecy is optional and
   application-managed — and _Message Encryption_, a per-member
   Double-Ratchet-style scheme with strong forward secrecy, aimed at
   chat ([docs.rs: p2panda-encryption](https://docs.rs/p2panda-encryption/latest/p2panda_encryption/)).
   Real E2EE for group data exists today in a published crate (first
   released 2025-07-07). But the README's security section is unusually
   candid: the crate **has not received a security audit**; the authors
   currently _cannot recommend it for high-risk use-cases_; **group
   control messages are currently unencrypted** (membership metadata is
   observable); and it is **not post-quantum**.
3. **At rest:** nothing. The SQLite database holds operations in the
   clear; disk/volume encryption is the operator's problem.

`p2panda-spaces` — the crate wiring auth groups to the Data Encryption
scheme — is also the **youngest** in the stack: first published 2026-07-07,
and its integration into the high-level Node API landed only in 0.8.0
(2026-11-09), accompanied by a visible tail of spaces fixes in the
changelog.

## Transport & NAT traversal

`p2panda-net` does not implement its own transport; its `Endpoint` **is**
iroh: QUIC connections, hole punching, and relays as fallback — most of
the lower-level networking is made possible by iroh
([docs.rs: p2panda-net](https://docs.rs/p2panda-net/latest/p2panda_net/)).

- **Gossip** is iroh-gossip (PlumTree + HyParView overlays, one per
  topic) for ephemeral live messages; offline peers miss them, and sync
  is the catch-up path.
- **Discovery** is three mechanisms: mDNS on LANs; a **confidential topic
  discovery** protocol in which nodes prove shared knowledge of a secret
  topic via Private Set Intersection / Private Equality Testing over
  salted topic hashes — topics are never revealed to non-members —
  combined with a random-walk peer-sampling strategy bootstrapped from
  configured bootstrap nodes; and learning peers from gossip overlays.
  No DHT is used for topic discovery, deliberately: topics must not leak
  into global structures
  ([docs.rs: p2panda-discovery](https://docs.rs/p2panda-discovery/latest/p2panda_discovery/)).
- Relays are operator-replaceable (the Node builder takes a `relay_url`;
  relays can require bearer tokens), and the project ships a configurable
  bootstrap-node repo. Running without n0's public relay infrastructure
  is supported by the underlying iroh design.
- The architecture is explicitly transport-agnostic below the Endpoint
  ("Walkaway Stack"): core data types are designed to move over packet
  radio, LoRa, BLE, or USB sticks, and the sync traits work over any
  Sink/Stream pair. Today, over IP, the only realised transport is iroh.

## Sync semantics

The shipped protocol is `LogSync`: a two-party, per-topic, **log-height
exchange**. Both sides send `Have(LogHeights)` — for every author and log
in the topic, the highest sequence number held — then a `PreSync` summary,
then the missing operations stream in sequence, then `Done`. Because logs
are single-writer with contiguous sequence numbers, the height vector _is_
the state vector; reconciliation is exact set difference, not a heuristic
([docs.rs: p2panda-sync](https://docs.rs/p2panda-sync/latest/p2panda_sync/protocols/enum.LogSyncMessage.html)).

- Sync is partial by topic; sessions run once per peer per topic with
  optional resync schedules. **Offline behaviour is the design centre:**
  publish locally, gossip if connected, sync on next contact.
- **Conflict story:** at the log layer there are no merge conflicts —
  single-writer logs can only fork (author equivocation) or be
  incomplete. All real conflict semantics live one layer up: in your CRDT
  data types (bring your own — core ships none), in the auth CRDT's
  resolver, and in the encryption layer's handling of concurrent
  membership changes.
- The crate also exposes traits so applications can sync _other_
  convergent data types through the same session machinery.
- Maturity signal: the unreleased changelog section already contains a
  sync-session-management rewrite and fixes for a session memory leak and
  dropped operations — the sync layer is being actively repaired as of
  November 2026. Blobs ride `p2panda-blobs`, a wrapper over iroh-blobs
  (BLAKE3/bao verified streaming).

## Storage fit

`p2panda-store` defines backend traits (log, operation, topic, plus
processor/orderer/spaces stores) and ships **one** real implementation:
**SQLite via SQLx**, with transactional writes and in-crate migrations,
plus an in-memory store for tests. **Turso/libsql: no support exists** —
nothing in the workspace depends on libsql. The trait surface is small
enough that a libsql-backed store is implementable (libsql speaks SQLite
semantics), but it would be our code to write and maintain. Fit for the
estate: a p2panda node keeping its own SQLite file beside the broker's
stores is operationally natural; expecting p2panda operations to live
_inside_ Turso replication is not something the project offers.

## Rust maturity

All crates are versioned in lockstep, MIT OR Apache-2.0, published from a
single monorepo (~600 GitHub stars) by a small named team — three people
appear in the crates.io publish records. Lower layers (core, store, sync,
net) have ~2 years of published history and steady monthly releases
(0.5.0 in January 2026 → 0.8.1 in November 2026, breaking changes between
minors). The layers this estate cares most about — auth, encryption, and
especially their integration (spaces) and the high-level API — are 4–16
months old. Download counts are small in absolute terms; a large share of
recent downloads is the workspace's own CI and downstream app projects
(Toolkitty, Dash Chat, Reflection — the apps listed on
[p2panda.org](https://p2panda.org)).

## Cloudflare fit

Essentially none, and by design. p2panda is a native Rust stack on tokio +
QUIC/UDP via iroh; Cloudflare Workers cannot host it, and the current
generation has **no browser/WASM support** (the project lead confirmed at
Local-First Week China, October 2026, that browsers cannot act as peers in
the current form). Composition with the estate's Cloudflare doors would be
at arm's length: a native p2panda node behind a Worker door, bridging
HTTP ↔ topics. KV/R2/Durable Objects have no p2panda bindings.

## iroh fit

Native — p2panda-net _is_ an iroh application (Endpoint, iroh-gossip,
iroh-blobs, iroh-mdns), and p2panda tracks iroh closely (0.8.0 moved to
iroh 1.4.1 / iroh-gossip 0.102). For the estate's
[#13752](https://github.com/developing-today/code/issues/13752) design,
adopting p2panda-net means adopting exactly that transport with the same
self-hosting options — plus topic discovery, gossip, and sync on top,
which raw iroh does not provide. The overlap is total and the composition
is clean: p2panda consumes iroh; it does not compete with it. The cost of
that coupling: p2panda inherits iroh's churn and its relay model of NAT
traversal; there is no alternative IP transport in p2panda-net.

## Spec / interop status

There is no standards-track specification and no second implementation.
The effective spec is the crate docs + source. Interop today means
"another Rust program using the same crates", plus experimental FFI
(UniFFI bindings for Node.js, Python, Go) and GObject bindings, both
labelled experimental. Wire-format stability is not yet promised; expect
format evolution before 1.0.

## Project health

Funding is EU public research money — the READMEs list Horizon 2020 / NGI
grants (NGI-POINTER, NGI-ASSURE, NGI0-ENTRUST, NGI0-COMMONS — the
NLnet-administered NGI0 programme family). This is grant-funded open
infrastructure, not a venture-backed product. Activity is high and
current: 25 monorepo releases, 0.8.0 and 0.8.1 shipped four days apart in
November 2026, ~1,600 commits. Community is small (~600 stars); real
downstream apps exist but the user base is the local-first
research/activist niche. Risk shape: a three-person core team on grant
cycles; the mitigation is that everything the estate would depend on is
small, readable Rust (the whole workspace is on the order of 60k lines)
under a permissive license — forkable in a way a hosted service is not.

## Operational complexity

Running p2panda "for everything" here means operating: one node per seat
(embedded in the fleet binary), our own bootstrap nodes and iroh relays
(or accepting n0's public relays as a dependency), allow-list management,
topic design per data domain, and the nodes' SQLite files. There is no
server component — which is the point — but also no admin tooling and no
metrics ecosystem beyond emitted events. Debugging a sync stall means
reading Rust logs from a pre-1.0 stack whose changelog shows sync-session
bugs being fixed this month.

## What p2panda pointedly does NOT do

1. **Delegated capabilities** — no tokens, no attenuation chains, no
   expiry, no scoped one-shot grants (see Auth above; the project points
   to Meadowcap/UCAN as compose-in extensions).
2. **Identity beyond raw keys** — no PKI/CA integration, no naming, no
   key rotation/recovery, no binding to the estate's IdP/OIDC or fleet CA.
3. **Application data modelling** — no CRDTs shipped, no schemas, no
   queries, no materialised views (the old generation had these; the new
   one removed them on purpose).
4. **Server-side anything** — no broker, no RPC, no request/response
   semantics; broadcast + sync only. Fleet's queue/placement coordination
   would have to be modelled as replicated state, and single-writer logs
   make claims/leases awkward: a placement claim is inherently
   multi-writer state.
5. **Audited cryptography** — the DCGKA implementation is unaudited;
   group control messages are unencrypted (metadata exposure); no
   post-quantum story.
6. **Browser / Cloudflare / WASM** — native only.
7. **Turso** — SQLite-only shipped store.
8. **Stability** — pre-1.0, breaking changes per minor, wire format still
   evolving; the auth+encryption integration layer is months old.
9. **Non-Rust reach** — FFI is experimental; mcpx (Go) could only consume
   p2panda via those bindings or a sidecar process.

## Where it genuinely shines for this estate

- **Seat-to-seat replication with offline tolerance** — agent seats on
  laptops/sandboxes that meet opportunistically is exactly the topology
  p2panda-sync + gossip + confidential discovery were built for, riding
  the iroh transport the estate already chose.
- **Group-structured trust** — fleet seats, operator keys, and relay-only
  nodes map onto Manage/Write/Read/Pull and nested device groups without
  forcing.
- **Encrypted replicated data** — spaces give E2EE group data over
  untrusted relays/intermediaries, something neither raw iroh nor the
  current Tailscale/tsnet setup provides for stored, replicated content.
- **Rust, SQLite, permissive license, self-hostable relays** — every
  estate hard-preference is natively satisfied by the lower two-thirds of
  the stack (core/store/sync/net/discovery).
