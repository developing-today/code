// The status page's phone-alert pieces: a service worker that shows a
// notification for each (payloadless) Web Push, the manifest that makes
// the page installable, its icon, and the page script behind the
// "Turn on phone alerts" and "Send a test" buttons.

export const ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><rect width="512" height="512" rx="96" fill="#0d1117"/><circle cx="256" cy="256" r="150" fill="none" stroke="#3fb950" stroke-width="48"/><circle cx="256" cy="256" r="56" fill="#3fb950"/></svg>`;

export const MANIFEST = JSON.stringify({
  name: "Queue broker status",
  short_name: "Queue broker",
  start_url: "/",
  scope: "/",
  display: "standalone",
  background_color: "#0d1117",
  theme_color: "#0d1117",
  icons: [{ src: "/icon.svg", sizes: "any", type: "image/svg+xml", purpose: "any" }],
});

// Each push carries no payload: read the latest alert from /status.json.
export const SERVICE_WORKER = `
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));
self.addEventListener("push", (event) => {
  event.waitUntil((async () => {
    let title = "Queue broker";
    let body = "Status changed. Tap to see it.";
    try {
      const res = await fetch("/status.json", { cache: "no-store" });
      const s = await res.json();
      if (s.last_push && s.last_push.title) {
        title = s.last_push.title;
        body = s.last_push.body || body;
      } else if (s.status) {
        title = "Queue broker " + s.status;
      }
    } catch (e) {}
    await self.registration.showNotification(title, {
      body,
      icon: "/icon.svg",
      badge: "/icon.svg",
      tag: "queue-broker",
      renotify: true,
      data: { url: "/" },
    });
  })());
});
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil((async () => {
    const all = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    for (const c of all) if ("focus" in c) return c.focus();
    return self.clients.openWindow("/");
  })());
});
`;

export const PAGE_SCRIPT = `
(() => {
  const box = document.getElementById("push");
  const btn = document.getElementById("push-on");
  const test = document.getElementById("push-test");
  const note = document.getElementById("push-note");
  const say = (t) => { note.textContent = t; };
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) {
    say("This browser can't receive push alerts.");
    btn.hidden = true;
    return;
  }
  const b64 = (s) => {
    const p = "=".repeat((4 - (s.length % 4)) % 4);
    const raw = atob((s + p).replace(/-/g, "+").replace(/_/g, "/"));
    return Uint8Array.from(raw, (c) => c.charCodeAt(0));
  };
  const post = (path, sub) => fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ endpoint: sub.endpoint }),
  });
  let reg;
  const show = async () => {
    reg = await navigator.serviceWorker.register("/sw.js");
    const sub = await reg.pushManager.getSubscription();
    btn.hidden = !!sub;
    test.hidden = !sub;
    say(sub ? "Phone alerts are on for this device." : "");
    if (sub) await post("/subscribe", sub);
  };
  btn.onclick = async () => {
    try {
      const perm = await Notification.requestPermission();
      if (perm !== "granted") return say("Notifications are blocked for this site in your browser settings.");
      const key = (await (await fetch("/vapid.json", { cache: "no-store" })).json()).publicKey;
      const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64(key) });
      const res = await post("/subscribe", sub);
      if (!res.ok) throw new Error(await res.text());
      await show();
    } catch (e) { say("Couldn't turn on alerts: " + e.message); }
  };
  test.onclick = async () => {
    const sub = await reg.pushManager.getSubscription();
    if (!sub) return show();
    const res = await post("/test-push", sub);
    say(res.ok ? "Test sent. It should arrive in a few seconds." : "Test failed: " + (await res.text()));
  };
  box.hidden = false;
  show().catch((e) => say(e.message));
})();
`;
