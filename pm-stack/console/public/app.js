// Console front end. No framework and no markup strings: every node is
// built with h(), and text from the fleet only ever becomes text.
const API = "/console/api/";
const view = document.getElementById("view");
const tabs = document.getElementById("tabs");
const toastEl = document.getElementById("toast");

function h(tag, attrs, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v === null || v === undefined) continue;
    if (k === "class") el.className = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "value") el.value = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) if (c !== null && c !== undefined && c !== false) el.append(c);
  return el;
}

async function api(path, body) {
  const init = body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json", "X-PM-Console": "1" }, body: JSON.stringify(body) };
  const res = await fetch(API + path, init);
  let data = null;
  try {
    data = await res.json();
  } catch {
    // A sign-in page instead of JSON means the edge session ended.
    if (res.redirected || res.status === 401 || res.status === 403) location.reload();
  }
  if (!res.ok) throw new Error((data && data.error) || `http ${res.status}`);
  return data;
}

let toastTimer;
function toast(message, bad) {
  toastEl.textContent = message;
  toastEl.className = bad ? "toast bad" : "toast";
  toastEl.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toastEl.hidden = true), bad ? 7000 : 3500);
}

/** Ask before acting. Resolves to the typed text ("" when none is asked
 * for), or null when the person backs out. */
function ask({ title, text, label, okLabel, danger, required }) {
  const dialog = document.getElementById("ask");
  const input = document.getElementById("ask-input");
  const ok = document.getElementById("ask-ok");
  document.getElementById("ask-title").textContent = title;
  document.getElementById("ask-text").textContent = text || "";
  document.getElementById("ask-label").textContent = label || "";
  input.hidden = !label;
  input.value = "";
  input.required = Boolean(required);
  ok.textContent = okLabel || "Confirm";
  ok.className = danger ? "danger" : "";
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "ok" ? input.value.trim() : null), { once: true });
    dialog.returnValue = "";
    dialog.showModal();
    if (label) input.focus();
  });
}

/** Run one action from a button: disable it, report, then redraw.
 * `done` is the message, or a function from the result to the message. */
async function act(button, path, body, done) {
  button.disabled = true;
  try {
    const { result } = await api(path, body);
    toast(typeof done === "function" ? done(result) : done);
    await show(current);
  } catch (error) {
    toast(error.message, true);
    button.disabled = false;
  }
}

const badge = (text, kind) => h("span", { class: `badge ${kind || ""}` }, text);
const empty = (text) => h("p", { class: "empty" }, text);
const card = (...children) => h("section", { class: "card" }, ...children);

// ---- views ---------------------------------------------------------------

async function overview() {
  const o = await api("overview");
  const stat = (label, value, tab) => h("button", { class: "card stat ghost", onclick: () => show(tab) }, h("b", {}, String(value)), h("span", { class: "meta" }, label));
  const q = o.queue.error ? {} : o.queue;
  const out = [
    h("div", { class: "grid" },
      stat("queued", q.queued ?? "–", "queue"), stat("claimed", q.claimed ?? "–", "queue"), stat("blocked", q.blocked ?? "–", "queue"), stat("parked", q.parked ?? "–", "queue"),
      stat("PRs ready", o.prs.error ? "–" : o.prs.ready, "prs"), stat("agents", o.agents.error ? "–" : o.agents.registered, "agents"),
      stat("units failed", o.services.error ? "–" : o.services.failed.length, "services")),
  ];
  for (const [name, part] of Object.entries(o)) if (part.error) out.push(card(h("h3", {}, `${name}: unavailable`), h("p", { class: "why" }, part.error)));
  if (!o.services.error && o.services.failed.length) out.push(card(h("h3", {}, "Failed units"), h("p", { class: "why" }, o.services.failed.join(", "))));
  return out;
}

