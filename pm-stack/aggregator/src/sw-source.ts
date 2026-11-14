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
    data: { url: data.url || '/alerts', class: data.class || null, objectRef: data.objectRef || null, items: Array.isArray(data.items) ? data.items : null },
  };
  // #8568: a needs-Drew push carries tappable answers — the first two
  // options become notification actions, and the answer authority
  // (ledger id + token, parsed out of the answer-page URL) rides in
  // the notification data so a tap can POST the answer directly.
  if (data.answer && data.answer.ledgerId) {
    const opts = Array.isArray(data.answer.options) ? data.answer.options : [];
    options.actions = opts.slice(0, 2).map((o) => ({ action: o.id, title: o.label }));
    let token = null;
    try { token = new URL(data.answer.url || data.url || '', self.location.origin).searchParams.get('t'); } catch {}
    options.data.answerLedgerId = data.answer.ledgerId;
    options.data.answerToken = token;
    options.data.answerUrl = data.answer.url || data.url || null;
  }
  event.waitUntil(self.registration.showNotification(title, options));
});
self.addEventListener('notificationclick', (event) => {
  const nd = event.notification.data || {};
  // #8568: an answer-action tap answers the question in place — POST
  // the chosen option to the answer endpoint (the per-question token
  // is the authority), close, done. Errors are silent: the answer
  // page remains one tap away on the notification body.
  if (event.action && nd.answerLedgerId && nd.answerToken) {
    event.notification.close();
    event.waitUntil((async () => {
      try {
        await fetch('/api/needs-drew/' + encodeURIComponent(nd.answerLedgerId) + '/answer', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ token: nd.answerToken, optionId: event.action }),
        });
      } catch {}
    })());
    return;
  }
  event.notification.close();
  // A single-item digest deep-links to that item; multi-item digests open
  // their (refs-filtered) landing URL with the items carried in data.
  const target = (Array.isArray(nd.items) && nd.items.length === 1 && nd.items[0] && nd.items[0].url) || nd.url || '/alerts';
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
