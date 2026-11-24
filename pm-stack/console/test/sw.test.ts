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
  options: {
    body: string;
    tag?: string;
    renotify?: boolean;
    icon: string;
    badge: string;
    data: { url: string; actions?: unknown[]; tag?: string };
    actions?: { action: string; title: string; type?: string; placeholder?: string }[];
  };
}

const ORIGIN = "https://pm.example";
const PUBLIC = join(import.meta.dir, "../public");

/** The browser's cache, as far as the worker uses it. */
function fakeCaches() {
  const stores = new Map<string, Map<string, Response>>();
  /** A device with no room left: every write is refused. */
  const full = { on: false };
  /** A browser that gives this site no cache at all: it cannot be opened. */
  const none = { on: false };
  const keyOf = (key: string | Request) => {
    const url = new URL(typeof key === "string" ? key : key.url, ORIGIN);
    return url.pathname + url.search;
  };
  const open = async (name: string) => {
    if (none.on) throw new DOMException("The operation is insecure.", "SecurityError");
    if (!stores.has(name)) stores.set(name, new Map());
    const store = stores.get(name) as Map<string, Response>;
    return {
      put: async (key: string | Request, res: Response) => {
        if (full.on) throw new DOMException("Quota exceeded", "QuotaExceededError");
        store.set(keyOf(key), res);
      },
      match: async (key: string | Request) => store.get(keyOf(key))?.clone(),
      keys: async () => [...store.keys()].map((k) => new Request(ORIGIN + k)),
      delete: async (key: string | Request) => store.delete(keyOf(key)),
    };
  };
  return {
    open,
    keys: async () => {
      if (none.on) throw new DOMException("The operation is insecure.", "SecurityError");
      return [...stores.keys()];
    },
    delete: async (name: string) => stores.delete(name),
    /** For the tests: what is kept under a name. */
    kept: (name = "pm-console-v1") => [...(stores.get(name)?.keys() ?? [])].sort(),
    stores,
    full,
    none,
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
        data: { url: "/console/#ask", tag: "ask-12501" },
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

// A notification carries what can be done about it. The oracle for the
// message is what the platform sends (src/notify.ts: PushAction); for
// what pressing a button does, it is what the app's own send would do
// with the same action: one request to /console/api/act, kept on the
// device and sent again if it cannot go now.
const REPLY = {
  title: "The fleet replied on #12501",
  body: "Done. Shall I also update the docs?",
  url: "/console/#ask",
  tag: "ask-12501",
  class: "replies",
  actions: [
    { action: "reply", title: "Reply", kind: "ask.reply", args: { issue: 12501 }, reply: true, url: "/console/#ask" },
    { action: "close", title: "Close it", kind: "ask.close", args: { issue: 12501 } },
  ],
};
type KeptEntry = { id: string; kind: string; args: Record<string, unknown>; state: string };
/** A worker with leave to notify, a network that takes what it is
 * sent (or does not), and somewhere to keep what it could not send. */
function buttons(net?: Net, windows: { url: string }[] = []) {
  const acts: { kind: string; args: Record<string, unknown>; id: string }[] = [];
  const answer: Net =
    net ??
    ((url, init) => {
      if (url.pathname !== "/console/api/act") return json({}, {}, 404);
      const body = JSON.parse(String((init as { body?: string }).body)) as (typeof acts)[number];
      acts.push({ kind: body.kind, args: body.args, id: body.id });
      return json({ ok: true, result: {} });
    });
  const w = worker(windows, { net: answer, permission: "granted" });
  const outbox = w.scope.PMOutbox as {
    memoryStore: () => { all: () => Promise<KeptEntry[]>; put: (e: KeptEntry) => Promise<void> };
    idbStore: unknown;
  };
  const store = outbox.memoryStore();
  outbox.idbStore = () => store;
  const press = async (shown: Shown, action: string, reply?: string | null) => {
    const closed: string[] = [];
    await w.fire("notificationclick", {
      action,
      ...(reply === undefined ? {} : { reply }),
      notification: { close: () => closed.push("closed"), data: shown.options.data, tag: shown.options.tag },
    });
    return closed;
  };
  return { w, acts, store, press };
}

test("a pushed message shows its buttons: two at most, and one that takes a line of text where it asks for one", async () => {
  const { w } = buttons();
  await w.fire("push", pushed(JSON.stringify(REPLY)));
  expect(w.shown[0].options.actions).toEqual([
    { action: "reply", title: "Reply", type: "text", placeholder: "Reply" },
    { action: "close", title: "Close it" },
  ]);
  // What each button does is kept with the notification, for when it is pressed.
  expect(w.shown[0].options.data).toEqual({ url: "/console/#ask", tag: "ask-12501", actions: REPLY.actions });
  // More than two: a phone shows two, and the first two are the most useful.
  const three = { ...REPLY, actions: [...REPLY.actions, { action: "open", title: "Open", url: "/console/#ask" }] };
  await w.fire("push", pushed(JSON.stringify(three)));
  expect(w.shown[1].options.actions?.map((a) => a.action)).toEqual(["reply", "close"]);
  // Buttons that are not buttons are left out, and the notification is still shown.
  const odd = { ...REPLY, actions: [{ title: "No name" }, "nonsense", { action: "ok", title: 7 }, null] };
  await w.fire("push", pushed(JSON.stringify(odd)));
  await w.fire("push", pushed(JSON.stringify({ ...REPLY, actions: "many" })));
  expect(w.shown.slice(2).map((s) => [s.title, s.options.actions])).toEqual([
    [REPLY.title, undefined],
    [REPLY.title, undefined],
  ]);
});

test("a button that does something does it as the app would: one request, under a name, and says what became of it", async () => {
  const { w, acts, store, press } = buttons();
  await w.fire("push", pushed(JSON.stringify(REPLY)));
  const closed = await press(w.shown[0], "close");
  expect(closed).toEqual(["closed"]);
  expect(acts.map((a) => [a.kind, a.args])).toEqual([["ask.close", { issue: 12501 }]]);
  expect(acts[0].id).toMatch(/^[0-9a-f-]{36}$/);
  expect((await store.all()).map((e) => [e.kind, e.state])).toEqual([["ask.close", "done"]]);
  // No window was opened for it, and with no page open the owner is told it went.
  expect(w.did).toEqual([]);
  // Said once, by the button's own notification (not also as "what you did offline").
  expect(w.shown.slice(1).map((shown) => shown.title)).toEqual(["Close it"]);
  expect(w.shown.at(-1)).toMatchObject({ title: "Close it", options: { body: "Done.", tag: "ask-12501" } });
});

test("a reply typed on the notification is sent as the reply", async () => {
  const { w, acts, press } = buttons();
  await w.fire("push", pushed(JSON.stringify(REPLY)));
  await press(w.shown[0], "reply", "  Yes, the docs too.  ");
  expect(acts.map((a) => [a.kind, a.args])).toEqual([["ask.reply", { issue: 12501, text: "Yes, the docs too." }]]);
  expect(w.did).toEqual([]);
});

test("a reply button pressed where no text can be typed on a notification opens the thread to type it there", async () => {
  for (const reply of [undefined, null, "", "   "]) {
    const { w, acts, press } = buttons();
    await w.fire("push", pushed(JSON.stringify(REPLY)));
    await press(w.shown[0], "reply", reply);
    expect([reply, acts, w.did]).toEqual([reply, [], ["open /console/#ask"]]);
  }
});

test("with no connection the button's action is kept and sent later, and the owner is told it is kept", async () => {
  const offline: Net = () => {
    throw new TypeError("Failed to fetch");
  };
  const { w, store, press } = buttons(offline);
  await w.fire("push", pushed(JSON.stringify(REPLY)));
  await press(w.shown[0], "close");
  expect((await store.all()).map((e) => [e.kind, e.args, e.state])).toEqual([
    ["ask.close", { issue: 12501 }, "waiting"],
  ]);
  expect(w.shown.at(-1)).toMatchObject({
    title: "Close it",
    options: {
      body: "Kept on this device: it is sent when there is a connection.",
      data: { url: "/console/#more/device" },
    },
  });
});

test("an action the platform turned down is said to have failed, with where to look", async () => {
  const refused: Net = (url) =>
    url.pathname === "/console/api/act" ? json({ error: "no such issue" }, {}, 404) : json({}, {}, 404);
  const { w, press } = buttons(refused);
  await w.fire("push", pushed(JSON.stringify(REPLY)));
  await press(w.shown[0], "close");
  expect(w.shown.at(-1)).toMatchObject({
    title: "Close it",
    options: { body: "That could not be done. Open the app to see why.", data: { url: "/console/#more/device" } },
  });
});

test("a button that only goes somewhere goes there, a tap on the notification itself opens its screen, and a button nobody knows does too", async () => {
  const HANDS = {
    title: "Something needs you",
    body: "Plug it in (o/r#3)",
    url: "/console/#needs/hands",
    tag: "hands-o/r#3",
    actions: [
      { action: "done", title: "I did it", kind: "hands.done", args: { ref: "o/r#3" } },
      { action: "open", title: "Open", url: "/console/#needs/hands" },
    ],
  };
  const { w, acts, press } = buttons();
  await w.fire("push", pushed(JSON.stringify(HANDS)));
  await press(w.shown[0], "open");
  await press(w.shown[0], "");
  await press(w.shown[0], "not-a-button");
  expect(acts).toEqual([]);
  expect(w.did).toEqual(["open /console/#needs/hands", "open /console/#needs/hands", "open /console/#needs/hands"]);
  // And the one that does something, does it.
  await press(w.shown[0], "done");
  expect(acts.map((a) => [a.kind, a.args])).toEqual([["hands.done", { ref: "o/r#3" }]]);
});

test("with the app open, pressing a button tells the app, which shows what became of it itself", async () => {
  const { w, acts, press } = buttons(undefined, [{ url: `${ORIGIN}/console/#now` }]);
  await w.fire("push", pushed(JSON.stringify(REPLY)));
  const before = w.shown.length;
  await press(w.shown[0], "close");
  expect(acts.map((a) => a.kind)).toEqual(["ask.close"]);
  expect(w.shown.length).toBe(before);
  expect(w.posted).toEqual([{ type: "outbox", changed: [{ state: "done" }] }]);
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

/** The app's own list of its files, as the services serve it. */
const LIST = readFileSync(join(PUBLIC, "files.json"), "utf8");
/** A network on which the app is served: its list is the real one, and
 * anything else asked for is a file. */
const served: Net = (url) =>
  url.pathname === "/console/files.json"
    ? new Response(LIST, { headers: { "content-type": "application/json" } })
    : new Response(`the file ${url.pathname}`, { status: 200 });

test("installed, the worker has the app's own files, and one it cannot fetch does not stop it", async () => {
  const w = worker([], {
    net: (url) => {
      // One is refused, and one gets no answer at all.
      if (url.pathname === "/console/app.css") return new Response("no", { status: 503 });
      if (url.pathname === "/icons/icon-512.png") throw new TypeError("Failed to fetch");
      return served(url);
    },
  });
  await w.fire("install", {});
  // Written out by hand: the page, the list, and what the list says is
  // kept from the start. No typeface is among them (22 files nobody
  // may ever choose), and neither is the worker itself.
  expect(w.caches.kept()).toEqual([
    "/console/",
    "/console/app.js",
    "/console/files.json",
    "/console/icon.svg",
    "/console/look.css",
    "/console/look.js",
    "/console/outbox.js",
    "/console/themes.css",
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

test("without the app's list of files the page alone is kept at first, and each file as the page asks for it", async () => {
  // The list could not be fetched when the worker was installed (the
  // sign-in had lapsed, say). The worker is installed all the same.
  const w = worker([], {
    net: (url) => (url.pathname === "/console/files.json" ? new Response("no", { status: 503 }) : served(url)),
  });
  await w.fire("install", {});
  expect(w.caches.kept()).toEqual(["/console/", "/icons/icon-192.png", "/icons/icon-512.png", "/manifest.webmanifest"]);
  expect(w.did).toEqual(["skipWaiting"]);
  // The next visit asks for the app's files through the worker, and
  // each is kept as it is answered.
  for (const path of ["/console/app.js", "/console/themes.css"]) {
    const asked = w.get(path);
    expect(await (await asked.given)?.text()).toBe(`the file ${path}`);
    await asked.settled();
  }
  expect(w.caches.kept()).toContain("/console/app.js");
  expect(w.caches.kept()).toContain("/console/themes.css");
  // A list that is not a list (a sign-in page answered in its place) is no list.
  const odd = worker([], {
    net: (url) => (url.pathname === "/console/files.json" ? new Response("<html>sign in</html>") : served(url)),
  });
  await odd.fire("install", {});
  expect(odd.caches.kept()).toEqual([
    "/console/",
    "/icons/icon-192.png",
    "/icons/icon-512.png",
    "/manifest.webmanifest",
  ]);
});

test("a typeface is kept when it is first used, and shown from that copy with no connection", async () => {
  const w = worker([], { net: served });
  await w.fire("install", {});
  const font = "/console/fonts/inter.woff2";
  expect(w.caches.kept()).not.toContain(font);
  const first = w.get(font);
  expect(await (await first.given)?.text()).toBe(`the file ${font}`);
  await first.settled();
  expect(w.caches.kept()).toContain(font);
  w.state.net = down;
  expect(await (await w.get(font).given)?.text()).toBe(`the file ${font}`);
  // One never asked for is not there to show: the page falls back to
  // the device's own typeface.
  await expect(w.get("/console/fonts/lexend.woff2").given).rejects.toThrow("Failed to fetch");
});

test("what is under /console and is not one of the app's files is left to the browser", async () => {
  const w = worker([], { net: served });
  await w.fire("install", {});
  // The Android build is a download, and large: not kept, not answered here.
  expect(w.get("/console/pm-console.apk").given).toBeUndefined();
  // The worker's own file is the browser's to fetch and keep.
  expect(w.get("/console/sw.js").given).toBeUndefined();
  // Nothing outside the app's own folder shape.
  expect(w.get("/console/../secrets.json").given).toBeUndefined();
  expect(w.get("/console/Some File.js").given).toBeUndefined();
  expect(w.asked.filter((a) => a.includes("apk") || a.includes("sw.js"))).toEqual([]);
});

test("asked to forget, the worker drops what the app read and keeps the app itself", async () => {
  const w = worker([], {
    net: (url) => (url.pathname === "/console/files.json" ? served(url) : json({ path: url.pathname })),
  });
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
    "/console/files.json",
    "/console/icon.svg",
    "/console/look.css",
    "/console/look.js",
    "/console/outbox.js",
    "/console/themes.css",
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

test("a device with no room to keep a copy still gets the network's answer", async () => {
  const w = worker([], { net: () => json({ v: 1 }) });
  w.caches.full.on = true;
  const read = w.get("/console/api/doc/core");
  expect(await (await read.given)?.json()).toEqual({ v: 1 });
  await read.settled();
  const page = w.get("/console/", { mode: "navigate" });
  expect((await page.given)?.status).toBe(200);
  await page.settled();
  // Told its copy is current, with nowhere to note it: the answer is passed on all the same.
  w.state.net = () => new Response(null, { status: 304 });
  expect((await w.get("/console/api/doc/core").given)?.status).toBe(304);
  // Nothing was kept, so with no connection there is nothing to show: said as that.
  w.state.net = down;
  await expect(w.get("/console/api/doc/core").given).rejects.toThrow("Failed to fetch");
  // With room again, it keeps again.
  w.caches.full.on = false;
  w.state.net = () => json({ v: 2 });
  await w.get("/console/api/doc/core").settled();
  w.state.net = down;
  expect(await (await w.get("/console/api/doc/core").given)?.json()).toEqual({ v: 2 });
});

test("a browser that gives no cache at all: the network answers as if the worker were not there", async () => {
  // The network takes a moment, as a real one does: long enough for
  // anything left to fail on its own to be noticed.
  const w = worker([], {
    net: async () => {
      await new Promise((r) => setTimeout(r, 3));
      return json({ v: 1 });
    },
  });
  w.caches.none.on = true;
  const unhandled: unknown[] = [];
  const note = (reason: unknown) => unhandled.push(reason);
  process.on("unhandledRejection", note);
  try {
    // It is installed and takes the page all the same, for what else
    // it does: notifications, and sending what was done offline.
    await w.fire("install", {});
    await w.fire("activate", {});
    expect(w.did).toEqual(["skipWaiting", "claim"]);
    const read = w.get("/console/api/doc/core");
    expect(await (await read.given)?.json()).toEqual({ v: 1 });
    await read.settled();
    const page = w.get("/console/", { mode: "navigate" });
    expect((await page.given)?.status).toBe(200);
    await page.settled();
    // With no connection there is nothing to fall back on, and it fails as itself.
    w.state.net = async () => {
      await new Promise((r) => setTimeout(r, 3));
      return down(new URL(ORIGIN));
    };
    await expect(w.get("/console/api/doc/core").given).rejects.toThrow("Failed to fetch");
    await new Promise((r) => setTimeout(r, 5));
  } finally {
    process.off("unhandledRejection", note);
  }
  // Nothing was left to fail on its own behind an answer.
  expect(unhandled).toEqual([]);
});

test("woken with the app closed and something still not taken, the worker asks to be woken again", async () => {
  type Entry = { id: string; kind: string; state: string; note: string };
  const answers: Record<string, () => Response> = {
    "ask.send": () => json({ ok: true, result: {} }),
    // No machine in reach for it just now.
    "message.send": () =>
      json({ error: "The laptop cannot be reached, and too much is already waiting for it." }, {}, 503),
  };
  const net: Net = (_url, init) => {
    const body = JSON.parse(String((init as { body?: string }).body)) as { kind: string };
    return answers[body.kind]();
  };
  const w = worker([], { net });
  const outbox = w.scope.PMOutbox as {
    memoryStore: () => { all: () => Promise<Entry[]>; put: (e: unknown) => Promise<void> };
    make: (kind: string, args: unknown, now: number) => unknown;
    idbStore: unknown;
  };
  const store = outbox.memoryStore();
  outbox.idbStore = () => store;
  await store.put(outbox.make("message.send", { to: ["a"], text: "x" }, Date.now() - 2));
  await store.put(outbox.make("ask.send", { text: "y" }, Date.now() - 1));
  // One went, one could not be taken yet: the browser is to call again.
  await expect(w.fire("sync", { tag: "pm-outbox" })).rejects.toThrow("not all sent yet");
  expect((await store.all()).map((e) => [e.kind, e.state]).sort()).toEqual([
    ["ask.send", "done"],
    ["message.send", "waiting"],
  ]);
  // Being done right now under its name: the same.
  answers["message.send"] = () => json({ error: "This is still being done.", running: true }, {}, 409);
  await expect(w.fire("sync", { tag: "pm-outbox" })).rejects.toThrow("not all sent yet");
  // Taken at last: nothing is left, and the browser need not call again.
  answers["message.send"] = () => json({ ok: true, held: true, result: null });
  await w.fire("sync", { tag: "pm-outbox" });
  expect((await store.all()).map((e) => e.state).sort()).toEqual(["done", "held"]);
  // What only waits for the owner to look again is not a reason to be woken.
  await store.put(outbox.make("pr.merge", { pr: 55 }, Date.now() - 3_600_000));
  await w.fire("sync", { tag: "pm-outbox" });
  expect((await store.all()).map((e) => e.state).sort()).toEqual(["done", "held", "stale"]);
});

test("on the browser's last call, what still waits is said, since nothing will send it until the app is opened", async () => {
  type Entry = { id: string; kind: string; state: string };
  const taken = { yes: false };
  const net: Net = (_url, init) => {
    const body = JSON.parse(String((init as { body?: string }).body)) as { kind: string };
    if (body.kind === "ask.send") return json({ ok: true, result: {} });
    return taken.yes ? json({ ok: true, result: {} }) : json({ error: "no machine" }, {}, 503);
  };
  const setUp = async (windows: { url: string }[], permission = "granted") => {
    const w = worker(windows, { net, permission });
    const outbox = w.scope.PMOutbox as {
      memoryStore: () => { all: () => Promise<Entry[]>; put: (e: unknown) => Promise<void> };
      make: (kind: string, args: unknown, now: number) => unknown;
      idbStore: unknown;
    };
    const store = outbox.memoryStore();
    outbox.idbStore = () => store;
    await store.put(outbox.make("ask.send", { text: "goes" }, Date.now() - 3));
    await store.put(outbox.make("message.send", { to: ["a"], text: "x" }, Date.now() - 2));
    await store.put(outbox.make("message.send", { to: ["b"], text: "y" }, Date.now() - 1));
    return { w, store };
  };
  // Not the last call: the browser will call again, and only what went is said.
  const early = await setUp([]);
  await expect(early.w.fire("sync", { tag: "pm-outbox", lastChance: false })).rejects.toThrow("not all sent yet");
  expect(early.w.shown.map((s) => s.options.body)).toEqual(["1 sent."]);
  // The last call: what is left is said with it, in the one notification.
  const last = await setUp([]);
  await expect(last.w.fire("sync", { tag: "pm-outbox", lastChance: true })).rejects.toThrow("not all sent yet");
  expect(last.w.shown).toEqual([
    {
      title: "What you did offline",
      options: {
        body: "1 sent, 2 still waiting (open the app to send them).",
        tag: "outbox",
        icon: "/icons/icon-192.png",
        badge: "/icons/icon-192.png",
        data: { url: "/console/#more/device" },
      },
    },
  ]);
  // Called once more by something else with nothing new to say but what is left: one, said as one.
  last.w.shown.length = 0;
  const one = (await last.store.all()).find((e) => e.kind === "message.send") as Entry;
  await last.store.put({ ...one, state: "done" });
  await expect(last.w.fire("sync", { tag: "pm-outbox", lastChance: true })).rejects.toThrow("not all sent yet");
  expect(last.w.shown.map((s) => s.options.body)).toEqual(["1 still waiting (open the app to send it)."]);
  // With the app open, the app shows what waits itself; without leave to notify, nothing is shown.
  const open = await setUp([{ url: `${ORIGIN}/console/#now` }]);
  await expect(open.w.fire("sync", { tag: "pm-outbox", lastChance: true })).rejects.toThrow("not all sent yet");
  expect(open.w.shown).toEqual([]);
  const quiet = await setUp([], "default");
  await expect(quiet.w.fire("sync", { tag: "pm-outbox", lastChance: true })).rejects.toThrow("not all sent yet");
  expect(quiet.w.shown).toEqual([]);
  // The last call with nothing left: only what went.
  taken.yes = true;
  const clean = await setUp([]);
  await clean.w.fire("sync", { tag: "pm-outbox", lastChance: true });
  expect(clean.w.shown.map((s) => s.options.body)).toEqual(["3 sent."]);
});
