// The history page: a form to send a message or an instruction to
// Claude, ChatGPT or both, and every message sent and received on either, newest first. Works without JavaScript
// (the form posts and the server redirects back); with it, sending
// doesn't reload the page and the list refreshes every 10 seconds.

export function escapeHtml(s) {
  return String(s ?? "").replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c],
  );
}

const NAMES = { claude: "Claude", chatgpt: "ChatGPT" };
const nameOf = (m) => NAMES[m.channel] ?? "Claude";

function label(m) {
  if (m.direction === "outbound") return `${nameOf(m)} → you`;
  if (m.kind === "instruction") return `Instruction → ${nameOf(m)}`;
  return `You → ${nameOf(m)}`;
}

function status(m) {
  if (m.direction === "outbound") return "";
  return m.read_at ? `read by ${nameOf(m)}` : "unread";
}

export function statsLine(s) {
  return `${s.total} messages · ${s.inbound} from you (${s.unread_inbound} unread) · ${s.outbound} to you`;
}

export function renderRow(m) {
  // Agents never see an instruction's sender; this page does.
  const who = m.sender ? ` · ${m.kind === "instruction" ? "from " : ""}${escapeHtml(m.sender)}` : "";
  const kind = m.kind === "instruction" ? " instruction" : "";
  return `<li class="msg ${m.direction} ${escapeHtml(m.channel ?? "claude")}${kind}">
  <div class="meta"><b>${label(m)}</b>${who} · <time datetime="${escapeHtml(m.ts)}">${escapeHtml(m.ts)}</time> · via ${escapeHtml(m.source)} · #${m.id}<span class="st">${status(m) ? ` · ${status(m)}` : ""}</span></div>
  <div class="body">${escapeHtml(m.body)}</div>
</li>`;
}

/**
 * basePath is "/p/<token>" and apiPath "/api/<token>"; both come from
 * the request, so the token is never baked into anything but this
 * response (which is no-store and no-referrer).
 */
