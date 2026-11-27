// The app's service worker. Three jobs:
//
//   1. It shows a notification when one is pushed, and opens the right
//      screen when it is tapped.
//   2. It keeps a copy of the app's own files and of the last answer to
//      everything the app reads, so the app opens and shows what it
//      last saw with no connection. The network is always asked first:
//      a copy is only shown when the network gives no answer. And
//      when there is no copy either — the app never kept on this
//      device, opened with no connection — the worker answers the
//      page itself, with a small offline page of its own.
//   3. It sends what the owner did while offline (outbox.js) when the
//      connection is back, if the app is not open to do it itself.
//
// Nothing here is written with a secret in it: what is kept is what the
// app showed, and what the owner did.
self.importScripts("/console/outbox.js");

const KEPT = "pm-console-v1";
/** Where the app's own files are named: one list (public/files.json)
 * that the services, the Worker's build and this worker all read. */
const LIST = "/console/files.json";
const PAGE = "/console/";
const EXTRAS = ["/manifest.webmanifest", "/icons/icon-192.png", "/icons/icon-512.png"];
/** What one of the app's files ends in. Anything else under /console
 * (the Android build, say, which is large and is a download) is none
 * of this worker's business. */
const APP_FILE = /^\/console\/[a-z0-9/_-]+\.(?:js|css|svg|json|woff2)$/;

/** The files to fetch when the worker is installed, so that the first
 * visit is already enough to open the app offline: the page, the list,
 * and what the list says is kept from the start. A typeface is kept
 * when it is first used instead, so nobody fetches the ones they did
 * not choose. Without the list (it could not be fetched) it is the
 * page alone; the rest is kept as the page next asks for it. */
async function filesToKeep() {
  try {
    const res = await self.fetch(LIST, { cache: "no-store", redirect: "manual" });
    if (!res.ok) return [PAGE];
    const list = await res.json();
    const named = (list.files ?? []).filter((f) => f.kept === "installed").map((f) => `/console/${f.path}`);
    return [...new Set([PAGE, LIST, ...named])];
  } catch {
    return [PAGE];
  }
}
/** Every file the list names, as paths, or null when the list cannot
 * be read: then nothing kept can be told apart, and nothing is pruned. */
async function listedPaths() {
  try {
    const res = await self.fetch(LIST, { cache: "no-store", redirect: "manual" });
    if (!res.ok) return null;
    const list = await res.json();
    return new Set((list.files ?? []).map((f) => `/console/${f.path}`));
  } catch {
    return null;
  }
}
/** How long the network is waited for before a copy is shown instead.
 * The network's answer still replaces the copy when it comes. */
const PAGE_WAIT_MS = 4_000;
const READ_WAIT_MS = 8_000;

self.addEventListener("install", (event) => {
  event.waitUntil(
    (async () => {
      // Keeping the files is on a best effort. With no cache to be had
      // the worker is installed all the same, for what else it does:
      // notifications, and sending what was done offline.
      try {
        const cache = await self.caches.open(KEPT);
        // Each by itself: one that cannot be fetched (not signed in,
        // say) does not stop the others, or the worker.
        const files = await filesToKeep();
        await Promise.all([...files, ...EXTRAS].map((path) => keep(cache, path).catch(() => {})));
      } catch {
        // Nothing is kept this time.
      }
      await self.skipWaiting();
    })(),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      try {
        for (const name of await self.caches.keys()) {
          if (name.startsWith("pm-console-") && name !== KEPT) await self.caches.delete(name);
        }
      } catch {
        // An older copy that cannot be cleared is only space.
      }
      // What an older build kept that this one no longer knows — a
      // file the list has since dropped, say — goes too: a new build
      // replaces what is kept; it does not pile onto it. The page,
      // the list itself and what the app read stay: they are the
      // offline copy, refreshed as the network answers. When the
      // list cannot be read (no connection, say) nothing is pruned:
      // what this build wants cannot be told from what it dropped.
      try {
        const listed = await listedPaths();
        if (listed) {
          const cache = await self.caches.open(KEPT);
          for (const request of await cache.keys()) {
            const path = new URL(request.url).pathname;
            if (path === PAGE || path === LIST || listed.has(path)) continue;
            if (EXTRAS.includes(path) || path.startsWith("/icons/")) continue;
            if (path.startsWith("/console/api/") || path === "/console/health" || path.startsWith("/console/__seen__/")) continue;
            await cache.delete(request);
          }
        }
      } catch {
        // What cannot be cleared is only space.
      }
      await self.clients.claim();
    })(),
  );
});

