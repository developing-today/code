/**
 * #13918 slices 1 and 3: the service worker's routing and cache policy.
 *
 * Design of record: doc/2026-11-25_design_id_service_worker_offline_13918.md.
 * Everything here is pure or takes its platform objects (CacheStorage,
 * fetch) as arguments, so the policy is unit-tested without a browser
 * and `sw.ts` stays a thin event adapter.
 *
 * Contract summary:
 * - hashed shell assets (main.<8 base36>.js, styles.<8 hex>.css, the
 *   #6779 emitted-name contract) are precached and served cache-first;
 * - navigations are network-first; a plain 200 HTML response that is
 *   not redirected and not `no-store` is kept per URL; on network
 *   failure the kept copy is served, else a 503 offline page;
 * - content-addressed blobs (`/blob/<hash>`, slice 3) are served
 *   cache-first: a blob is immutable by hash, so a kept copy never goes
 *   stale. Only a plain 200 the server marks `immutable` with a declared
 *   size up to MAX_CACHED_BLOB_BYTES is kept, at most MAX_CACHED_BLOBS
 *   of them; range requests pass through so media seeking still works;
 * - writes, API, WebSocket, token-bearing and cross-origin requests are
 *   never intercepted, so nothing that mutates state or carries a
 *   credential is cached or replayed.
 */

export const SHELL_CACHE_PREFIX = "id-shell-";
export const PAGES_CACHE_PREFIX = "id-pages-";
/** Bump when the cache layout itself changes (not per build). */
export const CACHE_FORMAT = "v1";
/** Most recent visited pages kept for offline reading. */
export const MAX_CACHED_PAGES = 50;
export const BLOBS_CACHE_PREFIX = "id-blobs-";
/** Blobs are immutable by hash, so their cache is not versioned per build. */
export const BLOBS_CACHE_NAME = `${BLOBS_CACHE_PREFIX}${CACHE_FORMAT}`;
/** Most recently fetched blobs kept for offline use. */
export const MAX_CACHED_BLOBS = 64;
/** Larger blobs are never kept, which bounds the cache at 64 MiB. */
export const MAX_CACHED_BLOB_BYTES = 1024 * 1024;

/** `/blob/<hash>`: one path segment of hash characters, as `parse_hash` accepts. */
const BLOB_PATH = /^\/blob\/[A-Za-z0-9]{1,128}$/;

const HASHED_JS = /^main\.([a-z0-9]{8})\.js$/;
const HASHED_CSS = /^styles\.([0-9a-f]{8})\.css$/;

/** The shell manifest the build injects into sw.js (logical name -> hashed file). */
export type ShellManifest = { "main.js": string; "styles.css": string };

/**
 * Validate an injected manifest. Anything outside the emitted-name
 * contract yields null, and a worker with no manifest passes every
 * request through (the safe failure and the kill switch).
 */
export function parseShellManifest(raw: unknown): ShellManifest | null {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return null;
  const record = raw as Record<string, unknown>;
  const js = record["main.js"];
  const css = record["styles.css"];
  if (typeof js !== "string" || typeof css !== "string") return null;
  if (!HASHED_JS.test(js) || !HASHED_CSS.test(css)) return null;
  return { "main.js": js, "styles.css": css };
}

function buildTag(manifest: ShellManifest): string {
  const js = HASHED_JS.exec(manifest["main.js"])?.[1] ?? "";
  const css = HASHED_CSS.exec(manifest["styles.css"])?.[1] ?? "";
  return `${CACHE_FORMAT}-${js}-${css}`;
}

export function shellCacheName(manifest: ShellManifest): string {
  return `${SHELL_CACHE_PREFIX}${buildTag(manifest)}`;
}

/** Pages are versioned with the shell: a kept page names this build's hashed assets. */
export function pagesCacheName(manifest: ShellManifest): string {
  return `${PAGES_CACHE_PREFIX}${buildTag(manifest)}`;
}

export function precacheUrls(manifest: ShellManifest): string[] {
  return [`/assets/${manifest["main.js"]}`, `/assets/${manifest["styles.css"]}`];
}

const OWNED_PREFIXES = [SHELL_CACHE_PREFIX, PAGES_CACHE_PREFIX, BLOBS_CACHE_PREFIX];

/**
 * Caches this worker owns that do not belong to the current build. The
 * blob cache outlives builds; with no valid manifest (the kill switch)
 * every owned cache goes, blobs included.
 */
