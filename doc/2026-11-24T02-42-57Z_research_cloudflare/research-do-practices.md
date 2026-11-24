# Durable Objects — Architecture Truths, Pitfalls, and Best Practices

Researched 2026-10-20. Primary sources are official Cloudflare docs
(developers.cloudflare.com) and the Cloudflare blog. Community sources are
used second and are explicitly marked **[community]**. Claims I could not
verify against a page I actually opened are flagged **UNVERIFIED**.

Headline official sources:

- Rules of Durable Objects (official best-practices guide, updated 2026-10-18):
  https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/
- Changelog announcing the guide (2025-12-15):
  https://developers.cloudflare.com/changelog/post/2025-12-15-rules-of-durable-objects/
- Workers / Durable Objects pricing (official):
  https://developers.cloudflare.com/workers/platform/pricing/
- Workers limits (official): https://developers.cloudflare.com/workers/platform/limits/

---

## 1. Does a Durable Object have to store data in its own storage? (Short answer: No)

### 1.1 DO storage is optional — a DO can be pure coordination/compute

Nothing in the platform requires a Durable Object to call its storage API.
A DO is a class with RPC methods / a `fetch()` handler plus a `ctx` object;
`ctx.storage` is an available API, not a toll every object must pay. The
official Rules guide describes three _state layers_ a DO may use, one of
which is external storage — i.e. the object's meaningful state can live
entirely outside its own storage:

| Type                         | Speed    | Persistence                  | Use case                    |
| ---------------------------- | -------- | ---------------------------- | --------------------------- |
| In-memory (class properties) | Fastest  | Lost on eviction or crash    | Caching, active connections |
| SQLite storage               | Fast     | Durable across restarts      | Primary data storage        |
| External (R2, D1)            | Variable | Durable, cross-DO accessible | Large files, shared data    |

Source: Rules of Durable Objects,
https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/

Pricing corroborates the "storage is opt-in" model: you are billed for
stored data only while data exists — "Durable Objects will be billed for
stored data until the data is removed. Once the data is removed, the object
will be cleaned up automatically by the system." An object that stores
nothing incurs only request and duration charges.
Source: https://developers.cloudflare.com/workers/platform/pricing/

Official examples of DOs that coordinate without persisting through DO
storage:

- The official **in-memory state example** keeps a counter purely in a class
  property and explicitly accepts losing it on eviction — a supported,
  documented pattern, not an abuse:
  https://developers.cloudflare.com/durable-objects/examples/durable-object-in-memory-state/index.md
- Cloudflare's own **Browser Run + Durable Objects tutorial** has a DO whose
  whole job is producing screenshots and writing them straight to R2 — it
  never writes the payloads to DO storage (see 1.2).

### 1.2 Bindings (including R2) work inside DO code exactly as in a Worker

A DO class receives the Worker's full `env` bindings object as the second
constructor argument (`super(ctx, env)`; in the `DurableObject` base class
it is exposed as `this.env`). Every binding the Worker has — R2 buckets,
D1 databases, KV namespaces, Queues, service bindings, secrets — is
reachable from inside DO methods. This is not a trick; it is how
Cloudflare's own tutorials are written.

Proof from an official tutorial: the Browser Run DO declares
`BUCKET: R2Bucket` in its `Env` interface, and inside the DO's `fetch()`
handler writes browser screenshots directly to the bucket:

```ts
await this.env.BUCKET.put(`${folder}/${fileName}.jpg`, sc);
```

Source (official tutorial):
https://developers.cloudflare.com/browser-run/how-to/browser-run-with-do/

So: **a DO can pass data forward into R2 without the bytes ever touching
DO storage or "the DO's disk."** The DO is compute + coordination; R2 is
the blob store. (A DO has no user-visible disk at all — its only "disk" is
the optional storage API.)

### 1.3 Streaming data _through_ a Worker or a DO into R2

Three distinct data-plane options, in increasing order of "keep bytes away
from your compute":

**(a) `put()` with a stream (Worker or DO).** The R2 Workers binding accepts
a `ReadableStream` as the value of `put(key, value)`. Cloudflare's official
upload tutorial does exactly this with a request body:

```ts
await env.MY_BUCKET.put(key, request.body);
```

Source (official tutorial):
https://developers.cloudflare.com/workers/tutorials/upload-assets-with-r2/

Practical constraint: a single `put()` needs to know the object's length.
A `fetch()` response body that carries `Content-Length`, or a stream
wrapped in the Workers runtime's `FixedLengthStream`, satisfies this; a
chunked/unknown-length stream does not, and the fallback is buffering or
multipart (option b). [community: the FixedLengthStream pattern and the
known-length requirement are spelled out in
https://dev.to/movoa/copy-a-remote-video-into-r2-from-a-cloudflare-worker-without-buffering-it-1fg
— I did not find this exact constraint stated on the R2 docs pages I opened,
so treat the mechanism as documented-by-practitioners rather than quoted
from the R2 reference.]

