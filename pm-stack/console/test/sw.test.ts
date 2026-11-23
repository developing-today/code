// The service worker: what the phone shows when a notification is
// pushed, and where a tap on it goes.
//
// Oracles: the worker file itself is run, against a stand-in for the
// browser's worker scope that records what it is asked to do. The
// expected notifications are written out by hand. That the same file
// shows the same notification in a real Chrome, handed a message the way
// Chrome hands one over on arrival, was checked by hand when this was
// written (DevTools ServiceWorker.deliverPushMessage).
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

interface Shown {
  title: string;
  options: { body: string; tag?: string; renotify?: boolean; icon: string; badge: string; data: { url: string } };
}

const ORIGIN = "https://pm.example";
const PUBLIC = join(import.meta.dir, "../public");

/** The browser's cache, as far as the worker uses it. */
function fakeCaches() {
  const stores = new Map<string, Map<string, Response>>();
  const keyOf = (key: string | Request) => {
    const url = new URL(typeof key === "string" ? key : key.url, ORIGIN);
    return url.pathname + url.search;
  };
  const open = async (name: string) => {
    if (!stores.has(name)) stores.set(name, new Map());
    const store = stores.get(name) as Map<string, Response>;
    return {
      put: async (key: string | Request, res: Response) => void store.set(keyOf(key), res),
      match: async (key: string | Request) => store.get(keyOf(key))?.clone(),
      keys: async () => [...store.keys()].map((k) => new Request(ORIGIN + k)),
      delete: async (key: string | Request) => store.delete(keyOf(key)),
    };
  };
  return {
    open,
    keys: async () => [...stores.keys()],
    delete: async (name: string) => stores.delete(name),
    /** For the tests: what is kept under a name. */
    kept: (name = "pm-console-v1") => [...(stores.get(name)?.keys() ?? [])].sort(),
    stores,
  };
}

type Net = (url: URL, init?: RequestInit) => Response | Promise<Response>;

function worker(
  windows: { url: string; canNavigate?: boolean }[] = [],
  options: { net?: Net; permission?: string } = {},
) {
  const listeners: Record<string, (event: unknown) => void> = {};
  const shown: Shown[] = [];
  const did: string[] = [];
  const posted: unknown[] = [];
  const asked: string[] = [];
  const caches = fakeCaches();
  /** Waits the worker asked for, run when a test says so. */
  const timers: (() => void)[] = [];
  const state = { net: options.net ?? (() => new Response("not found", { status: 404 })) };
  const scope: Record<string, unknown> = {
    addEventListener: (name: string, fn: (event: unknown) => void) => {
      listeners[name] = fn;
    },
    skipWaiting: async () => void did.push("skipWaiting"),
    location: { origin: ORIGIN },
    navigator: { onLine: true },
    Notification: { permission: options.permission ?? "default" },
    crypto,
    caches,
    setTimeout: (fn: () => void) => {
      timers.push(fn);
      return timers.length;
    },
    fetch: async (input: string | Request, init?: RequestInit) => {
      const url = new URL(typeof input === "string" ? input : input.url, ORIGIN);
      asked.push(`${(typeof input === "string" ? init?.method : input.method) ?? "GET"} ${url.pathname}${url.search}`);
      return state.net(url, typeof input === "string" ? init : { method: input.method, headers: input.headers });
    },
    importScripts: (path: string) => {
      new Function("self", readFileSync(join(PUBLIC, path.replace("/console/", "")), "utf8"))(scope);
    },
    registration: {
      showNotification: async (title: string, opts: Shown["options"]) => {
        shown.push({ title, options: opts });
      },
    },
    clients: {
      claim: async () => did.push("claim"),
      matchAll: async () =>
        windows.map((w) => ({
          url: w.url,
          focus: async () => did.push(`focus ${w.url}`),
          postMessage: (message: unknown) => posted.push(message),
          ...(w.canNavigate === false ? {} : { navigate: async (to: string) => did.push(`navigate ${to}`) }),
        })),
      openWindow: async (to: string) => did.push(`open ${to}`),
    },
  };
  new Function("self", readFileSync(join(PUBLIC, "sw.js"), "utf8"))(scope);
  /** Deliver an event and wait for everything it asked the browser to wait for. */
  const fire = async (name: string, event: Record<string, unknown>) => {
    const waits: Promise<unknown>[] = [];
    listeners[name]({ ...event, waitUntil: (p: Promise<unknown>) => waits.push(p) });
    await Promise.all(waits);
  };
  /** Ask for something the way a page does. Answers with what the
   * worker gave, or undefined when it left the request alone. */
  const get = (path: string, init: { mode?: string; headers?: Record<string, string>; method?: string } = {}) => {
    const waits: Promise<unknown>[] = [];
    let given: Promise<Response> | undefined;
    const request = new Request(ORIGIN + path, { method: init.method ?? "GET", headers: init.headers });
    // A request's mode cannot be set to "navigate" from a script: said beside it.
    const event = {
      request: new Proxy(request, {
        get: (target, prop) => (prop === "mode" ? (init.mode ?? "cors") : Reflect.get(target, prop, target)),
      }),
      waitUntil: (p: Promise<unknown>) => waits.push(p),
      respondWith: (p: Promise<Response>) => {
        given = p;
      },
    };
    listeners.fetch(event);
    return { given, settled: () => Promise.all(waits) };
  };
  /** Let the waits the worker is in run out. */
  const tick = async () => {
    for (const fn of timers.splice(0)) fn();
    await new Promise((r) => setTimeout(r, 0));
  };
  return { shown, did, fire, listeners, caches, asked, posted, state, scope, get, tick };
}

