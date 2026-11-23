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
  options: { body: string; tag?: string; renotify: boolean; icon: string; badge: string; data: { url: string } };
}

function worker(windows: { url: string; canNavigate?: boolean }[] = []) {
  const listeners: Record<string, (event: unknown) => void> = {};
  const shown: Shown[] = [];
  const did: string[] = [];
  const scope = {
    addEventListener: (name: string, fn: (event: unknown) => void) => {
      listeners[name] = fn;
    },
    skipWaiting: () => did.push("skipWaiting"),
    registration: {
      showNotification: async (title: string, options: Shown["options"]) => {
        shown.push({ title, options });
      },
    },
    clients: {
      claim: async () => did.push("claim"),
      matchAll: async () =>
        windows.map((w) => ({
          url: w.url,
          focus: async () => did.push(`focus ${w.url}`),
          ...(w.canNavigate === false ? {} : { navigate: async (to: string) => did.push(`navigate ${to}`) }),
        })),
      openWindow: async (to: string) => did.push(`open ${to}`),
    },
  };
  new Function("self", readFileSync(join(import.meta.dir, "../public/sw.js"), "utf8"))(scope);
  /** Deliver an event and wait for everything it asked the browser to wait for. */
  const fire = async (name: string, event: Record<string, unknown>) => {
    const waits: Promise<unknown>[] = [];
    listeners[name]({ ...event, waitUntil: (p: Promise<unknown>) => waits.push(p) });
    await Promise.all(waits);
  };
  return { shown, did, fire, listeners };
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