export function obsoleteCacheNames(existing: readonly string[], manifest: ShellManifest | null): string[] {
  const keep = manifest
    ? new Set([shellCacheName(manifest), pagesCacheName(manifest), BLOBS_CACHE_NAME])
    : new Set<string>();
  return existing.filter((name) => OWNED_PREFIXES.some((prefix) => name.startsWith(prefix)) && !keep.has(name));
}

export type RouteDecision = "asset-cache-first" | "blob-cache-first" | "page-network-first" | "passthrough";

/** The request fields the router looks at (a Request satisfies this). */
export type RoutableRequest = { method: string; url: string; mode: string; headers?: Pick<Headers, "has"> };

const ONLINE_ONLY_PREFIXES = ["/api/", "/ws/", "/blob/", "/.well-known/"];

export function classifyRequest(request: RoutableRequest, origin: string): RouteDecision {
  if (request.method !== "GET") return "passthrough";
  let url: URL;
  try {
    url = new URL(request.url);
  } catch {
    return "passthrough";
  }
  if (url.origin !== origin) return "passthrough";
  const path = url.pathname;
  if (BLOB_PATH.test(path)) {
    // A credential must never become a cache key, and a partial (range)
    // response cannot answer a whole-blob request.
    if (url.searchParams.has("token") || request.headers?.has("range")) return "passthrough";
    return "blob-cache-first";
  }
  if (path === "/sw.js" || ONLINE_ONLY_PREFIXES.some((prefix) => path.startsWith(prefix))) return "passthrough";
  if (path.startsWith("/assets/")) {
    const name = path.slice("/assets/".length);
    return HASHED_JS.test(name) || HASHED_CSS.test(name) ? "asset-cache-first" : "passthrough";
  }
  if (request.mode === "navigate") {
    // A credential must never become a cache key (security.rs `?token=` bootstrap).
    if (url.searchParams.has("token")) return "passthrough";
    return "page-network-first";
  }
  return "passthrough";
}

/** Only a plain, final, storable HTML 200 is kept for offline reading. */
export function isCacheablePageResponse(response: Response): boolean {
  if (response.status !== 200 || response.redirected) return false;
  if (response.type !== "basic" && response.type !== "default") return false;
  const contentType = response.headers.get("content-type") ?? "";
  if (!contentType.toLowerCase().includes("text/html")) return false;
  const cacheControl = (response.headers.get("cache-control") ?? "").toLowerCase();
  return !/(^|[\s,])no-store($|[\s,])/.test(cacheControl);
}

/**
 * Only a plain, final 200 the server marks `immutable`, with a declared
 * size up to MAX_CACHED_BLOB_BYTES, is kept. A response without a
 * Content-Length is not kept, so the cap is never exceeded unseen.
 */
export function isCacheableBlobResponse(response: Response): boolean {
  if (response.status !== 200 || response.redirected) return false;
  if (response.type !== "basic" && response.type !== "default") return false;
  const cacheControl = (response.headers.get("cache-control") ?? "").toLowerCase();
  if (!/(^|[\s,])immutable($|[\s,])/.test(cacheControl) || /(^|[\s,])no-store($|[\s,])/.test(cacheControl)) {
    return false;
  }
  const length = response.headers.get("content-length");
  if (length === null || !/^\d+$/.test(length.trim())) return false;
  return Number(length) <= MAX_CACHED_BLOB_BYTES;
}

/** Cache key for a page: the URL without its fragment. */
export function pageCacheKey(url: string): string {
  const parsed = new URL(url);
  parsed.hash = "";
  return parsed.toString();
}

export const OFFLINE_PAGE_HTML =
  '<!doctype html><html lang="en"><head><meta charset="utf-8">' +
  '<meta name="viewport" content="width=device-width, initial-scale=1">' +
  "<title>Offline</title></head><body>" +
  "<h1>You are offline</h1>" +
  "<p>This page has not been saved on this device yet. Pages you have opened before can still be read.</p>" +
  '<p><a href="/">Go home</a></p></body></html>';

export function offlineResponse(): Response {
  return new Response(OFFLINE_PAGE_HTML, {
    status: 503,
    statusText: "Offline",
    headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" },
  });
}

