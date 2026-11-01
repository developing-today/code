// PM aggregator push client (#3676 / design 2026-11-01T03-54-00Z
// §§4+6): registers the real /sw.js worker on every page, reports the
// registrations it sees to /api/sw-probe, and unregisters any FOREIGN
// registration (a script URL other than /sw.js — the stranded
// opencode worker's eviction, carried page-side per §6's fallback).
// Also exposes window.pmPush for the /alerts page's subscribe /
// unsubscribe / preference controls.
(function () {
  if (!("serviceWorker" in navigator)) return;

  function scriptUrlOf(reg) {
    var w = reg.active || reg.waiting || reg.installing;
    return w ? w.scriptURL : "";
  }

  async function registerAndProbe() {
    try {
      var before = await navigator.serviceWorker.getRegistrations();
      var evicted = [];
      for (var i = 0; i < before.length; i++) {
        var url = scriptUrlOf(before[i]);
        var path = "";
        try { path = new URL(url).pathname; } catch (e) {}
        if (url && path !== "/sw.js") {
          try { await before[i].unregister(); evicted.push(url); } catch (e) {}
        }
      }
      await navigator.serviceWorker.register("/sw.js");
      var after = await navigator.serviceWorker.getRegistrations();
      var urls = after.map(scriptUrlOf).filter(Boolean);
      await fetch("/api/sw-probe", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ registrations: urls, evicted: evicted }),
      }).catch(function () {});
    } catch (e) {
      /* probe/registration failures never break the page */
    }
  }

  if (document.readyState === "complete") registerAndProbe();
  else addEventListener("load", registerAndProbe);

  function urlBase64ToUint8Array(base64String) {
    var padding = "=".repeat((4 - (base64String.length % 4)) % 4);
    var base64 = (base64String + padding).replace(/-/g, "+").replace(/_/g, "/");
    var raw = atob(base64);
    var out = new Uint8Array(raw.length);
    for (var i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
    return out;
  }

  async function ready() {
    return navigator.serviceWorker.ready;
  }

  window.pmPush = {
    supported: function () {
      return "PushManager" in window && "Notification" in window;
    },
    current: async function () {
      var reg = await ready();
      return reg.pushManager.getSubscription();
    },
    subscribe: async function (vapidPublicKey, prefs) {
      var permission = await Notification.requestPermission();
      if (permission !== "granted") throw new Error("notification permission: " + permission);
      var reg = await ready();
      var sub = await reg.pushManager.getSubscription();
      if (!sub) {
        sub = await reg.pushManager.subscribe({
          userVisibleOnly: true,
          applicationServerKey: urlBase64ToUint8Array(vapidPublicKey),
        });
      }
      var json = sub.toJSON();
      var res = await fetch("/api/push/subscriptions", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          endpoint: json.endpoint,
          keys: json.keys,
          classes: prefs && prefs.classes,
          quiet: prefs && prefs.quiet,
          userAgent: navigator.userAgent,
        }),
      });
      if (!res.ok) throw new Error("subscription save failed: HTTP " + res.status);
      return sub;
    },
    updatePrefs: async function (prefs) {
      var sub = await window.pmPush.current();
      if (!sub) throw new Error("not subscribed on this browser");
      var json = sub.toJSON();
      var res = await fetch("/api/push/subscriptions", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          endpoint: json.endpoint,
          keys: json.keys,
          classes: prefs.classes,
          quiet: prefs.quiet,
          userAgent: navigator.userAgent,
        }),
      });
      if (!res.ok) throw new Error("preference save failed: HTTP " + res.status);
    },
    unsubscribe: async function () {
      var sub = await window.pmPush.current();
      if (!sub) return;
      await fetch("/api/push/subscriptions", {
        method: "DELETE",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ endpoint: sub.endpoint }),
      }).catch(function () {});
      await sub.unsubscribe();
    },
  };
})();
