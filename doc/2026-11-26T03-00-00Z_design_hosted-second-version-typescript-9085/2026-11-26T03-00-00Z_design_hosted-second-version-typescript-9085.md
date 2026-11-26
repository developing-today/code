# Design (component G): the TypeScript second version, its WASM verdicts, transports and direct-Turso prudence

Status: design record for issue #9085, component **G** of epic #9078 (hosted
editions program). Docs only; no code changes in this record's commit.
Written against `origin/main` at `647b4082`, 2026-11-26.

Inputs (answered here, not restated):

- Issue #9085's scope and acceptance; program note
  `doc/2026-11-12T23-04-00Z_design_hosted-editions-workers/note.md` (S11 to
  S14, open question 5); the two binding amendments on the issue (dual tier-1:
  parity plus deliberate divergence, never porting down; #9106: REST and
  WebSocket are first adapters, not the architecture).
- Neighbours: C (`doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition/`,
  which defers transport framing and the program prudence statement to G in
  §3.3 and §8 item 1), E (`doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture/`,
  transport seam §5.5, credential model §7), F
  (`doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/`),
  A (`doc/2026-11-23T03-22-03Z_design_pm-service-workers-edition/`) and D's
  IndexedDB decision (`doc/2026-11-23T18-08-00Z_decision_id9082_indexeddb_request_batch/`).
- Platform facts from `doc/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/`
  (no UDP, 128 MB per isolate including WASM, no WASM threads, CPU caps,
  32 MiB DO WebSocket message) and the world-VM audit
  `doc/2026-10-30T19-36-00Z_investigation_world_vm_wasm_portability/`.
- Issue states at writing time: A #9079 open, B #9080 open, C #9081, D #9082,
  E #9083 and #5671 closed.

## 1. Problem

The hosted editions cannot run the Rust binaries: Workers have no UDP (so no
Iroh/QUIC peer), no threads, no filesystem, and a 128 MB isolate. The program
note (S11) says the hosted editions are therefore substantially a second
version in TypeScript. That second version is no longer hypothetical. Three
TypeScript Workers already carry it:

- `pkgs/id/hosted-worker/` (about 9,600 lines in its top-level `src/*.ts`): per-tenant
  `UserDO`, the signed-object bridge, a TypeScript canonical CBOR
  (`src/canonical-cbor.ts`, a mirror of `crates/id-core/src/canonical_cbor.rs`),
  a pure TypeScript BLAKE3 (`src/blake3.ts`), Ed25519 over WebCrypto
  (`src/ed25519.ts`), range reconciliation transcribed from `id-events`'
  `range.rs` (`src/range.ts`), native ingest semantics mirrored on SQLite
  (`src/bridge-store.ts`), and E's credential issuance (`src/credentials.ts`).
- `pkgs/fleet/hosted-worker/` (fleet-hosted): sessions, stamping, query, the
  queue fallback broker, kv, ingestion, sync, and a direct-read token mint
  (`src/direct-read.ts`) that states C's interim prudence rules and says G
  supersedes them.
- `pm-stack/pm-service/`: the PM tenant store and read API, ported from the
  laptop aggregator, which is itself TypeScript (`pm-stack/aggregator/`).

Each was built against its own component design, and the cross-cutting
choices were made locally. The evidence of drift is already in the tree:

1. BLAKE3 is implemented twice in TypeScript (`pkgs/id/hosted-worker/src/blake3.ts`
   and `pkgs/fleet/hosted-worker/src/blake3.ts`, the second using the first as
   its test oracle).
2. Parity with Rust is kept three different ways: a shared fixture file read
   by both suites (`pkgs/id/tests/fixtures/port-v1.json`, read by
   `pkgs/id/tests/conformance.rs` and `pkgs/id/browser/test/indexeddb_conformance.test.ts`),
   values printed by a Rust test and pinned by hand in TypeScript
   (`pkgs/id/tests/bridge_fixtures.rs`), and golden tables copied into each
   suite (the canonical CBOR table "shared with the Rust suite" in
   `test/canonical-cbor.test.ts`). Only the first catches a one-sided change.