**(b) Multipart uploads via the binding (Worker or DO).** For large or
unknown-length payloads, the binding exposes the full multipart API:
`bucket.createMultipartUpload(key)` → `upload.uploadPart(n, streamOrBody)`
per part → `upload.complete(parts)` / `upload.abort()`, and
`bucket.resumeMultipartUpload(key, uploadId)` to continue an upload across
invocations. Parts are a minimum of 5 MB each except the last; uncompleted
multipart uploads are automatically aborted after 7 days. The official
guide's design has the _client_ track `uploadId` + part etags and call the
Worker per part — state does not need to live in the Worker at all.
Sources (official):
https://developers.cloudflare.com/r2/api/workers/workers-multipart-usage/
and the R2 Workers API reference linked from it.

**(c) Presigned URLs — the data plane bypasses your compute entirely.**
R2 supports S3-style presigned URLs (GET/HEAD/PUT/DELETE; expiry from
1 second to 7 days). They are generated locally with SigV4 signing — "no
communication with R2" is needed to mint one. The client then uploads
directly to `<ACCOUNT_ID>.r2.cloudflarestorage.com`; the bytes never pass
through any Worker or DO. Caveats from the docs: a presigned URL is a
bearer token (anyone holding it can perform the operation until expiry —
use short expiries and pin `Content-Type` in the signature), and presigned
URLs only work on the S3 API domain, not on custom domains.
Source (official): https://developers.cloudflare.com/r2/api/s3/presigned-urls/
[community note: the AWS SDK presigner shown in the R2 docs does not run
inside the Workers runtime; in-Worker signing is typically done with a
small SigV4 library such as `aws4fetch` —
https://gebna.gg/blog/fix-fs-readfile-is-not-implemented-yet]

### 1.4 The memory constraint when data flows _through_ compute

- **Both Workers and Durable Objects run in V8 isolates with a hard
  128 MB memory limit per isolate** — same on Free and Paid, not
  configurable. Official limits page: "Memory per isolate: 128 MB… This
  limit is per-isolate, not per-invocation. A single isolate can handle
  many concurrent requests."
  Source: https://developers.cloudflare.com/workers/platform/limits/
- The DO pricing page adds a sharp edge: duration billing assumes the full
  128 MB allocation per object, and "many instances of a single Durable
  Object class… may run in the same isolate on the same physical machine
  and share the 128 MB of memory. These Durable Objects are still billed
  as if they are allocated a full 128 MB of memory."
  Source: https://developers.cloudflare.com/workers/platform/pricing/
- **[community] consequence, well documented in production write-ups:**
  because the 128 MB is per _isolate_ and isolates are shared, the object
  that gets reset for exceeding memory is often not the object that used
  the memory ("noisy neighbour" resets), and you cannot profile a
  production DO's heap — only after-the-fact GraphQL percentile metrics:
  https://polylane.com/blog/how-we-fixed-our-cloudflare-durable-objects-memory-exceeded-errors/

What this means for streaming designs:

- **Buffering is the trap.** `await request.arrayBuffer()` / `.text()` /
  `.json()` on a large body puts the whole payload in the isolate heap —
  for a DO _and_ for a plain Worker — and is the most common way to hit
  the 128 MB wall.
- **True streaming** (`put(key, request.body)`, `uploadPart(n, request.body)`,
  `response.body.pipeThrough(...)`) keeps heap use roughly constant
  regardless of object size. The data passes _through_ the isolate's I/O
  layer, not _into_ its memory.
- **But a DO is the wrong bulk data plane even when streaming**, for two
  reasons a plain Worker doesn't have:
  1. _Serialization._ One object = one thread. A fat stream routed
     through a per-entity DO occupies that object's single execution
     context for the whole transfer; other requests to the same object
     interleave only at `await` points and compete for the same isolate.
  2. _Duration billing._ DO compute is billed on **wall-clock duration**
     while the object is active (see §3.8), so a slow 500 MB upload
     trickling through a DO bills DO duration for the entire transfer.
     A plain Worker bills CPU time (I/O wait is not CPU), and Workers
     have "no charge or limit for duration" on the Standard model.
     Source: https://developers.cloudflare.com/workers/platform/pricing/

### 1.5 So what _is_ the DO's role in an upload/blob architecture?

The pattern that falls out of the docs (and the Rules guide's own layering
table) is: **DO as coordinator, R2 as blob store, D1/KV as index.**

The (small, cheap) control plane goes through the DO; the (large) data
plane does not:

- **Authorize & mint**: the DO validates "may this user write this key?"
  under strong consistency, then returns a presigned PUT URL (or a
  multipart `uploadId` + per-part instructions) so bytes flow
  client → R2 directly.
- **Register metadata**: on completion the client (or an R2 event/queue
  consumer) calls back; the DO records the object's key, size, hash,
  owner, and state machine status (`pending → stored → processed`) in its
  SQLite storage — a few hundred bytes per artifact.
- **Order operations**: because all calls for one entity serialize through
  one object, "the latest version wins", quota checks, and
  check-then-write sequences are race-free without locks.
- **Fan out**: the DO pushes "new artifact available" to connected
  WebSocket subscribers (hibernatable WebSockets, §5.2) and schedules
  follow-up work (thumbnails, virus scan, expiry) via its alarm (§5.3).
- **Enforce invariants a bucket can't**: per-user byte quotas, rate
  limits, single-writer-per-document semantics, "only one export job per
  tenant at a time".

---

## 2. Core model facts

All from official sources unless marked.

1. **One ID → at most one live instance, globally.** "Durable Objects are
   defined by a JavaScript class and an ID. There is only ever one instance
   of a Durable Object with a given ID running anywhere in the world. All
   Workers making subrequests for the Durable Object with that ID are
   routed to the same instance." — Cloudflare blog (official):
   https://blog.cloudflare.com/durable-objects-ga/
   The Rules guide states the same as its opening premise: "Each Durable
   Object is a single-threaded, globally-unique instance with its own
   persistent storage."
2. **Single-threaded actor semantics.** Events (requests, RPC calls,
   WebSocket messages, alarms) for one object are processed one at a time;
   synchronous JavaScript never runs concurrently with itself inside an
   object. Interleaving can only happen at `await` points, and the
   runtime's input/output gates (§5.4) constrain even that. This removes
   lock-based reasoning _within_ an object — and caps its throughput.
3. **Throughput ceiling per object.** Official Rules guide: a single DO
   handles roughly **500–1,000 requests/second** depending on work per
   request (~1,000 simple pass-through, ~500–750 with JSON/validation,
   ~200–500 with transformation/storage writes). Design must shard before
   this matters.
4. **Co-located storage.** Each object has private storage physically
   attached to the object's execution location — SQLite (recommended) or
   legacy key-value. Storage calls are local and synchronous-feeling
   (the SQL API `ctx.storage.sql.exec()` is literally synchronous), which
   is why read-modify-write on every request is affordable.
   SQLite storage per object: **10 GB** (GA'd 2025-04-07; official
   changelog: https://developers.cloudflare.com/changelog/post/2025-04-07-sqlite-in-durable-objects-ga/).
5. **Placement: created near the first request; sticky thereafter.**
   Official Rules guide: "By default, a Durable Object is created near the
   location of the first request it receives." You can pass a
   `locationHint` (e.g. `wnam`, `weur`, `enam`) on `namespace.get(id, { locationHint })`,
   but "location hints are suggestions, not guarantees."
   Because compute is co-located with storage, an object's location is
   effectively fixed once created — there is no live migration of a hot
   object to follow its users. [community detail: jurisdictions
   (`namespace.jurisdiction("eu" | "us" | "fedramp")`) are the hard
   version of this — a separate sub-namespace whose objects _only_ run
   and store data inside the jurisdiction, used for data residency;
   summarized in https://www.aicassindra.com/blogs/cloud/cloudflare_durable_objects.html
   — I did not open the dedicated Data Location doc page, so treat the
   exact jurisdiction list as UNVERIFIED beyond that summary.]
6. **In-memory state is a cache, never the truth.** Class properties live
   only while the object is in memory. The object is evicted when idle,
   on crashes/uncaught exceptions, on deploys and runtime updates.
   Official in-memory-state doc: "Variables in a Durable Object will
   maintain state as long as your Durable Object is not evicted from
   memory," and warns that _global_ variables are actively dangerous
   because multiple object instances can share one isolate's globals.
   Sources: https://developers.cloudflare.com/durable-objects/examples/durable-object-in-memory-state/index.md
   and Rules guide ("In-memory state is **not preserved** if the Durable
   Object is evicted from memory due to inactivity, or if it crashes from
   an uncaught exception. Always persist important state to SQLite
   storage.").
7. **No shutdown hooks, by design.** Rules guide: "Durable Objects may
   shut down at any time due to deployments, inactivity, or runtime
   decisions… Shutdown hooks or lifecycle callbacks that run before
   shutdown are not provided." Correct code writes state incrementally
   ("write progress as you go"), it does not flush on exit.
8. **Virtual existence.** Objects are never explicitly created or
   destroyed by user code: you derive an ID (`idFromName`, `newUniqueId`,
   `idFromString`) and the platform activates the object on first use and
   evicts it when idle; an object with no stored data is automatically
   cleaned up. Creating a stub does **not** wake the object — only
   calling a method on the stub does (Rules guide).
   [community: this is the classic "virtual actor" model — a good
   explanation by a Cloudflare engineer:
   http://www.lambrospetrou.com/articles/durable-objects-are-virtual-objects/]

---

## 3. Pitfalls (sourced)

### 3.1 The hot object / global singleton

The Rules guide's headline anti-pattern: one DO for everything (a global
counter, a **global rate limiter**, one chat room for all rooms) funnels
all traffic through one single-threaded instance. The guide calls out
global rate limiting by name: identify natural coordination boundaries
(per user, per room, per document) instead. A hot object degrades into
queueing, then errors carrying `.overloaded` / `.retryable` properties
that callers are expected to handle with backoff (Rules guide, Error
handling section). Fix is always sharding: more, smaller objects.

### 3.2 `blockConcurrencyWhile()` misuse

It "guarantees that no other events are processed until the provided
callback completes, even if the callback performs asynchronous I/O"
(Rules guide). Legitimate use: one-time initialization/migrations in the
constructor. Anti-pattern: wrapping per-request work — at ~5 ms per call
it caps the object at ~200 requests/second. Prefer input/output gates,
write coalescing, or `transaction()` for atomicity during request
handling. [community: the callback also has a ~30-second timeout after
which the object is reset, so never wrap slow network calls in it —
https://www.aicassindra.com/blogs/cloud/cloudflare_durable_objects.html]

### 3.3 Assuming single-threaded means race-free across `await`s

Input gates protect you _during storage operations_. They open during
non-storage I/O. The Rules guide's own example is directly relevant to
R2-centric designs: read item → `await fetch(...)` (or write to R2) →
write item back is a race, because another request can run during the
external call and process the same item. Use check-and-set / version
numbers (optimistic locking) around any read-modify-write that spans
non-storage I/O.

### 3.4 Write coalescing breaks if you `await` between writes

Synchronous, un-awaited storage writes are batched into one implicit
atomic transaction. Interleaving `await`s on KV-style `get`/`put` calls
breaks coalescing: the guide's broken-transfer example shows a debit
committing before the credit, leaving inconsistent state on failure.
With the SQL API, keep multi-write sequences synchronous, or use an
explicit transaction.

### 3.5 Non-hibernating WebSockets burn duration — the classic cost surprise

Official pricing doc, verbatim in substance: "Calling `accept()` on a
WebSocket in an Object will incur duration charges for the entire time
the WebSocket is connected. It is recommended to use the WebSocket
Hibernation API to avoid incurring duration charges once all event
handlers finish running." Duration is billed in **wall-clock time** for
as long as the object is active and not hibernation-eligible — so the
legacy pattern of `ws.accept()` + keeping sockets in a JS array bills
you for every idle hour of every connected client. The Hibernation API
(`ctx.acceptWebSocket()`, `webSocketMessage`/`webSocketClose`/
`webSocketError` handlers, `ctx.getWebSockets()`) lets the object be
evicted from memory while connections stay open at the edge; "Durable
Objects that are idle and eligible for hibernation are not billed for
duration, even before the runtime has hibernated them."
Sources: https://developers.cloudflare.com/workers/platform/pricing/ ,
Rules guide (WebSockets section).
[community: a Cloudflare engineer walks through debugging exactly this
distinction — https://thomasgauvin.com/writing/how-cloudflare-durable-objects-websocket-hibernation-works/]

### 3.6 Holding the object open: `waitUntil`, long handlers, open requests

Anything that keeps an event in flight keeps the object "active" and
billing duration: slow external calls inside handlers, `ctx.waitUntil()`
promises doing slow work, long-lived response streams. Community
gotcha compilations (summarizing the docs) put it bluntly: for deferred
work prefer `storage.setAlarm()` (wakes the object later, briefly) or a
Queue over `waitUntil()` on a slow call [community:
https://deepwiki.com/dmmulroy/cloudflare-skill/6.4-durable-objects-gotchas].
Also: DOs remain active while "a request, RPC call, response stream,
WebSocket, or pending I/O is in flight" (official Workers limits page,
https://developers.cloudflare.com/workers/platform/limits/).

### 3.7 In-memory state assumptions

Covered in §2.6–2.7: fields vanish on eviction/crash/hibernation wake;
constructors re-run on every activation, so expensive constructors tax
every cold wake; per-connection state kept in JS variables is lost
across hibernation — use `serializeAttachment()` (survives hibernation;
per-attachment size is small — 2 KB limit is commonly cited [community/
UNVERIFIED against a doc page I opened]). The Rules guide's rule:
**persist first, cache second** — write storage, then update/invalidate
the in-memory copy.

### 3.8 Cost surprises (beyond WebSockets)

From the official pricing page (Workers Paid):

- **Duration**: 400,000 GB-s/month included, then $12.50 per million
  GB-s — billed on the full 128 MB allocation per object regardless of
  actual usage, and rounded _up_ to the next billable unit.
- **Requests**: 1M/month included, then $0.15/M. "Requests" includes HTTP
  requests, **each RPC method call** (one stub call = one RPC session =
  one billed request), **alarm invocations**, and incoming WebSocket
  messages at a **20:1 ratio** (100 incoming messages bill as 5
  requests; outgoing messages and protocol pings are free).
- **SQLite rows**: reads are nearly free (25 billion/month included,
  then $0.001/M) but **writes are 1,000× that** (50M/month included,
  then $1.00/M). Every `setAlarm()` is billed as one row written;
  deletes count as rows written; **KV-style `get/put/delete/list` on a
  SQLite-backed object are billed as hidden SQLite rows**; index
  maintenance adds row writes per indexed column [the index point is
  stated on the official D1 section of the same pricing page and in the
  community analysis at https://bex.co/blog/2026/08/22/cloudflare-zero-egress-durable-objects-storage-bill-hetzner].
  A chatty object doing a storage write per WebSocket message is the
  canonical way to discover this — batch writes.
- **Stored data**: 5 GB-month included, then $0.20/GB-month (SQLite
  backend). Legacy KV-backed classes (Paid only): 1M read units
  included then $0.20/M, 1M write/delete units then $1.00/M, 1 GB stored
  included then $0.20/GB-month; a request unit is 4 KB.
- Free plan: DOs limited to SQLite-backed classes; 100K requests/day and
  13,000 GB-s/day included.
  All from https://developers.cloudflare.com/workers/platform/pricing/

### 3.9 Hard limits that bite

- **128 MB per isolate** (shared between co-located objects of the same
  class + Worker code) — §1.4. Exceeding it resets objects; the reset
  victim may not be the memory hog [community: Polylane write-up above].
- **10 GB SQLite storage per object** (official changelog, §2.4) — a hard
  cap; an entity that outgrows it must be split or spill to R2/D1.
  KV-style values on SQLite-backed objects are capped at 2 MB per
  key/value; SQL statements at 100 KB and tables at 100 columns
  [community-compiled limits table, consistent with the docs:
  https://deepwiki.com/dmmulroy/cloudflare-skill/6.4-durable-objects-gotchas
  — I did not open the dedicated DO limits page directly, so treat these
  three numbers as UNVERIFIED against primary docs in this pass].
- **CPU time**: DO invocations live under the Workers CPU limits —
  Paid default 30 s, configurable to 5 min max per invocation
  (official: https://developers.cloudflare.com/workers/platform/limits/).
  Alarm invocations have a 15-minute duration limit (same page).
- **WebSocket message size**: max 32 MiB (raised from 1 MiB on
  2025-10-25; official DO release notes:
  https://developers.Cloudflare.com/durable-objects/release-notes/).
- **One alarm per object at a time**: `setAlarm()` replaces any existing
  alarm — multiple logical timers must be implemented as a stored
  event queue with the single alarm set to the earliest deadline
  [pattern per Rules guide alarms section + community gotchas].

### 3.10 Placement latency for far-away users

The object is born near its _first_ requester and stays there (§2.5).
Every subsequent request from anywhere on Earth pays a round trip to
that one location. A room created by a user in Frankfurt serves Sydney
users at Frankfurt latency forever. Mitigations are all design-time:
choose object granularity with locality (per-team/per-match rather than
per-global-entity), pass `locationHint` when you know better than the
first request, and keep far-flung read paths on KV/D1/R2 (globally
replicated) with the DO only on the write/coordinate path.

### 3.11 Class lifecycle, migrations, and versioning pain

- A class's **storage backend is chosen at creation** (`new_sqlite_classes`
  vs legacy `new_classes` in migrations) and cannot be flipped in place;
  picking `new_classes` for a new class is a common self-own [community
  - Rules guide's insistence on SQLite for new classes]. Cloudflare says
    a KV→SQLite migration path will be offered "in the future" (official
    changelog, 2025-04-07).
- Class renames/deletes/transfers are deploy-time operations (migrations
  array, or the newer declarative `exports` field — Rules guide shows
  `state: "renamed"` / `state: "deleted"` entries). Deleting a class
  **destroys all its objects' data**; migration tags must be unique and
  cannot be rolled back [community gotchas table, consistent with docs:
  https://deepwiki.com/dmmulroy/cloudflare-skill/6.4-durable-objects-gotchas].
- **Data-schema** migrations are _your_ problem and run lazily, per
  object, in the constructor under `blockConcurrencyWhile()` (Rules
  guide's `_sql_schema_migrations` pattern) — meaning old-schema and
  new-schema objects coexist across deploys, and a bad migration only
  detonates when that particular object next wakes.
- Code deploys replace the code for _all_ objects of a class at once;
  in-flight objects are restarted under the new code (another reason
  in-memory state is not a contract).

### 3.12 Testing pain points

Officially, testing is now first-class: `@cloudflare/vitest-plugin` /
`@cloudflare/vitest-pool-workers` run DOs in the real runtime with
isolated storage, `runInDurableObject(stub, (instance, state) => …)` for
arranging/asserting internals, and `runDurableObjectAlarm(stub)` to fire
alarms immediately (Rules guide, Testing section). The remaining pain is
intrinsic rather than tooling [assessment, informed by the sources]:
alarms/hibernation/eviction are time- and runtime-driven, so tests can
trigger them but production timing (when exactly eviction happens, how
long a non-hibernating object lingers) is not deterministic; placement
and cross-region latency can't be reproduced locally at all; and
concurrency bugs that depend on gate interleavings (§3.3) only appear
under realistic concurrent load, not unit tests.

---

## 4. Advantages vs the alternatives

|               | Durable Objects                                                                            | Workers KV                                                                          | D1                                                                        | Plain Workers                                    | External Redis/Postgres                                                   |
| ------------- | ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------- |
| Consistency   | **Strong**, per object; serialized execution + transactional storage                       | Eventual (global propagation, typically seconds–~60 s)                              | Strong at the single primary; read replicas are eventually consistent     | None (stateless)                                 | Depends (Redis: per-node; Postgres: strong)                               |
| Coordination  | **Native** — single-writer actor per entity; no locks, no CAS loops                        | None — read-modify-write races are real                                             | SQL transactions, but contended global hot rows serialize at one primary  | None                                             | Redis locks/Lua, Postgres advisory locks — you build and operate it       |
| Connections   | **Hibernatable WebSockets owned per object**; fan-out is a loop over `ctx.getWebSockets()` | n/a                                                                                 | n/a                                                                       | Can proxy WebSockets but holds no per-room state | You run the socket servers                                                |
| Scheduling    | **Per-entity alarms** (one timer per object, wakes the object)                             | n/a                                                                                 | n/a                                                                       | Cron Triggers are per-Worker, global schedule    | External cron/queues                                                      |
| Latency shape | ~0 ms to own storage (co-located); possibly a cross-planet trip to _reach_ the object      | ~0 ms reads at every edge (cached)                                                  | Reads near user with replication; writes travel to the primary's location | Runs at the nearest edge, always                 | A network hop (mitigated by Hyperdrive pooling/caching) from every Worker |
| Scale model   | Scales by adding objects (IDs); each object caps at ~500–1,000 req/s                       | Effectively unlimited reads; writes ~1/s per key guidance [UNVERIFIED in this pass] | One database: 10 GB cap, single-writer throughput                         | Effectively unlimited                            | Whatever you provision                                                    |
| Ops burden    | Zero servers; pay requests + duration + rows                                               | Zero                                                                                | Zero                                                                      | Zero                                             | Yours (or a vendor's)                                                     |

Narrative version:

- **vs KV**: KV is a globally replicated, eventually consistent cache —
  superb for read-mostly config/session data at every edge, wrong for
  anything needing atomic check-and-set. The canonical failure: a rate
  limiter on KV where two concurrent requests both read "99" and both
  pass [community book chapter making this exact point:
  https://architectingoncloudflare.com/chapter-12/]. If the operation is
  "read state, decide, write state" it belongs in a DO (or D1), not KV.
- **vs D1**: D1 is one SQLite database (itself built on DO technology)
  with a single primary location — great for cross-entity queries,
  joins, and read models; its write path funnels to one place, and one
  hot row is everyone's problem. DOs invert this: no cross-object
  queries at all, but millions of independent single-writer databases
  that never contend with each other. The standard pairing is DOs as the
  write/coordination tier and D1 as the queryable projection/index
  (see §5.6).
- **vs plain Workers**: Workers are stateless and run at the nearest
  edge — strictly better for anything that doesn't need shared state,
  identity, or connections. The Rules guide's framing: "Use Durable
  Objects for stateful coordination, not stateless request handling";
  front them with a Worker that does auth/validation/routing.
- **vs external Redis/Postgres**: a DO gives you the Redis-list/lock
  class of primitives and the Postgres-transaction class of guarantees
  per entity, with no connection management from Workers (a real pain
  point that Hyperdrive exists to soften), no servers, and scale-to-zero
  billing. What you give up: cross-entity transactions and queries,
  mature tooling/observability, big-memory compute, and an exit ramp —
  DO storage APIs are Cloudflare-proprietary, so the state layer does
  not port.

The advantages that have no real substitute elsewhere on the platform:
strong consistency + transactions _per entity_ without a database round
trip; WebSocket fan-out where the connection owner _is_ the state owner;
per-entity alarms (a million independent timers with zero polling); and
the actor-model fit — code reads like a single-threaded class, because
it is one.

---

## 5. Best practices and common techniques

Mostly from the official Rules guide
(https://developers.cloudflare.com/durable-objects/best-practices/rules-of-durable-objects/),
published 2025-12-15, plus marked community sources.

### 5.1 One DO per entity — the "atom of coordination"

"Create one Durable Object per logical unit that needs coordination: a
chat room, a game session, a document, a user's data, or a tenant's
workspace." Route deterministically with `getByName()` / `idFromName()`
so the same entity always lands on the same object; reserve
`newUniqueId()` for when you genuinely need a fresh random instance and
will store the ID mapping elsewhere (D1/KV). Parent/child structures
(one index DO per tenant, one DO per match under it) keep list/query
operations off the hot objects. Compute needed shard count up front:
total req/s ÷ ~500–1,000 per object.

### 5.2 Hibernatable WebSockets, always

Use `ctx.acceptWebSocket()` + `webSocketMessage/Close/Error` handlers
instead of the legacy `ws.accept()` pattern (§3.5). Store per-connection
metadata (user ID, session info) with `serializeAttachment()` so it
survives hibernation; enumerate sockets with `ctx.getWebSockets()` for
broadcasts instead of keeping a JS array (which hibernation erases).
On compatibility dates ≥ 2026-04-07 the runtime auto-replies to Close
frames (`web_socket_auto_reply_to_close`); on older dates you must call
`ws.close()` in `webSocketClose` or clients see 1006 errors (Rules
guide). Incoming messages bill at 20:1 as requests, outgoing are free —
chatty _inbound_ protocols are the cost driver, so batch client updates
where the protocol allows.

### 5.3 Alarms instead of polling

One alarm per object (`ctx.storage.setAlarm(timestamp)`, ms precision;
re-set it inside `alarm()` for recurrence — alarms don't repeat on
their own). Canonical uses: session/game expiry, subscription renewal,
deferred flush of buffered writes, cleanup/TTL of the object's own data,
retry-with-backoff workflows. Rules: make `alarm()` **idempotent**
(alarms can fire more than once), inspect `alarmInfo.retryCount`
(alarms retry on throw; the guide's pattern re-schedules manually near
the retry limit rather than losing the work), and "only schedule alarms
when there is work to do" — an alarm per object per few seconds across
a million objects is a million billable wake-ups (each `setAlarm` is
also a billed row write, §3.8). For multiple logical timers, keep a
sorted events table in SQLite and set the single alarm to the earliest.

### 5.4 Let the gates work for you; keep write paths synchronous

- Input gates: no new event is delivered while your synchronous code
  runs, and storage awaits hold the gate — so a read-modify-write done
  entirely against storage is race-free _without_ locks.
- Output gates: responses and outgoing fetches are held until pending
  storage writes commit — clients can never observe success for data
  that wasn't persisted. This is why `sql.exec()` writes don't need to
  be awaited for correctness.
- Write coalescing: a run of synchronous storage writes commits as one
  implicit atomic transaction (§3.4). Structure hot methods as:
  load → decide → write synchronously → return.
- `blockConcurrencyWhile()`: constructor init/migrations only (§3.2).
- `transaction()` / `transactionSync()`: for explicit multi-step
  atomicity in request handling without freezing the object.

### 5.5 Storage hygiene

- New classes: SQLite backend (`new_sqlite_classes`), SQL API, indexes
  on filtered columns, schema versioned via a `_sql_schema_migrations`
  table applied in the constructor (all Rules guide patterns).
- **Batch writes**: coalesce, use transactions, or buffer high-frequency
  updates (game positions, metrics) in memory and flush on an interval
  or via alarm — rows written are the expensive meter (§3.8).
- **Idempotency keys**: for any operation a client may retry (payments,
  uploads-complete callbacks, webhook deliveries), accept a
  client-supplied ID, store it in SQLite in the same transaction as the
  effect, and return the stored result on duplicates. Single-threaded
  execution makes the check-and-set trivially correct — this is one of
  the highest-value DO patterns [technique; illustrated in community
  write-ups, e.g. dedupe-by-event-ID in
  https://dev.to/instawebhook/routing-webhooks-through-cloudflare-workers-kv-and-durable-objects-building-a-globally-distributed-1gje].
- Cache in memory _after_ persisting; treat every wake as a cold start;
  keep constructors cheap (schema DDL with `IF NOT EXISTS` only).
- `deleteAll()` to fully retire an object (data + alarm; compat-date
  nuances in the Rules guide).

### 5.6 The reference architecture: DO coordinates, R2 stores blobs, D1 indexes

For artifact/file-shaped systems (the §1.5 pattern, generalized):

```
Client ──control──▶ Worker ──▶ DO (per user/room/doc)
  │                              ├─ auth, quota, ordering, state machine (SQLite)
  │                              ├─ mints presigned URL / multipart session
  │                              └─ records metadata + fans out over WebSockets
  └──────data (presigned/multipart direct)──────▶ R2 (blobs)
Worker/Queue consumer ──▶ D1 (cross-entity index, search, admin queries)
```

- Small, hot, consistency-critical state → DO SQLite.
- Bytes → R2, addressed by keys the DO minted and recorded.
- Anything you need to `SELECT … WHERE` across entities → projected
  into D1 (synchronously for small writes, or via Queues/alarms for
  fan-out), because cross-DO queries don't exist.
- Reads that are global and stale-tolerant → KV or R2 public/custom
  domain delivery, bypassing the DO entirely.

### 5.7 Canonical DO roles (with the sharding caveat applied)

- **Chat room / live presence** — one DO per room; hibernatable
  WebSockets; history in SQLite; attachments in R2.
- **Multiplayer game session / collaborative document** — one DO per
  session/document; single-threaded authority replaces OT/CRDT server
  coordination for the live state; alarm as the game tick/timeout.
- **Rate limiter / token bucket** — one DO **per entity being limited**
  (per user, per API key, per tenant). Atomic check-and-increment in
  synchronous code against storage. A _global_ limiter DO is the Rules
  guide's named anti-pattern (§3.1) — shard by entity or use
  approximate counting in the Worker with periodic DO reconciliation.
- **Leader election / single-flight** — one DO per lock name: the
  platform's guarantee that only one instance exists _is_ the election;
  the holder renews via alarm, contenders ask the DO. Dedup/single-flight
  for webhook processing and cache-stampede protection are the same
  shape [pattern assessment; the uniqueness guarantee it rests on is
  official, §2.1].
- **Per-user/per-tenant anything** — inbox, cart, game state, agent
  session: the Agents SDK itself is built as DO-per-agent for exactly
  these properties.
- **Coordinator for heavy compute** — the DO holds session state and
  drives a Container or Browser Run session (Cloudflare's own Browser
  Run DO tutorial is this pattern), keeping the heavy runtime outside
  the isolate while ordering stays in the DO.

### 5.8 RPC over `fetch()` for DO APIs

Official guidance (Rules guide): for projects with compatibility date
≥ 2024-04-03, define public methods on the DO class and call them on the
stub (`await stub.bookSeat(...)`) instead of implementing a `fetch()`
handler and hand-parsing Requests. You get TypeScript types end-to-end,
structured-cloneable arguments/returns, and exceptions that propagate.
Billing note: **every stub method call is one billed request**; methods
that return an `RpcTarget` stub let subsequent calls ride the same RPC
session unbilled (official pricing footnotes). Keep `fetch()` for the
cases that genuinely are HTTP: WebSocket upgrades (which need the
Request/Response pair) and proxying/streaming a request body through to
somewhere else (e.g. §1.3a) — a body stream is not an RPC-shaped value.
[UNVERIFIED: whether RPC supports passing ReadableStream arguments —
not stated in the pages opened here; design as if it does not.]

### 5.9 Errors and overload are part of the API

Stub calls can fail with errors exposing `.retryable` and `.overloaded`
(Rules guide). Production callers should retry retryable errors with
exponential backoff + jitter and treat `overloaded` as the signal that
an entity has outgrown one object (time to sub-shard it), not as a
transient blip.

---

## Appendix: numbers used above, with provenance

| Fact                                                         | Value                                                                                | Source                                                                        |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| Memory limit                                                 | 128 MB per isolate, all plans                                                        | Official: workers/platform/limits                                             |
| DO duration billing basis                                    | Wall-clock while active & not hibernation-eligible; billed as full 128 MB allocation | Official: workers/platform/pricing                                            |
| DO requests (Paid)                                           | 1M/mo incl., +$0.15/M; incl. HTTP, per-RPC-call, alarm invocations, WS msgs at 20:1  | Official: workers/platform/pricing                                            |
| DO duration (Paid)                                           | 400,000 GB-s/mo incl., +$12.50/M GB-s                                                | Official: workers/platform/pricing                                            |
| SQLite rows read (Paid)                                      | 25B/mo incl., +$0.001/M                                                              | Official: workers/platform/pricing                                            |
| SQLite rows written (Paid)                                   | 50M/mo incl., +$1.00/M; setAlarm = 1 write; deletes = writes                         | Official: workers/platform/pricing                                            |
| SQLite stored data (Paid)                                    | 5 GB-month incl., +$0.20/GB-month                                                    | Official: workers/platform/pricing                                            |
| SQLite storage per object                                    | 10 GB                                                                                | Official: changelog 2025-04-07 (SQLite GA)                                    |
| Per-object throughput                                        | ~500–1,000 req/s                                                                     | Official: Rules of Durable Objects                                            |
| WebSocket max message size                                   | 32 MiB (since 2025-10-25)                                                            | Official: DO release notes                                                    |
| CPU per invocation (Paid)                                    | 30 s default, 5 min max; alarm duration limit 15 min                                 | Official: workers/platform/limits                                             |
| KV value cap on SQLite-backed DOs; SQL statement/column caps | 2 MB; 100 KB; 100 cols                                                               | [community compilation — UNVERIFIED against primary limits page in this pass] |
| DO classes per account                                       | 100 (Free) / 500 (Paid)                                                              | [community compilation — UNVERIFIED in this pass]                             |
| blockConcurrencyWhile callback timeout                       | ~30 s, then object resets                                                            | [community — aicassindra deep-dive]                                           |
| WebSocket attachment size cap                                | 2 KB commonly cited                                                                  | [community — UNVERIFIED in this pass]                                         |