const pushed = (text: string | null) => ({
  data: text === null ? null : { json: () => JSON.parse(text) },
});

test("a pushed message is shown as the notification it describes", async () => {
  const w = worker();
  await w.fire(
    "push",
    pushed(
      JSON.stringify({
        title: "The fleet replied on #12501",
        body: "Done — “quoted” ✓",
        url: "/console/#ask",
        tag: "ask-12501",
        class: "replies",
      }),
    ),
  );
  expect(w.shown).toEqual([
    {
      title: "The fleet replied on #12501",
      options: {
        body: "Done — “quoted” ✓",
        tag: "ask-12501",
        renotify: true, // a second message about the same thing sounds again, in place of the first
        icon: "/icons/icon-192.png",
        badge: "/icons/icon-192.png",
        data: { url: "/console/#ask" },
      },
    },
  ]);
});

test("a message the worker cannot read still shows something, as the browser requires", async () => {
  // A push that shows nothing makes Chrome show its own "this site has
  // been updated in the background" and counts against the site.
  const w = worker();
  await w.fire("push", pushed("not json at all"));
  await w.fire("push", pushed(null));
  await w.fire("push", pushed(JSON.stringify({ title: "No tag", url: "/console/#needs" })));
  expect(w.shown.map((s) => [s.title, s.options.body, s.options.tag, s.options.renotify, s.options.data.url])).toEqual([
    ["PM", "", undefined, false, "/console/"],
    ["PM", "", undefined, false, "/console/"],
    ["No tag", "", undefined, false, "/console/#needs"],
  ]);
});

