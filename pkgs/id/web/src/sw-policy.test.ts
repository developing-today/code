import { describe, expect, it, vi } from "vitest";
import {
  BLOBS_CACHE_NAME,
  classifyRequest,
  createShellWorker,
  isCacheableBlobResponse,
  isCacheablePageResponse,
  MAX_CACHED_BLOB_BYTES,
  MAX_CACHED_BLOBS,
  MAX_CACHED_PAGES,
  obsoleteCacheNames,
  offlineBlobResponse,
  offlineResponse,
  pageCacheKey,
  pagesCacheName,
  parseShellManifest,
  precacheUrls,
  registerShellServiceWorker,
  type ShellManifest,
  shellCacheName,
} from "./sw-policy";

/**
 * #13918 slices 1 and 3 behavioral tests. Oracle: the design record
 * (doc/2026-11-25_design_id_service_worker_offline_13918.md §4–§5):
 * hashed shell assets cache-first, navigations network-first with a
 * per-URL offline copy, content-addressed blobs cache-first (slice 3,
 * #14774), and nothing that writes or carries a credential is ever
 * intercepted.
 */

const ORIGIN = "http://localhost:3000";
const MANIFEST: ShellManifest = { "main.js": "main.ab12cd34.js", "styles.css": "styles.0123abcd.css" };

/** In-memory CacheStorage keyed by URL string, insertion-ordered like the real one. */
class FakeCache {
  readonly entries = new Map<string, Response>();
  async match(key: RequestInfo | URL): Promise<Response | undefined> {
    const hit = this.entries.get(keyOf(key));
    return hit?.clone();
  }
  async put(key: RequestInfo | URL, response: Response): Promise<void> {
    this.entries.set(keyOf(key), response.clone());
  }
  async delete(key: RequestInfo | URL): Promise<boolean> {
    return this.entries.delete(keyOf(key));
  }
  async keys(): Promise<Request[]> {
    return [...this.entries.keys()].map((url) => new Request(url));
  }
}

function keyOf(key: RequestInfo | URL): string {
  if (typeof key === "string") return new URL(key, ORIGIN).toString();
  if (key instanceof URL) return key.toString();
  return key.url;
}

class FakeCacheStorage {
  readonly stores = new Map<string, FakeCache>();
  async open(name: string): Promise<FakeCache> {
    let cache = this.stores.get(name);
    if (!cache) {
      cache = new FakeCache();
      this.stores.set(name, cache);
    }
    return cache;
  }
  async keys(): Promise<string[]> {
    return [...this.stores.keys()];
  }
  async delete(name: string): Promise<boolean> {
    return this.stores.delete(name);
  }
}

function html(body: string, init: ResponseInit = {}): Response {
  return new Response(body, {
    status: 200,
    ...init,
    headers: { "content-type": "text/html; charset=utf-8", ...(init.headers as Record<string, string>) },
  });
}

/** What the server's `/blob/<hash>` handler sends for a whole blob. */
function blob(body: string, init: ResponseInit = {}): Response {
  return new Response(body, {
    status: 200,
    ...init,
    headers: {
      "content-type": "application/octet-stream",
      "content-length": String(new TextEncoder().encode(body).length),
      "cache-control": "public, max-age=31536000, immutable",
      ...(init.headers as Record<string, string>),
    },
  });
}

function navigate(path: string): Request {
  // `mode` cannot be set to "navigate" through the constructor; tests carry it explicitly.
  const request = new Request(`${ORIGIN}${path}`);
  Object.defineProperty(request, "mode", { value: "navigate" });
  return request;
}

function setup(fetchImpl: (input: RequestInfo | URL) => Promise<Response>, manifest: ShellManifest | null = MANIFEST) {
  const caches = new FakeCacheStorage();
  const fetch = vi.fn(fetchImpl);
  const worker = createShellWorker({
    caches: caches as unknown as CacheStorage,
    fetch,
    manifest,
    origin: ORIGIN,
  });
  return { caches, fetch, worker };
}

