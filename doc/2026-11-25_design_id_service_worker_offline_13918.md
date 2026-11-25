# ID web: service-worker offline shell and local-first web app

Date: 2026-11-25
Status: design record and first slice for [#13918](https://github.com/developing-today/code/issues/13918) (ID high-level roadmap item 23). The first slice (§7, slice 1) ships with this record; later slices are listed with their own acceptance.

## 1. Problem and context

The ID web app (`pkgs/id/web`, served by `id-web` from `web/dist` through rust-embed) works only while its server answers. A dropped connection, a laptop lid closed on the node, or a flaky hosted link leaves the browser with the browser's own error page, even for pages the user just read.

Pieces of the local-first story already exist and this design composes them rather than replacing them:

- **World snapshot store** (`web/src/world-snapshot-store.ts`, #10927): the world shell keeps its last applied journal sequence and rendered state in `localStorage`, reads the world offline, and resumes from that sequence on reconnect. Offline, the world shell is a reader, not a writer.
- **IndexedDB store adapter** (roadmap item 21, #10213 prototype, adopted by #9082): the browser store behind the same store interface as native (roadmap item 16). Production wiring is item 21's work.
- **Content-hashed assets** (`id-web/src/assets.rs`, #6779): `main.<8 base36>.js` and `styles.<8 hex>.css` are immutable and served with a one-year cache; everything else is `no-cache`.

What is missing is the layer that makes the app itself load with no network: a service worker that owns the app shell and the pages the user already saw.

## 2. Goals and non-goals

### Goals

- The ID web app opens with no network: the app shell (HTML of visited pages, the hashed JS and CSS) comes from a local cache.
- Pages the user visited can be read offline, exactly as last rendered.
- When the network is back, the next navigation shows live data again with no user action.
- No stale-asset trap: a new build replaces the cached shell atomically, and old caches are removed.
- Nothing that mutates state or carries a credential is cached or replayed by the service worker.

### Non-goals (this record)

- Offline writes. Queued edits, conflict resolution and replay belong to the store layer (item 16/21) and the collab/journal protocols, not to an HTTP cache. §5 states the contract.
- Caching API JSON, WebSocket traffic or blobs in slice 1.
- Push notifications and background sync (shell notification work, #10930).
- The PM console. It is the second surface named in #13918 and reuses this pattern once slice 1 is proven; it gets its own slice issue.

## 3. Surfaces in scope

1. **ID web app** served by `id serve --web` (`create_router_inner`): home, files, settings, peers, blocks, groups, editor and viewer pages.
2. **Program surface** (`create_program_router`): one world on one listener. It serves `/` and `/assets/*`, so the same worker applies with no special case.
3. **PM console**: later slice, same contract.

## 4. Cache strategy

### 4.1 Service worker location and scope

The worker is built from `web/src/sw.ts` into `web/dist/sw.js` and served at `/sw.js` (root scope) by `id-web`, with `Cache-Control: no-cache` so the browser checks for a new worker on every navigation, and `Service-Worker-Allowed: /`. It is also reachable at `/assets/sw.js` through the ordinary asset route (no-cache, not hashed).

### 4.2 Versioning and invalidation

`web/scripts/build-manifest.ts` already writes `dist/manifest.json` with the hashed JS and CSS names. After writing it, the same script prefixes `dist/sw.js` with one line that sets the shell manifest:

```
self.__ID_SHELL_MANIFEST__={"main.js":"main.<hash>.js","styles.css":"styles.<hash>.css"};
```

so the worker's bytes change whenever any shell asset changes. That is what makes the browser install a new worker. The step is idempotent: a previous prefix line is replaced, never stacked.

Cache names are derived from the two hashes:

- `id-shell-v1-<js hash>-<css hash>`: the precached app shell (immutable assets).
- `id-pages-v1-<js hash>-<css hash>`: HTML of visited pages.

Pages are versioned with the shell because a cached page references the hashed asset names of the build that rendered it. On `activate` the worker deletes every `id-shell-*` and `id-pages-*` cache that is not the current pair, then claims open clients. A manifest that fails validation (names outside the #6779 emitted-name contract) installs nothing and makes the worker pass every request through, which is the safe failure.

### 4.3 Request routing

| Request | Strategy |
|---|---|
| Not `GET`, or another origin | Not intercepted |
| `/api/*`, `/ws/*`, `/blob/*`, `/.well-known/*`, `/sw.js` | Not intercepted (online only) |
| `/assets/main.<hash>.js`, `/assets/styles.<hash>.css` | Cache first, then network (immutable) |
| Navigation (`mode: navigate`) without a `token` query parameter | Network first; a cacheable response is stored; on network failure, the stored copy for that exact URL, else an offline page |
| Anything else (partials, manifest, other assets) | Not intercepted |

A page response is stored only when it is a plain `200` HTML response, not redirected, and not marked `Cache-Control: no-store`. A server error, a `401` from the token guard, or a redirect is passed to the page unchanged and never cached, so the worker never hides an auth failure behind a stale page. URLs carrying `?token=` are never intercepted, so a credential never becomes a cache key. The pages cache keeps the 50 most recent entries.

An unvisited page requested offline gets a small `503` offline page with a link home, not a different page's content.

## 5. Offline contract

- **Fully offline:** opening the app; reading any page visited on this browser since the current build was installed, as last rendered; reading worlds through the world snapshot store.
- **Online only (slice 1):** every write (`/api/*` POST, collab and tag WebSockets), search and listing refreshes, blob downloads, identity registration.
- **On reconnect:** the next navigation goes to the network and replaces the stored copy. The world shell resumes from its last applied sequence (#10927). No write is replayed by the worker.
- **Conflicts:** none can arise from the worker, because it never queues writes. Offline drafting belongs to slice 4 and goes through the store layer, where conflicts surface as store conflicts.
- **Privacy:** cached pages stay in this browser profile under this origin, the same exposure as the browser's HTTP cache. The server can exclude any page with `Cache-Control: no-store`.

## 6. Alternatives considered

- **Workbox or another SW library.** Rejected for slice 1: the routing table is five rules, the build is Bun with no plugin pipeline, and a dependency would add more surface than code.
- **Hashing `sw.js` itself.** Not possible: a worker's URL must stay stable for updates to be found.
- **Caching every GET.** Rejected: API partials and listings would show stale data online, and blobs can be large.
- **App-shell-only navigation fallback (serve `/` for every offline navigation).** Rejected: the server renders per-page content, so serving the home page at another URL would show the wrong content.

## 7. Slices, acceptance and verification

1. **Slice 1 (this change): offline shell and visited pages.** `sw.ts` + `sw-policy.ts`, registration from `main.ts`, the `/sw.js` route in `id-web`, the build-manifest injection step.
   Acceptance: unit tests for request classification, response cacheability, versioned cache names, install/activate cleanup, network-first fallback and the offline page; Rust tests for the `/sw.js` headers; the build-manifest fixture covers injection and idempotency.
2. **Slice 2: real-browser offline pass.** A Playwright test in `pkgs/id/e2e` that loads the app, goes offline through the browser context, reloads a visited page and an unvisited one, then reconnects. Acceptance: the visited page renders from cache, the unvisited one shows the offline page, and a new build replaces the shell caches.
3. **Slice 3: content-addressed blobs.** Cache-first for `/blob/<hash>` under a size cap, since blob responses are immutable by hash.
4. **Slice 4: offline drafts.** Through the IndexedDB store adapter (item 21) once wired: drafts are written locally and synced by the store layer; the worker stays a cache.
5. **Slice 5: PM console.** Apply the same worker contract to the PM console.

Rollout: the worker registers only in a secure context (HTTPS or `localhost`), so plain-HTTP LAN deployments keep today's behaviour. Kill switch: shipping a build whose manifest fails validation makes the worker pass everything through and delete its caches on activate; `navigator.serviceWorker.getRegistrations()` can also be cleared from devtools.