test("a tap opens the screen the notification is about, in the app that is already open", async () => {
  const tap = (url?: string) => {
    const closed: string[] = [];
    return { closed, event: { notification: { close: () => closed.push("closed"), data: url ? { url } : undefined } } };
  };
  // The app is open: it is brought forward and taken to the screen.
  const open = worker([{ url: "https://example.org/elsewhere" }, { url: "https://pm.security.cab/console/#now" }]);
  const t = tap("/console/#ask");
  await open.fire("notificationclick", t.event);
  expect(t.closed).toEqual(["closed"]);
  expect(open.did).toEqual(["focus https://pm.security.cab/console/#now", "navigate /console/#ask"]);
  // A window that cannot be navigated is still brought forward.
  const fixed = worker([{ url: "https://pm.security.cab/console/", canNavigate: false }]);
  await fixed.fire("notificationclick", tap("/console/#needs").event);
  expect(fixed.did).toEqual(["focus https://pm.security.cab/console/"]);
  // The app is not open: a window is opened on the screen.
  const closed = worker([{ url: "https://example.org/elsewhere" }]);
  await closed.fire("notificationclick", tap("/console/#more/services").event);
  expect(closed.did).toEqual(["open /console/#more/services"]);
  // A notification that names no screen opens the app.
  const bare = worker();
  await bare.fire("notificationclick", tap().event);
  expect(bare.did).toEqual(["open /console/"]);
});

test("a new worker takes over at once, so a fix to it does not wait for every tab to close", async () => {
  const w = worker();
  await w.fire("install", {});
  await w.fire("activate", {});
  expect(w.did).toEqual(["skipWaiting", "claim"]);
});

// ---- the app with no connection ------------------------------------------
//
// Oracles: the worker file is run against a stand-in cache and network
// that record what is kept and what is asked. What the app must see in
// each case is written out by hand. That a real Chrome, taken offline,
// opens the app from these copies is checked in a browser (see the
// design record); here it is the rules.