/** Fetch one of the app's files and keep it. */
async function keep(cache, path) {
  const res = await self.fetch(path, { cache: "no-store", redirect: "manual" });
  if (res.ok) await cache.put(path, res);
}

/** What a request is, to this worker: the app's page, one of its
 * files, something it reads, or none of its business. */
function kindOf(request, url) {
  if (request.method !== "GET" || url.origin !== self.location.origin) return null;
  // Asked on purpose past this worker: is there a sign-in, is there a network.
  if (request.headers.get("x-pm-probe")) return null;
  const path = url.pathname;
  if (path.startsWith("/console/api/") || path === "/console/health") return "read";
  if (EXTRAS.includes(path) || path.startsWith("/icons/")) return "file";
  if (!path.startsWith("/console")) return null;
  // The app is one page, at one address (its screens are after the #).
  if (path === "/console/" || path === "/console") return "page";
  return APP_FILE.test(path) && path !== "/console/sw.js" ? "file" : null;
}

/** The address a copy is kept under: what was asked, without the mark
 * that only says "ask the source again" (the answer is the same thing). */
function keyOf(url, kind) {
  if (kind === "page") return "/console/";
  const params = new URLSearchParams(url.search);
  params.delete("fresh");
  const query = params.toString();
  return url.pathname + (query ? `?${query}` : "");
}

/** Where it is noted that a copy was said to be current still. Not an
 * address the app ever asks for. */
const seenKey = (key) => `/console/__seen__/${encodeURIComponent(key)}`;

/** A copy of an answer, marked with when it was kept. */
async function stamped(res, extra) {
  const headers = new Headers(res.headers);
  for (const [name, value] of Object.entries(extra)) headers.set(name, value);
  return new Response(await res.blob(), { status: res.status, statusText: res.statusText, headers });
}

const after = (ms) => new Promise((resolve) => self.setTimeout(() => resolve(null), ms));

/** The network first; the copy when the network gives no answer, or is
 * too slow to wait for. An answer that is not a plain success (a
 * refusal, a way to the sign-in, an error) is passed on as it is and
 * never replaced by a copy: the app has to see it. Gives what to show,
 * and the keeping of the network's answer, which may come after. */
function answer(request, kind) {
  const key = keyOf(new URL(request.url), kind);
  // With no cache to be had (a browser that refuses this site any
  // storage) nothing is kept and nothing kept is shown: the network
  // answers as if this worker were not here.
  const opened = self.caches.open(KEPT).catch(() => null);
  const fromNetwork = (async () => {
    const res = await self.fetch(request);
    // Keeping it is on a best effort: an answer the network gave is
    // the answer, whether or not this device had room to keep a copy.
    try {
      const cache = await opened;
      if (!cache) return res;
      if (res.status === 200) {
        const copy = kind === "read" ? await stamped(res.clone(), { "x-pm-kept-at": String(Date.now()) }) : res.clone();
        await cache.put(key, copy);
      } else if (res.status === 304 && kind === "read") {
        // The app's own copy is current, and so is the one kept here:
        // nothing new to keep but when that was, which is small.
        const seen = { "x-pm-kept-at": String(Date.now()), "x-as-of": res.headers.get("x-as-of") ?? "" };
        await cache.put(seenKey(key), new Response("", { headers: seen }));
      }
    } catch {
      // No room: nothing is kept this time.
    }
    return res;
  })();
  const kept = async () => {
    try {
      const cache = await opened;
      const copy = await cache?.match(key);
      if (!copy) return null;
      if (kind !== "read") return copy;
      // As current as it was last said to be, which may be later than
      // when it was last sent whole.
      const marks = { "x-pm-offline": "1" };
      const seen = await cache.match(seenKey(key));
      if (seen && Number(seen.headers.get("x-pm-kept-at")) > Number(copy.headers.get("x-pm-kept-at"))) {
        marks["x-pm-kept-at"] = seen.headers.get("x-pm-kept-at");
        if (seen.headers.get("x-as-of")) marks["x-as-of"] = seen.headers.get("x-as-of");
      }
      return await stamped(copy, marks);
    } catch {
      // A copy that cannot be read is no copy.
      return null;
    }
  };
  const shown = (async () => {
    const first = await Promise.race([
      fromNetwork.then(
        (res) => ({ res }),
        () => ({ failed: true }),
      ),
      after(kind === "read" ? READ_WAIT_MS : PAGE_WAIT_MS),
    ]);
    if (first?.res) return first.res;
    const copy = await kept();
    if (copy) return copy;
    // No copy to show: the network's answer, however long it takes.
    // For the page, a network that gives no answer at all gets the
    // worker's own offline page instead of the browser's error: the
    // owner is told what happened and given the way back. A read or
    // a file is left to fail as it does — the app answers for those
    // itself, and a made-up answer could be mistaken for the server's.
    if (kind === "page") return fromNetwork.catch(() => offlinePage());
    return fromNetwork;
  })();
  return {
    shown,
    // Whatever becomes of the race above, the network's answer is kept when it comes.
    keeping: fromNetwork.then(
      () => {},
      () => {},
    ),
  };
}