describe("parseShellManifest", () => {
  it("accepts only the #6779 emitted-name contract", () => {
    expect(parseShellManifest({ ...MANIFEST, extra: "x" })).toEqual(MANIFEST);
    expect(parseShellManifest({ "main.js": "main.js", "styles.css": MANIFEST["styles.css"] })).toBeNull();
    expect(parseShellManifest({ "main.js": MANIFEST["main.js"], "styles.css": "styles.darkmode.css" })).toBeNull();
    expect(parseShellManifest({ "main.js": "../main.ab12cd34.js", "styles.css": MANIFEST["styles.css"] })).toBeNull();
    expect(parseShellManifest(null)).toBeNull();
    expect(parseShellManifest([MANIFEST])).toBeNull();
    expect(parseShellManifest(undefined)).toBeNull();
  });

  it("derives versioned cache names and precache URLs from the build hashes", () => {
    expect(shellCacheName(MANIFEST)).toBe("id-shell-v1-ab12cd34-0123abcd");
    expect(pagesCacheName(MANIFEST)).toBe("id-pages-v1-ab12cd34-0123abcd");
    expect(precacheUrls(MANIFEST)).toEqual(["/assets/main.ab12cd34.js", "/assets/styles.0123abcd.css"]);
  });
});

describe("classifyRequest", () => {
  const get = (path: string, mode = "cors") => ({ method: "GET", url: `${ORIGIN}${path}`, mode });

  it("serves hashed shell assets cache-first", () => {
    expect(classifyRequest(get("/assets/main.ab12cd34.js"), ORIGIN)).toBe("asset-cache-first");
    expect(classifyRequest(get("/assets/styles.0123abcd.css"), ORIGIN)).toBe("asset-cache-first");
  });

  it("passes unhashed assets, the manifest and the worker itself through", () => {
    expect(classifyRequest(get("/assets/manifest.json"), ORIGIN)).toBe("passthrough");
    expect(classifyRequest(get("/assets/sw.js"), ORIGIN)).toBe("passthrough");
    expect(classifyRequest(get("/assets/main.ab12cd34.js.map"), ORIGIN)).toBe("passthrough");
    expect(classifyRequest(get("/sw.js", "navigate"), ORIGIN)).toBe("passthrough");
  });

  it("routes navigations network-first", () => {
    expect(classifyRequest(get("/", "navigate"), ORIGIN)).toBe("page-network-first");
    expect(classifyRequest(get("/edit/notes.md", "navigate"), ORIGIN)).toBe("page-network-first");
  });

  it("never intercepts writes, online-only paths, credentials or other origins", () => {
    expect(classifyRequest({ method: "POST", url: `${ORIGIN}/api/save`, mode: "cors" }, ORIGIN)).toBe("passthrough");
    expect(classifyRequest({ method: "POST", url: `${ORIGIN}/`, mode: "navigate" }, ORIGIN)).toBe("passthrough");
    for (const path of ["/api/files", "/ws/tags", "/blob/", "/blob/a/b", "/.well-known/id-transport-advertisement"]) {
      expect(classifyRequest(get(path, "navigate"), ORIGIN)).toBe("passthrough");
    }
    expect(classifyRequest(get("/?token=secret", "navigate"), ORIGIN)).toBe("passthrough");
    expect(classifyRequest(get("/files?a=1&token=x", "navigate"), ORIGIN)).toBe("passthrough");
    expect(classifyRequest({ method: "GET", url: "https://cdn.example/x.js", mode: "navigate" }, ORIGIN)).toBe(
      "passthrough",
    );
    expect(classifyRequest(get("/files", "cors"), ORIGIN)).toBe("passthrough");
  });

  it("serves whole content-addressed blobs cache-first", () => {
    expect(classifyRequest(get("/blob/abc123"), ORIGIN)).toBe("blob-cache-first");
    expect(classifyRequest(get("/blob/abc123?filename=a.png", "navigate"), ORIGIN)).toBe("blob-cache-first");
  });

  it("passes token-bearing and range blob requests through", () => {
    expect(classifyRequest(get("/blob/abc123?token=secret"), ORIGIN)).toBe("passthrough");
    expect(classifyRequest(get("/blob/abc123?filename=a&token=x"), ORIGIN)).toBe("passthrough");
    const range = { ...get("/blob/abc123"), headers: new Headers({ range: "bytes=0-99" }) };
    expect(classifyRequest(range, ORIGIN)).toBe("passthrough");
    expect(classifyRequest({ method: "HEAD", url: `${ORIGIN}/blob/abc123`, mode: "cors" }, ORIGIN)).toBe("passthrough");
    expect(classifyRequest({ method: "GET", url: "https://cdn.example/blob/abc123", mode: "cors" }, ORIGIN)).toBe(
      "passthrough",
    );
  });
});