const json = (body: unknown, headers: Record<string, string> = {}, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", ...headers } });
const down: Net = () => {
  throw new TypeError("Failed to fetch");
};

test("installed, the worker has the app's own files, and one it cannot fetch does not stop it", async () => {
  const w = worker([], {
    net: (url) => {
      // One is refused, and one gets no answer at all.
      if (url.pathname === "/console/app.css") return new Response("no", { status: 503 });
      if (url.pathname === "/icons/icon-512.png") throw new TypeError("Failed to fetch");
      return new Response(`the file ${url.pathname}`, { status: 200 });
    },
  });
  await w.fire("install", {});
  expect(w.caches.kept()).toEqual([
    "/console/",
    "/console/app.js",
    "/console/icon.svg",
    "/console/outbox.js",
    "/icons/icon-192.png",
    "/manifest.webmanifest",
  ]);
  expect(w.did).toEqual(["skipWaiting"]);
  // An older worker's copies are dropped when this one takes over; nobody else's are.
  await w.caches.open("pm-console-v0");
  await w.caches.open("someone-elses");
  await w.fire("activate", {});
  expect(await w.caches.keys()).toEqual(["pm-console-v1", "someone-elses"]);
});

test("the app opens with no connection, from the page as it was last fetched", async () => {
  const w = worker([], { net: () => new Response("<html>the app, today</html>", { status: 200 }) });
  // Any address of the app that is navigated to is the one page.
  const first = w.get("/console/", { mode: "navigate" });
  expect(await (await first.given)?.text()).toBe("<html>the app, today</html>");
  await first.settled();
  w.state.net = down;
  for (const path of ["/console/", "/console"]) {
    const offline = w.get(path, { mode: "navigate" });
    expect(await (await offline.given)?.text()).toBe("<html>the app, today</html>");
  }
  // Its files, the same.
  w.state.net = () => new Response("code", { status: 200 });
  await w.get("/console/app.js").settled();
  w.state.net = down;
  expect(await (await w.get("/console/app.js").given)?.text()).toBe("code");
  // With nothing kept, no connection is no answer: the browser says so.
  const bare = worker([], { net: down });
  await expect(bare.get("/console/", { mode: "navigate" }).given).rejects.toThrow("Failed to fetch");
  // A way to the sign-in page is passed on, not answered from the copy:
  // the owner has to be able to sign in.
  w.state.net = () => new Response(null, { status: 302, headers: { location: "https://login.example/" } });
  expect((await w.get("/console/", { mode: "navigate" }).given)?.status).toBe(302);
});

test("what the app read is kept, and shown with no connection, marked as the device's own copy", async () => {
  let n = 0;
  const w = worker([], { net: () => json({ n: ++n }, { etag: '"a"', "x-as-of": "1000" }) });
  const live = await w.get("/console/api/doc/core").given;
  expect(await live?.json()).toEqual({ n: 1 });
  // The network's answer goes to the app as it came: not marked.
  expect(live?.headers.get("x-pm-offline")).toBeNull();
  w.state.net = down;
  const kept = await w.get("/console/api/doc/core").given;
  expect(await kept?.json()).toEqual({ n: 1 });
  expect(kept?.headers.get("x-pm-offline")).toBe("1");
  expect(Number(kept?.headers.get("x-pm-kept-at"))).toBeGreaterThan(0);
  // What the app needs of the answer is still on it.
  expect([kept?.headers.get("etag"), kept?.headers.get("x-as-of")]).toEqual(['"a"', "1000"]);
  // Asked with "only if it changed", the copy is still given whole: the
  // app may have nothing (it was reloaded).
  const again = await w.get("/console/api/doc/core", { headers: { "if-none-match": '"a"' } }).given;
  expect(await again?.json()).toEqual({ n: 1 });
  // Every read goes the same way: a screen's own, with its question, and health.
  w.state.net = (url) => json({ path: url.pathname + url.search });
  for (const path of ["/console/api/roadmap?area=id", "/console/api/queue?status=blocked", "/console/health"]) {
    await w.get(path).settled();
  }
  w.state.net = down;
  expect(await (await w.get("/console/api/roadmap?area=id").given)?.json()).toEqual({
    path: "/console/api/roadmap?area=id",
  });
  expect(await (await w.get("/console/health").given)?.json()).toEqual({ path: "/console/health" });
  // Asked afresh, with no connection: the same copy, not "none".
  expect(await (await w.get("/console/api/roadmap?area=id&fresh=1").given)?.json()).toEqual({
    path: "/console/api/roadmap?area=id",
  });
  // One area's copy is not another's.
  await expect(w.get("/console/api/roadmap?area=fleet").given).rejects.toThrow("Failed to fetch");
});

test("an answer that says no is passed on as it is, never papered over with a copy", async () => {
  const w = worker([], { net: () => json({ ok: 1 }) });
  await w.get("/console/api/agents").settled();
  for (const status of [401, 403, 404, 500, 503]) {
    w.state.net = () => json({ error: "no" }, {}, status);
    const res = await w.get("/console/api/agents").given;
    expect([res?.status, res?.headers.get("x-pm-offline")]).toEqual([status, null]);
  }
  // And a refusal is not kept in place of the good copy.
  w.state.net = down;
  expect(await (await w.get("/console/api/agents").given)?.json()).toEqual({ ok: 1 });
});

test("told its copy is still current, the worker notes when, so the copy is as fresh as it was last said to be", async () => {
  const realNow = Date.now;
  let now = 1_000_000;
  Date.now = () => now;
  try {
    const w = worker([], { net: () => json({ v: 1 }, { "x-as-of": "900000" }) });
    await w.get("/console/api/doc/core").settled();
    now += 60_000;
    w.state.net = () => new Response(null, { status: 304, headers: { "x-as-of": "1050000" } });
    const unchanged = w.get("/console/api/doc/core", { headers: { "if-none-match": '"a"' } });
    expect((await unchanged.given)?.status).toBe(304);
    await unchanged.settled();
    w.state.net = down;
    const kept = await w.get("/console/api/doc/core").given;
    expect(await kept?.json()).toEqual({ v: 1 });
    expect([kept?.headers.get("x-pm-kept-at"), kept?.headers.get("x-as-of")]).toEqual(["1060000", "1050000"]);
  } finally {
    Date.now = realNow;
  }
});

test("a network too slow to wait for is answered from the copy, and its answer kept when it comes", async () => {
  const w = worker([], { net: () => json({ v: "old" }) });
  await w.get("/console/api/doc/ops").settled();
  let arrive: (res: Response) => void = () => {};
  w.state.net = () =>
    new Promise<Response>((resolve) => {
      arrive = resolve;
    });
  const slow = w.get("/console/api/doc/ops");
  await new Promise((r) => setTimeout(r, 0));
  await w.tick();
  const shown = await slow.given;
  expect([await shown?.json(), shown?.headers.get("x-pm-offline")]).toEqual([{ v: "old" }, "1"]);
  arrive(json({ v: "new" }));
  await slow.settled();
  w.state.net = down;
  expect(await (await w.get("/console/api/doc/ops").given)?.json()).toEqual({ v: "new" });
  // With no copy, the network is waited for however long it takes.
  let late: (res: Response) => void = () => {};
  w.state.net = () =>
    new Promise<Response>((resolve) => {
      late = resolve;
    });
  const first = w.get("/console/api/bugs");
  await new Promise((r) => setTimeout(r, 0));
  await w.tick();
  late(json({ bugs: [] }));
  expect(await (await first.given)?.json()).toEqual({ bugs: [] });
});

test("what is not the app's reading is left alone", async () => {
  const w = worker([], { net: () => json({}) });
  // Anything that changes something goes straight out.
  expect(w.get("/console/api/act", { method: "POST" }).given).toBeUndefined();
  // Asked on purpose past the worker: is there a sign-in, is there a network.
  expect(w.get("/console/api/me", { headers: { "x-pm-probe": "1" } }).given).toBeUndefined();
  // A download, the phone's own address, and other parts of the site.
  for (const path of ["/console/pm-console.apk", "/console/device/summary", "/dashboard", "/cdn-cgi/access/logout"]) {
    expect(w.get(path, { mode: "navigate" }).given).toBeUndefined();
  }
  // Another site altogether.
  const waits: Promise<unknown>[] = [];
  let given: unknown;
  w.listeners.fetch({
    request: new Request("https://elsewhere.example/console/api/doc/core"),
    waitUntil: (p: Promise<unknown>) => waits.push(p),
    respondWith: (p: unknown) => {
      given = p;
    },
  });
  expect(given).toBeUndefined();
  expect(w.asked).toEqual([]);
});

test("asked to forget, the worker drops what the app read and keeps the app itself", async () => {
  const w = worker([], { net: (url) => json({ path: url.pathname }) });
  await w.fire("install", {});
  for (const path of ["/console/api/doc/core", "/console/api/roadmap?area=id", "/console/health"]) {
    await w.get(path).settled();
  }
  w.state.net = () => new Response(null, { status: 304 });
  await w.get("/console/api/doc/core").settled();
  expect(w.caches.kept().some((k) => k.startsWith("/console/__seen__/"))).toBe(true);
  const told: unknown[] = [];
  await w.fire("message", { data: { type: "forget" }, source: { postMessage: (m: unknown) => told.push(m) } });
  expect(w.caches.kept()).toEqual([
    "/console/",
    "/console/app.css",
    "/console/app.js",
    "/console/icon.svg",
    "/console/outbox.js",
    "/icons/icon-192.png",
    "/icons/icon-512.png",
    "/manifest.webmanifest",
  ]);
  expect(told).toEqual([{ type: "forgotten" }]);
  // Any other message is nothing to it.
  await w.fire("message", { data: { type: "other" } });
  await w.fire("message", {});
});

test("with the app closed, the worker sends what was kept when the connection is back, and says what became of it", async () => {
  type Entry = { id: string; kind: string; args: Record<string, unknown>; state: string; at: number; said: string };
  const acts: { kind: string; id: string }[] = [];
  const net: Net = (url, init) => {
    if (url.pathname !== "/console/api/act") return json({}, {}, 404);
    const body = JSON.parse(String((init as { body?: string }).body)) as { kind: string; id: string };
    acts.push({ kind: body.kind, id: body.id });
    return body.kind === "issue.comment" ? json({ error: "no such issue" }, {}, 404) : json({ ok: true, result: {} });
  };
  const w = worker([], { net, permission: "granted" });
  const outbox = w.scope.PMOutbox as {
    memoryStore: () => { all: () => Promise<Entry[]>; put: (e: Entry) => Promise<void> };
    make: (kind: string, args: unknown, now: number) => Entry;
    idbStore: unknown;
  };
  const store = outbox.memoryStore();
  outbox.idbStore = () => store;
  const now = Date.now();
  await store.put(outbox.make("ask.send", { text: "Typed in a tunnel" }, now - 3_000));
  await store.put(outbox.make("issue.comment", { issue: 13, text: "x" }, now - 2_000));
  // A merge asked for an hour ago is not sent unasked.
  await store.put(outbox.make("pr.merge", { pr: 55 }, now - 3_600_000));
  await w.fire("sync", { tag: "pm-outbox" });
  expect(acts.map((a) => a.kind)).toEqual(["ask.send", "issue.comment"]);
  expect((await store.all()).map((e) => [e.kind, e.state]).sort()).toEqual([
    ["ask.send", "done"],
    ["issue.comment", "failed"],
    ["pr.merge", "stale"],
  ]);
  // Nobody is looking: it says so, and a tap opens the list.
  expect(w.shown).toEqual([
    {
      title: "What you did offline",
      options: {
        body: "1 sent, 1 could not be done, 1 kept back for you to look at.",
        tag: "outbox",
        icon: "/icons/icon-192.png",
        badge: "/icons/icon-192.png",
        data: { url: "/console/#more/device" },
      },
    },
  ]);
  // With the app open, the app is told and shows it itself.
  const open = worker([{ url: `${ORIGIN}/console/#now` }], { net, permission: "granted" });
  const opened = open.scope.PMOutbox as typeof outbox;
  const second = opened.memoryStore();
  opened.idbStore = () => second;
  await second.put(opened.make("ask.send", { text: "Another" }, Date.now()));
  await open.fire("sync", { tag: "pm-outbox" });
  expect(open.shown).toEqual([]);
  expect(open.posted).toEqual([{ type: "outbox", changed: [{ state: "done" }] }]);
  // Without leave to notify, nothing is shown; nothing is lost either.
  const quiet = worker([], { net });
  const third = (quiet.scope.PMOutbox as typeof outbox).memoryStore();
  (quiet.scope.PMOutbox as typeof outbox).idbStore = () => third;
  await third.put((quiet.scope.PMOutbox as typeof outbox).make("ask.send", { text: "Quiet" }, Date.now()));
  await quiet.fire("sync", { tag: "pm-outbox" });
  expect(quiet.shown).toEqual([]);
  expect((await third.all()).map((e) => e.state)).toEqual(["done"]);
  // A wake under another name is not for this: what waits stays waiting.
  await store.put(outbox.make("ask.send", { text: "Not for that wake" }, Date.now()));
  const before = acts.length;
  await w.fire("sync", { tag: "something-else" });
  expect(acts.length).toBe(before);
  expect((await store.all()).filter((e) => e.state === "waiting")).toHaveLength(1);
});

test("still no connection when the browser wakes the worker: it says so, and the browser tries again later", async () => {
  const w = worker([], { net: down });
  const outbox = w.scope.PMOutbox as {
    memoryStore: () => { all: () => Promise<{ state: string }[]>; put: (e: unknown) => Promise<void> };
    make: (kind: string, args: unknown, now: number) => unknown;
    idbStore: unknown;
  };
  const store = outbox.memoryStore();
  outbox.idbStore = () => store;
  await store.put(outbox.make("ask.send", { text: "Still in the tunnel" }, Date.now()));
  await expect(w.fire("sync", { tag: "pm-outbox" })).rejects.toThrow("still offline");
  expect((await store.all()).map((e) => e.state)).toEqual(["waiting"]);
  expect(w.shown).toEqual([]);
});

test("the sign-in ran out while the app was closed: nothing is sent, and the owner is told to sign in", async () => {
  // What a browser gives a worker that was sent towards a sign-in page.
  const towardsSignIn = () => ({ type: "opaqueredirect", status: 0, json: async () => null }) as unknown as Response;
  const w = worker([], { net: towardsSignIn, permission: "granted" });
  const outbox = w.scope.PMOutbox as {
    memoryStore: () => { all: () => Promise<{ state: string; note: string }[]>; put: (e: unknown) => Promise<void> };
    make: (kind: string, args: unknown, now: number) => unknown;
    idbStore: unknown;
  };
  const store = outbox.memoryStore();
  outbox.idbStore = () => store;
  await store.put(outbox.make("ask.send", { text: "one" }, Date.now() - 2));
  await store.put(outbox.make("ask.send", { text: "two" }, Date.now() - 1));
  // It is not a failure of the connection: the browser is not asked to call again.
  await w.fire("sync", { tag: "pm-outbox" });
  expect((await store.all()).map((e) => [e.state, e.note])).toEqual([
    ["waiting", "Sign in again to send this."],
    ["waiting", ""],
  ]);
  expect(w.shown).toEqual([
    {
      title: "Sign in to send what you did offline",
      options: {
        body: "2 things are kept on this device until you do.",
        tag: "outbox",
        icon: "/icons/icon-192.png",
        badge: "/icons/icon-192.png",
        data: { url: "/console/#more/device" },
      },
    },
  ]);
  // With the app open it says so itself.
  const open = worker([{ url: `${ORIGIN}/console/` }], { net: towardsSignIn, permission: "granted" });
  const other = (open.scope.PMOutbox as typeof outbox).memoryStore();
  (open.scope.PMOutbox as typeof outbox).idbStore = () => other;
  await other.put((open.scope.PMOutbox as typeof outbox).make("ask.send", { text: "one" }, Date.now()));
  await open.fire("sync", { tag: "pm-outbox" });
  expect(open.shown).toEqual([]);
});

test("the app's page is already sending: the worker leaves it to the page", async () => {
  const sent: string[] = [];
  const w = worker([], {
    net: (url) => {
      sent.push(url.pathname);
      return json({ ok: true, result: {} });
    },
  });
  const outbox = w.scope.PMOutbox as {
    memoryStore: () => { all: () => Promise<{ state: string }[]>; put: (e: unknown) => Promise<void> };
    make: (kind: string, args: unknown, now: number) => unknown;
    idbStore: unknown;
  };
  const store = outbox.memoryStore();
  outbox.idbStore = () => store;
  await store.put(outbox.make("ask.send", { text: "the page has this" }, Date.now()));
  // The browser's lock, as it answers when someone else holds it, and when nobody does.
  const asked: unknown[] = [];
  const lock = { held: true };
  (w.scope.navigator as Record<string, unknown>).locks = {
    request: async (name: string, options: unknown, work: (lock: unknown) => unknown) => {
      asked.push([name, options]);
      return work(lock.held ? null : { name });
    },
  };
  await w.fire("sync", { tag: "pm-outbox" });
  expect(sent).toEqual([]);
  expect((await store.all()).map((e) => e.state)).toEqual(["waiting"]);
  expect(asked).toEqual([["pm-outbox", { ifAvailable: true }]]);
  lock.held = false;
  await w.fire("sync", { tag: "pm-outbox" });
  expect(sent).toEqual(["/console/api/act"]);
  expect((await store.all()).map((e) => e.state)).toEqual(["done"]);
});
