/**
 * #13918 slice 1: the ID web app's service worker (built to dist/sw.js,
 * served at /sw.js). A thin event adapter over sw-policy.ts; see
 * doc/2026-11-25_design_id_service_worker_offline_13918.md.
 *
 * The build (scripts/build-manifest.ts) prefixes dist/sw.js with
 * `self.__ID_SHELL_MANIFEST__={...};`, so this file's bytes change with
 * every shell build and the browser installs the new worker. With no
 * valid manifest the worker caches nothing, passes every request
 * through, and deletes its old caches on activate.
 */

import { createShellWorker, parseShellManifest } from "./sw-policy";

// tsconfig targets the DOM lib; these are the few worker-scope shapes used here.
type ExtendableEvent = Event & { waitUntil(promise: Promise<unknown>): void };
type FetchEvent = ExtendableEvent & { request: Request; respondWith(response: Promise<Response>): void };
type WindowClient = { url: string; focus(): Promise<WindowClient>; postMessage(message: unknown): void };
type NotificationClickEvent = ExtendableEvent & {
  notification: { data: unknown; close(): void };
};
type ServiceWorkerScope = {
  __ID_SHELL_MANIFEST__?: unknown;
  caches: CacheStorage;
  location: Location;
  fetch: typeof fetch;
  skipWaiting(): Promise<void>;
  clients: {
    claim(): Promise<void>;
    matchAll(options: { type: "window", includeUncontrolled: boolean }): Promise<WindowClient[]>;
    openWindow(url: string): Promise<WindowClient | null>;
  };
  addEventListener(type: "install" | "activate", listener: (event: ExtendableEvent) => void): void;
  addEventListener(type: "fetch", listener: (event: FetchEvent) => void): void;
  addEventListener(type: "notificationclick", listener: (event: NotificationClickEvent) => void): void;
};

const scope = self as unknown as ServiceWorkerScope;

const worker = createShellWorker({
  caches: scope.caches,
  fetch: (input, init) => scope.fetch(input, init),
  manifest: parseShellManifest(scope.__ID_SHELL_MANIFEST__),
  origin: scope.location.origin,
});

scope.addEventListener("install", (event) => {
  event.waitUntil(worker.install().then(() => scope.skipWaiting()));
});

scope.addEventListener("activate", (event) => {
  event.waitUntil(worker.activate().then(() => scope.clients.claim()));
});

scope.addEventListener("fetch", (event) => {
  const response = worker.respond(event.request);
  if (response) event.respondWith(response);
});

// Return seam only: notification delivery and its local permission gate
// belong to the provider adapter, not to this worker's fetch/cache code.
// Never interpret notification data as authoritative world state.
scope.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const data = event.notification.data;
  if (typeof data !== "object" || data === null || Array.isArray(data) || (data as Record<string, unknown>).type !== "client.notify") {
    return;
  }
  event.waitUntil(
    (async () => {
      const clients = await scope.clients.matchAll({ type: "window", includeUncontrolled: true });
      const worldClient = clients.find((candidate) => new URL(candidate.url).pathname === "/world");
      const client = worldClient ?? (await scope.clients.openWindow("/world?client-notify=1"));
      if (!client) return;
      await client.focus();
      client.postMessage({ type: "client.notify" });
    })(),
  );
});