describe("isCacheableBlobResponse", () => {
  it("keeps only immutable, sized 200s within the cap", () => {
    expect(isCacheableBlobResponse(blob("abc"))).toBe(true);
    expect(isCacheableBlobResponse(blob("x", { status: 404 }))).toBe(false);
    expect(isCacheableBlobResponse(blob("x", { headers: { "cache-control": "public, max-age=60" } }))).toBe(false);
    expect(isCacheableBlobResponse(blob("x", { headers: { "cache-control": "immutable, no-store" } }))).toBe(false);
    expect(isCacheableBlobResponse(new Response("x", { headers: { "cache-control": "immutable" } }))).toBe(false);
    expect(
      isCacheableBlobResponse(blob("x", { headers: { "content-length": String(MAX_CACHED_BLOB_BYTES + 1) } })),
    ).toBe(false);
    expect(isCacheableBlobResponse(blob("x", { headers: { "content-length": String(MAX_CACHED_BLOB_BYTES) } }))).toBe(
      true,
    );
    const redirected = blob("x");
    Object.defineProperty(redirected, "redirected", { value: true });
    expect(isCacheableBlobResponse(redirected)).toBe(false);
  });

  it("answers an unkept blob offline with an uncacheable 503", () => {
    const response = offlineBlobResponse();
    expect(response.status).toBe(503);
    expect(response.headers.get("cache-control")).toBe("no-store");
  });
});

describe("isCacheablePageResponse", () => {
  it("keeps only plain final HTML 200s", () => {
    expect(isCacheablePageResponse(html("<p>ok</p>"))).toBe(true);
    expect(isCacheablePageResponse(html("<p>ok</p>", { headers: { "cache-control": "no-cache" } }))).toBe(true);
    expect(isCacheablePageResponse(html("x", { status: 401 }))).toBe(false);
    expect(isCacheablePageResponse(html("x", { status: 500 }))).toBe(false);
    expect(isCacheablePageResponse(html("x", { headers: { "cache-control": "private, no-store" } }))).toBe(false);
    expect(isCacheablePageResponse(new Response("{}", { headers: { "content-type": "application/json" } }))).toBe(
      false,
    );
    const redirected = html("x");
    Object.defineProperty(redirected, "redirected", { value: true });
    expect(isCacheablePageResponse(redirected)).toBe(false);
  });
});

describe("pageCacheKey and offlineResponse", () => {
  it("drops the fragment but keeps the query", () => {
    expect(pageCacheKey(`${ORIGIN}/view/a.md?x=1#top`)).toBe(`${ORIGIN}/view/a.md?x=1`);
  });

  it("is an uncacheable 503 page with a way home", async () => {
    const response = offlineResponse();
    expect(response.status).toBe(503);
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(await response.text()).toContain('href="/"');
  });
});

