// render.ts split (#13051): alerts, notification preferences, notifications and digest views.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import { DIGEST_SECTION_LIMIT, type Digest, type DigestKind } from "./digest.ts";
import type { Incident, IncidentEvent, PushSubscriptionRow } from "./alerts.ts";
import type { NotificationRollup } from "./notifications.ts";
import { NOTIFICATION_CLASS_DEFAULTS, NOTIFICATION_CLASSES } from "./notifications.ts";
import type { Provenance, ProvenanceDetail } from "./model.ts";

import { esc, fmtDate, freshnessChips, layout } from "./render-core.ts";

// --- Alerts page (#3676, the alert throat) ---------------------------------

export interface AlertsPageView {
  available: boolean;
  incidents: Incident[];
  events: IncidentEvent[];
  subscriptions: PushSubscriptionRow[];
  ackWindowMin: number;
}

function incidentCard(i: Incident): string {
  const sev = i.severity === "critical" ? "closed" : i.severity === "warning" ? "implementing" : "open";
  const times = [
    `first seen ${fmtDate(i.first_seen_at)}`,
    i.occurrences > 1 ? `${i.occurrences} occurrences, last ${fmtDate(i.last_seen_at)}` : null,
    i.delivered_at
      ? `pushed ${fmtDate(i.delivered_at)}`
      : i.severity === "critical" && i.status === "open"
        ? "not yet pushed"
        : null,
    i.escalated_at ? `escalated ${fmtDate(i.escalated_at)}` : null,
    i.acked_at ? `acked ${fmtDate(i.acked_at)}` : null,
    i.closed_at ? `closed ${fmtDate(i.closed_at)}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const actions =
    i.status === "open"
      ? `<button data-ack="${esc(i.id)}">Ack</button> <button data-close="${esc(i.id)}">Close</button>`
      : i.status === "acked"
        ? `<button data-close="${esc(i.id)}">Close</button>`
        : "";
  return `<div class="card incident" id="incident-${esc(i.id)}">
<strong>${esc(i.title)}</strong> <span class="badge ${sev}">${esc(i.severity)}</span> <span class="badge">${esc(i.status)}</span>
${i.body ? `<br>${esc(i.body)}` : ""}
<br><span class="muted">source <code>${esc(i.source)}</code> · key <code>${esc(i.incident_key)}</code> · ${esc(times)}</span>
${actions ? `<br>${actions}` : ""}
</div>`;
}

function subscriptionRow(s: PushSubscriptionRow): string {
  let host = s.endpoint;
  try {
    host = new URL(s.endpoint).host;
  } catch {}
  const classes = Object.entries(s.classes ?? {})
    .map(([k, v]) => `${esc(k)}: ${v ? "on" : "off"}`)
    .join(", ");
  const quiet =
    s.quiet?.enabled === false
      ? "quiet hours off"
      : `quiet hours ${s.quiet?.start ?? "?"}–${s.quiet?.end ?? "?"} ${s.quiet?.tz ?? ""}`;
  return `<div class="card"><strong>${esc(host)}</strong> <span class="muted">subscribed ${fmtDate(s.created_at)} · classes ${esc(classes || "—")} · ${esc(quiet)}${s.gone_count ? ` · gone ×${s.gone_count}` : ""}</span>
<br><span class="muted">${esc(s.user_agent ?? "")}</span>
<br><button data-unsub="${esc(s.endpoint)}">Remove this subscription</button></div>`;
}

export function renderAlerts(view: AlertsPageView, provenance: Provenance, vapidPublicKey: string): string {
  if (!view.available) {
    return layout(
      "Alerts",
      `<h1>Alerts</h1><p class="muted">The alerts ledger (Turso fleet-platform <code>alerts_*</code> tables) is unreachable from this process right now — intake, delivery, and ack are paused with it. The rest of the dashboard is unaffected.</p>`,
      provenance,
      "/alerts",
    );
  }
  const open = view.incidents.filter((i) => i.status !== "closed");
  const closed = view.incidents.filter((i) => i.status === "closed");
  const events = view.events
    .map(
      (e) =>
        `<tr><td>${fmtDate(e.at)}</td><td>${esc(e.kind)}</td><td>${esc(e.title ?? e.incident_id)}</td><td class="muted">${esc(e.source ?? "")}${e.incident_key ? ` · ${esc(e.incident_key)}` : ""}</td></tr>`,
    )
    .join("");
  // The push form's initial state is rendered from the newest stored
  // subscription (Drew 2026-11-01): the page shows the saved truth on
  // load even before — or entirely without — the client script
  // matching this browser's own subscription. The client script
  // still refines it per-browser via applyPrefs once it runs.
  const formSub = [...view.subscriptions].sort((a, b) => (a.last_seen_at < b.last_seen_at ? 1 : -1))[0];
  const formIncidentChecked = !formSub || formSub.classes?.incident !== false;
  const formQuietOn = !formSub || formSub.quiet?.enabled !== false;
  const formQuietStart = formSub?.quiet?.start ?? "23:00";
  const formQuietEnd = formSub?.quiet?.end ?? "07:00";
  const body = `<h1>Alerts</h1>
<p class="muted">The alert throat (#3676): every sentinel posts to one intake; this page is the ack surface. Critical incidents push immediately to subscribed browsers; an unacknowledged critical escalates once after ${view.ackWindowMin} minutes. Warnings are recorded here and in the ledger — no push in v1. Machine-first: <a href="/api/alerts.json">/api/alerts.json</a>. Per-class toggles, quiet hours, and per-class test sends live on the <a href="/alerts/preferences">notification preferences page</a>.</p>
<style>.incident:target { outline: 2px solid #1f6feb; }</style>
<h2>Push delivery on this browser</h2>
<div class="card" id="push-panel">
<span id="push-status" class="muted">checking…</span>
<div id="push-guide" hidden>
<div id="push-guide-ios" hidden>
<p><strong>On iPhone, Apple only allows web push from the installed app:</strong></p>
<ol>
<li>Open this page in <strong>Safari</strong> — if you got here from inside another app, use its menu to open the page in Safari instead.</li>
<li>Tap the <strong>Share</strong> button, then <strong>Add to Home Screen</strong>.</li>
<li>Open the dashboard from the new home-screen icon, come back to this Alerts page, and tap <strong>Enable push on this browser</strong> — and tap <strong>Allow</strong> when it asks about notifications.</li>
</ol>
</div>
<div id="push-guide-other" hidden>
<p><strong>This browser can't receive web push</strong> — most often because the page is open inside another app (an in-app browser) or in a browser without push support. Open this page's address in your regular browser (Chrome, Safari, or Firefox) and enable push there.</p>
</div>
</div>
<p><label><input type="checkbox" id="push-class-incident"${formIncidentChecked ? " checked" : ""}> <strong>Incident alerts</strong></label><br>
<span class="muted">A watchdog reports something down. Incident alerts always come through immediately, the moment they're posted.</span></p>
<p><label><input type="checkbox" id="push-quiet-enabled"${formQuietOn ? " checked" : ""}> <strong>Quiet hours</strong></label><br>
<span class="muted">Pause non-urgent notifications overnight, between <input type="time" id="push-quiet-start" value="${formQuietStart}"${formQuietOn ? "" : " disabled"}> and <input type="time" id="push-quiet-end" value="${formQuietEnd}"${formQuietOn ? "" : " disabled"}> (Chicago time). Untick the box for no time-of-day limit at all. Incident alerts always come through immediately, quiet hours or not.</span></p>
<button id="push-enable">Enable push on this browser</button>
<button id="push-test">Send a test alert</button>
<button id="push-save-prefs">Save preferences</button>
<button id="push-disable">Disable push on this browser</button>
<br><span id="push-test-result" class="muted"></span>
</div>
<h2>Subscriptions (${view.subscriptions.length})</h2>
${view.subscriptions.length ? view.subscriptions.map(subscriptionRow).join("") : `<p class="muted">No push subscriptions yet — enable push above.</p>`}
<h2>Open incidents (${open.length})</h2>
${open.length ? open.map(incidentCard).join("") : `<p class="muted">No open incidents. Silence is golden.</p>`}
${closed.length ? `<h2>Closed (recent)</h2>${closed.map(incidentCard).join("")}` : ""}
<h2>Ledger — recent transitions</h2>
 <div class="tblwrap"><table><caption>Recent alert-ledger transitions</caption><tr><th scope="col">at</th><th scope="col">kind</th><th scope="col">incident</th><th scope="col">source · key</th></tr>${events}</table></div>`;
  const script = `<script>
const VAPID = ${JSON.stringify(vapidPublicKey)};
const SUBS = ${JSON.stringify(view.subscriptions.map((s) => ({ endpoint: s.endpoint, classes: s.classes, quiet: s.quiet })))};
async function post(path) { const r = await fetch(path, { method: "POST" }); if (!r.ok) alert("failed: HTTP " + r.status); location.reload(); }
document.querySelectorAll("[data-ack]").forEach((b) => b.addEventListener("click", () => post("/api/alerts/" + b.dataset.ack + "/ack")));
document.querySelectorAll("[data-close]").forEach((b) => b.addEventListener("click", () => post("/api/alerts/" + b.dataset.close + "/close")));
document.querySelectorAll("[data-unsub]").forEach((b) => b.addEventListener("click", async () => {
  await fetch("/api/push/subscriptions", { method: "DELETE", headers: { "content-type": "application/json" }, body: JSON.stringify({ endpoint: b.dataset.unsub }) });
  location.reload();
}));
const status = document.getElementById("push-status");
const DENIED_TEXT = "Notifications are blocked for this site on this device. On iPhone: Settings → Notifications → this app — allow notifications; or delete the home-screen app, re-add it, and tap Allow when asked. On a computer: the lock icon in the address bar → site settings → Notifications → Allow.";
const PLAIN_ERRORS = {
  denied: DENIED_TEXT,
  dismissed: "The notification prompt was closed without a choice — nothing was turned on. Tap Enable push again and choose Allow.",
  unsupported: "Push isn't available in this browser — follow the steps below to get it working.",
  "subscribe-failed": "The browser couldn't register for push — check the connection and try again.",
  "save-failed": "Your choice couldn't be saved on the server — try again.",
  "not-subscribed": "This browser isn't subscribed yet — enable push first.",
};
function showError(e) {
  status.textContent = PLAIN_ERRORS[e && e.code] || "Something went wrong — try again.";
}
function syncQuietInputs() {
  const on = document.getElementById("push-quiet-enabled").checked;
  document.getElementById("push-quiet-start").disabled = !on;
  document.getElementById("push-quiet-end").disabled = !on;
}
document.getElementById("push-quiet-enabled").addEventListener("change", syncQuietInputs);
function applyPrefs(mine) {
  if (mine.classes && typeof mine.classes.incident === "boolean") {
    document.getElementById("push-class-incident").checked = mine.classes.incident;
  }
  if (mine.quiet) {
    document.getElementById("push-quiet-enabled").checked = mine.quiet.enabled !== false;
    if (mine.quiet.start) document.getElementById("push-quiet-start").value = mine.quiet.start;
    if (mine.quiet.end) document.getElementById("push-quiet-end").value = mine.quiet.end;
    syncQuietInputs();
  }
}
// Capability is checked against the window directly, NOT via
// window.pmPush: push-client.js loads with defer, so when this
// inline script first runs pmPush does not exist yet. The first
// build read that absence as "this browser can't do push" and
// painted the unsupported guide on every load in every browser —
// and returned before applying saved prefs (Drew 2026-11-01:
// pushes worked while the page said unsupported, and a saved
// quiet-hours-off reloaded as on). pmPush itself is awaited below.
function pushSupportedHere() {
  return "PushManager" in window && "Notification" in window && "serviceWorker" in navigator;
}
function waitForPmPush() {
  if (window.pmPush) return Promise.resolve(window.pmPush);
  return new Promise((resolve) => {
    let n = 0;
    const t = setInterval(() => {
      n++;
      if (window.pmPush || n > 60) { clearInterval(t); resolve(window.pmPush || null); }
    }, 50);
  });
}
async function refreshStatus() {
  const guide = document.getElementById("push-guide");
  if (!pushSupportedHere()) {
    status.textContent = "Push isn't available in this browser yet — the steps below get it working.";
    guide.hidden = false;
    const ios = /iphone|ipad|ipod/i.test(navigator.userAgent);
    document.getElementById("push-guide-ios").hidden = !ios;
    document.getElementById("push-guide-other").hidden = ios;
    return;
  }
  guide.hidden = true;
  const pm = await waitForPmPush();
  if (!pm) { status.textContent = "The push controls are still loading — reload the page if this sticks."; return; }
  const sub = await pm.current().catch(() => null);
  if (sub) {
    status.textContent = "This browser is subscribed — pushes will land here.";
    // Endpoint match first; with a single stored subscription it is
    // this browser's in practice (push endpoints rotate) — fall back
    // to it so saved prefs still reach the form.
    const mine = SUBS.find((s) => s.endpoint === sub.endpoint) || (SUBS.length === 1 ? SUBS[0] : null);
    if (mine) applyPrefs(mine);
  } else if (pm.permission() === "denied") {
    status.textContent = DENIED_TEXT;
  } else {
    status.textContent = "This browser is not subscribed yet — tap Enable push below.";
  }
}
refreshStatus();
syncQuietInputs();
function prefs() {
  return {
    classes: { incident: document.getElementById("push-class-incident").checked },
    quiet: {
      enabled: document.getElementById("push-quiet-enabled").checked,
      start: document.getElementById("push-quiet-start").value || "23:00",
      end: document.getElementById("push-quiet-end").value || "07:00",
      tz: "America/Chicago",
    },
  };
}
document.getElementById("push-enable").addEventListener("click", async () => {
  try { await window.pmPush.subscribe(VAPID, prefs()); location.reload(); } catch (e) { showError(e); }
});
document.getElementById("push-save-prefs").addEventListener("click", async () => {
  try {
    const p = prefs();
    await window.pmPush.updatePrefs(p);
    // Confirm in place, no reload: state the saved values in words
    // so the result is checkable without trusting the form.
    status.textContent = p.quiet.enabled
      ? "Saved — quiet hours on, " + p.quiet.start + "–" + p.quiet.end + "."
      : "Saved — quiet hours off, no time-of-day limit.";
  } catch (e) { showError(e); }
});
document.getElementById("push-disable").addEventListener("click", async () => {
  try { await window.pmPush.unsubscribe(); location.reload(); } catch (e) { showError(e); }
});
document.getElementById("push-test").addEventListener("click", async () => {
  const out = document.getElementById("push-test-result");
  out.textContent = "sending…";
  try {
    const r = await fetch("/api/push/test", { method: "POST" });
    if (!r.ok) { out.textContent = "The test couldn't be sent — the server answered HTTP " + r.status + "."; return; }
    const s = await r.json();
    if (!s.subscriptions) {
      out.textContent = "No subscriptions yet — enable push first, then try the test.";
    } else if (s.sent > 0) {
      out.textContent = "Test sent to " + s.sent + " subscription" + (s.sent === 1 ? "" : "s") + " — look for a notification titled 'Test alert'.";
    } else {
      out.textContent = "The test reached no device (" + s.errors + " failed, " + s.gone + " no longer there) — the subscriptions may be stale; disable push and enable it again.";
    }
  } catch (e) {
    out.textContent = "The test couldn't be sent — network or server error.";
  }
});
</script>`;
  return layout("Alerts", body, provenance, "/alerts", script);
}

// --- Notification preferences page (#3318, slice (d) of #1254) ------------

export interface NotificationPreferencesView {
  available: boolean;
  subscriptions: PushSubscriptionRow[];
}

/** The /alerts/preferences page: per-class subscription toggles,
 * quiet-hours editing, this browser's subscription state, and a
 * send-test button per class. Preferences are subscription state —
 * saving writes through the same /api/push/subscriptions upsert the
 * Alerts form uses (provided-classes merge: only the keys sent
 * change), and the engine reads the stored map via classEnabled /
 * inQuietHours on its next tick, so a toggle here IS the next
 * engine decision. The route is /alerts/preferences, not
 * /notifications: that route is the #3319 measurement page and
 * stays read-only. */
export function renderNotificationPreferences(
  view: NotificationPreferencesView,
  provenance: Provenance,
  vapidPublicKey: string,
): string {
  if (!view.available) {
    return layout(
      "Notification preferences",
      `<h1>Notification preferences</h1><p class="muted">The alerts ledger (Turso fleet-platform <code>alerts_*</code> tables) is unreachable from this process right now — subscription preferences cannot be read or saved until it is back. The rest of the dashboard is unaffected.</p>`,
      provenance,
      "/alerts/preferences",
    );
  }
  // Same initial-state rule as the Alerts form: render the newest
  // stored subscription's saved truth server-side; the client
  // script refines it to this browser's own subscription.
  const formSub = [...view.subscriptions].sort((a, b) => (a.last_seen_at < b.last_seen_at ? 1 : -1))[0];
  const quietOn = !formSub || formSub.quiet?.enabled !== false;
  const quietStart = formSub?.quiet?.start ?? "23:00";
  const quietEnd = formSub?.quiet?.end ?? "07:00";
  const classRows = NOTIFICATION_CLASSES.map((c) => {
    const checked = !formSub || (formSub.classes?.[c.id] ?? NOTIFICATION_CLASS_DEFAULTS[c.id]) !== false;
    return `<div class="card"><p><label><input type="checkbox" id="pref-class-${c.id}"${checked ? " checked" : ""}> <strong>${esc(c.label)}</strong> <span class="muted">(<code>${esc(c.id)}</code>)</span></label><br>
<span class="muted">${esc(c.blurb)}</span></p>
<button data-test-class="${esc(c.id)}">Send a test ${esc(c.id)} notification</button></div>`;
  }).join("\n");
  const body = `<h1>Notification preferences</h1>
<p class="muted">Per-class push preferences for this browser's subscription (#3318, slice (d) of the #1254 design). Toggling a class or editing quiet hours and saving changes the stored subscription; the notification engine reads the stored values on its next tick, so the change is the next engine decision. This browser's subscription state is shown below. Machine-first: <a href="/api/alerts/preferences.json">/api/alerts/preferences.json</a>. The <a href="/notifications">/notifications</a> page stays the read-only measurement roll-up (#3319).</p>
<h2>This browser</h2>
<div class="card" id="pref-panel">
<span id="pref-status" class="muted">checking…</span>
<p><button id="pref-enable">Enable push on this browser</button>
<button id="pref-disable">Disable push on this browser</button></p>
</div>
<h2>Classes</h2>
${classRows}
<div class="card"><p><label><input type="checkbox" id="pref-quiet-enabled"${quietOn ? " checked" : ""}> <strong>Quiet hours</strong></label><br>
<span class="muted">Pause non-urgent notifications between <input type="time" id="pref-quiet-start" value="${quietStart}"${quietOn ? "" : " disabled"}> and <input type="time" id="pref-quiet-end" value="${quietEnd}"${quietOn ? "" : " disabled"}> (Chicago time). Untick for no time-of-day limit at all. Incident alerts always come through immediately, quiet hours or not; digest-only notifications wait for the quiet window to end.</span></p>
<button data-test-class="digest">Send a test digest notification</button>
<span class="muted">The digest is the one list-shaped landing allowed: the attention view filtered to the digest's refs.</span></div>
<p><button id="pref-save">Save preferences</button><br><span id="pref-test-result" class="muted"></span></p>
<h2>Subscriptions (${view.subscriptions.length})</h2>
${view.subscriptions.length ? view.subscriptions.map(subscriptionRow).join("") : `<p class="muted">No push subscriptions yet — enable push above.</p>`}`;
  const script = `<script>
const VAPID = ${JSON.stringify(vapidPublicKey)};
const SUBS = ${JSON.stringify(view.subscriptions.map((s) => ({ endpoint: s.endpoint, classes: s.classes, quiet: s.quiet })))};
const CLASS_IDS = ${JSON.stringify(NOTIFICATION_CLASSES.map((c) => c.id))};
const status = document.getElementById("pref-status");
const DENIED_TEXT = "Notifications are blocked for this site on this device. On iPhone: Settings → Notifications → this app — allow notifications; or delete the home-screen app, re-add it, and tap Allow when asked. On a computer: the lock icon in the address bar → site settings → Notifications → Allow.";
const PLAIN_ERRORS = {
  denied: DENIED_TEXT,
  dismissed: "The notification prompt was closed without a choice — nothing was turned on. Tap Enable push again and choose Allow.",
  unsupported: "Push isn't available in this browser — open this page in your regular browser (Chrome, Safari, or Firefox), or install the dashboard to the home screen on iPhone first.",
  "subscribe-failed": "The browser couldn't register for push — check the connection and try again.",
  "save-failed": "Your choice couldn't be saved on the server — try again.",
  "not-subscribed": "This browser isn't subscribed yet — enable push first.",
};
function showError(e) {
  status.textContent = PLAIN_ERRORS[e && e.code] || "Something went wrong — try again.";
}
function syncQuietInputs() {
  const on = document.getElementById("pref-quiet-enabled").checked;
  document.getElementById("pref-quiet-start").disabled = !on;
  document.getElementById("pref-quiet-end").disabled = !on;
}
document.getElementById("pref-quiet-enabled").addEventListener("change", syncQuietInputs);
function applyPrefs(mine) {
  if (mine.classes) {
    for (const id of CLASS_IDS) {
      if (typeof mine.classes[id] === "boolean") document.getElementById("pref-class-" + id).checked = mine.classes[id];
    }
  }
  if (mine.quiet) {
    document.getElementById("pref-quiet-enabled").checked = mine.quiet.enabled !== false;
    if (mine.quiet.start) document.getElementById("pref-quiet-start").value = mine.quiet.start;
    if (mine.quiet.end) document.getElementById("pref-quiet-end").value = mine.quiet.end;
    syncQuietInputs();
  }
}
function pushSupportedHere() {
  return "PushManager" in window && "Notification" in window && "serviceWorker" in navigator;
}
function waitForPmPush() {
  if (window.pmPush) return Promise.resolve(window.pmPush);
  return new Promise((resolve) => {
    let n = 0;
    const t = setInterval(() => {
      n++;
      if (window.pmPush || n > 60) { clearInterval(t); resolve(window.pmPush || null); }
    }, 50);
  });
}
async function refreshStatus() {
  if (!pushSupportedHere()) {
    status.textContent = "Push isn't available in this browser yet — open this page in your regular browser, or install the dashboard to the home screen on iPhone first.";
    return;
  }
  const pm = await waitForPmPush();
  if (!pm) { status.textContent = "The push controls are still loading — reload the page if this sticks."; return; }
  const sub = await pm.current().catch(() => null);
  if (sub) {
    status.textContent = "This browser is subscribed — pushes will land here, shaped by the preferences below.";
    const mine = SUBS.find((s) => s.endpoint === sub.endpoint) || (SUBS.length === 1 ? SUBS[0] : null);
    if (mine) applyPrefs(mine);
  } else if (pm.permission() === "denied") {
    status.textContent = DENIED_TEXT;
  } else {
    status.textContent = "This browser is not subscribed yet — tap Enable push to subscribe it with the preferences below.";
  }
}
refreshStatus();
syncQuietInputs();
function prefs() {
  const classes = {};
  for (const id of CLASS_IDS) classes[id] = document.getElementById("pref-class-" + id).checked;
  return {
    classes,
    quiet: {
      enabled: document.getElementById("pref-quiet-enabled").checked,
      start: document.getElementById("pref-quiet-start").value || "23:00",
      end: document.getElementById("pref-quiet-end").value || "07:00",
      tz: "America/Chicago",
    },
  };
}
document.getElementById("pref-enable").addEventListener("click", async () => {
  try { await window.pmPush.subscribe(VAPID, prefs()); location.reload(); } catch (e) { showError(e); }
});
document.getElementById("pref-save").addEventListener("click", async () => {
  try {
    const p = prefs();
    await window.pmPush.updatePrefs(p);
    const on = CLASS_IDS.filter((id) => p.classes[id]);
    status.textContent = "Saved — classes on: " + (on.length ? on.join(", ") : "none") + "; " + (p.quiet.enabled ? "quiet hours on, " + p.quiet.start + "–" + p.quiet.end + "." : "quiet hours off, no time-of-day limit.");
  } catch (e) { showError(e); }
});
document.getElementById("pref-disable").addEventListener("click", async () => {
  try { await window.pmPush.unsubscribe(); location.reload(); } catch (e) { showError(e); }
});
document.querySelectorAll("[data-test-class]").forEach((b) => b.addEventListener("click", async () => {
  const out = document.getElementById("pref-test-result");
  const cls = b.dataset.testClass;
  out.textContent = "sending test " + cls + "…";
  try {
    const r = await fetch("/api/push/test-class", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ class: cls }) });
    if (!r.ok) { out.textContent = "The test couldn't be sent — the server answered HTTP " + r.status + "."; return; }
    const s = await r.json();
    if (!s.subscriptions) {
      out.textContent = "No subscriptions yet — enable push first, then try the test.";
    } else if (s.eligible === 0) {
      out.textContent = "No subscription has the " + cls + " class on — turn it on above, save, then test again.";
    } else if (s.sent > 0) {
      out.textContent = "Test " + cls + " sent to " + s.sent + " subscription" + (s.sent === 1 ? "" : "s") + " — tapping it lands on " + s.url + ".";
    } else {
      out.textContent = "The test reached no device (" + s.errors + " failed, " + s.gone + " no longer there) — the subscriptions may be stale; disable push and enable it again.";
    }
  } catch (e) {
    out.textContent = "The test couldn't be sent — network or server error.";
  }
}));
</script>`;
  return layout("Notification preferences", body, provenance, "/alerts/preferences", script);
}

// --- Notifications page (#3319, notification measurement) -------------------

export interface NotificationsPageView {
  available: boolean;
  rollup: NotificationRollup | null;
}

function pct(ratio: number | null): string {
  return ratio === null ? "—" : `${Math.round(ratio * 100)}%`;
}

export function renderNotifications(view: NotificationsPageView, provenance: Provenance): string {
  if (!view.available || !view.rollup) {
    return layout(
      "Notifications",
      `<h1>Notifications</h1><p class="muted">The notification send ledger (Turso fleet-platform <code>alerts_notification_sends</code>) is unreachable from this process right now — the roll-up is paused with it. The rest of the dashboard is unaffected.</p>`,
      provenance,
      "/notifications",
    );
  }
  const rollup = view.rollup;
  const proposals = rollup.classes.filter((c) => c.proposal);
  const rows = rollup.classes
    .map((c) => {
      const cells = c.weeks.map((w) => `<td>${w.sent} sent · ${w.acted} acted (${pct(w.ratio)})</td>`).join("");
      return `<tr><td><strong>${esc(c.class)}</strong></td>${cells}<td>${c.sent} sent · ${c.acted} acted (${pct(c.ratio)})</td></tr>`;
    })
    .join("");
  const weekHeads = rollup.classes[0]
    ? rollup.classes[0].weeks.map((w) => `<th scope="col">week of ${esc(fmtDate(w.start))}</th>`).join("")
    : "";
  const proposalHtml = proposals.length
    ? `<h2>Default-off proposals</h2>${proposals
        .map(
          (c) =>
            `<div class="card"><strong>Proposal: default class <code>${esc(c.class)}</code> off</strong> — acted-on ratio ${pct(c.ratio)} over the rolling window (${c.acted} of ${c.sent} sends acted on within 24h), under the 10% threshold.<br><span class="muted">Proposal only — this page never changes a preference or a default. Turning a class off by default is Drew's call, made on the <a href="/alerts/preferences">notification preferences page</a>.</span></div>`,
        )
        .join("")}`
    : `<h2>Default-off proposals</h2><p class="muted">No class is under the 10% acted-on threshold over the rolling window.</p>`;
  const body = `<h1>Notifications</h1>
<p class="muted">Weekly sent-vs-acted-on roll-up over the alert throat's send ledger (<code>alerts.notification_sends</code>), rolling four weeks ending ${esc(fmtDate(rollup.asOf))}. Read-only: this page measures; it never changes a notification preference or default. Machine-first: <a href="/api/notifications.json">/api/notifications.json</a>.</p>
<h2>How acted-on is counted (attribution)</h2>
<p class="muted">A send counts as acted-on when the incident's Ack lands within 24 hours of the send: Ack is the exactly-once transition that stamps the send's <code>acted_on_at</code>, so it is the durable acted-on signal. Only ledger rows with outcome <code>sent</code> count as sends — suppressed (quiet hours / dedupe), gone, and error attempts never reached a device and never enter the denominator, and no send is inferred from delivered/escalated incident events. What this cannot attribute: reading a notification without acking, acting on the incident through another surface (GitHub, chat, a later dashboard visit that does not ack), and sends to subscriptions that were later removed — those count as sent-but-not-acted even when a human did see them. Test sends (class <code>test</code>) are real ledger rows and are reported as their own class.</p>
${rollup.undated ? `<p class="muted">${rollup.undated} ledger row(s) carried no usable send timestamp and were excluded from the roll-up.</p>` : ""}
<h2>Per class, per week</h2>
${
  rollup.classes.length
    ? `<div class="tblwrap"><table class="table table-zebra"><caption>Sent vs acted-on by class and rolling week</caption><tr><th scope="col">class</th>${weekHeads}<th scope="col">rolling 4 weeks</th></tr>${rows}</table></div>`
    : `<p class="muted">No sends in the rolling four-week window.</p>`
}
${proposalHtml}`;
  return layout("Notifications", body, provenance, "/notifications");
}

/** The digest page (#1278): renders a Digest verbatim — the archived
 * JSON is the page's data, so a re-read shows exactly what was
 * delivered to chat. /digest?date=<date> renders that day's archive;
 * /digest renders today's live build with the archive index. */
export function renderDigest(
  digest: Digest,
  archives: { kind: DigestKind; date: string; route: string }[],
  provenance: Provenance,
  detail?: ProvenanceDetail,
): string {
  const freshness = digest.freshness.length
    ? `<p class="muted">Data freshness: ${digest.freshness
        .map((f) => `${esc(f.facet)}=${esc(f.state)}${f.ageMs !== null ? ` (${esc(ageTextDigest(f.ageMs))})` : ""}`)
        .join(" · ")}</p>`
    : `<p class="muted">Data freshness: not measured by this source — treat every section as unverified.</p>`;
  const sections = digest.sections
    .map((s) => {
      const items = s.items.length
        ? `<ul>${s.items
            .map(
              (i) =>
                `<li><a href="${esc(i.url)}">${esc(i.title)}</a> <span class="muted">${esc(i.ref)} — ${esc(i.note)}</span></li>`,
            )
            .join("")}</ul>${s.truncated ? `<p class="muted">… capped at ${DIGEST_SECTION_LIMIT} items</p>` : ""}`
        : `<p class="muted">${s.stale ? "(unavailable — see the stale note)" : "(nothing)"}</p>`;
      return `<h2>${esc(s.title)}${s.stale ? ` — <span class="stale">STALE: ${esc(s.staleNote ?? "source degraded")}</span>` : ""}</h2>${items}`;
    })
    .join("\n");
  const archiveLinks = archives.length
    ? `<h2>Archive</h2><ul>${archives
        .map((a) => `<li><a href="${esc(a.route)}">${esc(a.date)}</a> <span class="muted">${esc(a.kind)}</span></li>`)
        .join("")}</ul>`
    : `<h2>Archive</h2><p class="muted">No digests archived yet — the generator (scripts/generate-digest.ts) writes one per day.</p>`;
  const body = `<h1>PM ${esc(digest.kind)} digest — ${esc(digest.date)}</h1>
<p class="muted">Generated ${esc(digest.generatedAt)} from the canonical projection · window ${digest.windowHours}h from ${esc(digest.windowStart)} · machine twin: <a href="${esc(`/api/digest.json?date=${digest.date}&kind=${digest.kind}`)}">${esc(`/api/digest.json?date=${digest.date}&kind=${digest.kind}`)}</a></p>
${freshness}
${sections}
${archiveLinks}`;
  return layout(
    `Digest ${digest.date}`,
    body,
    provenance,
    "/digest",
    "",
    detail ? freshnessChips(detail, ["items", "quota"]) : "",
  );
}

function ageTextDigest(ageMs: number): string {
  const mins = Math.max(0, Math.round(ageMs / 60000));
  if (mins < 60) return `${mins}m old`;
  const hours = Math.floor(mins / 60);
  if (hours < 48) return `${hours}h ${mins % 60}m old`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h old`;
}