/** A blob that was never kept, asked for offline: a plain uncacheable 503. */
export function offlineBlobResponse(): Response {
  return new Response("Offline: this file has not been saved on this device.", {
    status: 503,
    statusText: "Offline",
    headers: { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" },
  });
}

/** Drop the oldest entries beyond `max` (Cache.keys() is insertion-ordered). */
export async function trimCache(cache: Cache, max: number): Promise<void> {
  const keys = await cache.keys();
  for (const stale of keys.slice(0, Math.max(0, keys.length - max))) {
    await cache.delete(stale);
  }
}

export type ShellWorkerDeps = {
  caches: CacheStorage;
  fetch: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
  manifest: ShellManifest | null;
  origin: string;
};

export type ShellWorker = {
  /** Precache the shell; rejects (failing install) if a shell asset cannot be fetched. */
  install(): Promise<void>;
  /** Delete every cache this worker owns that is not the current build's. */
  activate(): Promise<void>;
  /** A response promise for intercepted requests, or null to let the browser handle it. */
  respond(request: Request): Promise<Response> | null;
};

export function createShellWorker(deps: ShellWorkerDeps): ShellWorker {
  const { caches, manifest, origin } = deps;
  const fetchFn = deps.fetch;

  async function install(): Promise<void> {
    if (!manifest) return;
    const shell = await caches.open(shellCacheName(manifest));
    for (const url of precacheUrls(manifest)) {
      const response = await fetchFn(url, { cache: "no-cache" });
      if (!response.ok) throw new Error(`precache ${url}: HTTP ${response.status}`);
      await shell.put(url, response);
    }
  }

  async function activate(): Promise<void> {
    const names = await caches.keys();
    for (const name of obsoleteCacheNames(names, manifest)) {
      await caches.delete(name);
    }
  }

  async function assetCacheFirst(manifest: ShellManifest, request: Request): Promise<Response> {
    const shell = await caches.open(shellCacheName(manifest));
    const cached = await shell.match(request.url);
    if (cached) return cached;
    const response = await fetchFn(request);
    if (response.ok) await shell.put(request.url, response.clone());
    return response;
  }

  async function blobCacheFirst(request: Request): Promise<Response> {
    // The query (`?filename=`) selects the served headers, so it stays in the key.
    const key = pageCacheKey(request.url);
    const blobs = await caches.open(BLOBS_CACHE_NAME);
    const cached = await blobs.match(key);
    if (cached) return cached;
    let response: Response;
    try {
      response = await fetchFn(request);
    } catch {
      return offlineBlobResponse();
    }
    if (isCacheableBlobResponse(response)) {
      await blobs.put(key, response.clone());
      await trimCache(blobs, MAX_CACHED_BLOBS);
    }
    return response;
  }

  async function pageNetworkFirst(manifest: ShellManifest, request: Request): Promise<Response> {
    const key = pageCacheKey(request.url);
    let response: Response;
    try {
      response = await fetchFn(request);
    } catch {
      const pages = await caches.open(pagesCacheName(manifest));
      return (await pages.match(key)) ?? offlineResponse();
    }
    if (isCacheablePageResponse(response)) {
      const pages = await caches.open(pagesCacheName(manifest));
      // Re-insert so the entry counts as most recent for trimming.
      await pages.delete(key);
      await pages.put(key, response.clone());
      await trimCache(pages, MAX_CACHED_PAGES);
    }
    return response;
  }

  function respond(request: Request): Promise<Response> | null {
    if (!manifest) return null;
    switch (classifyRequest(request, origin)) {
      case "asset-cache-first":
        return assetCacheFirst(manifest, request);
      case "blob-cache-first":
        return blobCacheFirst(request);
      case "page-network-first":
        return pageNetworkFirst(manifest, request);
      default:
        return null;
    }
  }

  return { install, activate, respond };
}

/** Where the worker is served and the scope it controls. */
export const SERVICE_WORKER_URL = "/sw.js";
export const SERVICE_WORKER_SCOPE = "/";

/** Registration gate: service workers need a secure context (HTTPS or localhost). */
export function shouldRegisterServiceWorker(env: { isSecureContext: boolean; hasServiceWorker: boolean }): boolean {
  return env.isSecureContext && env.hasServiceWorker;
}

/** Register the shell worker from the page; failures are logged, never thrown. */
export async function registerShellServiceWorker(
  container: Pick<ServiceWorkerContainer, "register"> | undefined,
  isSecureContext: boolean,
): Promise<boolean> {
  if (!container || !shouldRegisterServiceWorker({ isSecureContext, hasServiceWorker: true })) return false;
  try {
    await container.register(SERVICE_WORKER_URL, { scope: SERVICE_WORKER_SCOPE });
    return true;
  } catch (err) {
    console.warn("[id] service worker registration failed:", err);
    return false;
  }
}