3. The PM port copies laptop TypeScript instead of importing it
   (`pm-stack/pm-service/src/collector/pool.ts` is described as "a port of"
   the laptop module), so a second implementation exists where a shared one
   was possible.
4. E's code issues `readwrite` Turso tokens (`credentials.ts` accepts the
   scopes `read` and `readwrite`, with a 15 minute cap), while C's rules
   forbid every direct write. Turso tokens are database-wide (the `a` claim is `read-only` or
   `full-access`, per `direct-read.ts`), so a `readwrite` token can write
   every table in the tenant DB, including the ones the service gates.
5. The next Rust logic the hosted side needs (the #9106/B32 acceptance gate at
   hosted ingress, E §5.2: signer authorized, scope authorized) is
   authorization and group logic, where a second hand-written implementation
   is a security risk, not a maintenance cost.

## 2. Goals and non-goals

Goals:

- State plainly what the second version covers (A, B, C) and what is not
  rewritten.
- One verdict per piece of Rust logic the hosted side needs: platform
  primitive, TypeScript rewrite, or WASM reuse (in a Worker, in the browser,
  or both), with the criteria and the parity mechanism.
- A base-function inventory per component and a transport contract for each
  function, including offline and reconnect behaviour.
- The program-wide definition of "prudent" for direct Turso access.
- One shared-semantics discipline and a divergence protocol.

Non-goals:

