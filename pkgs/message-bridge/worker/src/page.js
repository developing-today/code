// The history page: a form to send a message to Claude, and every
// message sent and received, newest first. Works without JavaScript
// (the form posts and the server redirects back); with it, sending
// doesn't reload the page and the list refreshes every 10 seconds.

export function escapeHtml(s) {
  return String(s ?? "").replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c],
  );
}

function label(m) {
  if (m.direction === "outbound") return "Claude → you";
  return "You → Claude";
}

function status(m) {
  if (m.direction === "outbound") return "";
  return m.read_at ? "read by Claude" : "unread";
}

export function renderRow(m) {
  const who = m.sender ? ` · ${escapeHtml(m.sender)}` : "";
  return `<li class="msg ${m.direction}">
  <div class="meta"><b>${label(m)}</b>${who} · <time datetime="${escapeHtml(m.ts)}">${escapeHtml(m.ts)}</time> · via ${escapeHtml(m.source)} · #${m.id}<span class="st">${status(m) ? ` · ${status(m)}` : ""}</span></div>
  <div class="body">${escapeHtml(m.body)}</div>
</li>`;
}

/**
 * basePath is "/p/<token>" and apiPath "/api/<token>"; both come from
 * the request, so the token is never baked into anything but this
 * response (which is no-store and no-referrer).
 */
export function renderPage({ messages, stats, basePath, apiPath, notice }) {
  const rows = messages.map(renderRow).join("\n");
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta name="robots" content="noindex, nofollow">
<title>Message bridge</title>
<style>
:root { --bg:#fafaf9; --fg:#1c1917; --muted:#78716c; --card:#fff; --line:#e7e5e4; --in:#eff6ff; --out:#f0fdf4; --accent:#2563eb; }
@media (prefers-color-scheme: dark) { :root { --bg:#1c1917; --fg:#f5f5f4; --muted:#a8a29e; --card:#292524; --line:#44403c; --in:#1e293b; --out:#14281c; --accent:#60a5fa; } }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font:16px/1.45 system-ui, -apple-system, sans-serif; }
main { max-width: 760px; margin: 0 auto; padding: 16px; }
h1 { font-size: 1.3rem; margin: 0 0 4px; }
.sub { color: var(--muted); font-size: .9rem; margin: 0 0 16px; }
form { background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 12px; margin-bottom: 16px; }
textarea { width: 100%; min-height: 90px; font: inherit; padding: 8px; border-radius: 8px; border: 1px solid var(--line); background: var(--bg); color: var(--fg); resize: vertical; }
.row { display: flex; gap: 8px; margin-top: 8px; align-items: center; flex-wrap: wrap; }
input[type=text] { flex: 1; min-width: 140px; font: inherit; padding: 6px 8px; border-radius: 8px; border: 1px solid var(--line); background: var(--bg); color: var(--fg); }
button { font: inherit; padding: 7px 16px; border-radius: 8px; border: 0; background: var(--accent); color: #fff; cursor: pointer; }
button:disabled { opacity: .6; }
.notice { color: var(--muted); font-size: .9rem; }
ul { list-style: none; padding: 0; margin: 0; display: grid; gap: 8px; }
.msg { border: 1px solid var(--line); border-radius: 10px; padding: 10px 12px; }
.msg.inbound { background: var(--in); }
.msg.outbound { background: var(--out); }
.meta { color: var(--muted); font-size: .8rem; margin-bottom: 4px; overflow-wrap: anywhere; }
.meta b { color: var(--fg); font-weight: 600; }
.body { white-space: pre-wrap; overflow-wrap: anywhere; }
.empty { color: var(--muted); }
</style>
</head>
<body>
<main>
<h1>Message bridge</h1>
<p class="sub" id="stats">${stats.total} messages · ${stats.inbound} to Claude (${stats.unread_inbound} unread) · ${stats.outbound} from Claude</p>
<form method="post" action="${escapeHtml(basePath)}" id="send">
  <textarea name="body" required maxlength="16000" placeholder="Message to Claude"></textarea>
  <div class="row">
    <input type="text" name="sender" maxlength="80" placeholder="From (optional)">
    <button type="submit">Send</button>
    <span class="notice" id="notice">${escapeHtml(notice ?? "")}</span>
  </div>
</form>
<ul id="list">${rows || '<li class="empty">No messages yet.</li>'}</ul>
</main>
<script>
(() => {
  const api = ${JSON.stringify(apiPath).replace(/</g, "\\u003c")} + "/messages";
  const list = document.getElementById("list");
  const statsEl = document.getElementById("stats");
  const form = document.getElementById("send");
  const notice = document.getElementById("notice");
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const row = (m) => {
    const lab = m.direction === "outbound" ? "Claude → you" : "You → Claude";
    const st = m.direction === "outbound" ? "" : (m.read_at ? " · read by Claude" : " · unread");
    const who = m.sender ? " · " + esc(m.sender) : "";
    return '<li class="msg ' + esc(m.direction) + '"><div class="meta"><b>' + lab + "</b>" + who +
      ' · <time datetime="' + esc(m.ts) + '">' + esc(m.ts) + "</time> · via " + esc(m.source) + " · #" + m.id +
      '<span class="st">' + st + '</span></div><div class="body">' + esc(m.body) + "</div></li>";
  };
  const localTimes = () => document.querySelectorAll("time").forEach((t) => {
    const d = new Date(t.getAttribute("datetime"));
    if (!isNaN(d)) t.textContent = d.toLocaleString();
  });
  async function refresh() {
    try {
      const r = await fetch(api + "?order=desc&limit=200", { cache: "no-store" });
      if (!r.ok) return;
      const data = await r.json();
      list.innerHTML = data.messages.length ? data.messages.map(row).join("") : '<li class="empty">No messages yet.</li>';
      const s = data.stats;
      statsEl.textContent = s.total + " messages · " + s.inbound + " to Claude (" + s.unread_inbound + " unread) · " + s.outbound + " from Claude";
      localTimes();
    } catch {}
  }
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const btn = form.querySelector("button");
    btn.disabled = true;
    notice.textContent = "Sending…";
    try {
      const r = await fetch(api, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ body: form.body.value, sender: form.sender.value, source: "page" }),
      });
      const data = await r.json().catch(() => ({}));
      if (r.ok) { form.body.value = ""; notice.textContent = "Sent #" + data.message.id; await refresh(); }
      else notice.textContent = "Not sent: " + (data.error || r.status);
    } catch (err) { notice.textContent = "Not sent: " + err; }
    btn.disabled = false;
  });
  localTimes();
  setInterval(refresh, 10000);
})();
</script>
</body>
</html>`;
}