let queueStatus = "blocked";
let queueSearch = "";
async function queue() {
  const data = await api(`queue?status=${queueStatus}&limit=100&q=${encodeURIComponent(queueSearch)}`);
  const pick = h("select", { class: "narrow", "aria-label": "Status", onchange: (e) => { queueStatus = e.target.value; show("queue"); } },
    ["queued", "claimed", "blocked", "parked"].map((s) => h("option", { value: s, selected: s === queueStatus }, `${s} (${data.counts[s] ?? 0})`)));
  const search = h("input", { type: "search", placeholder: "Filter by title", value: queueSearch, "aria-label": "Filter by title", onchange: (e) => { queueSearch = e.target.value; show("queue"); } });
  const rows = data.tasks.map((t) => card(
    h("h3", {}, `${t.id} · ${t.title}`),
    h("p", { class: "meta" }, `priority ${t.priority ?? "–"} · updated ${t.updated ?? "–"} · ${t.dedupe_key ?? ""}`),
    t.why ? h("p", { class: "why" }, t.why) : null,
    h("div", { class: "row end" },
      t.status === "blocked" ? h("button", { onclick: async (e) => {
        const reason = await ask({ title: `Unblock task ${t.id}`, text: t.title, label: "Why it can proceed", okLabel: "Unblock", required: true });
        if (reason) act(e.target, `queue/${t.id}/unblock`, { reason }, `Task ${t.id} unblocked`);
      } }, "Unblock") : null,
      h("button", { class: "ghost", onclick: async (e) => {
        const reason = await ask({ title: `Cancel task ${t.id}`, text: t.title, label: "Why it is cancelled", okLabel: "Cancel task", danger: true, required: true });
        if (reason) act(e.target, `queue/${t.id}/cancel`, { reason }, `Task ${t.id} cancelled`);
      } }, "Cancel"))));
  return [card(h("div", { class: "row" }, pick, h("span", { class: "grow" }), search)), ...(rows.length ? rows : [empty("No tasks in this state.")])];
}

const RANKS = ["expedite", "priority/critical", "now", "priority/high", "next", "none"];
async function priorities() {
  const { issues: list, truncated } = await api("priorities");
  const title = h("input", { placeholder: "A new idea or piece of work", "aria-label": "Title" });
  const body = h("textarea", { rows: 3, placeholder: "What you want, in your words (optional)", "aria-label": "Details" });
  const rank = h("select", { class: "narrow", "aria-label": "Rank" }, RANKS.map((r) => h("option", { value: r, selected: r === "next" }, r)));
  const file = card(h("h2", {}, "File work"), title, body, h("div", { class: "row end" }, rank,
    h("button", { onclick: (e) => title.value.trim() ? act(e.target, "ideas", { title: title.value, body: body.value, rank: rank.value }, (r) => (r.queued ? `Filed #${r.issue} and queued it` : `Filed #${r.issue}. ${r.note || "It is not queued yet."}`)) : toast("Give it a title", true) }, "File issue")));
  const rows = list.map((i) => card(
    h("h3", {}, h("a", { href: i.url, target: "_blank", rel: "noopener" }, `#${i.number}`), ` ${i.title}`),
    h("p", { class: "meta" }, `updated ${i.updated} · ${i.comments} comments · ${i.labels.filter((l) => !RANKS.includes(l)).slice(0, 6).join(", ")}`),
    h("div", { class: "row end" },
      h("select", { class: "narrow", "aria-label": `Rank for #${i.number}`, onchange: (e) => act(e.target, `priorities/${i.number}/rank`, { rank: e.target.value }, `#${i.number} is now ${e.target.value}`) },
        RANKS.map((r) => h("option", { value: r, selected: r === i.rank }, r))),
      h("button", { class: "ghost", onclick: async (e) => {
        const text = await ask({ title: `Comment on #${i.number}`, text: i.title, label: "Comment", okLabel: "Post", required: true });
        if (text) act(e.target, `issues/${i.number}/comment`, { body: text }, "Comment posted");
      } }, "Comment"))));
  const more = truncated ? [empty("There are more ranked issues than this list reads. The most urgent may not all be here.")] : [];
  return [file, ...more, ...(rows.length ? rows : [empty("No ranked issues.")])];
}

async function agents() {
  const [list, log] = await Promise.all([api("agents"), api("messages?limit=20")]);
  const groups = [...new Set(list.flatMap((a) => a.groups || []))].sort();
  const target = h("select", { "aria-label": "Send to" },
    h("option", { value: "all" }, "everyone"),
    groups.map((g) => h("option", { value: `group:${g}` }, `group: ${g}`)),
    list.map((a) => h("option", { value: `agent:${a.name}` }, a.name)));
  const kind = h("select", { class: "narrow", "aria-label": "Kind" }, h("option", { value: "message" }, "idea / message"), h("option", { value: "command" }, "command"));
  const body = h("textarea", { rows: 4, placeholder: "The idea, in your words", "aria-label": "Text" });
  const send = card(h("h2", {}, "Send to agents"), target, body, h("div", { class: "row end" }, kind,
    h("button", { onclick: (e) => body.value.trim() ? act(e.target, "messages", { to: [target.value], text: body.value, command: kind.value === "command" }, "Sent") : toast("Write something to send", true) }, "Send")));
  const roster = card(h("h2", {}, `Agents (${list.length})`), ...list.map((a) => h("div", { class: "row" },
    h("strong", {}, a.name), h("span", { class: "meta" }, (a.groups || []).join(", ")), h("span", { class: "grow" }),
    a.unread_commands ? badge(`${a.unread_commands} unread commands`, "warn") : null, a.unread_messages ? badge(`${a.unread_messages} unread`, "warn") : null,
    h("span", { class: "meta" }, (a.last_seen || "").slice(0, 16)))));
  const recent = card(h("h2", {}, "Recent messages"), ...(log.length ? log.map((m) => h("div", {},
    h("p", { class: "meta" }, `${(m.ts || "").slice(0, 16)} · ${m.direction} · ${m.agent} · ${m.kind}`), h("p", { class: "why" }, (m.body || "").slice(0, 400)))) : [empty("Nothing sent yet.")]));
  return [send, roster, recent];
}

