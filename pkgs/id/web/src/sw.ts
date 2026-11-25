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
type PushEvent = ExtendableEvent & { data: { json(): unknown } | null };
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
  registration: {
    showNotification(title: string, options: { body: string; data: unknown; tag: string }): Promise<void>;
  };
  addEventListener(type: "install" | "activate", listener: (event: ExtendableEvent) => void): void;
  addEventListener(type: "fetch", listener: (event: FetchEvent) => void): void;
  addEventListener(type: "push", listener: (event: PushEvent) => void): void;
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

// Push content is intentionally reduced to a head-only hint. The
// notification text is fixed local copy; world data is fetched again
// from the host after the user returns to the PWA.
scope.addEventListener("push", (event) => {
  event.waitUntil(
    (async () => {
      let payload: unknown;
      try {
        payload = event.data?.json();
      } catch {
        return;
      }
      if (typeof payload !== "object" || payload === null || Array.isArray(payload)) return;
      const record = payload as Record<string, unknown>;
      if (record.type !== "client.notify") return;
      if (
        record.head !== undefined &&
        record.head !== null &&
        (typeof record.head !== "number" || !Number.isSafeInteger(record.head) || record.head < 0)
      ) {
        return;
      }
      const data = { type: "client.notify", ...(record.head == null ? {} : { head: record.head }) };
      await scope.registration.showNotification("World update", {
        body: "A world may have changed. Open ID to catch up.",
        data,
        tag: "id-world-update",
      });
    })(),
  );
});

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
      client.postMessage(data);
    })(),
  );
});