/** The page for the one case the copies cannot cover: the app was
 * never kept on this device and there is no connection. Small and
 * whole in itself — nothing it names could be fetched either — and
 * never kept: it is not the app, only word that the app is not here. */
function offlinePage() {
  return new Response(
    "<!doctype html><title>No connection</title><h1>No connection</h1><p>The app has not been kept on this device yet, and there is no connection to fetch it with. Connect once and open the app; from then on it opens from this device.</p><p><a href=\"/console/\">Try the app again</a></p>",
    { status: 503, headers: { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" } },
  );
}

self.addEventListener("fetch", (event) => {
  const kind = kindOf(event.request, new URL(event.request.url));
  if (!kind) return;
  const { shown, keeping } = answer(event.request, kind);
  event.respondWith(shown);
  event.waitUntil(keeping);
});

// ---- what the owner did while offline ------------------------------------

/** Send what is waiting. Told to every open page, so each shows it.
 * `last`: the browser will not call again for this by itself. `own`:
 * an entry whose outcome the caller says itself (a button pressed on a
 * notification), left out of what is said here. */
async function send(last = false, own = null) {
  const store = self.PMOutbox.idbStore();
  const flush = () =>
    self.PMOutbox.flush(store, (entry) => self.PMOutbox.httpSend(entry, (url, init) => self.fetch(url, init)));
  // One sender at a time, the app's page included: if it is sending
  // now, this has nothing to add.
  const locks = self.navigator?.locks;
  const out = locks?.request ? await locks.request("pm-outbox", { ifAvailable: true }, (lock) => (lock ? flush() : null)) : await flush();
  if (!out) return;
  const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  // What became of each, by state only: enough for a page to say it.
  const became = out.changed.map((e) => ({ state: e.state }));
  for (const client of windows) client.postMessage({ type: "outbox", changed: became });
  // With no page open to show it, say what became of it.
  const open = windows.some((client) => new URL(client.url).pathname.startsWith("/console"));
  const others = out.changed.filter((e) => e.id !== own);
  const done = others.filter((e) => e.state === "done" || e.state === "held").length;
  const failed = others.filter((e) => e.state === "failed").length;
  const stale = others.filter((e) => e.state === "stale").length;
  // On the browser's last call for this, what still waits is not sent
  // until the app is next opened: that is said too, or it would wait
  // unseen. (What waits for a sign-in is said below.)
  const left = last && !out.signin ? out.waiting : 0;
  if (!open && done + failed + stale + left > 0 && self.Notification?.permission === "granted") {
    const parts = [
      done ? `${done} sent` : "",
      failed ? `${failed} could not be done` : "",
      stale ? `${stale} kept back for you to look at` : "",
      left ? `${left} still waiting (open the app to send ${left === 1 ? "it" : "them"})` : "",
    ].filter(Boolean);
    await self.registration.showNotification("What you did offline", {
      body: `${parts.join(", ")}.`,
      tag: "outbox",
      icon: "/icons/icon-192.png",
      badge: "/icons/icon-192.png",
      data: { url: "/console/#more/device" },
    });
  }
  // The sign-in has run out: nothing was sent, and nothing will be
  // until the owner signs in. With no page open, that has to be said.
  if (out.signin && !open && self.Notification?.permission === "granted") {
    const waiting = (await store.all()).filter((e) => e.state === "waiting").length;
    await self.registration.showNotification("Sign in to send what you did offline", {
      body: `${waiting} thing${waiting === 1 ? " is" : "s are"} kept on this device until you do.`,
      tag: "outbox",
      icon: "/icons/icon-192.png",
      badge: "/icons/icon-192.png",
      data: { url: "/console/#more/device" },
    });
  }
  // Still no connection, or something is still waiting for the
  // platform to be able to take it: say so, and the browser will call
  // again. (Not when it waits for a sign-in: calling again would not
  // help, and the owner has been told.)
  if (out.offline) throw new Error("still offline");
  if (out.waiting > 0 && !out.signin) throw new Error("not all sent yet");
}

self.addEventListener("sync", (event) => {
  if (event.tag === "pm-outbox") event.waitUntil(send(event.lastChance === true));
});

self.addEventListener("message", (event) => {
  if (event.data?.type === "forget") {
    // The owner asked this device to forget what it keeps of the app's
    // answers. What they did and has not been sent is not touched.
    event.waitUntil(
      (async () => {
        const cache = await self.caches.open(KEPT);
        for (const request of await cache.keys()) {
          const path = new URL(request.url).pathname;
          if (path.startsWith("/console/api/") || path.startsWith("/console/__seen__/") || path === "/console/health") {
            await cache.delete(request);
          }
        }
        event.source?.postMessage({ type: "forgotten" });
      })(),
    );
  }
});

// ---- notifications -------------------------------------------------------

/** The buttons a pushed message carries that are buttons: each named,
 * with words on it. (What each does is the platform's to say:
 * PushAction in src/notify.ts.) */
function pushedActions(data) {
  const list = Array.isArray(data.actions) ? data.actions : [];
  return list.filter((a) => a && typeof a.action === "string" && a.action !== "" && typeof a.title === "string" && a.title !== "");
}

self.addEventListener("push", (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    data = {};
  }
  const actions = pushedActions(data);
  // A phone shows two buttons; the platform puts the most useful first.
  // One that takes a line of text is typed into on the notification
  // itself where the device can do that.
  const shown = actions.slice(0, Number(self.Notification?.maxActions) || 2).map((a) => ({
    action: a.action,
    title: a.title,
    ...(a.reply ? { type: "text", placeholder: a.title } : {}),
  }));
  event.waitUntil(
    self.registration.showNotification(data.title || "PM", {
      body: data.body || "",
      tag: data.tag || undefined,
      renotify: Boolean(data.tag),
      icon: "/icons/icon-192.png",
      badge: "/icons/icon-192.png",
      // What each button does is kept with the notification, for when one is pressed.
      data: { url: data.url || "/console/", ...(data.tag ? { tag: data.tag } : {}), ...(actions.length ? { actions } : {}) },
      ...(shown.length ? { actions: shown } : {}),
    }),
  );
});

/** Bring the app forward on a screen, or open it there. */
async function show(url) {
  const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  for (const client of windows) {
    if (new URL(client.url).pathname.startsWith("/console")) {
      await client.focus();
      return client.navigate ? client.navigate(url) : undefined;
    }
  }
  return self.clients.openWindow(url);
}

/** A button on a notification that does something was pressed: it is
 * done the way the app does anything, written down on the device first
 * and sent under a name, so with no connection it is kept and sent
 * later and never done twice. With the app open, the app says what
 * became of it; with it closed, a notification in the same place does. */
async function pressed(action, args, data) {
  const store = self.PMOutbox.idbStore();
  const entry = self.PMOutbox.make(action.kind, args, Date.now());
  await store.put(entry);
  // What is still waiting after this is sent when the browser next wakes the worker.
  await send(false, entry.id).catch(() => self.registration.sync?.register("pm-outbox").catch(() => {}));
  const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  if (windows.some((client) => new URL(client.url).pathname.startsWith("/console"))) return;
  if (self.Notification?.permission !== "granted") return;
  const now = (await store.all()).find((e) => e.id === entry.id);
  const state = now?.state ?? "waiting";
  const went = state === "done" || state === "held";
  await self.registration.showNotification(action.title, {
    body:
      state === "done"
        ? "Done."
        : state === "held"
          ? "Taken. It waits to be done."
          : state === "waiting"
            ? "Kept on this device: it is sent when there is a connection."
            : "That could not be done. Open the app to see why.",
    tag: data.tag || undefined,
    icon: "/icons/icon-192.png",
    badge: "/icons/icon-192.png",
    data: { url: went ? data.url || "/console/" : "/console/#more/device" },
  });
}

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const data = event.notification.data || {};
  const url = data.url || "/console/";
  const chosen = event.action ? (Array.isArray(data.actions) ? data.actions : []).find((a) => a?.action === event.action) : null;
  if (chosen?.kind) {
    const args = { ...(chosen.args || {}) };
    if (chosen.reply) {
      // The line typed on the notification. Where the device gave none
      // (it cannot take text there), the thread is opened to type it.
      const text = typeof event.reply === "string" ? event.reply.trim() : "";
      if (text === "") {
        event.waitUntil(show(chosen.url || url));
        return;
      }
      args.text = text;
    }
    event.waitUntil(pressed(chosen, args, data));
    return;
  }
  // The notification itself, a button that only goes somewhere, or one this worker does not know.
  event.waitUntil(show(chosen?.url || url));
});