describe("obsoleteCacheNames", () => {
  it("removes only this worker's caches from other builds", () => {
    const names = ["id-shell-v1-old00000-old00000", shellCacheName(MANIFEST), pagesCacheName(MANIFEST), "other-app"];
    expect(obsoleteCacheNames(names, MANIFEST)).toEqual(["id-shell-v1-old00000-old00000"]);
    expect(obsoleteCacheNames(names, null)).toEqual(names.slice(0, 3));
  });

  it("keeps the blob cache across builds and drops it with the kill switch", () => {
    const names = [BLOBS_CACHE_NAME, "id-blobs-v0", shellCacheName(MANIFEST)];
    expect(obsoleteCacheNames(names, MANIFEST)).toEqual(["id-blobs-v0"]);
    expect(obsoleteCacheNames(names, null)).toEqual(names);
  });
});

describe("createShellWorker", () => {
  it("precaches the shell on install and fails install when an asset is missing", async () => {
    const ok = setup(async (input) => new Response(`asset ${String(input)}`));
    await ok.worker.install();
    const shell = ok.caches.stores.get(shellCacheName(MANIFEST));
    expect([...(shell?.entries.keys() ?? [])]).toEqual(precacheUrls(MANIFEST).map((u) => `${ORIGIN}${u}`));

    const broken = setup(async () => new Response("nope", { status: 404 }));
    await expect(broken.worker.install()).rejects.toThrow(/precache/);
  });

  it("deletes other builds' caches on activate", async () => {
    const { caches, worker } = setup(async () => new Response("x"));
    await caches.open("id-pages-v1-old00000-old00000");
    await caches.open(shellCacheName(MANIFEST));
    await caches.open("unrelated");
    await worker.activate();
    expect(await caches.keys()).toEqual([shellCacheName(MANIFEST), "unrelated"]);
  });

  it("serves shell assets from cache without the network", async () => {
    const { worker, fetch } = setup(async (input) => new Response(`asset ${String(input)}`));
    await worker.install();
    fetch.mockClear();
    const response = await worker.respond(new Request(`${ORIGIN}/assets/main.ab12cd34.js`));
    expect(await response?.text()).toBe("asset /assets/main.ab12cd34.js");
    expect(fetch).not.toHaveBeenCalled();
  });

  it("reads a visited page offline and shows the offline page for an unvisited one", async () => {
    let online = true;
    const { worker } = setup(async (input) => {
      if (!online) throw new TypeError("Failed to fetch");
      return html(`<p>live ${new URL(String(input instanceof Request ? input.url : input)).pathname}</p>`);
    });
    expect(await (await worker.respond(navigate("/files")))?.text()).toBe("<p>live /files</p>");

    online = false;
    const kept = await worker.respond(navigate("/files#row-3"));
    expect(kept?.status).toBe(200);
    expect(await kept?.text()).toBe("<p>live /files</p>");
    const missing = await worker.respond(navigate("/settings"));
    expect(missing?.status).toBe(503);
  });

  it("returns live server errors unchanged and never caches them", async () => {
    let status = 401;
    let online = true;
    const { worker } = setup(async () => {
      if (!online) throw new TypeError("Failed to fetch");
      return html("denied", { status });
    });
    expect((await worker.respond(navigate("/")))?.status).toBe(401);
    status = 200;
    online = false;
    expect((await worker.respond(navigate("/")))?.status).toBe(503);
  });

  it("keeps at most MAX_CACHED_PAGES pages, dropping the least recent", async () => {
    const { worker, caches } = setup(async (input) => html(String(input instanceof Request ? input.url : input)));
    for (let i = 0; i <= MAX_CACHED_PAGES; i += 1) {
      await worker.respond(navigate(`/view/${i}`));
    }
    const pages = caches.stores.get(pagesCacheName(MANIFEST));
    expect(pages?.entries.size).toBe(MAX_CACHED_PAGES);
    expect(pages?.entries.has(`${ORIGIN}/view/0`)).toBe(false);
    expect(pages?.entries.has(`${ORIGIN}/view/${MAX_CACHED_PAGES}`)).toBe(true);
  });

  it("serves a fetched blob offline and answers an unkept one with 503", async () => {
    let online = true;
    const { worker, fetch } = setup(async () => {
      if (!online) throw new TypeError("Failed to fetch");
      return blob("blob bytes");
    });
    expect(await (await worker.respond(new Request(`${ORIGIN}/blob/abc123`)))?.text()).toBe("blob bytes");

    online = false;
    fetch.mockClear();
    const kept = await worker.respond(new Request(`${ORIGIN}/blob/abc123#x`));
    expect(kept?.status).toBe(200);
    expect(await kept?.text()).toBe("blob bytes");
    expect(fetch).not.toHaveBeenCalled();
    expect((await worker.respond(new Request(`${ORIGIN}/blob/def456`)))?.status).toBe(503);
    expect((await worker.respond(new Request(`${ORIGIN}/blob/abc123?filename=a.txt`)))?.status).toBe(503);
  });

  it("never keeps a blob the server does not mark immutable", async () => {
    const { worker, caches } = setup(async () => blob("x", { headers: { "cache-control": "no-cache" } }));
    expect((await worker.respond(new Request(`${ORIGIN}/blob/abc123`)))?.status).toBe(200);
    expect((await caches.open(BLOBS_CACHE_NAME)).entries.size).toBe(0);
  });

  it("keeps at most MAX_CACHED_BLOBS blobs, dropping the least recent", async () => {
    const { worker, caches } = setup(async () => blob("b"));
    for (let i = 0; i <= MAX_CACHED_BLOBS; i += 1) {
      await worker.respond(new Request(`${ORIGIN}/blob/h${i}`));
    }
    const blobs = caches.stores.get(BLOBS_CACHE_NAME);
    expect(blobs?.entries.size).toBe(MAX_CACHED_BLOBS);
    expect(blobs?.entries.has(`${ORIGIN}/blob/h0`)).toBe(false);
    expect(blobs?.entries.has(`${ORIGIN}/blob/h${MAX_CACHED_BLOBS}`)).toBe(true);
  });

  it("passes everything through with no valid manifest", async () => {
    const { worker } = setup(async () => html("x"), null);
    await worker.install();
    expect(worker.respond(navigate("/"))).toBeNull();
    expect(worker.respond(new Request(`${ORIGIN}/assets/main.ab12cd34.js`))).toBeNull();
    expect(worker.respond(new Request(`${ORIGIN}/blob/abc123`))).toBeNull();
  });

  it("leaves passthrough requests to the browser", () => {
    const { worker } = setup(async () => html("x"));
    expect(worker.respond(new Request(`${ORIGIN}/api/files`))).toBeNull();
    expect(worker.respond(new Request(`${ORIGIN}/api/save`, { method: "POST", body: "{}" }))).toBeNull();
    expect(worker.respond(new Request(`${ORIGIN}/blob/abc123`, { headers: { range: "bytes=0-9" } }))).toBeNull();
  });
});

describe("registerShellServiceWorker", () => {
  it("registers /sw.js at root scope in a secure context", async () => {
    const register = vi.fn(async () => ({}) as ServiceWorkerRegistration);
    expect(await registerShellServiceWorker({ register }, true)).toBe(true);
    expect(register).toHaveBeenCalledWith("/sw.js", { scope: "/" });
  });

  it("skips insecure contexts and missing support, and swallows failures", async () => {
    const register = vi.fn(async () => {
      throw new Error("blocked");
    });
    expect(await registerShellServiceWorker({ register }, false)).toBe(false);
    expect(register).not.toHaveBeenCalled();
    expect(await registerShellServiceWorker(undefined, true)).toBe(false);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(await registerShellServiceWorker({ register }, true)).toBe(false);
    warn.mockRestore();
  });
});
