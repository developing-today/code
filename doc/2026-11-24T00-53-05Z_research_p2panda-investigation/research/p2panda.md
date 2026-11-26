# Family research: p2panda (core, auth, sync, net, encryption, spaces, store)

Lane PANDA — for the auth + transport investigation (tracking issue
developing-today/code#13779). Sources are primary: p2panda.org, docs.rs crate
documentation, the p2panda GitHub repo (READMEs, CHANGELOG), and the crates.io
API. All version/download figures were read from the crates.io API on
2026-11-23. Where a claim rests on a secondary source (DeepWiki's code index,
a conference-talk transcript), it is marked as such.

## 0. Two generations — do not confuse them

p2panda has been through a ground-up redesign. Almost everything findable
about "p2panda" before ~2025 describes the **old generation**, which is dead:

- Old generation: the `p2panda-rs` client library (Bamboo append-only logs,
  YASMF hashes) plus the `aquadoggo` reference node — GraphQL client API,
  dynamic schema system with materialised views, libp2p networking, SQLite /
  PostgreSQL storage, WASM/JS bindings (`p2panda-js`). `aquadoggo` is
  explicitly "deprecated and unmaintained", its repo was archived
  2026-01-28, and its last release (0.9.1) was 2026-04-28
  ([lib.rs crate page](https://lib.rs/crates/aquadoggo),
  [docs.rs deprecation notice](https://docs.rs/aquadoggo/latest/aquadoggo/),
  [GitHub org listing showing aquadoggo as "Public archive"](https://github.com/p2panda)).
  The old spec handbook already marks Bamboo "Deprecated" and points to the
  `namakemono` design as its successor
  ([old spec site](https://aquadoggo.p2panda.org/specifications/aquadoggo/data-types/bamboo/),
  [namakemono draft spec](https://aquadoggo.p2panda.org/specifications/namakemono/)).
- New generation (everything below): a modular workspace of Rust crates,
  first published to crates.io 2024-12-06, currently at **0.8.1
  (2026-11-13)**. No Bamboo, no GraphQL, no schema system, no libp2p — logs
  are a new minimal operation format, networking is iroh, storage is SQLite
  via SQLx ([crates.io API, p2panda-core](https://crates.io/api/v1/crates/p2panda-core)).

Every crate README and the main README carry the same warning, verbatim:
"🚧 This library is under active development and the APIs are not yet
considered stable for production use. Core data types and user-facing APIs
may still undergo breaking changes. Stability guarantees will improve with
the release of v1.0.0."
([repo README](https://raw.githubusercontent.com/p2panda/p2panda/main/README.md)).

## 1. p2panda core — operations, logs, data formats, "schemas"

- The atom is an **Operation**: a signed `Header` plus an optional `Body`.
  Header fields: format version, author's Ed25519 verifying key, Ed25519
  signature, payload size + BLAKE3 payload hash, sequence number, backlink
  (BLAKE3 hash of the author's previous operation), and a generic
  **extensions** slot. The operation's identity is the BLAKE3 hash of its
  header bytes ([docs.rs p2panda-core](https://docs.rs/p2panda-core/latest/p2panda_core/)).
- An author (one Ed25519 keypair) maintains one or many **single-writer
  append-only logs**; multi-writer collaboration is many logs read together.
  Logs are fork-tolerant (an author who signs two operations at the same
  sequence number creates a detectable fork, not corruption), bodies can be
  handled "off-chain" and deleted independently of the log structure, and
  headers support **pruning** of outdated operations
  ([docs.rs p2panda-core](https://docs.rs/p2panda-core/latest/p2panda_core/)).
- Wire encoding is **CBOR**; application payloads are arbitrary bytes —
  serde types serialised by the application (the high-level API serialises
  message enums to CBOR automatically)
  ([docs.rs p2panda](https://docs.rs/p2panda/latest/p2panda/)).
- **There is no schema system in the new generation.** The old generation's
  distributed, versioned schemas and node-side validation/materialisation
  were dropped. "Schema" today means: your own Rust types, plus whatever
  validation your own stream processors implement. Core is deliberately
  data-type-agnostic — "compatible with any application data and CRDT"
  ([docs.rs p2panda-core](https://docs.rs/p2panda-core/latest/p2panda_core/)).
  The extension mechanism is where system features attach: pruning,
  tombstones, causal ordering, group encryption and auth all ride as header
  extensions defined by the sibling crates, and users can define their own
  ([docs.rs p2panda](https://docs.rs/p2panda/latest/p2panda/)).
- `p2panda-stream` is the processing pipeline between raw operations and the
  application: ingest (validation, out-of-order buffering), causal ordering
  (`Orderer`), log pruning, and processors for groups and spaces
  ([crates.io API, p2panda-stream](https://crates.io/api/v1/crates/p2panda-stream)).

## 2. Identity & key management

- Identity **is** an Ed25519 keypair. The verifying key is the author ID in
  every operation header; there is no account layer, no naming system, and
  no binding to external identity systems (no X.509, no DID, no OIDC) in any
  crate ([docs.rs p2panda-core](https://docs.rs/p2panda-core/latest/p2panda_core/)).
- The network identity is iroh's NodeId — also an Ed25519 public key — but
  it is a _separate_ key from the author key: `p2panda-net` exposes
  `NodeId`/`EndpointAddr` from iroh alongside core's `VerifyingKey`
  ([docs.rs p2panda](https://docs.rs/p2panda/latest/p2panda/)).
- For encrypted spaces, a member additionally holds encryption key material:
  `p2panda-spaces`' `Credentials` is "a member's private key and identity
  secret", and members publish **key bundles** (identity + pre-keys for
  X3DH) into the network so others can invite them asynchronously; 0.8.0
  added cross-signing of X3DH identity keys by the verifying key and
  automatic renewal of expired key bundles
  ([docs.rs p2panda-spaces](https://docs.rs/p2panda-spaces/latest/p2panda_spaces/),
  [CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md)).
- Multi-device is modelled socially, not by key linking: each device has its
  own keypair, and a **group whose members are your devices** acts as your
  collective identity (see §4)
  ([p2panda-spaces README](https://raw.githubusercontent.com/p2panda/p2panda/main/p2panda-spaces/README.md)).
- What does **not** exist: key rotation for author identities, key recovery,
  key escrow, hardware-key support, or any bridge to a CA/PKI. Lose the
  signing key and that author identity is gone; the design answer is the
  group-as-identity pattern (a manager can re-add a fresh device key). For
  this estate that means p2panda identities would live _alongside_ the fleet
  CA / IdP CA, not under them — nothing in p2panda consumes certificates.

## 3. Capability model — what p2panda-auth actually is

This is the axis where precision matters most, because p2panda-auth is
**not a capability system**. There are no capability tokens, no delegation
chains, no bearer credentials, no caveats, no expiry. It is a **replicated
group-membership state machine** from which authorisation decisions are
derived locally:

- A group is a DAG of signed **group operations** (create, add, remove,
  promote, demote), each naming its causal predecessors plus an
  application-defined dependencies field. Every peer that replicates the
  operations computes the same group state — it is a CRDT (`GroupCRDT`)
  ([docs.rs p2panda-auth](https://docs.rs/p2panda-auth/latest/p2panda_auth/)).
- Each member holds one of four strictly-ordered **access levels** —
  `Pull < Read < Write < Manage` — each a superset of the one below. Only
  `Manage` members may modify group state; operations from non-managers are
  simply not applied ("Strict Group Modification"). `Pull` is a
  replicator tier: sync metadata, no read access, no encryption secrets
  ([docs.rs p2panda-auth](https://docs.rs/p2panda-auth/latest/p2panda_auth/),
  [DeepWiki access-control page, summarising `p2panda-auth/src/access.rs`](https://deepwiki.com/p2panda/p2panda/4.2-access-control)).
- **Attenuation, of a sort:** each access level can carry application-defined
  **conditions** (`Access<C>`, `C: Conditions`) — e.g. a `Read` grant scoped
  to a path prefix of a dataset. Conditions are opaque to the crate; the
  application evaluates them when authorising. This is the only
  fine-grained lever, and it is a policy hook, not an enforced caveat
  language ([docs.rs p2panda-auth](https://docs.rs/p2panda-auth/latest/p2panda_auth/)).
- **Delegation:** there is no "delegate a subset of my rights to a third
  party" operation. The nearest mechanisms are (a) a manager adding someone
  at a lower level, and (b) **groups as members of groups** — an actor can
  be a group (0.8.0 added "checking if actor id is a group" to the auth API),
  so authority can be delegated to a _collective_ whose own managers control
  its membership ([CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md),
  [p2panda-spaces README](https://raw.githubusercontent.com/p2panda/p2panda/main/p2panda-spaces/README.md)).
  Notably, the project's own high-level docs name Willow's **Meadowcap** and
  **UCAN** as external capability systems that can be integrated via header
  extensions "for permission checks and delegated access control" — i.e.
  p2panda itself frames delegated capability auth as something you compose
  in, not something it provides
  ([docs.rs p2panda](https://docs.rs/p2panda/latest/p2panda/)).
- **Revocation:** removal is a group operation. Because operations can be
  concurrent, revocation semantics are defined by the **resolver**: the
  default `StrongRemove` resolver walks concurrent "bubbles" in the DAG and
  invalidates (1) any concurrent actions by a member who was concurrently
  removed or demoted from manager, (2) dependent operations transitively;
  mutual concurrent removals both apply; a remove-then-re-add keeps the
  member but still invalidates their concurrent actions. The `Resolver`
  trait is replaceable if an application wants different politics
  ([docs.rs p2panda-auth](https://docs.rs/p2panda-auth/latest/p2panda_auth/)).
  Revocation is therefore _eventually consistent and retroactive over
  concurrent work_ — a removed member's in-flight operations are annulled
  once the removal is seen, but there is no instant global revocation, and
  data already replicated to (and decrypted by) the removed member is not
  clawed back; forward protection comes from encryption key rotation (§5).
- **Expiry:** none built in. Membership does not time out; key bundles have
  lifetimes (encryption layer), group membership does not.
- **Enforcement point:** authorisation is evaluated by each receiving peer
  when processing operations (e.g. `p2panda-stream`'s spaces processor
  filters out application messages from concurrently removed members;
  `p2panda-net` 0.8.0 added a `SyncHook` to accept/reject sync sessions and
  an `Authoriser` maintaining allow/blocklists of node IDs). There is no
  reference monitor and no server to appeal to — correct behaviour is a
  property of honest peers running the same validation, which fits a fleet
  of our own agents but is a different trust shape from a broker enforcing
  ACLs ([CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md)).

## 4. Group management

- Groups are first-class, replicated, and nestable. `p2panda-spaces` builds
  on auth groups to provide: groups reusable across multiple encryption
  contexts ("spaces"), **nested groups** for multi-device profiles, private
  space identifiers, and a single global auth context (`global_groups_id`)
  shared across an application
  ([p2panda-spaces README](https://raw.githubusercontent.com/p2panda/p2panda/main/p2panda-spaces/README.md),
  [docs.rs p2panda-spaces](https://docs.rs/p2panda-spaces/latest/p2panda_spaces/)).
- Group operations ride the same operation/log machinery as data: they are
  signed operations in logs associated with a topic, synced and gossiped
  like everything else; the high-level Node emits group events and supports
  promote/demote and membership-change validation (all added in 0.8.0)
  ([CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md)).
- Practical shape for this estate: a "fleet" group with Manage = operator
  keys, Write = seats allowed to publish, Read = observers, Pull = dumb
  relays — and per-seat device groups nested inside. That maps well. What
  groups do _not_ give you: scoped one-shot grants ("this runner may read
  artifact X for 10 minutes") — that pattern needs conditions gymnastics or
  an external capability layer (§3).

## 5. Encryption

Three distinct layers, often conflated:

1. **In transit (pairwise):** iroh connections are QUIC with TLS 1.3 using
   self-certified Ed25519 node keys. The high-level docs state plainly that
   data exchanged over direct topic streams "is always end-to-end encrypted"
   by iroh's TLS — but that protects a _direct connection between two
   nodes_, not data replicated through intermediaries
   ([docs.rs p2panda](https://docs.rs/p2panda/latest/p2panda/)).
2. **End-to-end group encryption (`p2panda-encryption`):** a Rust
   implementation of **DCGKA** — Decentralised Continuous Group Key
   Agreement, from Weidner, Kleppmann, Hugenroth & Beresford (2020) — not
   MLS. Two-party key agreement ("2SM") bootstraps with Signal's **X3DH**
   (with XEdDSA) over published pre-key bundles and continues with **HPKE**;
   group secret distribution is O(n) in group size. Two schemes on top:
   - _Data Encryption_ (the default, and the one spaces use): a shared
     symmetric group key, XChaCha20-Poly1305, rotated when a member is
     removed; late joiners can decrypt history; keys are retained for old
     data unless the application deletes them, so forward secrecy is
     _optional and application-managed_, while post-compromise security
     comes from the key-agreement rounds.
   - _Message Encryption_: a per-member Double-Ratchet-style scheme with
     per-message keys dropped after use — strong forward secrecy, aimed at
     chat ([docs.rs p2panda-encryption](https://docs.rs/p2panda-encryption/latest/p2panda_encryption/),
     [p2panda-encryption README](https://raw.githubusercontent.com/p2panda/p2panda/main/p2panda-encryption/README.md)).
     So: **yes, there is real E2EE for group data, and it exists today** in a
     published crate (first released 2025-07-07) — this is not a roadmap item.
     But the README's own security section is unusually candid:
   - "**This crate has not yet received a security audit.**"
   - "We currently _cannot_ recommend using this technology for high-risk
     use-cases."
   - **Group control messages are currently unencrypted** — membership
     metadata (who is in which group) is observable to anyone who can see
     the traffic; encrypting control messages is in-progress work.
   - **Not post-quantum**: no HNDL (harvest-now-decrypt-later) resistance
     ([p2panda-encryption README, Security section](https://raw.githubusercontent.com/p2panda/p2panda/main/p2panda-encryption/README.md)).
3. **At rest:** nothing. p2panda has no encrypted-storage story; the SQLite
   database holds operations (and, in the high-level API, "soon" encryption
   and access-control state — see §10) in the clear. Disk/volume encryption
   is the operator's problem
   ([docs.rs p2panda](https://docs.rs/p2panda/latest/p2panda/)).

`p2panda-spaces` is the integration crate that wires auth groups to the Data
Encryption scheme — membership changes drive key agreement, removed members
lose future keys, write authority is validated when processing application
messages. It is also the **youngest** crate in the stack: first published
2026-07-07 (0.7.0), and its integration into the high-level `p2panda` Node
API landed only in **0.8.0 (2026-11-09)** under the changelog entry
"Encrypted spaces and groups management integration into high-level API",
accompanied by a long tail of spaces fixes (deadlocks in the pipeline,
flaky spaces tests, acking of space control messages, repair-task rewrites)
([crates.io API, p2panda-spaces](https://crates.io/api/v1/crates/p2panda-spaces),
[CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md)).

## 6. Transport & NAT traversal — p2panda-net on iroh

- `p2panda-net` does **not** implement its own transport. Its `Endpoint` is
  iroh: QUIC connections, STUN-based hole punching, and DERP-designated
  relays as fallback — "most of the lower-level networking of p2panda-net is
  made possible by the work of iroh". Feature flags make the layering
  explicit: `iroh_endpoint` (dep: iroh), `gossip` (dep: iroh-gossip),
  `sync` (dep: p2panda-sync), `discovery` (dep: p2panda-discovery),
  `iroh_mdns` ([docs.rs p2panda-net](https://docs.rs/p2panda-net/latest/p2panda_net/),
  [crates.io API, p2panda-net](https://crates.io/api/v1/crates/p2panda-net)).
- **Gossip** is iroh-gossip: PlumTree + HyParView broadcast overlays, one
  overlay per topic, for ephemeral live messages (no delivery guarantee —
  offline peers miss them; sync is the catch-up path)
  ([docs.rs p2panda-net lib.rs source](https://docs.rs/p2panda-net/0.2.0/i686-unknown-linux-gnu/src/p2panda_net/lib.rs.html)).
- **Discovery** is three mechanisms: mDNS on LANs; a **confidential topic
  discovery** protocol (`p2panda-discovery`) in which nodes prove shared
  knowledge of a secret topic via Private Set Intersection / Private
  Equality Testing over salted topic hashes — topics are never revealed to
  non-members — combined with a **random-walk** peer-sampling strategy over
  the address book, bootstrapped from configured bootstrap nodes; and
  learning peers from gossip overlays. (The discovery docs credit Willow
  for the PET idea.) No DHT is used for topic discovery, deliberately:
  topics must not leak into global structures
  ([docs.rs p2panda-discovery](https://docs.rs/p2panda-discovery/latest/p2panda_discovery/)).
- Relays: iroh relays are operator-replaceable — the Node builder takes a
  `relay_url`, relays can require bearer <redacted> (added 0.7.1), and the
  project ships a configurable bootstrap-node repo. Running without n0's
  public relay infrastructure is supported by the underlying iroh design
  ([docs.rs p2panda](https://docs.rs/p2panda/latest/p2panda/),
  [CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md),
  [GitHub org](https://github.com/p2panda)).
- Resilience: modules run under Erlang-style supervision and restart on
  failure; connections and sync sessions retry automatically
  ([p2panda-net README](https://raw.githubusercontent.com/p2panda/p2panda/main/p2panda-net/README.md)).
- The architecture is explicitly transport-agnostic below the Endpoint
  ("Walkaway Stack"): core data types are designed to move over packet
  radio, LoRa, BLE or USB sticks, and the sync traits work over any
  Sink/Stream pair. Today, over IP, the only realised transport is iroh
  ([docs.rs p2panda](https://docs.rs/p2panda/latest/p2panda/),
  [docs.rs p2panda-sync](https://docs.rs/p2panda-sync/latest/p2panda_sync/)).

## 7. Sync / replication semantics — p2panda-sync

- The shipped protocol is `LogSync` (wrapped by `TopicLogSync` after a
  topic handshake). It is a **two-party, per-topic, log-height exchange**:
  both sides send `Have(LogHeights)` — for every author and log in the
  topic, the highest sequence number held — then a `PreSync` summary
  (total operations / bytes), then the missing operations stream in
  sequence, then `Done`. Because logs are single-writer with contiguous
  sequence numbers, the height vector _is_ the state vector; reconciliation
  is exact set difference, not a heuristic
  ([docs.rs, LogSyncMessage enum](https://docs.rs/p2panda-sync/latest/p2panda_sync/protocols/enum.LogSyncMessage.html)).
- Sync is partial by topic: a node replicates only the topics it subscribes
  to. Sessions run once per peer per topic, with optional resync schedules
  and automatic re-attempts managed by `p2panda-net`'s sync manager
  ([docs.rs p2panda-net](https://docs.rs/p2panda-net/latest/p2panda_net/)).
- **Offline behaviour** is the design centre: publish locally, gossip if
  connected, sync on next contact. Operations received out of order are
  buffered by the stream layer until their causal predecessors arrive.
- **Conflict story:** at the log layer there are no merge conflicts —
  single-writer logs can't conflict, they can only fork (equivocation by
  the author) or be incomplete. All real conflict semantics live one layer
  up: in your CRDT/data types (BYO — core is "compatible with any CRDT" but
  ships none), in the auth CRDT's resolver (§3), and in the encryption
  layer's handling of concurrent membership changes.
- The crate also exposes `Protocol`/`Manager` traits so applications can
  sync _other_ convergent data types (sets, Merkle trees) through the same
  session machinery — sync is a framework, with log sync as the one built-in
  ([docs.rs p2panda-sync](https://docs.rs/p2panda-sync/latest/p2panda_sync/)).
- Maturity signal from the changelog: the unreleased section already
  contains a "Sync session management re-write", a fix for a memory leak
  from uncleaned sync sessions, and a fix for operations dropped when a
  channel ran full — the sync layer is being actively repaired as of
  November 2026 ([CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md)).
- Blobs: `p2panda-blobs` wraps **iroh-blobs** (BLAKE3/bao content-addressed
  verified streaming) for large files that don't belong in operation bodies
  ([docs.rs p2panda-core page on storage](https://deepwiki.com/p2panda/p2panda/5-storage-and-data-management) — secondary;
  [crates.io API, p2panda-blobs](https://crates.io/api/v1/crates/p2panda-blobs)).

## 8. Storage fit (SQLite / Turso)

- `p2panda-store` defines backend traits — `LogStore`, `OperationStore`,
  `TopicStore`, plus processor/orderer/spaces stores — and ships **one**
  real implementation: **SQLite via SQLx** (the `sqlite` feature, default),
  with transactional writes (`BEGIN IMMEDIATE`, a `tx!` macro) and SQL
  migrations in-crate. There is also an in-memory store for tests
  ([crates.io API, p2panda-store](https://crates.io/api/v1/crates/p2panda-store),
  [DeepWiki storage page](https://deepwiki.com/p2panda/p2panda/5-storage-and-data-management) — secondary).
- **Turso/libsql: no support exists.** Nothing in the workspace depends on
  libsql. The trait surface is small enough that a libsql-backed store is
  _implementable_ (libsql speaks SQLite semantics), but it would be our
  code to write and maintain, and SQLx is wired into the shipped store's
  query layer (0.8.0 moved to "sqlx 0.9.0, safer query builder").
- Fit assessment for the estate: fleet's stores are SQLite/Turso already,
  so a p2panda node keeping its own SQLite file beside the broker's stores
  is operationally natural; expecting p2panda operations to live _inside_
  Turso (with its replication) is not something the project offers.

## 9. Rust maturity — the crate table

All crates are versioned in lockstep, MIT OR Apache-2.0, published from the
single `p2panda/p2panda` monorepo (603 GitHub stars, 54 forks, repo created
2020-10-12) by a small named team — the crates.io publish records show three
people: Andreas Dzialocha (adzialocha), Sam Andreae (sandreae), glyph
(mycognosist) ([GitHub repo](https://github.com/p2panda/p2panda),
[crates.io API](https://crates.io/api/v1/crates/p2panda-core)).

| Crate              | Latest | First published | Total downloads | Recent downloads | Role                                |
| ------------------ | ------ | --------------- | --------------- | ---------------- | ----------------------------------- |
| p2panda-core       | 0.8.1  | 2024-12-06      | 26,649          | 8,600            | Operation/log data types            |
| p2panda-store      | 0.8.1  | 2024-12-06      | 14,925          | 5,138            | Store traits + SQLite (SQLx)        |
| p2panda-sync       | 0.8.1  | 2024-12-06      | 12,797          | 3,535            | Log sync + sync traits              |
| p2panda-discovery  | 0.8.1  | 2024-12-06      | 12,137          | 2,982            | PSI topic discovery, random walk    |
| p2panda-net        | 0.8.1  | 2024-12-06      | 10,879          | 1,969            | iroh endpoint, gossip, sync manager |
| p2panda-encryption | 0.8.1  | 2025-07-07      | 10,304          | 3,288            | DCGKA group encryption              |
| p2panda-stream     | 0.8.1  | 2024-12-06      | 9,158           | 1,579            | Ingest/order/prune pipeline         |
| p2panda-blobs      | 0.8.1  | 2024-12-06      | 4,649           | 47               | iroh-blobs wrapper                  |
| p2panda-auth       | 0.8.1  | 2025-07-07      | 4,532           | 2,667            | Group CRDT, access levels           |
| p2panda-spaces     | 0.8.1  | 2026-07-07      | 3,061           | 2,165            | Auth+encryption integration         |
| p2panda            | 0.8.1  | 2026-05-18      | 2,094           | 850              | High-level Node API                 |

(Source: crates.io API per crate, read 2026-11-23; "recent" is crates.io's
trailing-window figure.)

Reading the table: the _lower_ layers (core/store/sync/net) have ~2 years of
published history and steady monthly releases (0.5.0 Jan 2026 → 0.8.1 Nov
2026, breaking changes between minors). The layers this estate cares most
about — auth, encryption, and especially their integration (spaces) and the
high-level API — are 4–16 months old. Download counts are small in absolute
terms (this is a niche ecosystem), and a large share of recent downloads is
the workspace's own CI and downstream app projects (Toolkitty, Dash Chat,
Reflection — the apps listed on [p2panda.org](https://p2panda.org)). There is
a fuzz target directory in the repo, and the encryption crate ships fuzz
test utilities, but no crate is 1.0 and none claims API stability.

## 10. Cloudflare fit

- **Essentially none, and by design.** p2panda is a native Rust stack built
  on tokio + QUIC/UDP via iroh. Cloudflare Workers cannot host it (no UDP
  listeners, no long-lived QUIC endpoints), and the current generation has
  **no browser/WASM support**: asked at Local-First Week China (Oct 2026)
  whether browsers can act as peers, Sam Andreae answered "In the current
  form of p2panda, we don't have browser support implemented" — the old
  generation's WASM path was not carried over
  ([talk transcript](https://bd.ailishi.ai/blog/p2panda-local-first-building-blocks/transcript-en.pdf) — secondary source, project lead speaking).
- Composition with the estate's Cloudflare doors would be at arm's length:
  a native p2panda node behind a Worker door (the commander/mcpx pattern)
  bridging HTTP ↔ p2panda topics. KV/R2/Durable Objects have no p2panda
  bindings; R2 could serve as blob backup storage only via our own code.
- The high-level docs also flag unfinished integration relevant to any
  embedding: the Node's SQLite persistence covers "operations, sync state,
  address books, stream cursors, and **soon** encryption/access-control
  state", and decryption in the processing pipeline is described in future
  tense in parts of the docs — the 0.8.0 changelog shows this integration
  actively landing, docs lagging code
  ([docs.rs p2panda](https://docs.rs/p2panda/latest/p2panda/),
  [CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md)).

## 11. iroh fit

- **Native — p2panda-net _is_ an iroh application.** Endpoint (connections),
  iroh-gossip (overlays), iroh-blobs (files), iroh-mdns (LAN discovery) are
  all n0's crates; p2panda tracks iroh closely (0.8.0 moved to iroh 1.4.1 /
  iroh-gossip 0.102; unreleased work already bumps iroh to 1.5.0)
  ([CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md)).
- For the estate's #13752 design (iroh NodeId as identity, self-hosted
  relay/pkarr), adopting p2panda-net means adopting exactly that transport
  with the same self-hosting options — plus topic discovery, gossip and sync
  on top, which raw iroh does not provide. The overlap is total and the
  composition is clean: p2panda consumes iroh; it does not compete with it.
- The cost of that coupling: p2panda inherits iroh's churn (iroh itself is
  pre-1.0 in spirit — its 1.x line is recent) and its relay/DERP model of
  NAT traversal. There is no alternative transport for IP networking in
  p2panda-net; if iroh's relay model fails an estate requirement, p2panda
  can't swap it out without dropping to the sync traits over a custom
  transport.

## 12. Spec / interop status

- There is **no standards-track specification** (no IETF/W3C/RFC process)
  and no second implementation. The effective spec is the crate docs +
  source; the design lineage is documented in the (draft, old-site)
  namakemono spec and the DCGKA paper for encryption. Interop today means
  "another Rust program using the same crates" — plus experimental FFI:
  `p2panda-ffi` (UniFFI bindings for Node.js, Python, Go) and
  `p2panda-gobject`, both labelled experimental by the project
  ([repo README](https://raw.githubusercontent.com/p2panda/p2panda/main/README.md)).
- Wire-format stability is not yet promised: header has a version field,
  and 0.7.x already shipped a "new forwards-compatible API to create,
  encode and decode operations" — expect format evolution before 1.0
  ([CHANGELOG](https://raw.githubusercontent.com/p2panda/p2panda/main/CHANGELOG.md)).

## 13. Project health

- **Funding:** EU public research money — the READMEs list Horizon 2020 /
  NGI grants: NGI-POINTER (871528), NGI-ASSURE (957073), NGI0-ENTRUST
  (101069594), NGI0-COMMONS (101135429), i.e. the NLnet-administered NGI0
  programme family. This is grant-funded open infrastructure, not a
  venture-backed product and not a company
  ([repo README](https://raw.githubusercontent.com/p2panda/p2panda/main/README.md)).
- **Activity:** high and current — 25 releases of the monorepo, 0.8.0 and
  0.8.1 shipped four days apart in Nov 2026, repo updated 2026-11-12, ~1,600
  commits ([GitHub repo](https://github.com/p2panda/p2panda)).
- **Community:** small. ~600 stars; real downstream apps exist (Toolkitty,
  Dash Chat, GNOME Reflection, Meli on the old stack) but the user base is
  the local-first research/activist niche, not industry
  ([p2panda.org](https://p2panda.org)).
- **License:** MIT OR Apache-2.0 throughout the new stack (the old
  aquadoggo was AGPL-3.0 — another reason the generations must not be
  conflated) ([crates.io API](https://crates.io/api/v1/crates/p2panda-core)).
- **Risk shape:** a three-person core team on grant cycles. If a grant
  lapses, development slows; the mitigation is that everything the estate
  would depend on is small, readable Rust (the whole workspace is on the
  order of 60k lines) under a permissive license — forkable in a way a
  hosted service is not.

## 14. Operational complexity

To run p2panda "for everything" in this estate we would operate: one node
per seat (embedded in the fleet binary), our own **bootstrap nodes** and
**iroh relays** (or accept n0's public relays as a dependency), allow-list
management via net's `Authoriser`, topic design per data domain, and the
SQLite files the nodes keep. There is no server component to run — which is
the point — but also no admin tooling, no metrics ecosystem beyond emitted
events, and no hosted anything. Debugging a sync stall means reading Rust
logs from a pre-1.0 stack whose own changelog shows sync-session bugs being
fixed this month.

## 15. What p2panda pointedly does NOT do — the build-it-yourself list

1. **Delegated capabilities.** No UCAN/Biscuit-style tokens, no attenuation
   chains, no expiry, no scoped one-shot grants. Auth = replicated group
   state + 4 levels + app-evaluated conditions. The project itself points
   to Meadowcap/UCAN as compose-in extensions (§3).
2. **Identity beyond raw keys.** No PKI/CA integration, no naming, no key
   rotation/recovery, no binding to the estate's IdP/OIDC or fleet CA.
3. **Application data modelling.** No CRDTs shipped (BYO — Loro/Automerge/
   yrs integrate at the byte level), no schemas, no queries, no
   materialised views. The old generation had these; the new one removed
   them on purpose.
4. **Server-side anything.** No broker, no RPC, no request/response
   semantics — it is broadcast + sync only. Fleet's queue/placement
   coordination would have to be modelled as replicated state with all the
   contention semantics that implies (single-writer logs make claims/
   leases awkward: a placement claim is inherently multi-writer state).
5. **Audited cryptography.** DCGKA implementation unaudited; group control
   messages unencrypted (metadata exposure); no post-quantum story (§5).
6. **Browser / Cloudflare / WASM.** Native only (§10).
7. **Turso.** SQLite-only shipped store (§8).
8. **Stability.** Pre-1.0, breaking changes per minor, wire format still
   evolving; the auth+encryption integration layer is months old and its
   bug tail is visible in the current changelog (§9, §5).
9. **Non-Rust reach.** FFI is experimental; mcpx (Go) could only consume
   p2panda via those bindings or a sidecar process.

## 16. Estate fit — where it genuinely shines

- **Seat-to-seat replication with offline tolerance**: agent seats on
  laptops/sandboxes that meet opportunistically is exactly the topology
  p2panda-sync + gossip + confidential discovery were built for, and it
  rides the iroh transport the estate already chose in #13752.
- **Group-structured trust**: fleet seats, operator keys, relay-only nodes
  map onto Manage/Write/Read/Pull and nested device groups without forcing.
- **Encrypted replicated data**: spaces give E2EE group data over untrusted
  relays/intermediaries — something neither raw iroh nor the current
  Tailscale/tsnet setup provides for _stored, replicated_ content.
- **Rust, SQLite, permissive license, self-hostable relays** — every estate
  hard-preference is natively satisfied by the lower two-thirds of the
  stack (core/store/sync/net/discovery).