- Re-designing identity (#9077, H #9086), tenancy (E), media placement (F),
  or the store port (D). This record consumes them.
- Choosing NATS or any later transport adapter (#9106 §0 owns the matrix).
- Implementing anything, or authorizing any deployment or billable resource.
  The deployment gates already named in `pkgs/id/hosted-worker/README.md`
  (#2285, #3403) stand.
- Making the hosted edition a reduced edition. Where Workers cannot do a
  thing, it is a named deliberate divergence, Rust-side first or only.

## 3. Scope of the second version

**Covered.** The hosted editions of A (PM service), B (ID service) and C
(fleet tool), each as TypeScript on Workers and Durable Objects, plus the
browser client code that talks to them.

The phrase "second version" means three different things, and the verdicts
differ because of it:

| Component | Native (first) edition                                                                 | What the hosted edition is                                                                                              |
| --------- | -------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| A PM      | TypeScript on bun (`pm-stack/aggregator/`, about 580 `.ts` files) plus laptop scripts  | A runtime port of the same language. Pure logic should be shared source, not a second implementation (§4.3).            |
| B ID      | Rust (`pkgs/id/crates/*`: id-core, id-store, id-events, id-sync, id-grants, id-groups) | A true second implementation of selected semantics. This is where the WASM verdicts matter.                             |
| C fleet   | Rust binary (`pkgs/fleet/crates/*`) plus laptop Python and shell tools                 | A second front door onto shared contracts (`pkgs/fleet/repo-task-v1.schema.json`, the `pm_meta: 1` block); C's §6 rule. |

**Not rewritten** (deliberate divergences, Rust-side only, per the dual tier-1
framing): the Iroh/QUIC peer transport, hole punching, gossip, mainline DHT
lookup (`id-sync` feature `dht`); the wasmtime host (replaced by an isolate
host, guests unchanged, per the 2026-10-30 audit; `pkgs/id/worker-poc/`
demonstrates it); Roc compilation (`world_compile`); SSH and terminal surfaces;
threaded or long-running world guests; the redb backend (D keeps it native);
the Android native client (`pkgs/id/crates/id-client`); the native TUI and
hardware shells; the fleet laptop agent, proxy and dispatch crates; the queue
core itself (C: a second front door, not a second queue); and Kaneo, Pad and
Grist (A §4).

## 4. Rewrite or WASM, per piece of logic

### 4.1 Criteria

Each row is judged on three criteria, in this order:

1. **Correctness risk.** Cryptography and authorization rank highest: a
   subtle mismatch is a vulnerability or a forked trust decision, not a bug
   report. A hand-written second implementation of a protocol with few
   audited implementations is refused.
2. **Divergence.** How actively the Rust semantics change (the `id-events`
   and `id-store` history is busy: the three newest commits on main touch
   thread resolution events, prune and terminal lines), and whether a
   language-neutral fixture can pin the behaviour completely.
3. **Bundle and runtime limits.** The 128 MB isolate includes WASM memory;
   no WASM threads; startup is 1 s CPU; the compressed upload limit for a
   Worker (inferred from Cloudflare's published limits, to be re-checked by
   slice G9) is a few MB to about 10 MB depending on plan; every JS to WASM
   call has a boundary cost, so WASM is used at coarse boundaries (verify a
   batch, fold a log), never per field.

A fourth option, "keep server-side only", in this program means "native
editions only" (the Rust binaries and self-hosted peers), since every hosted
server-side piece is itself a Worker.

### 4.2 Verdict table (ID logic, plus the shared primitives)

| Rust logic (source)                                                                                       | Runs where in hosted                      | Verdict                                                                                                        | Why                                                                                                                                                                                      | Parity mechanism                                                                                                     |
| --------------------------------------------------------------------------------------------------------- | ----------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Ed25519 sign/verify (`ed25519-dalek`)                                                                     | Worker, browser                           | **Platform primitive** (WebCrypto), as `src/ed25519.ts` does today                                             | Audited platform code beats both options. Known gap: dalek's `verify_strict` versus WebCrypto on small-order keys, documented in `ed25519.ts`.                                           | RFC 8032 vectors plus an edge-case vector set (small-order, non-canonical) pinning accept/refuse in both (slice G4). |
| BLAKE3 (`blake3` crate, operation IDs, content IDs)                                                       | Worker                                    | **TypeScript, one module** (consolidate the two copies); WASM only if G8 measures a CPU problem on large blobs | Spec-defined pure function, fully pinned by the official vectors; no WebCrypto BLAKE3. Two copies is the divergence, not TypeScript.                                                     | Official vectors from one shared file (G1).                                                                          |
| Canonical CBOR (`id-core/src/canonical_cbor.rs`)                                                          | Worker, browser                           | **TypeScript** (exists)                                                                                        | Closed value model and typed rejections are completely fixture-pinnable; small; hot path for signature bytes.                                                                            | Golden table moved to one shared file, plus a seeded differential corpus (the `id://` codec's pattern).              |
| `id://` URI codec (`id-core/src/id_uri*.rs`)                                                              | Worker, browser                           | **TypeScript**                                                                                                 | Same reasoning; the seeded property corpus already pins it (`test/id-uri-vectors.test.ts`, digest shared with `crates/id-core/tests/id_uri_property.rs`).                                | Seeded corpus digest (exists).                                                                                       |
| Event decode and chain rules, bridge ingest, range reconciliation (`chain.rs`, `event_store`, `range.rs`) | Worker (`UserDO`)                         | **TypeScript, frozen to bridge v1 scope** (header-level rules; payload semantics not interpreted)              | Exists and is fixture-checked; the bridge store is a sync peer that re-signs nothing. Extending TypeScript into payload folds would multiply divergence on the busiest crates.           | Fixtures produced by Rust into one shared file and read by both suites (G1), not printed and pasted.                 |
| Grants authorization check (`crates/id-grants`, serde only, about 1,200 lines)                            | Worker ingress gate; browser for previews | **WASM reuse** (`wasm32-unknown-unknown`)                                                                      | Authorization is top of criterion 1, the crate is evolving (#14790 slices), and it already has no I/O dependencies, so it compiles without refactoring (inferred from its `Cargo.toml`). | Rust tests run unchanged; a shared decision-table fixture exercised through the WASM binding (G5).                   |
| Group fold and projection (`id-groups/src/groups`, `id-events` group projection)                          | Worker ingress gate                       | **WASM reuse after extracting a pure core**                                                                    | Same risk class as grants, but the crate pulls tokio and iroh-blobs today, so a feature-gated pure fold must be split out first (feasibility spike, G6).                                 | Same as grants.                                                                                                      |
| Group encryption (`id-groups/src/encryption`, vendored `p2panda-encryption` DCGKA)                        | Browser only                              | **WASM, client-side only.** Never in a Worker                                                                  | A messaging-layer key agreement must not be re-implemented. The server never holds group keys (E §2.3: no keys central).                                                                 | Rust suite; browser smoke over the WASM build.                                                                       |
| Key envelope and seed phrase (`id-core/src/key_envelope.rs`: Argon2id, XChaCha20-Poly1305)                | Browser only                              | **WASM, client-side only**                                                                                     | WebCrypto has neither Argon2 nor XChaCha20; a TypeScript Argon2 is slow and easy to get wrong. Server never sees the passphrase.                                                         | Envelope files made by Rust open in the browser build and the reverse (shared fixture).                              |
| OPAQUE login (`id-core/src/opaque_login.rs`, `opaque-ke`, RFC 9807)                                       | Browser (client) and Worker (server)      | **WASM both sides**                                                                                            | Few audited implementations; the ciphersuite (ristretto255, Argon2 KSF) must match byte for byte. The KSF runs on the client, so the Worker side carries no Argon2 cost.                 | A registration and login transcript fixture run across the four client/server pairings.                              |
| World guest execution (wasmtime host)                                                                     | Worker DO                                 | **Guest WASM as-is, host in TypeScript** (2026-10-30 audit; `pkgs/id/worker-poc/`)                             | The guest is the portable artifact.                                                                                                                                                      | `pkgs/id/worker-poc/compare/native-reference.jsonl` journal comparison (exists).                                     |
| Store backends (`id-store` redb, `port_adapter.rs`)                                                       | Worker, browser                           | **TypeScript adapters** to D's port contract                                                                   | I/O edges cannot be WASM'd usefully; D already decided this shape.                                                                                                                       | `pkgs/id/tests/fixtures/port-v1.json` (exists; the model for G1).                                                    |

### 4.3 Verdicts for A and C

- **A (PM).** Not a rewrite question. Pure TypeScript logic (pool parsing,
  the genealogy read model, roadmap derivation) should be imported from one
  shared module by both the laptop aggregator and `pm-service`; only the
  runtime edges differ (bun:sqlite versus Hrana over HTTP in
  `src/tenant-store/libsql-http.ts`; systemd versus Cron Triggers). Slice G11,
  handed to A.
- **C (fleet).** TypeScript against language-neutral contracts, as C decided:
  `repo-task/v1` JSON Schema (validated natively in `fleet-queue/src/validate.rs`
  and in `src/queue-validate.ts`), the Metadata block, and the session id
  format. Stamping is TypeScript and authoritative in the hosted world; the
  native stamping layer (#9073) is not built yet, so the shared stamp fixture
  corpus C names becomes the spec the Rust side meets later. No WASM is
  needed in C.

## 5. Transports

### 5.1 Contract classes

Every base function is described transport-neutrally as an operation with a
request, a result, an idempotency key (for writes) and, for sessions, a
resume token. REST and WebSocket are the first two adapters (#9106 §0). Three
contract classes:

- **RR, request/response.** JSON (or CBOR for signed bytes) over HTTPS.
  HTTP/1.1, HTTP/2 and HTTP/3 are negotiated by Cloudflare's edge with the
  client; the Worker sees a `fetch` either way, so QUIC is available to
  clients "where the platform provides it" without any Worker code (inferred
  from the investigation's §2.2: no Worker-visible UDP or QUIC). Every write
  carries a client-chosen operation id; a retry after an ambiguous outcome
  returns the first outcome, as D's batch contract and the Turso profile's
  "readback after an ambiguous outcome" already require.
- **ST, bounded streaming.** Chunked HTTP bodies (`ReadableStream`) for
  exports, blob upload and download, and large query results. Resumable by
  byte range or cursor, never by "start over".
- **RT, realtime session.** One ordered binary WebSocket channel to a
  Durable Object, as the bridge v1 wire already is (CHALLENGE/AUTH, HELLO,
  RESUME, RANGE, WANT/BATCH/ACK, DONE/TOKEN in
  `doc/2026-11-23T05-00-00Z_design_bridge_v1_implementation/`).

The rule that makes "all base functions work over REST or WebSocket" true:
**every base function has an RR or ST form.** RT is an additional form for
latency and push, never the only one, because some networks and proxies
block WebSocket and because a CLI should not need a socket to do one thing.
The bridge already follows this (`POST /v1/bridge/push`, `POST /v1/bridge/pull`
beside `GET /v1/bridge/ws`). Server-Sent Events are not a contract class: a
streaming response holds a Durable Object in memory (inferred), while a
hibernating WebSocket does not bill while idle (investigation §1.3).

### 5.2 Base-function inventory

Routes are cited where they exist; rows without a route are design-level.

| Component | Base function                               | Class           | Route today (or owner)                                                      |
| --------- | ------------------------------------------- | --------------- | --------------------------------------------------------------------------- |
| A         | Board registry and projects view            | RR              | `GET /api/projects.json` (`pm-service/src/read-api/api.ts`)                 |
| A         | Item detail with genealogy                  | RR              | `/api/item/<id>.json` (A §4)                                                |
| A         | Roadmap, releases                           | RR              | `GET /api/roadmap.json`, `GET /api/releases.json`                           |
| A         | Dashboard pages                             | RR (HTML)       | `src/dashboard/dashboard.ts`                                                |
| A         | Live board notices and presence             | RT, RR fallback | `BoardDO` (A §2, not built); fallback is RR with a snapshot version or ETag |
| A         | Collector run, reconcile run                | internal        | Cron Trigger and `ReconcileDO`; not client functions                        |
| B         | Tenant state, usage                         | RR              | `GET /v1/state`, `GET /v1/usage`                                            |
| B         | Key-value records                           | RR              | `PUT/GET/DELETE /v1/kv/<key>`                                               |
| B         | Blobs                                       | ST              | `PUT /v1/blobs`, `GET/DELETE /v1/blobs/<hash>`                              |
| B         | Export (tenant, bridge)                     | ST              | `GET /v1/export`, `GET /v1/bridge/export`                                   |
| B         | Signed-object sync                          | RT, RR form     | `GET /v1/bridge/ws`; `challenge`, `heads`, `push`, `pull` over HTTPS        |
| B         | Direct-Turso credential issuance            | RR              | `issueCredential` in `src/credentials.ts` (route owned by E4)               |
| B         | Public DB read                              | RR or direct    | E §6: Worker route or read token                                            |
| C         | Sessions open, resume, resolve, close       | RR              | `POST /v1/sessions[/resume,/resolve,/close]`                                |
| C         | Stamp                                       | RR              | `POST /v1/stamps`                                                           |
| C         | Metadata query                              | RR (ST later)   | `GET/POST /v1/query`                                                        |
| C         | Queue claim, heartbeat, release, status     | RR              | `/v1/queue/fallback/*`                                                      |
| C         | kv pipeline, kv blobs                       | RR, ST          | `/v1/kv/v2/pipeline`, `/v1/kv/blobs*`                                       |
| C         | Ingestion (GitHub webhook), backfill, sweep | RR              | `/v1/ingestion/*`                                                           |
| C         | Sync tools                                  | RR              | `POST /v1/sync/project`, `POST /v1/sync/queue-github`                       |
| C         | Agent message bridge                        | RR (RT later)   | `/v1/bridge/send`, `/v1/bridge/messages`, `/v1/bridge/agents`               |

### 5.3 Offline and reconnect

- **Writes are idempotent by operation id.** Slice G3 audits every RR write
  route above for a client-chosen operation id and adds it where missing
  (ingestion and the currency ledger already dedupe; most other routes have
  no explicit key today, per a grep of the three workers).
- **RT reconnect.** On close or DO eviction, the client reconnects with
  exponential backoff and jitter and presents its resume token. Positions in
  a resume are evidence; the server re-derives truth with the range exchange
  (bridge v1). If the token is refused or expired, the client starts a fresh
  session; correctness never depends on resume succeeding, only cost does.
- **RR fallback for RT.** A client that cannot hold a WebSocket polls the RR
  form with its last version or cursor. For A this is the snapshot version;
  for B it is `heads` plus `pull`.
- **Offline per component.** B: native and browser clients keep writing
  signed events to their own store (browser: D's IndexedDB adapter, outbox
  keyed by operation id) and sync when reachable (E §5.4). A: the dashboard
  shows the last snapshot with its as-of time and accepts no writes (writes
  are service-side). C: stamping and queue claims require the service and
  fail closed with a named error when it is unreachable; the CLI never stamps
  locally (C §4, non-forgeability). A claim lease that lapses during a
  disconnect is lost; the client must heartbeat and check the lease
  generation before acting after reconnect.
- **Limits that shape framing.** Bridge v1's bounds (64 KiB event, 64 events
  or 1 MiB per batch, 16 MiB frame) sit under the 32 MiB DO WebSocket
  message cap; RR bodies follow the same per-batch bounds so the two forms
  accept the same work.

## 6. Direct Turso access: what "prudent" means (S14)

The governing fact: **a Turso token's scope is a whole database**, not a
table or view (`a` is `read-only` or `full-access`, `id` is the database;
`pkgs/fleet/hosted-worker/src/direct-read.ts`). Anything in a database is
reachable by every holder of a token for it. "Prudent" therefore means:

1. **Own or public, never other.** The token audience is the caller's own
   tenant database or the public database, minted by the tenancy plane from a
   live session under E §7's refusal order, TTL at most 15 minutes
   (`MAX_TTL_SECONDS` in `credentials.ts`), recorded by `jti`. Delegates,
   agents acting under another owner's grant, and group members never get a
   direct token to someone else's database; their access goes through the
   service, which can evaluate the grant.
2. **Read is the default, and the only scope for user-facing callers.** A
   direct write bypasses everything the service enforces on the same tables:
   the per-object acceptance gate and fork detection (B), stamping (C), the
   storage ceiling and metering (`USER_CEILING_BYTES` in `UserDO`), derived
   state kept by Durable Objects (the reference index, `BoardDO`, the writer
   lease), and audit. Because the token cannot be limited to "safe" tables,
   G rules that `readwrite` is not issued to user-facing callers (browser,
   CLI, agents) under the current one-database-per-tenant layout. Whether an
   owner's own native node may hold one is open question 1.
3. **Never direct, whatever the scope:** stamping or provenance; the queue
   database (C); the tenancy database (E); any write to the public database
   (E §6: publication only through `PublishIndex`); publication and
   retraction; credential issuance; any authorization decision beyond "this
   is my database" (grants, roles, delegation, F's reference rule); any
   cross-tenant read or join.
4. **Nothing sits in a database that its readers may not read.** For the
   public DB this means a retraction must remove or tombstone the content
   row, not only set `retracted_at`, or a public read token keeps serving
   retracted content. For a tenant DB it means no row in it may be hidden
   from its owner by service logic alone.
5. **Direct reads are advisory.** A client may display or cache what it reads
   directly, but any decision that authorizes something is re-checked by the
   service. Direct reads may lag the service's own writes.
6. **Versioned read contract.** Direct readers bind to a documented read
   schema with a schema version row; a migration either keeps it or bumps
   the version, and a client that sees an unknown version falls back to the
   RR route.
7. **Token hygiene.** Tokens live in memory only, are refreshed through the
   authenticated session before expiry, and are refused as service
   authentication (already enforced: `data_plane_credential_refused` in
   `pkgs/fleet/hosted-worker/src/auth.ts`). Suspension revokes all of a
   tenant's credentials (E §7).
8. **Worth it only where RR is the wrong shape:** bulk and analytic reads
   (the collector, A's dashboard, query consumers), a read replica of one's
   own data, and high-volume public browsing. Low-volume reads use RR.

These rules supersede C's interim §3.3 rules only by generalizing them; C's
rules (read-only, metadata DB only, no queue, no stamping) remain correct
instances. One applicability note: B's tenant data currently lives in
Durable Object SQLite (`UserDO`), not in a per-user Turso database, so direct
Turso applies to B only once B and E settle where ID tenant data is homed
(inferred from the code; B #9080 question 2 owns it).

## 7. Shared-semantics discipline

1. **One fixture home per contract, read by both suites.** The model is
   `pkgs/id/tests/fixtures/port-v1.json`. Golden tables copied into each
   suite and values pasted from Rust output are retired in favour of files
   both suites load (G1). A fixture is the contract; a suite that disagrees
   with it is the bug.
2. **Contract first.** A change to a shared schema or wire format lands as a
   schema and fixture change first, implementations second (C §6, made
   program-wide). A Rust change to an `id-events` or `id-store` semantic that
   the TypeScript mirror implements must update the shared fixture in the same
   change, so the TypeScript suite fails until it follows.
3. **One TypeScript module per primitive.** Shared primitives (BLAKE3,
   canonical CBOR, hex and base64url helpers, JWT parsing) live in one
   internal TypeScript package that all Workers import (G2).
4. **Deliberate divergence is registered.** Each component design keeps its
   list of deliberate divergences (A §4 already does); a capability route
   (`/capabilities`, as fleet-hosted has) reports what the edition supports,
   so a peer can tell "not supported here" from "broken".
5. **WASM builds are one source of truth, not a third implementation.** The
   Rust crate's own tests are the WASM build's tests; the binding layer is
   thin and covered by the shared fixtures.

## 8. Alternatives considered

- **Do nothing; each component decides locally.** Cheapest now, and it is
  what produced the duplicate BLAKE3, three parity styles and the
  readwrite/read-only conflict in §1. Rejected.
- **Rust core compiled to WASM, TypeScript as a thin shell.** Most ID crates
  depend on tokio, iroh and redb; the I/O edges need rewriting anyway; WASM
  memory counts against 128 MB; no threads. Rejected as a strategy, kept per
  row where §4.1 favours it.
- **Rewrite everything in TypeScript.** Simplest toolchain, but a second
  OPAQUE, DCGKA, Argon2 or authorization implementation fails criterion 1.
- **Whole services in Rust on Workers (workers-rs).** Possible (investigation
  §2.1), but the hosted code is TypeScript and the tokio-based crates still
  would not come along. Not chosen; open question 5.
- **Turso replication as the sync protocol.** Rejected by E §5.1; G agrees.
- **WebSocket only, or REST only.** The first fails behind restrictive proxies
  and burdens one-shot CLI calls; the second gives up push and the bridge's
  ordered session.
- **`readwrite` direct tokens, trusting clients.** The token is
  database-wide, so it bypasses every gate in §6 rule 2. Rejected.

## 9. Slices

Each is its own future issue, filed under #9085 or handed to the named owner
with a cross-link, in dependency order.

| Slice | What                                                                                                                                                                 | Acceptance                                                                                        | Owner  |
| ----- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | ------ |
| G1    | Shared fixture home: canonical CBOR golden table, BLAKE3 vectors and bridge event fixtures move into files both the Rust and TypeScript suites read.                 | Deleting a vector from the file fails both suites; no transcribed table remains for these three.  | G      |
| G2    | One internal TypeScript primitives package: BLAKE3 (keeping the streaming hasher), canonical CBOR, encoding helpers.                                                 | Both Workers import it; their suites pass unchanged.                                              | G      |
| G3    | Idempotency audit: every RR write in §5.2 takes an operation id and returns the first outcome on retry.                                                              | A retry test per write route.                                                                     | G      |
| G4    | Ed25519 edge-case vectors (small-order, non-canonical) with expected accept or refuse in both suites.                                                                | Any accepted difference is documented beside the vector.                                          | B      |
| G5    | `id-grants` as WASM: `wasm32-unknown-unknown` build in the id flake (which already pins that target for examples), thin binding, shared decision-table fixture.      | Hosted ingress gate calls it; bundle size and cold-start CPU recorded.                            | B      |
| G6    | Pure group-fold core: feasibility spike for a feature-gated fold without tokio or iroh-blobs, then the WASM build if the split is clean.                             | Spike reports clean or not; build follows only if clean.                                          | B      |
| G7    | Client crypto WASM: OPAQUE client, key envelope, seed phrase in the browser; OPAQUE server in the Worker.                                                            | Four-way transcript fixture passes; browser bundle measured against the budget (open question 3). | B, H   |
| G8    | BLAKE3 CPU measurement for large blobs in a Worker, TypeScript versus WASM with SIMD.                                                                                | Recorded threshold and a keep-or-switch verdict.                                                  | G      |
| G9    | Bundle and limit check in CI per Worker; re-verify the platform limits cited in §4.1.                                                                                | CI fails over budget; limits doc updated with sources.                                            | G      |
| G10   | Prudence enforcement: no `readwrite` to user-facing callers (pending open question 1); public-DB retraction removes or tombstones content; read-schema version rows. | The §11 negative tests pass.                                                                      | E line |
| G11   | PM shared source: pure aggregator modules extracted into a package both editions import; copied versions removed.                                                    | No "port of" copies remain for the extracted modules.                                             | A      |
| G12   | Transport conformance test per Worker: every base function has an RR or ST form; every RT session survives a forced disconnect.                                      | Test in each Worker's suite.                                                                      | G      |

## 10. Rollout and rollback

- G1, G2 and G4 change no behaviour and land first; rollback is a revert.
- WASM modules (G5 to G7) land beside the path they serve. Where a TypeScript
  path exists, a differential mode runs both and logs mismatches before the
  switch. Where none exists (grants, group fold), hosted ingress fails closed
  on any WASM load or evaluation error and never falls back to an unchecked
  path, so rollback costs availability, not safety.
- G10 is a policy tightening, reverted by reverting the policy; its only data
  effect removes rows that were already retracted.
- Transport changes are additive; removing or changing a route needs a
  `/capabilities` version change and a deprecation window for native peers.
- Nothing here deploys anything (§2).

## 11. Verification

- Both suites pass on every shared fixture, in CI, on every change to either
  side (G1). A one-sided change to a fixture-covered semantic fails CI.
- Differential mode mismatch count is zero over a stated soak period before
  any WASM switch.
- Bundle size and cold-start CPU are recorded per Worker and held to budget
  (G9).
- Negative tests for prudence: a `readwrite` request from a user-facing
  caller is refused; a direct token presented as service auth is refused
  (exists in fleet-hosted); a token for one database fails against another;
  a retracted public row is unreadable with a public read token.
- Transport conformance (G12) plus the still-open live native-against-hosted
  bridge session test named in the bridge v1 record.

## 12. Open questions

Questions only Drew can decide are listed as open, with a recommendation
where there is one; none is decided here.

1. **Owner's native node and `readwrite`.** May an owner's own Rust node hold
   a `readwrite` token (or an embedded replica with writes) to its own tenant
   DB? Recommendation: no, until it writes through D's Turso store profile
   with the same acceptance gate and metering. Decider: Drew.
2. **Anonymous public reads.** Do public-DB read tokens go to anonymous
   readers (E §6 permits it) or only to signed-in ones, given cost and abuse?
   Decider: Drew.
3. **Browser crypto bundle budget.** What download size is acceptable for the
   client WASM bundle (OPAQUE, envelope, group encryption)? Decider: Drew.
4. **WASM in the build.** Adding `wasm32` builds of id crates to Nix and CI
   costs build time. Acceptable for grants and group fold? Decider: Drew.
5. **Rust on Workers.** Should workers-rs stay closed as an option, or be
   allowed for a hot path G8 or G9 identifies? Decider: Drew.
6. **Where ID tenant data is homed** (DO SQLite or per-user Turso). Decider: B
   (#9080) with E; it decides whether §6 applies to B at all.
7. **Group-fold extraction.** Whether the pure core splits cleanly is a
   technical finding of G6, not a judgment call. Decider: G6's result.

## 13. Acceptance mapping (#9085)

| Acceptance item                                 | Where                                       |
| ----------------------------------------------- | ------------------------------------------- |
| Per-component rewrite/WASM table                | §3, §4.2, §4.3                              |
| Base-function transport inventory and contracts | §5                                          |
| Direct-Turso prudence rules                     | §6                                          |
| Shared-semantics discipline                     | §7                                          |
| Granular implementation issues                  | §9 (to be filed; none filed by this record) |