export function renderPage({
  messages,
  stats,
  basePath,
  apiPath,
  notice,
  pageSize = 200,
  olderOnly = false,
  chatgptReady = true,
  connectorUrl = "",
}) {
  const rows = messages.map(renderRow).join("\n");
  // A full page means there may be more; the link pages back by id and
  // works without JavaScript. With it, the button appends in place.
  const oldest = messages.length ? messages[messages.length - 1].id : null;
  const olderHref = oldest !== null ? `${basePath}?before_id=${oldest}` : "";
  const hasOlder = messages.length >= pageSize;
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta name="robots" content="noindex, nofollow">
<title>Message bridge</title>
<style>
:root { --bg:#fafaf9; --fg:#1c1917; --muted:#78716c; --card:#fff; --line:#e7e5e4; --in:#eff6ff; --out:#f0fdf4; --accent:#2563eb; --cl:#d97706; --gpt:#10a37f; --ins:#fefce8; }
@media (prefers-color-scheme: dark) { :root { --bg:#1c1917; --fg:#f5f5f4; --muted:#a8a29e; --card:#292524; --line:#44403c; --in:#1e293b; --out:#14281c; --accent:#60a5fa; --cl:#f59e0b; --gpt:#34d399; --ins:#2a2614; } }
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
.msg.chatgpt { border-left: 4px solid var(--gpt); }
.msg.claude { border-left: 4px solid var(--cl); }
.msg.instruction { background: var(--ins); border-style: dashed; border-left-style: solid; }
.to { display: flex; gap: 14px; align-items: center; flex-wrap: wrap; margin-top: 8px; }
.to label { display: inline-flex; gap: 6px; align-items: center; }
.meta { color: var(--muted); font-size: .8rem; margin-bottom: 4px; overflow-wrap: anywhere; }
.meta b { color: var(--fg); font-weight: 600; }
.body { white-space: pre-wrap; overflow-wrap: anywhere; }
.empty { color: var(--muted); }
.nav { margin: 12px 0; display: flex; gap: 16px; }
.nav a { color: var(--accent); }
details.connect { margin: 0 0 16px; font-size: .9rem; }
details.connect summary { cursor: pointer; color: var(--accent); }
details.connect .row input { font-family: ui-monospace, monospace; font-size: .8rem; }
</style>
</head>
<body>
<main>
<h1>Message bridge</h1>
<p class="sub" id="stats">${statsLine(stats)}</p>
${
  connectorUrl
    ? `<details class="connect"><summary>Claude connector address</summary>
<p class="notice">In Claude: Settings, Connectors, Add custom connector, and paste this. It needs no Connect or sign-in step. Keep it private: it opens this bridge.</p>
<div class="row"><input type="text" id="connector" readonly value="${escapeHtml(connectorUrl)}" aria-label="Connector address"><button type="button" id="copy">Copy</button></div>
</details>`
    : ""
}
<form method="post" action="${escapeHtml(basePath)}" id="send">
  <textarea name="body" required maxlength="16000" placeholder="Message"></textarea>
  <div class="to" role="radiogroup" aria-label="Kind">
    <span>Send as</span>
    <label><input type="radio" name="kind" value="message" checked> Message</label>
    <label><input type="radio" name="kind" value="instruction"> Instruction</label>
    <span class="notice">Agents see an instruction without its sender.</span>
  </div>
  <div class="to" role="group" aria-label="Send to">
    <span>Send to</span>
    <label><input type="checkbox" name="to" value="claude" checked> Claude</label>
    <label><input type="checkbox" name="to" value="chatgpt"> ChatGPT</label>
    ${chatgptReady ? "" : '<span class="notice">ChatGPT\'s connector isn\'t set up yet, so its messages wait here.</span>'}
  </div>
  <div class="row">
    <input type="text" name="sender" maxlength="80" placeholder="From (optional)">
    <button type="submit">Send</button>
    <span class="notice" id="notice">${escapeHtml(notice ?? "")}</span>
  </div>
</form>
${olderOnly ? `<p class="nav"><a href="${escapeHtml(basePath)}">Back to newest</a></p>` : ""}
<ul id="list">${rows || '<li class="empty">No messages yet.</li>'}</ul>
<p class="nav"><a id="older" href="${escapeHtml(olderHref)}"${hasOlder ? "" : " hidden"}>Older messages</a></p>
</main>
<script>
(() => {
  const api = ${JSON.stringify(apiPath).replace(/</g, "\\u003c")} + "/messages";
  const PAGE = ${Number(pageSize)};
  const olderOnly = ${olderOnly ? "true" : "false"};
  const olderLink = document.getElementById("older");
  const byId = new Map();
  let hasOlder = ${hasOlder ? "true" : "false"};
  const list = document.getElementById("list");
  const statsEl = document.getElementById("stats");
  const form = document.getElementById("send");
  const notice = document.getElementById("notice");
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const NAMES = { claude: "Claude", chatgpt: "ChatGPT" };
  const nameOf = (m) => NAMES[m.channel] || "Claude";
  const statsLine = (s) => s.total + " messages · " + s.inbound + " from you (" + s.unread_inbound + " unread) · " + s.outbound + " to you";
  const boxes = [...form.querySelectorAll('input[name="to"]')];
  try {
    const saved = JSON.parse(localStorage.getItem("message-bridge-to") || "null");
    if (Array.isArray(saved) && saved.length) boxes.forEach((b) => { b.checked = saved.includes(b.value); });
  } catch {}
  const row = (m) => {
    const ins = m.kind === "instruction";
    const lab = m.direction === "outbound" ? nameOf(m) + " → you" : (ins ? "Instruction → " : "You → ") + nameOf(m);
    const st = m.direction === "outbound" ? "" : (m.read_at ? " · read by " + nameOf(m) : " · unread");
    const who = m.sender ? " · " + (ins ? "from " : "") + esc(m.sender) : "";
    return '<li class="msg ' + esc(m.direction) + " " + esc(m.channel || "claude") + (ins ? " instruction" : "") + '"><div class="meta"><b>' + lab + "</b>" + who +
      ' · <time datetime="' + esc(m.ts) + '">' + esc(m.ts) + "</time> · via " + esc(m.source) + " · #" + m.id +
      '<span class="st">' + st + '</span></div><div class="body">' + esc(m.body) + "</div></li>";
  };
  const localTimes = () => document.querySelectorAll("time").forEach((t) => {
    const d = new Date(t.getAttribute("datetime"));
    if (!isNaN(d)) t.textContent = d.toLocaleString();
  });
  const oldestId = () => (byId.size ? Math.min(...byId.keys()) : null);
  function render() {
    const ms = [...byId.values()].sort((a, b) => b.id - a.id);
    list.innerHTML = ms.length ? ms.map(row).join("") : '<li class="empty">No messages yet.</li>';
    olderLink.hidden = !hasOlder;
    if (oldestId() !== null) olderLink.href = location.pathname + "?before_id=" + oldestId();
    localTimes();
  }
  async function load(query) {
    const r = await fetch(api + "?order=desc&limit=" + PAGE + query, { cache: "no-store" });
    if (!r.ok) return null;
    const data = await r.json();
    for (const m of data.messages) byId.set(m.id, m);
    const s = data.stats;
    statsEl.textContent = statsLine(s);
    return data;
  }
  // Refresh merges the newest page into what is loaded, so older
  // messages fetched with the button stay on screen.
  async function refresh() {
    if (olderOnly) return;
    try { if (await load("")) render(); } catch {}
  }
  olderLink.addEventListener("click", async (e) => {
    if (olderOnly || oldestId() === null) return;
    e.preventDefault();
    try {
      const data = await load("&before_id=" + oldestId());
      if (data) { hasOlder = data.messages.length >= PAGE; render(); }
    } catch {}
  });
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const to = boxes.filter((b) => b.checked).map((b) => b.value);
    const kind = (form.querySelector('input[name="kind"]:checked') || {}).value || "message";
    if (!to.length) { notice.textContent = "Pick Claude, ChatGPT or both."; return; }
    try { localStorage.setItem("message-bridge-to", JSON.stringify(to)); } catch {}
    const btn = form.querySelector("button");
    btn.disabled = true;
    notice.textContent = "Sending…";
    try {
      const r = await fetch(api, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ body: form.body.value, sender: form.sender.value, source: "page", to, kind }),
      });
      const data = await r.json().catch(() => ({}));
      if (r.ok) {
        form.body.value = "";
        notice.textContent = (kind === "instruction" ? "Sent instruction " : "Sent ") + (data.messages || [data.message]).map((m) => "#" + m.id + " to " + nameOf(m)).join(", ");
        await refresh();
      }
      else notice.textContent = "Not sent: " + (data.error || r.status);
    } catch (err) { notice.textContent = "Not sent: " + err; }
    btn.disabled = false;
  });
  const copy = document.getElementById("copy");
  if (copy) copy.addEventListener("click", async () => {
    const field = document.getElementById("connector");
    field.select();
    try { await navigator.clipboard.writeText(field.value); copy.textContent = "Copied"; } catch { copy.textContent = "Selected"; }
  });
  localTimes();
  refresh();
  setInterval(refresh, 10000);
})();
</script>
</body>
</html>`;
}
