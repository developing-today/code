// The real aggregator service worker, served at /sw.js (#3676 — the
// #1254 design's slice (b), doc 2026-11-01T03-54-00Z §6 step 2).
//
// History: /sw.js served a self-unregistering kill switch to evict
// the opencode Workbox worker stranded on this origin. The design
// gates outright replacement on 14 days of /api/sw-probe data showing
// no foreign registration; no probe data exists yet, so per the
// design's fallback path THIS worker carries the eviction job
// forward for this release: on activate it purges every cache on the
// origin (a foreign worker's precache is its residue — the worker
// itself is superseded the moment the browser updates this scope's
// registration to this script), and the page-side probe script
// (src/assets/push-client.js) unregisters any registration whose
// script URL is not /sw.js. The invariant holds: /sw.js is never
// served in a form that neither evicts the stranded worker nor is
// the real worker.
export const SW_SOURCE = `// pm-aggregator service worker — real worker (alert throat, #3676).
// push + notificationclick handlers; activate purges foreign caches
// (design 2026-11-01T03-54-00Z §6 fallback eviction).
self.addEventListener('install', () => { self.skipWaiting(); });
self.addEventListener('activate', (event) => {
  event.waitUntil((async () => {
    try {
      const keys = await caches.keys();
      await Promise.all(keys.map((k) => caches.delete(k)));
    } catch {}
    await self.clients.claim();
  })());
});
self.addEventListener('push', (event) => {
  let data = {};
  try { data = event.data ? event.data.json() : {}; } catch { data = {}; }
  const title = data.title || 'PM alert';
  const options = {
    body: data.body || '',
    tag: data.objectRef || undefined,
    renotify: !!data.escalation,
    data: { url: data.url || '/alerts', class: data.class || null, objectRef: data.objectRef || null },
  };
  event.waitUntil(self.registration.showNotification(title, options));
});
self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const target = (event.notification.data && event.notification.data.url) || '/alerts';
  event.waitUntil((async () => {
    const url = new URL(target, self.location.origin);
    const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    for (const client of windows) {
      try {
        if (new URL(client.url).origin === url.origin) {
          await client.navigate(url.href);
          return client.focus();
        }
      } catch {}
    }
    return self.clients.openWindow(url.href);
  })());
});
`;
