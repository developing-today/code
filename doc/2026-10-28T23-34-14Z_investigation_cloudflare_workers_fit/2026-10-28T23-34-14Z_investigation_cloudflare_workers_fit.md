# Investigation: Fitting the ID Platform onto Cloudflare Workers

- **Date:** 2026-10-28
- **Directive:** Drew, LIST 12 — a serverless edition of ID: no containers or conventional
  servers per user; Cloudflare Workers isolates + Durable Objects + SQLite, per-user
  stores, Google SSO sign-in as the hosted-data option. A little container use is
  acceptable; indefinite per-user-interaction container spend is not.
- **Subject system:** `pkgs/id` in developing-today/code — a P2P platform on Iroh
  (QUIC transport, hole punching, relays) with content-addressed blobs, CRDT docs,
  gossip, a dual-index tag/graph layer, signed objects/envelopes, an event store,
  and "worlds" (sandboxed programs in shared journaled worlds, rendered to web + SSH).
  Component names below are taken from `pkgs/id/ARCHITECTURE.md` and `src/`
  (protocol.rs, store.rs, tags.rs, chain.rs, event_store/, world_*, web/, access.rs,
  discovery.rs, dht.rs).
- **Method:** Cloudflare and Google official documentation only for platform facts;
  every load-bearing number is cited inline. Architecture judgments are labelled as
  conclusions, not doc facts.

## Verdict up front

A Workers edition of ID is feasible **only as an HTTP/WebSocket-shaped subset** —
a hosted "home server" per user, not a port of the Iroh node. The single hard wall
is transport: Iroh runs on QUIC/UDP, and Workers expose **no UDP sockets at all**
and no general inbound listening (see §2.2). Everything above transport — signed
graph objects, tag indexes, event journals, blob storage, live world sessions —
maps onto Durable Objects + SQLite + R2 with varying amounts of rework. The
per-user Durable Object (one object per user, SQLite-backed) is the natural center
of the design: it is simultaneously the per-user server, the per-user database,
and the live-connection holder, and it costs nothing while idle.

---

## 1. Durable Objects as the per-user "server"

### 1.1 Consistency model