async function services() {
  const list = await api("services");
  const state = (u) => badge(`${u.active} · ${u.sub}`, u.active === "failed" ? "bad" : u.active === "active" ? "good" : "warn");
  return list.map((u) => {
    const logs = h("pre", { hidden: true });
    const run = (action, danger) => h("button", { class: danger ? "danger" : "ghost", onclick: async (e) => {
      const yes = await ask({ title: `${action} ${u.unit}`, text: u.description, okLabel: action, danger });
      if (yes !== null) act(e.target, `services/${encodeURIComponent(u.unit)}/${action}`, {}, `${u.unit}: ${action} requested`);
    } }, action);
    return card(h("div", { class: "row" }, h("h3", {}, u.unit), h("span", { class: "grow" }), state(u)), h("p", { class: "meta" }, u.description),
      h("div", { class: "row end" },
        h("button", { class: "ghost", onclick: async () => {
          if (!logs.hidden) return (logs.hidden = true);
          try { logs.textContent = (await api(`services/${encodeURIComponent(u.unit)}/logs?lines=120`)).log || "(no output)"; logs.hidden = false; } catch (error) { toast(error.message, true); }
        } }, "Logs"),
        run("restart"), u.active === "active" ? run("stop", true) : run("start")), logs);
  });
}

async function prs() {
  const list = await api("prs");
  if (!list.length) return [empty("No open pull requests.")];
  return list.map((p) => {
    const detail = h("div", { class: "meta" });
    const merge = h("button", { disabled: true, onclick: async (e) => {
      const yes = await ask({ title: `Merge #${p.number}`, text: p.title, okLabel: "Merge" });
      if (yes !== null) act(e.target, `prs/${p.number}/merge`, {}, `#${p.number} merged`);
    } }, "Merge");
    const check = h("button", { class: "ghost", onclick: async (e) => {
      e.target.disabled = true;
      try {
        const d = await api(`prs/${p.number}`);
        detail.textContent = `+${d.additions} −${d.deletions} in ${d.changed_files} files · ${d.review_comments} review comments · ${d.mergeable === true ? "mergeable" : d.mergeable === false ? "conflicts" : "mergeability pending"} (${d.mergeable_state})`;
        merge.disabled = d.mergeable !== true || d.draft;
      } catch (error) { toast(error.message, true); }
      e.target.disabled = false;
    } }, "Check");
    return card(h("h3", {}, h("a", { href: p.url, target: "_blank", rel: "noopener" }, `#${p.number}`), ` ${p.title}`),
      h("p", { class: "meta" }, `${p.author} · ${p.branch} · opened ${p.created}`), detail,
      h("div", { class: "row end" }, p.draft ? badge("draft", "warn") : null, check, merge));
  });
}

// ---- shell -----------------------------------------------------------------

const VIEWS = { overview: ["Overview", overview], queue: ["Queue", queue], priorities: ["Priorities", priorities], agents: ["Agents", agents], services: ["Services", services], prs: ["Pull requests", prs] };
let current = "overview";
let showSeq = 0;

async function show(name) {
  // Views load at different speeds. Only the latest call may draw, or a
  // slow earlier view would land on top of the one now selected.
  const seq = ++showSeq;
  current = VIEWS[name] ? name : "overview";
  history.replaceState(null, "", `#${current}`);
  for (const b of tabs.children) b.toggleAttribute("aria-current", false);
  const active = tabs.querySelector(`[data-view="${current}"]`);
  if (active) active.setAttribute("aria-current", "page");
  view.replaceChildren(empty("Loading…"));
  try {
    const drawn = await VIEWS[current][1]();
    if (seq === showSeq) view.replaceChildren(...drawn);
  } catch (error) {
    if (seq !== showSeq) return;
    view.replaceChildren(card(h("h3", {}, "This section could not load"), h("p", { class: "why" }, error.message), h("div", { class: "row end" }, h("button", { onclick: () => show(current) }, "Try again"))));
  }
}

for (const [name, [label]] of Object.entries(VIEWS)) tabs.append(h("button", { "data-view": name, onclick: () => show(name) }, label));
api("me").then((me) => {
  document.getElementById("who").textContent = me.email || me.certName || me.via;
  document.getElementById("apk").hidden = !me.androidApp;
}).catch(() => {});
show(location.hash.slice(1));
