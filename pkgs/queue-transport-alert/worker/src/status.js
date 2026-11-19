// The public status view: GET /status (JSON, CORS-open, for the PM stack
// and scripts) and GET / (a small page to open on a phone). No secrets
// and no failure bodies beyond the classified detail line.
import { span } from "./logic.js";
import { PAGE_SCRIPT } from "./pwa.js";

const STALE_AFTER_MS = 25 * 60 * 1000; // more than two missed 10-minute ticks

function issue(repo, n) {
  return n ? { number: n, url: `https://github.com/${repo}/issues/${n}` } : null;
}

export function publicStatus(state, env, nowMs) {
  const repo = env.REPO || "developing-today/code";
  const s = state || {};
  const lastProbeMs = s.last_probe ? Date.parse(s.last_probe) : null;
  let status;
  if (!lastProbeMs) status = "unknown";
  else if (nowMs - lastProbeMs > STALE_AFTER_MS) status = "stale";
  else if (s.alerted) status = "down";
  else if ((s.consecutive_failures || 0) > 0) status = "degraded";
  else status = "up";
  const outage = s.last_outage
    ? {
        start: s.last_outage.start,
        end: s.last_outage.end,
        duration: span(Date.parse(s.last_outage.start), Date.parse(s.last_outage.end)),
        incident: issue(repo, s.last_outage.incident),
      }
    : null;
  return {
    service: "queue-broker",
    status, // up | degraded (failing, below the alert threshold) | down (alerted) | stale | unknown
    up: status === "up" ? true : status === "down" || status === "degraded" ? false : null,
    last_check: s.last_probe || null,
    last_ok: s.last_ok || null,
    failing_since: s.window_start || null,
    consecutive_failures: s.consecutive_failures || 0,
    last_failure: s.consecutive_failures ? s.last_failure_detail || null : null,
    incident: s.alerted ? issue(repo, s.incident) : null,
    last_outage: outage,
    // The latest phone alert; the service worker shows it for each push.
    last_push: s.last_push || null,
    pending_deliveries: Array.isArray(s.pending) ? s.pending.map((p) => p.type) : [],
    last_errors: Array.isArray(s.last_errors) ? s.last_errors : [],
    probe: `${env.BROKER_URL || ""}/healthz via Cloudflare Access`,
    cadence_minutes: 10,
    alert_after_failures: Number(env.THRESHOLD || 2),
    issue_title_prefix: "Queue transport incident:",
    source: "pkgs/queue-transport-alert/worker (code#6369)",
  };
}

const esc = (v) =>
  String(v ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

export function renderStatusPage(view) {
  const colour = { up: "#1a7f37", degraded: "#9a6700", down: "#cf222e" }[view.status] || "#57606a";
  const row = (k, v) => (v ? `<tr><th>${esc(k)}</th><td>${v}</td></tr>` : "");
  const link = (i) => (i ? `<a href="${esc(i.url)}">#${esc(i.number)}</a>` : "");
  const outage = view.last_outage
    ? `${esc(view.last_outage.start)} to ${esc(view.last_outage.end)} (${esc(view.last_outage.duration)}) ${link(view.last_outage.incident)}`
    : "none recorded";
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Queue broker status</title>
<link rel="manifest" href="/manifest.webmanifest"><link rel="icon" href="/icon.svg"><meta name="theme-color" content="#0d1117">
<style>body{font:16px system-ui,sans-serif;margin:16px;max-width:640px;color:#1f2328;background:#fff}
@media (prefers-color-scheme:dark){body{color:#e6edf3;background:#0d1117}a{color:#58a6ff}}
.badge{display:inline-block;padding:4px 12px;border-radius:999px;color:#fff;background:${colour};font-weight:600}
table{border-collapse:collapse;margin-top:12px;width:100%}th,td{text-align:left;padding:6px 8px;vertical-align:top;border-bottom:1px solid #8884}
th{white-space:nowrap;font-weight:500;color:#8b949e}
button{font:inherit;padding:10px 16px;border-radius:8px;border:1px solid #8886;background:#1f6feb;color:#fff;margin:4px 8px 4px 0}
#push-test{background:transparent;color:inherit}</style></head>
<body><h1>Queue broker <span class="badge">${esc(view.status)}</span></h1>
<table>
${row("Last check", esc(view.last_check || "not yet"))}
${row("Last healthy", esc(view.last_ok))}
${row("Failing since", esc(view.failing_since))}
${row("Failed checks", view.consecutive_failures ? esc(view.consecutive_failures) : "")}
${row("Last failure", esc(view.last_failure))}
${row("Incident", link(view.incident))}
${row("Last outage", outage)}
${row("Checks", `every ${esc(view.cadence_minutes)} min; alerts after ${esc(view.alert_after_failures)} failures in a row`)}
</table>
<div id="push" hidden><p><button id="push-on" hidden>Turn on phone alerts</button><button id="push-test" hidden>Send a test</button></p>
<p id="push-note"></p></div>
<p><a href="/status">JSON</a></p>
<script>${PAGE_SCRIPT}
let busy=false;document.addEventListener("click",()=>{busy=true});
setTimeout(()=>{if(!busy)location.reload()},60000);</script></body></html>`;
}