- A Durable Object is "a special kind of Cloudflare Worker" that combines compute
  with storage; each has a globally unique name and its attached storage "is
  strongly consistent yet fast to access"
  ([DO overview](https://developers.cloudflare.com/durable-objects/)).
- Execution is single-threaded per object and requests to one object are
  serialized, which is what makes the strong consistency cheap: no locks or
  consensus are needed inside one user's store. For ID this matches the existing
  semantics well — a single user's graph/journal is already effectively
  single-writer (its owner), so serialization is a feature, not a bottleneck,
  until one object must serve many _other_ users' writes (see shared worlds, §6).
- Throughput ceiling per object: a **soft limit of ~1,000 requests/second** per
  individual object; beyond that, requests queue and then return "overloaded"
  errors ([DO limits](https://developers.cloudflare.com/durable-objects/platform/limits/)).
  One object per user makes this a per-user cap, which is the right blast radius.

### 1.2 SQLite storage API limits (per object)

From [DO limits](https://developers.cloudflare.com/durable-objects/platform/limits/)
(SQLite-backed objects, the recommended backend):

| Limit                                  | Value                                      |
| -------------------------------------- | ------------------------------------------ |
| Storage per Durable Object             | **10 GB** (Workers Paid); **1 GB** on Free |
| Storage per account                    | Unlimited (Paid) / 5 GB (Free)             |
| Number of objects                      | Unlimited                                  |
| Max Durable Object classes per account | 500 (Paid) / 100 (Free)                    |
| Row / string / BLOB size               | **2 MB** max                               |
| KV key+value combined                  | 2 MB max                                   |
| Columns per table                      | 100                                        |
| SQL statement length                   | 100 KB                                     |
| Bound parameters per query             | 100                                        |
| WebSocket received message size        | 32 MiB                                     |
| CPU per request                        | 30 s default, configurable to 5 min        |

Consequences for ID:

- Signed graph objects, tag rows, journal entries, membership/invite records all
  fit comfortably as SQLite rows — they are small, structured, and indexed.
- **Blobs do not belong in the per-user SQLite.** Anything over 2 MB cannot be a
  row at all, and a 10 GB per-object ceiling is the user's _entire_ hosted store.
  Blob bytes go to R2 (§3.3); SQLite keeps hash → R2-key metadata.
- Point-in-time recovery exists for DO SQLite (30-day bookmark window is a
  platform feature of the storage API), which is a reasonable analogue of journal
  replay for disaster recovery, though it is not a substitute for ID's own
  signed, portable journal format.

### 1.3 WebSocket hibernation — yes, a DO can hold live connections and sleep

- With the Hibernation WebSocket API (`ctx.acceptWebSocket(server)` instead of
  `server.accept()`), "clients remain connected while the Durable Object is not
  in memory", "billable Duration (GB-s) charges do not accrue during hibernation",
  and "when a message arrives, the Durable Object wakes up automatically"
  ([DO WebSockets](https://developers.cloudflare.com/durable-objects/best-practices/websockets/)).
- During hibernation in-memory state is reset and the constructor re-runs on
  wake; per-connection state survives via `serializeAttachment` /
  `deserializeAttachment` (max **16,384 bytes** per attachment — enough for a
  session's identity + subscriptions, not for payloads).
- Protocol ping/pong is answered by the runtime without waking the object, and
  `setWebSocketAutoResponse()` can answer application-level keepalives the same
  way ([DO WebSockets](https://developers.cloudflare.com/durable-objects/best-practices/websockets/),
  [DO pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/)).
- Cloudflare's own guidance targets exactly ID's shape: one DO coordinating many
  clients (chat rooms, multiplayer games, collaborative editing), and batching
  many small logical messages into one WebSocket frame to cut per-message
  overhead — directly relevant to LIST 8/9's "many small packets" traffic.

**Conclusion:** a per-user DO _can_ act as that user's always-reachable server —
holding WebSocket connections from the user's own clients and from guests,
fanning out graph updates, and sleeping (unbilled) between events. What it
cannot do is hold an Iroh/QUIC session or a raw inbound TCP listener (§2.2).

---

## 2. Workers runtime limits that bite a P2P/graph workload

From [Workers limits](https://developers.cloudflare.com/workers/platform/limits/):

| Limit                             | Free                | Paid                                     |
| --------------------------------- | ------------------- | ---------------------------------------- |
| CPU time per HTTP request         | 10 ms               | 30 s default, **5 min max**              |
| Memory per isolate                | **128 MB**          | **128 MB** (does not increase with plan) |
| Subrequests per invocation        | 50                  | 10,000                                   |
| Simultaneous outgoing connections | 6                   | 6                                        |
| Requests                          | 100,000/day         | unlimited (billed)                       |
| Worker bundle size                | 64 MiB uncompressed | 64 MiB                                   |
| Startup (global scope)            | 1 s CPU             | 1 s CPU                                  |

Notes that matter for ID:

- CPU time counts only active execution; waiting on `fetch()`/storage does not
  count. Signature verification and graph traversal are pure CPU, so they are
  the budget to watch — a full-graph sync or a large journal replay must be
  chunked across invocations, alarms, or Queue/Workflow steps rather than run
  in one request.
- 128 MB is per _isolate_, shared by concurrent requests, and includes Wasm
  allocations. No in-memory whole-graph working set: the graph must live in
  SQLite/R2 and be paged through, which ID's storage layer mostly already does
  on disk — but any code that loads a whole doc/world into RAM needs rework.
- A DO invocation (HTTP/RPC/WebSocket message/alarm) gets the same CPU limits;
  wall-clock duration for HTTP-triggered work is unlimited while the client
  stays connected, and DO alarm handlers get 15 minutes of wall time
  ([DO limits](https://developers.cloudflare.com/durable-object
  ...[truncated 15581 chars]s/platform/limits/)).

### 2.1 Wasm support

Workers run Rust/Go/C compiled to WebAssembly via `WebAssembly.instantiate()`,
or an entire Worker written in Rust (workers-rs); SIMD is supported;
**threading is not** — "each Worker runs in a single thread"; WASI support is
experimental with only some syscalls implemented
([Workers Wasm](https://developers.cloudflare.com/workers/runtime-apis/webassembly/)).

**Conclusion:** ID's pure logic (signature checks, tuple/tag encoding, journal
replay, CRDT merge, world VM if it is single-threaded and syscall-light) has a
plausible Rust→Wasm path. Anything in the Rust stack that assumes Tokio,
threads, filesystem, or sockets does not come along — the network and storage
edges must be rewritten against Fetch/WebSocket/SQLite APIs regardless.

### 2.2 Networking: the Iroh wall

- Outbound: `fetch()` (HTTP/HTTPS/WebSocket) plus **outbound TCP** via
  `connect()` from `cloudflare:sockets` — enough for TCP application protocols
  such as MQTT, IRC, Postgres/MySQL wire protocols
  ([TCP sockets](https://developers.cloudflare.com/workers/runtime-apis/tcp-sockets/)).
- Inbound: HTTP/WebSocket only, in the generally available runtime. The TCP
  sockets doc states a Worker can receive an inbound TCP connection only when
  forwarded from another Worker over a service binding, with support for other
  inbound sources still "coming soon". Sockets cannot be created in global
  scope, and an open TCP socket from a DO pins it in memory (duration charges)
  for up to 15 minutes per connection.
- **UDP: there is no UDP socket API.** Iroh's transport is QUIC, and QUIC runs
  over UDP; hole punching and Iroh relay (DERP-style) flows are likewise
  UDP/QUIC-shaped. None of that can execute inside a Worker or Durable Object.

**Conclusion (the central finding):** the Worker edition **cannot be an Iroh
peer**. It cannot join the swarm, accept QUIC connections, hole-punch, or speak
to a native `id serve` node over Iroh protocols. What it can be:

1. A **hosted home server** speaking ID's _application_ semantics (signed
   objects, graph, journals) over HTTPS + WebSocket to browsers and to native
   clients that add an HTTPS/WS transport.
2. A **bridge/gateway**: native Iroh nodes and the hosted edition exchange
   signed objects through an HTTPS/WS sync endpoint (the DO accepts pushed
   objects, verifies signatures, merges into SQLite; native nodes pull the
   same way). Signature validity is transport-independent, so the trust model
   survives the bridge unchanged.
3. TCP `connect()` leaves a narrow door for outbound federation to TCP-based
   services (e.g. MQTT brokers — relevant to LIST 6's ESP32/Home Assistant
   path), but not for Iroh.

This is a _subset by design_, exactly as Drew framed it ("acting as actual
servers, or technically peer-to-peer servers, sort of"): the hosted node is a
peer in the graph/trust sense, not in the QUIC sense.

---

## 3. Per-user SQLite: DO-SQLite vs D1-per-user vs R2/KV

### 3.1 Option A — one SQLite-backed Durable Object per user (recommended)

- Identity: `namespace.idFromName("google:" + sub)` gives a stable, globally
  unique object per user; unlimited objects per account (§1.2).
- The same unit holds compute + strongly consistent SQLite + live WebSockets,
  which is precisely "a SQLite for each user" _and_ their server.
- No provisioning step: an object (and its database) materializes on first
  request. No per-user binding or API bookkeeping.
- Caps: 10 GB/user store, ~1,000 req/s soft per user, single-threaded per
  user — all acceptable for a personal graph store; a _shared_ world with many
  writers would need its own DO (per world, not per user) and inherits the
  same per-object throughput ceiling.

### 3.2 Option B — one D1 database per user

From [D1 limits](https://developers.cloudflare.com/d1/platform/limits/):

| Limit                         | Free   | Paid       |
| ----------------------------- | ------ | ---------- |
| Databases per account         | 10     | **50,000** |
| Max database size             | 500 MB | 10 GB      |
| Total storage per account     | 5 GB   | **1 TB**   |
| Queries per Worker invocation | 50     | 1,000      |
| Max SQL query duration        | 30 s   | 30 s       |
| Row size                      | 2 MB   | 2 MB       |

Cloudflare explicitly designs D1 for "horizontal scale out across multiple,
smaller (10 GB) databases, such as per-user, per-tenant or per-entity
databases", and each D1 database is itself backed by a single Durable Object
and is single-threaded for writes. But for ID's per-user model D1 is strictly
weaker than Option A:

- **Hard user ceiling:** 50,000 databases/account. DO objects are unlimited.
- **Account storage ceiling:** 1 TB across all users, vs unlimited for
  DO-SQLite on Paid.
- **Provisioning friction:** databases must be created via API/dashboard, and a
  Worker script supports only ~5,000 bindings — per-user databases cannot all
  be statically bound; every query becomes dynamic-routing bookkeeping.
- **No live connections, no compute at the data:** D1 is query-only. The
  WebSocket fan-out and write serialization logic still needs a DO in front,
  so D1-per-user _adds_ a layer instead of replacing one.
- Where D1 _does_ fit: a single shared **directory/registry database**
  (Google-subject → user record, public-server directory, name resolution for
  LIST 13's @/# lookup), where cross-user SQL queries are the point and
  read replication helps. One D1 (or a few), not one per user.

### 3.3 Blobs and caches — R2 and KV

- **R2** (blob bytes): object size up to **5 TiB**, unlimited objects/buckets
  storage, 1M buckets/account, zero egress fees
  ([R2 limits](https://developers.cloudflare.com/r2/platform/limits/),
  [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/)).
  Content-addressed layout (`blobs/<hash>`) maps 1:1 onto Iroh blob hashes.
  Caveat: max **1 concurrent write/sec to the same key** — fine for immutable
  content-addressed objects (write-once), wrong for mutable files.
- **KV** (cache only): value up to 25 MiB, key 512 B, but **1 write/sec to the
  same key**, eventual consistency (~60 s propagation), and expensive writes
  ([KV limits](https://developers.cloudflare.com/kv/platform/limits/)).
  Suitable for public profiles, discovery records, rendered-page caches.
  **Not** suitable as graph storage: ID's graph is write-heavy and
  consistency-sensitive, and KV's model fights both.

### 3.4 Comparison summary

| Criterion          | DO-SQLite per user                    | D1 per user              | R2 / KV                        |
| ------------------ | ------------------------------------- | ------------------------ | ------------------------------ |
| Per-user cap       | 10 GB                                 | 10 GB                    | R2: ~unlimited                 |
| Account ceiling    | Unlimited storage (Paid)              | 50k DBs, 1 TB            | —                              |
| Consistency        | Strong, serialized                    | Single-writer primary    | R2: object-level; KV: eventual |
| Live WebSockets    | **Yes, in the same unit**             | No                       | No                             |
| Provisioning       | None (name-derived)                   | API per user             | Bucket/namespace once          |
| Cross-user queries | No (by design)                        | Awkward (per-DB routing) | No                             |
| Best ID role       | **Per-user graph + journal + server** | Shared directory DB      | R2: blobs; KV: public cache    |

---

## 4. Google SSO on Workers, and binding it to an ID identity

### 4.1 The flow (all plain HTTPS — fully Worker-shaped)

Google's OAuth 2.0 APIs "conform to the OpenID Connect specification and are
OpenID Certified"; Google publishes a Discovery document, and the recommended
"server flow" is: create anti-forgery `state` → redirect to Google's
authorization endpoint → confirm `state` → **exchange the `code` for an access
token and ID token** at the token endpoint → read user info from the ID token
([Google OIDC](https://developers.google.com/identity/protocols/oauth2/openid-connect)).

On Workers this needs no special service:

- The token exchange is one `fetch()` POST with the client secret, held as a
  Worker secret (never in the bundle). It counts as one subrequest.
- ID-token validation is local: verify the JWT signature against Google's JWKS
  (from the Discovery document's `jwks_uri`), check `iss`, `aud` = your
  client ID, `exp`, and `nonce`. Libraries exist for this (e.g. `jose`-style
  JWT verification runs fine in the V8/WebCrypto environment), or Cloudflare
  Access can do the whole login instead (below).
- After login, issue the app's own session (signed cookie / token). The
  long-lived Google tokens should not be the session.

### 4.2 The stable user key: `sub`, never email

Google's ID-token claim table: `sub` is "an identifier for the user, **unique
among all Google Accounts and never reused**… the `sub` value is never
changed. Use `sub` within your application as the unique-identifier key for
the user" (max 255 case-sensitive ASCII chars)
([Google OIDC](https://developers.google.com/identity/protocols/oauth2/openid-connect)).
Email addresses change; `sub` does not. So:

- Hosted account key = `google:<sub>` — and it should be stored hashed in any
  shared directory, since it is a permanent cross-service correlator.
- This key derives the user's DO name (§3.1) and their directory record.

### 4.3 Binding Google identity → ID (Iroh) identity — the part SSO does not solve

A Google login proves _account_ identity; an ID identity is an Ed25519-style
keypair whose public key _is_ the ID. These must be explicitly bound, and the
binding design is a real decision for the ID program (flagged, not settled
here):

1. **Claim binding (recommended):** the user's native ID key signs a statement
   "this key claims hosted account `google:<sub-hash>`", and/or the hosted
   account (once logged in) records the user's public key after the user signs
   a login-issued challenge with their native node. The binding record lives
   in the directory D1 + the user's DO. Result: hosted data is _theirs_ in
   the same trust graph; Google is only the door, not the identity.
2. **Custodial key:** the hosted edition generates/holds a key for the user
   server-side. Zero-friction, but the operator can sign as the user — this
   breaks ID's "validly-signed objects + allowed writers" trust model
   (LIST 7) unless custodial objects are visibly marked as a different trust
   class.
3. **Browser-held key:** key generated and kept in the browser (WebCrypto /
   local storage), signing client-side, Worker verifies. Middle path;
   key-loss = identity-loss unless paired with (1) for recovery.

### 4.4 Alternative: let Cloudflare Access run the login

Cloudflare Access supports Google (and generic OIDC) as an identity provider:
you register the Google OAuth client, add it under Zero Trust → Integrations →
Identity providers, and Access handles the redirect/token exchange and passes
a validated identity JWT to the application
([Cloudflare One — Google Workspace IdP](https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/google-workspace/),
[Generic OIDC](https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/generic-oidc/)).

Trade-off: Access removes the hand-rolled OIDC code and is the faster pilot
path, but it is built to _gate applications_, not to be a consumer identity
system — session/policy semantics, per-user provisioning hooks, and JWT claim
contents are Access-shaped. For a public hosted ID service where sign-up _is_
the product, hand-rolled OIDC in the Worker (§4.1) keeps identity semantics
under ID's control. Recommendation: pilot with hand-rolled OIDC; it is ~one
endpoint plus JWT verification; keep Access for admin/operator surfaces.

---

## 5. Cost shape — what bills per request vs per duration

All rates Workers Paid (after the $5/mo account minimum), from
[Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/)
and [DO pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/):

| Primitive            | Billed per                                      | Included                          | Overage                                                               |
| -------------------- | ----------------------------------------------- | --------------------------------- | --------------------------------------------------------------------- |
| Workers              | **request** + **CPU time** (no duration charge) | 10M req/mo; 30M CPU-ms/mo         | $0.30/M req; $0.02/M CPU-ms                                           |
| Worker static assets | nothing                                         | free, unlimited                   | —                                                                     |
| Durable Objects      | **request** + **duration (GB-s)** + rows        | 1M req/mo; 400,000 GB-s/mo        | $0.15/M req; $12.50/M GB-s                                            |
| DO-SQLite / D1 rows  | **rows read** / **rows written**                | 25B read/mo; 50M written/mo       | $0.001/M read; **$1.00/M written**                                    |
| DO-SQLite storage    | GB-month                                        | 5 GB-mo                           | $0.20/GB-mo                                                           |
| D1 storage           | GB-month                                        | 5 GB                              | $0.75/GB-mo                                                           |
| R2                   | storage + operations, **egress free**           | 10 GB-mo; 1M Class A; 10M Class B | $0.015/GB-mo; $4.50/M Class A (writes/lists); $0.36/M Class B (reads) |
| KV                   | reads / writes / storage                        | 10M reads; 1M writes/mo           | $0.50/M reads; **$5.00/M writes**                                     |

Key mechanics for ID's design:

- **Idle users cost ~zero.** "Inactive objects receiving no requests do not
  incur any duration charges", and a hibernating DO accrues no duration even
  with WebSockets connected. The only idle cost is stored bytes
  ($0.20/GB-mo in DO-SQLite). This is exactly Drew's requirement: no
  per-user standing charge, no containers idling.
- **The non-hibernation trap is the one way to blow this up.** Cloudflare's
  own worked example: 100 always-active DOs cost **$419.30/mo**, while the
  same workload with WebSocket Hibernation costs **$20.65/mo** — the
  difference is purely duration billing on non-hibernating WebSockets
  ([DO pricing examples](https://developers.cloudflare.com/durable-objects/platform/pricing/)).
  Rule: `acceptWebSocket()` everywhere; never `ws.accept()`; no open TCP
  sockets pinning DOs (§2.2); alarms instead of timers.
- **Writes are the scarce currency.** Rows written cost 1,000× rows read
  ($1.00 vs $0.001 per million). An append-heavy signed journal is affordable
  (50M writes/mo included), but chatty patterns — one row per keystroke,
  per-presence-ping, per-index-touch (indexed writes count extra rows) —
  must be batched. This lands directly on LIST 8/9: ephemeral live traffic
  should flow over WebSockets _without_ journaling every packet (consistent
  with LIST 9's low-retention ephemeral tier), and only settled events become
  rows.
- **WebSocket message billing is discounted 20:1** for DO requests (100
  incoming messages bill as 5 requests), and outgoing messages and protocol
  pings are free — live sync is cheap _if_ hibernated and batched.
- **Egress is free** on Workers and R2, so serving blobs/pages to many
  readers costs operations, not bandwidth — well matched to LIST 14's
  "pull data by ID from whoever has it" reads, at least for the hosted leg.

**Per-user idle cost, concretely:** a dormant user with a 100 MB graph store
costs ≈ $0.02/mo in storage and nothing else, account-wide meters aside.
Cost scales with _activity and writes_, not with registered users.

---

## 6. Component-by-component fit map

Legend: **fits** = maps onto a primitive with no semantic change ·
**fits-with-changes** = viable, named rework required ·
**does-not-fit** = cannot run on Workers; excluded from the hosted edition or
handled by a bridge/native node.

| ID component (pkgs/id)                                                               | Workers primitive                                                  | Verdict               | Notes                                                                                                                                                                                             |
| ------------------------------------------------------------------------------------ | ------------------------------------------------------------------ | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Iroh QUIC transport (`start_node`, QUIC/hole-punching)                               | — none (no UDP, no inbound listen)                                 | **does-not-fit**      | §2.2. Hosted edition is not an Iroh peer.                                                                                                                                                         |
| Iroh relay / DERP-style relaying                                                     | Worker + DO (HTTPS/WS relay)                                       | **fits-with-changes** | Becomes an application-layer store-and-forward relay over WebSocket; LIST 9's optional privacy-relay role fits here.                                                                              |
| Peer discovery (mDNS, DHT — discovery.rs, dht.rs)                                    | Directory D1 + KV                                                  | **fits-with-changes** | mDNS/DHT do not exist at the edge; replaced by a central directory lookup (Google-key → hosted endpoint / public node addresses). Loses decentralization for the hosted leg only.                 |
| P2P RPC protocol (protocol.rs, postcard MetaRequest/Response)                        | HTTPS + WebSocket RPC into the user's DO                           | **fits-with-changes** | Same request/response semantics and serialization can be reused; transport and connection model rewritten.                                                                                        |
| Blob store (store.rs, content-addressed)                                             | **R2** + hash index in DO-SQLite                                   | **fits**              | Content addressing maps 1:1; bytes in R2 (≤5 TiB/object), metadata rows in SQLite; write-once suits R2's 1 write/s/key.                                                                           |
| Tag/graph dual index (tags.rs, tuple.rs, NamespacePair docs)                         | DO-SQLite tables (alpha/omega as SQL indexes)                      | **fits-with-changes** | CRDT doc sync replaced by single-writer serialized SQL inside the owner's DO — simpler locally; cross-user/shared namespaces need an explicit merge protocol over the bridge (§2.2).              |
| Signed objects / chains (chain.rs, tag_claims.rs, envelope)                          | DO-SQLite rows + Wasm/JS verification                              | **fits**              | Verification is pure compute; must be chunked to CPU limits for bulk import/replay.                                                                                                               |
| Event store / journal (event_store/, log_sync.rs)                                    | DO-SQLite append tables + DO alarms                                | **fits-with-changes** | Append + replay fits; replay of long journals must be incremental (CPU/wall limits). Sync becomes push/pull over HTTPS/WS, not gossip.                                                            |
| Gossip (Iroh gossip)                                                                 | WebSocket fan-out from DOs                                         | **fits-with-changes** | Only reaches hosted/connected subscribers; no epidemic propagation. Native gossip stays native-side of the bridge.                                                                                |
| Identity (node keypair, `id id`)                                                     | Google `sub` binding + signed claim (§4.3)                         | **fits-with-changes** | SSO is the door; the ID key remains the identity. Custodial-key shortcut explicitly _not_ recommended.                                                                                            |
| Access / invites / groups / membership (access.rs, invite.rs, groups/, membership/)  | DO-SQLite rows, enforced in the DO                                 | **fits**              | Per-user DO is the natural enforcement point; allowed-writer lists (LIST 7) are SQL rows.                                                                                                         |
| Worlds — shared journaled world state (world_store.rs, world_records.rs)             | One DO **per world** + its SQLite                                  | **fits-with-changes** | Journal in SQLite; all writers serialize through the world DO (~1,000 req/s soft cap per world).                                                                                                  |
| Worlds — sandboxed program execution (sandbox.rs, world_compile.rs, world_native.rs) | Wasm in Worker/DO                                                  | **fits-with-changes** | Only if the world VM is single-threaded, syscall-light Wasm (§2.1); native-code worlds do not fit. CPU-capped per event (30 s default / 5 min max) — long turns must checkpoint into the journal. |
| Worlds — web rendering (web/, Axum UI)                                               | Worker serving HTML/assets + world DO                              | **fits**              | Static assets free; dynamic rendering is ordinary Worker work.                                                                                                                                    |
| Worlds — SSH rendering (world_ssh.rs)                                                | —                                                                  | **does-not-fit**      | No inbound TCP/SSH listener in the GA runtime (§2.2). SSH stays a native-node surface. (Inbound TCP via Spectrum is only newly announced/beta — not a foundation to design on.)                   |
| Collaborative editor / live sessions (web WebSocket editing)                         | DO + Hibernation WebSockets                                        | **fits**              | The platform's flagship use case; batch small messages per §1.3.                                                                                                                                  |
| REPL / CLI (repl/, cli.rs)                                                           | Native client → HTTPS/WS API                                       | **fits-with-changes** | CLI stays native; it gains a "hosted remote" transport beside Iroh remotes.                                                                                                                       |
| Scheduled/background work (sync, expiry, cleanup)                                    | Cron Triggers + DO alarms + Queues                                 | **fits**              | Alarms wake individual user/world DOs; no always-on daemon.                                                                                                                                       |
| Large media / file transfer                                                          | R2 + HTTP range/streaming                                          | **fits**              | Egress free; chunked/BitTorrent-style swarming (LIST 14) does **not** fit — hosted leg serves whole objects over HTTP only.                                                                       |
| Abuse controls (LIST 8: rate limits, budgets, blocks)                                | DO per user/topic as the serialization point + Worker-level checks | **fits**              | A DO is a natural rate-limit/budget counter with strong consistency; per-IP/ID/topic budgets are rows + checks on the write path. Row-write pricing (§5) argues for coarse-grained counters.      |

---

## 7. Recommended shape of the hosted edition (for the design phase)

1. **One Worker** (API + web UI + OIDC endpoints) fronting:
   - **UserDO** (one per Google `sub`): the user's graph, tag indexes,
     journal, invites/membership, live WebSocket sessions — SQLite-backed,
     hibernating sockets only.
   - **WorldDO** (one per shared world): journal + serialized event
     application, Wasm world logic where portable.
   - **Directory D1** (single, shared): hashed Google-subject → user record,
     public-key bindings (§4.3), public-server/name directory for @/#
     resolution.
   - **R2 bucket(s)**: content-addressed blobs; **KV**: public profile /
     discovery cache only.
2. **Bridge protocol** (new, transport-level only): native `id` nodes and
   UserDOs exchange the existing signed objects over HTTPS/WebSocket push/pull.
   No trust-model changes — signatures verify identically on both sides.
3. **Hard rules** carried into implementation: hibernating WebSockets only;
   journal writes batched; ephemeral live traffic never journaled per-packet;
   no custodial signing keys; blobs never in SQLite; world execution
   checkpointed to fit CPU caps.

## 8. Open questions for follow-on investigations

- World VM portability audit: which parts of world execution are Wasm-clean
  (single-thread, no filesystem/sockets) vs native-bound?
- Bridge sync semantics: how do CRDT doc merges (tags/namespaces) reconcile
  when one side is a serialized-SQL DO — is the DO an authoritative writer, a
  CRDT replica, or a snapshot holder (cf. LIST 11's issuer-snapshot model)?
- Shared-world throughput: one DO per world caps at ~1,000 req/s soft —
  sharding strategy for hot worlds?
- Key recovery: Google account loss vs ID key loss, and what the §4.3 binding
  allows each side to recover.
- Data-residency/jurisdiction pinning for DOs (relevant if hosted data carries
  compliance expectations) — not covered in this pass.

## Sources (official documentation, accessed 2026-10-28)

- Cloudflare Durable Objects — overview, limits, pricing, WebSocket/hibernation:
  https://developers.cloudflare.com/durable-objects/ ·
  https://developers.cloudflare.com/durable-objects/platform/limits/ ·
  https://developers.cloudflare.com/durable-objects/platform/pricing/ ·
  https://developers.cloudflare.com/durable-objects/best-practices/websockets/
- Cloudflare Workers — limits, pricing, Wasm, TCP sockets:
  https://developers.cloudflare.com/workers/platform/limits/ ·
  https://developers.cloudflare.com/workers/platform/pricing/ ·
  https://developers.cloudflare.com/workers/runtime-apis/webassembly/ ·
  https://developers.cloudflare.com/workers/runtime-apis/tcp-sockets/
- Cloudflare D1 / KV / R2 limits:
  https://developers.cloudflare.com/d1/platform/limits/ ·
  https://developers.cloudflare.com/kv/platform/limits/ ·
  https://developers.cloudflare.com/r2/platform/limits/
- Cloudflare Access identity providers (Google Workspace, generic OIDC):
  https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/google-workspace/ ·
  https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/generic-oidc/
- Google — OpenID Connect (server flow, discovery, `sub` claim):
  https://developers.google.com/identity/protocols/oauth2/openid-connect
- Subject system: `pkgs/id/ARCHITECTURE.md`, `pkgs/id/README.md`,
  `pkgs/id/src/` in developing-today/code.
