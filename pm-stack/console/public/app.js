// The PM app: what is in progress, what is next, what needs you, and a
// place to say what you want in your own words. Everything else is
// under More.
//
// The page draws from three documents the server keeps ready (core,
// questions, ops), so moving between sections never waits on a fetch.
const API = "/console/api/";
// Without a page (the tests import the small functions) nothing is found.
const $ = (id) => (typeof document === "undefined" ? null : document.getElementById(id));
const view = $("view");
const tabsEl = $("tabs");
const toastEl = $("toast");
const freshEl = $("fresh");

// ---- small helpers -----------------------------------------------------

function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v === null || v === undefined) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else if (v === true) el.setAttribute(k, "");
    else el.setAttribute(k, v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

function icon(d) {
  const NS = "http://www.w3.org/2000/svg";
  const s = document.createElementNS(NS, "svg");
  s.setAttribute("viewBox", "0 0 24 24");
  s.setAttribute("aria-hidden", "true");
  const p = document.createElementNS(NS, "path");
  p.setAttribute("d", d);
  s.append(p);
  return s;
}

/** "just now", "12 min ago", "3 h ago", "2 d ago" since an ISO time
 * (times without a zone are UTC). */
export function ago(iso, now = Date.now()) {
  if (!iso) return "";
  const t = Date.parse(/[zZ]$|[+-]\d\d:?\d\d$/.test(iso) ? iso : `${iso}Z`);
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, (now - t) / 1000);
  if (s < 90) return "just now";
  if (s < 5400) return `${Math.round(s / 60)} min ago`;
  if (s < 129600) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

/** What a pull request's check found, in one line, from whatever of it
 * GitHub gave: a part it did not give is left out, not shown as
 * "undefined". */
export function prCheck(d) {
  if (!d || typeof d !== "object") return "GitHub did not say.";
  const state = d.mergeable_state ? ` (${d.mergeable_state})` : "";
  const parts = [
    d.mergeable === true ? `can merge${state}` : d.mergeable === false ? `cannot merge${state}` : "mergeability not known yet",
    Number.isFinite(d.additions) && Number.isFinite(d.deletions)
      ? `+${d.additions} −${d.deletions}${Number.isFinite(d.changed_files) ? ` in ${d.changed_files} ${d.changed_files === 1 ? "file" : "files"}` : ""}`
      : null,
    Number.isFinite(d.review_comments) ? `${d.review_comments} review ${d.review_comments === 1 ? "comment" : "comments"}` : null,
  ];
  return parts.filter(Boolean).join(" · ");
}

/** A word with its first letter raised, for a button. */
export const capital = (word) => (word ? word[0].toUpperCase() + word.slice(1) : "");

/** The queue's priority bands, in words. */
export function band(priority) {
  const p = Number(priority);
  if (!priority && priority !== 0) return "Normal";
  if (p <= 10) return "Top";
  if (p <= 20) return "High";
  if (p <= 30) return "Normal";
  if (p <= 50) return "Low";
  return "Later";
}

const card = (...kids) => h("div", { class: "card" }, ...kids);
const empty = (text) => h("div", { class: "empty" }, text);
const chip = (text, tone) => h("span", { class: `chip${tone ? ` ${tone}` : ""}` }, text);
const ext = (href, text) => h("a", { href, target: "_blank", rel: "noopener" }, text);

let toastTimer;
function toast(message, bad, undo) {
  toastEl.replaceChildren(message);
  if (undo) toastEl.append(" ", h("button", { class: "small ghost", onclick: undo }, "Undo"));
  toastEl.className = bad ? "toast bad" : "toast";
  toastEl.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toastEl.hidden = true;
  }, bad ? 8000 : undo ? 5000 : 3500);
}

/** Ask before acting. Resolves to the typed text ("" when none is asked
 * for), or null when the person backs out. */
function ask({ title, text, label, okLabel, danger, required, value }) {
  const dialog = $("ask");
  const input = $("ask-input");
  const ok = $("ask-ok");
  $("ask-title").textContent = title;
  $("ask-text").textContent = text || "";
  $("ask-label").textContent = label || "";
  input.hidden = !label;
  input.value = value || "";
  input.required = Boolean(required);
  ok.textContent = okLabel || "Confirm";
  ok.className = danger ? "danger" : "";
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "ok" ? input.value.trim() : null), {
      once: true,
    });
    // A dialog keeps its last return value: clear it, or Escape after an
    // earlier confirmation would confirm again.
    dialog.returnValue = "";
    dialog.showModal();
    if (label) input.focus();
  });
}

// ---- state -------------------------------------------------------------

const docs = { core: null, questions: null, ops: null };
const etags = {};
const asOf = {};
let me = {};
let signedOut = false;

async function api(path, body) {
  const init =
    body === undefined
      ? {}
      : {
          method: "POST",
          headers: { "Content-Type": "application/json", "X-PM-Console": "1" },
          body: JSON.stringify(body),
        };
  let res;
  try {
    res = await fetch(API + path, init);
  } catch {
    // A sign-in that has run out answers with a redirect to another site,
    // which a fetch may not follow.
    signedOut = navigator.onLine !== false;
    throw new Error(signedOut ? "Your sign-in may have run out. Reload to sign in again." : "You are offline.");
  }
  let data = null;
  let read = true;
  try {
    data = await res.json();
  } catch {
    read = false;
    if (res.status === 401 || res.status === 403) signedOut = true;
  }
  if (!res.ok) throw new Error((data && data.error) || `http ${res.status}`);
  // Every read answers with JSON. Something else (an empty answer, a
  // page from something in between) is no answer, and saying so here
  // keeps each screen from tripping over it in its own way.
  if (body === undefined && (!read || data === null)) throw new Error("the answer could not be read");
  return data;
}

/** Fetch one document if it changed. Returns true when it did. */
async function loadDoc(name) {
  let res;
  try {
    res = await fetch(`${API}doc/${name}`, { headers: etags[name] ? { "If-None-Match": etags[name] } : {} });
  } catch {
    signedOut = navigator.onLine !== false;
    throw new Error(signedOut ? "Your sign-in may have run out. Reload to sign in again." : "You are offline.");
  }
  if (res.headers.get("x-as-of")) asOf[name] = Number(res.headers.get("x-as-of"));
  // Served from the edge: when the laptop behind it last reported.
  if (res.headers.get("x-host-seen")) hostSeen = Number(res.headers.get("x-host-seen")) || null;
  if (res.status === 304) return false;
  if (!res.ok) {
    if (res.status === 401 || res.status === 403) signedOut = true;
    throw new Error(`could not load (${res.status})`);
  }
  docs[name] = await res.json();
  etags[name] = res.headers.get("etag");
  if (name === "questions") withoutAnswered(docs[name]);
  return true;
}

/** Questions answered on this device that the list may still show for a
 * while: the answer is on its way, or held until the laptop is back. */
const answeredHere = new Set();
const questionKey = (q) => `${q.set}|${q.ledger}|${q.qid}`;

function withoutAnswered(doc) {
  const q = doc?.questions;
  if (!q?.open) return;
  const present = new Set(q.open.map(questionKey));
  // Gone from the list: the answer arrived, and there is nothing to hide.
  for (const key of answeredHere) if (!present.has(key)) answeredHere.delete(key);
  if (answeredHere.size === 0) return;
  q.open = q.open.filter((x) => !answeredHere.has(questionKey(x)));
  q.total = q.open.length;
}

/** When the laptop behind the edge last reported, or null when the app
 * is not served from an edge. */
let hostSeen = null;
/** How far behind the laptop may be before the app says so. */
const HOST_QUIET_MS = 3 * 60_000;
const HELD_NOTE = "Held: the laptop cannot be reached. It will be sent when it is back.";

/** Run one action, say how it went, and draw what changed. */
async function act(kind, args, done, button) {
  if (button) button.disabled = true;
  try {
    const { result, held } = await api("act", { kind, args });
    // Held is not done: the caller is told nothing happened yet, and the
    // owner is told where it is waiting.
    toast(held ? HELD_NOTE : typeof done === "function" ? done(result) : done);
    await refresh(true);
    return held ? null : result;
  } catch (error) {
    toast(error.message, true);
    if (button) button.disabled = false;
    return null;
  }
}

// ---- now ---------------------------------------------------------------

function taskCard(t, extra) {
  const who = [t.worker, t.lane, t.model].filter(Boolean).join(" · ");
  const when = t.claimed ? `claimed ${ago(t.claimed)}` : `queued ${ago(t.created)}`;
  return card(
    h(
      "div",
      { class: "row" },
      t.kind ? chip(String(t.kind).replace(/_/g, " ")) : null,
      t.issue && t.url ? ext(t.url, `#${t.issue}`) : null,
      Number(t.attempts) > 1 ? chip(`attempt ${t.attempts}`, "warn") : null,
    ),
    h("h3", {}, t.title),
    h("p", { class: "meta" }, [who, when].filter(Boolean).join(" · ")),
    t.step || t.goal
      ? h("div", { class: "step" }, h("b", {}, t.step ? "Next step" : "Goal"), t.step || t.goal)
      : null,
    extra,
  );
}

function notices(doc) {
  const out = [];
  // Served from the edge with the laptop out of reach: say how old this
  // is, and that working on is fine.
  if (doc === docs.core && hostSeen && Date.now() - hostSeen > HOST_QUIET_MS) {
    out.push(
      h(
        "div",
        { class: "card flag" },
        h("h3", {}, "The laptop is out of reach"),
        h(
          "p",
          { class: "meta" },
          `It last reported ${ago(new Date(hostSeen).toISOString())}; this is the state as of then. You can keep working: what is safe to do late is held and sent when it is back.`,
        ),
      ),
    );
  }
  if (doc === docs.core && doc?.edge?.held > 0) {
    const n = doc.edge.held;
    out.push(
      h("div", { class: "card flag" }, h("p", { class: "meta" }, `${n} ${n === 1 ? "thing" : "things"} you did ${n === 1 ? "is" : "are"} waiting for the laptop. See “What you did” below.`)),
    );
  }
  for (const [name, s] of Object.entries(doc?.health || {})) {
    if (!s.ok && s.error) {
      out.push(
        h("div", { class: "card flag" }, h("p", { class: "meta" }, `Could not read ${name} just now: ${s.error}. Showing the last good copy.`)),
      );
    }
  }
  return out;
}

function viewNow() {
  const c = docs.core;
  const b = c.board || { counts: {}, now: [], next: [], blockedWhy: [] };
  const s = c.sessions || { moving: [], waiting: [] };
  const needs = needCounts();
  const stat = (value, label, to) =>
    h("button", { class: "stat", onclick: () => go(to) }, h("b", {}, String(value ?? "–")), h("span", {}, label));
  const out = [
    h(
      "div",
      { class: "stats" },
      stat(b.counts.claimed ?? 0, "in progress", "now"),
      stat(b.counts.queued ?? 0, "queued", "next"),
      stat(needs.total, "need you", "needs"),
      stat(b.counts.blocked ?? 0, "blocked", "more/queue"),
    ),
    ...notices(c),
    h("h2", {}, "In progress"),
    ...(b.now.length ? b.now.map((t) => taskCard(t)) : [empty("No task is claimed right now.")]),
  ];
  if (s.moving.length) {
    out.push(
      h("h2", {}, "Agent sessions moving"),
      ...s.moving.map((m) =>
        card(
          h("h3", {}, m.title || "(untitled session)"),
          h(
            "p",
            { class: "meta" },
            [m.agent, m.model, m.where, m.last ? `active ${ago(m.last)}` : "", m.tools ? `${m.tools} tool calls running` : ""]
              .filter(Boolean)
              .join(" · "),
          ),
        ),
      ),
    );
  }
  if ((b.counts.blocked ?? 0) > 0 && b.blockedWhy?.length) {
    out.push(
      h("h2", {}, `Why ${b.counts.blocked} tasks are blocked`),
      card(
        ...b.blockedWhy.map((w) => h("p", {}, h("b", {}, `${w.n} `), h("span", { class: "meta" }, String(w.why)))),
        h("div", { class: "row end" }, h("button", { class: "ghost small", onclick: () => go("more/queue") }, "Open blocked tasks")),
      ),
    );
  }
  if (c.acts?.length) {
    out.push(
      h("h2", {}, "What you did"),
      card(
        ...c.acts.slice(0, 6).map((a) =>
          h(
            "p",
            { class: "meta" },
            `${ago(a.at)} · ${a.kind} · `,
            h("span", { class: a.status === "failed" ? "bad" : "" }, a.status + (a.note ? `: ${a.note}` : "")),
          ),
        ),
      ),
    );
  }
  return out;
}

// ---- next --------------------------------------------------------------

function viewNext() {
  const b = docs.core.board || { counts: {}, next: [] };
  const out = [...notices(docs.core), h("h2", {}, "Queued next, in the order it will be picked up")];
  if (!b.next.length) out.push(empty("Nothing is queued. Blocked and parked tasks are under More, Queue."));
  b.next.forEach((t, i) => {
    const moves = h(
      "div",
      { class: "row end" },
      chip(band(t.priority), Number(t.priority) <= 10 ? "accent" : ""),
      h("span", { class: "grow" }),
      Number(t.priority) > 10
        ? h("button", { class: "small", onclick: (e) => act("task.move", { task: t.id, to: "first" }, "Moved to the top band", e.target) }, "Do first")
        : null,
      h("button", { class: "small ghost", onclick: (e) => act("task.move", { task: t.id, to: "sooner" }, "Moved sooner", e.target) }, "Sooner"),
      h("button", { class: "small ghost", onclick: (e) => act("task.move", { task: t.id, to: "later" }, "Moved later", e.target) }, "Later"),
      h(
        "button",
        {
          class: "small ghost",
          onclick: async (e) => {
            const why = await ask({ title: "Cancel this task?", text: t.title, label: "Why", okLabel: "Cancel task", danger: true, required: true });
            if (why) act("task.cancel", { task: t.id, reason: why }, "Cancelled", e.target);
          },
        },
        "Cancel",
      ),
    );
    out.push(h("div", { class: "lead" }, h("span", { class: "num" }, String(i + 1)), h("div", {}, taskCard(t, moves))));
  });
  const ranked = docs.ops?.priorities?.issues;
  out.push(h("h2", {}, "Ranked issues"));
  if (!ranked) out.push(empty("Loading…"));
  else {
    out.push(...ranked.slice(0, 12).map(issueCard));
    if (ranked.length > 12) out.push(h("button", { class: "ghost", onclick: () => go("more/issues") }, `All ${ranked.length} ranked issues`));
  }
  return out;
}

const RANKS = ["expedite", "priority/critical", "now", "priority/high", "next", "none"];
function issueCard(i) {
  return card(
    h("h3", {}, ext(i.url, `#${i.number}`), ` ${i.title}`),
    h("p", { class: "meta" }, `${i.updated ? `updated ${ago(i.updated)} · ` : ""}${i.comments} comments`),
    h(
      "div",
      { class: "row end" },
      h(
        "select",
        {
          class: "narrow",
          "aria-label": `Rank for #${i.number}`,
          onchange: (e) => act("issue.rank", { issue: i.number, rank: e.target.value }, `#${i.number} is now ${e.target.value}`, e.target),
        },
        RANKS.map((r) => h("option", { value: r, selected: r === i.rank }, r)),
      ),
      h(
        "button",
        {
          class: "small ghost",
          onclick: async (e) => {
            const text = await ask({ title: `Comment on #${i.number}`, text: i.title, label: "Comment", okLabel: "Post", required: true });
            if (text) act("issue.comment", { issue: i.number, text }, "Comment posted", e.target);
          },
        },
        "Comment",
      ),
    ),
  );
}

// ---- needs me ----------------------------------------------------------

function needCounts() {
  const c = docs.core || {};
  const census = c.census || { hands: [], reviews: [], other: [] };
  const waiting = c.sessions?.waiting || [];
  const questions = docs.questions ? docs.questions.questions?.total : c.questionCount?.total;
  const counts = {
    questions: questions ?? 0,
    hands: census.hands.length + waiting.length,
    reviews: census.reviews.length,
    other: census.other.length,
  };
  // What needs the owner is what only he can do or answer; the
  // escalated-with-no-action list is the fleet's to decide.
  counts.total = counts.questions + counts.hands + counts.reviews;
  return counts;
}

let shown = 12;
const pending = new Map();
/** Choices ticked on a many-answer question, kept across redraws. */
const picks = new Map();

/** Answer after a short pause, so a slip of the thumb can be undone. */
function later(key, label, send) {
  const timer = setTimeout(() => {
    pending.delete(key);
    send();
  }, 4500);
  pending.set(key, { timer, send });
  draw();
  toast(label, false, () => {
    clearTimeout(timer);
    pending.delete(key);
    toastEl.hidden = true;
    draw();
  });
}

function flushPending() {
  for (const [key, p] of pending) {
    clearTimeout(p.timer);
    pending.delete(key);
    p.send();
  }
}

async function sendAnswer(kind, args, key, said) {
  try {
    const { result, held } = await api("act", { kind, args });
    if (result?.refused?.length) toast(`Not recorded: ${result.refused[0].why}`, true);
    else {
      answeredHere.add(key);
      // A choice was already acknowledged when it was made (with its
      // Undo); anything else says so now, or the question would simply
      // vanish.
      if (held) toast(HELD_NOTE);
      else if (said) toast(said);
    }
    picks.delete(key);
    if (docs.questions?.questions) {
      const open = docs.questions.questions.open.filter((q) => `${q.set}|${q.ledger}|${q.qid}` !== key);
      docs.questions.questions.total = open.length;
      docs.questions.questions.open = open;
    }
  } catch (error) {
    toast(error.message, true);
  }
  // The list above is already right; only the counts need reading again.
  await loadDoc("core").catch(() => {});
  draw();
}

/** A line of background, and the rest of it when the question is opened. */
function background(q) {
  const body = h("div", {}, q.context ? h("p", {}, q.context) : null);
  let asked = false;
  return h(
    "details",
    {
      ontoggle: async (e) => {
        if (!e.target.open || asked || !q.more) return;
        asked = true;
        try {
          const full = await api(`question?set=${encodeURIComponent(q.set)}&ledger=${encodeURIComponent(q.ledger)}&qid=${encodeURIComponent(q.qid)}`);
          body.replaceChildren(full.context ? h("p", {}, full.context) : null, full.detail ? h("p", { class: "meta" }, full.detail) : null);
        } catch {
          asked = false;
        }
      },
    },
    h("summary", {}, "Background"),
    body,
  );
}

function questionCard(q) {
  const key = `${q.set}|${q.ledger}|${q.qid}`;
  if (pending.has(key)) return null;
  const id = { set: q.set, ledger: q.ledger, qid: q.qid };
  const picked = picks.get(key) ?? new Set();
  picks.set(key, picked);
  const choose = (o) => later(key, `Answered: ${o.label.slice(0, 60)}`, () => sendAnswer("question.answer", { ...id, option: o.id }, key));
  const options = q.options.map((o) => {
    const rec = q.recommended.includes(o.id);
    return h(
      "button",
      {
        class: `option${rec ? " rec" : ""}`,
        "aria-pressed": q.multi ? String(picked.has(o.id)) : null,
        onclick: (e) => {
          if (!q.multi) return choose(o);
          const b = e.currentTarget;
          const on = b.getAttribute("aria-pressed") !== "true";
          b.setAttribute("aria-pressed", String(on));
          b.classList.toggle("hot", on);
          if (on) picked.add(o.id);
          else picked.delete(o.id);
        },
      },
      rec ? h("small", {}, "Recommended") : null,
      o.label,
      o.tags?.filter((t) => t !== "recommended").length ? h("span", { class: "meta" }, ` · ${o.tags.filter((t) => t !== "recommended").join(", ")}`) : null,
    );
  });
  return card(
    h("p", { class: "meta" }, q.item, ...q.links.slice(0, 2).map((l) => [" · ", ext(l, `#${l.split("/").pop()}`)])),
    h("h3", {}, q.q),
    q.context || q.more ? background(q) : null,
    h("div", { class: "options" }, options),
    h(
      "div",
      { class: "row end" },
      q.multi
        ? h(
            "button",
            {
              class: "small",
              onclick: () =>
                picked.size
                  ? later(key, `Answered with ${picked.size} choices`, () => sendAnswer("question.answer", { ...id, options: [...picked] }, key))
                  : toast("Choose at least one option", true),
            },
            "Submit choices",
          )
        : null,
      h(
        "button",
        {
          class: "small ghost",
          onclick: async () => {
            const text = await ask({ title: "Your own answer", text: q.q, label: "Answer", okLabel: "Send", required: true });
            if (text) sendAnswer("question.answer", { ...id, text }, key, "Answer sent");
          },
        },
        "Write an answer",
      ),
      h(
        "button",
        { class: "small ghost", onclick: () => later(key, "Handed back: decide for me", () => sendAnswer("question.defer", { ...id, lean: "approve" }, key)) },
        "Decide for me",
      ),
    ),
  );
}

function viewQuestions() {
  const doc = docs.questions;
  if (!doc) return [empty("Loading the questions…")];
  const all = doc.questions?.open || [];
  const out = [...notices(doc)];
  if (!all.length) return [...out, empty("No open questions.")];
  let lastSet = null;
  for (const q of all.slice(0, shown)) {
    if (q.set !== lastSet) {
      lastSet = q.set;
      const inSet = all.filter((x) => x.set === q.set);
      const withRec = inSet.filter((x) => x.recommended.length).length;
      out.push(
        h(
          "div",
          { class: "set" },
          h("h2", {}, `${q.setTitle} · ${inSet.length} open`),
          withRec
            ? h(
                "button",
                {
                  class: "small",
                  onclick: async (e) => {
                    const yes = await ask({
                      title: `Accept ${withRec} recommended answers?`,
                      text: `${q.setTitle}: every open question that has a recommendation gets the recommended option.`,
                      okLabel: "Accept all",
                    });
                    if (yes !== null) {
                      await act("questions.accept", { set: q.set }, (r) => `${r.applied} answered`, e.target);
                      await loadDoc("questions").catch(() => {});
                      draw();
                    }
                  },
                },
                `Accept ${withRec} recommended`,
              )
            : null,
        ),
      );
    }
    out.push(questionCard(q));
  }
  if (all.length > shown) {
    out.push(
      h(
        "button",
        {
          class: "ghost",
          onclick: () => {
            shown += 20;
            draw();
          },
        },
        `Show more (${all.length - shown} left)`,
      ),
    );
  }
  return out;
}

function needCard(i, kind) {
  const reply = h(
    "button",
    {
      class: "small ghost",
      onclick: async (e) => {
        const text = await ask({ title: `Reply on ${i.ref}`, text: i.title, label: "Reply", okLabel: "Post", required: true });
        if (text) act("need.comment", { ref: i.ref, text }, "Reply posted", e.target);
      },
    },
    "Reply",
  );
  return card(
    h("div", { class: "row" }, kind ? chip(kind.replace("-", " "), "warn") : null, i.url ? ext(i.url, i.ref) : h("span", { class: "meta" }, i.ref || "")),
    i.action ? h("h3", {}, i.action) : h("h3", {}, i.title),
    i.action ? h("p", { class: "meta" }, i.title) : null,
    i.why ? h("p", { class: "meta" }, i.why) : null,
    i.updated ? h("p", { class: "meta" }, `updated ${ago(i.updated)}`) : null,
    i.ref && /#\d+$/.test(i.ref)
      ? h(
          "div",
          { class: "row end" },
          reply,
          kind
            ? h(
                "button",
                {
                  class: "small",
                  onclick: async (e) => {
                    const note = await ask({
                      title: "Mark this as done?",
                      text: i.action || i.title,
                      label: "Anything the agents should know (optional)",
                      okLabel: "I did it",
                    });
                    if (note !== null) act("hands.done", { ref: i.ref, note }, "Marked done; the fleet carries on", e.target);
                  },
                },
                "I did it",
              )
            : null,
        )
      : null,
  );
}

function viewNeeds(sub) {
  const counts = needCounts();
  const census = docs.core.census || { hands: [], reviews: [], other: [] };
  const waiting = (docs.core.sessions?.waiting || []).map((w) => ({ ref: w.ref.replace(/^gh:/, ""), title: w.title, why: w.why, action: w.title, url: null }));
  const which = sub || (counts.questions ? "questions" : counts.hands ? "hands" : counts.reviews ? "reviews" : "questions");
  const seg = (name, label, n) =>
    h("button", { "aria-pressed": String(which === name), onclick: () => go(`needs/${name}`) }, `${label} ${n}`);
  const out = [
    h(
      "div",
      { class: "seg" },
      seg("questions", "Questions", counts.questions),
      seg("hands", "Only you can do", counts.hands),
      seg("reviews", "Reviews", counts.reviews),
      seg("other", "Escalated", counts.other),
    ),
    ...notices(docs.core).slice(0, 2),
  ];
  if (which === "questions") out.push(...viewQuestions());
  else if (which === "hands") {
    const items = [...census.hands.map((i) => needCard(i, i.kind)), ...waiting.map((i) => needCard(i, "by hand"))];
    out.push(...(items.length ? items : [empty("Nothing needs your hands.")]));
  } else if (which === "reviews") {
    out.push(...(census.reviews.length ? census.reviews.map((i) => needCard(i)) : [empty("Nothing is waiting for your review.")]));
  } else {
    out.push(
      h("p", { class: "meta" }, "Escalated without a stated action. These are the fleet's to decide; they are listed so nothing is hidden."),
      ...(census.other.length ? census.other.map((i) => needCard(i)) : [empty("Nothing here.")]),
    );
  }
  return out;
}

// ---- ask ---------------------------------------------------------------

const STATUS = {
  queued: ["Queued", ""],
  waiting: ["Waiting for a worker", "warn"],
  working: ["Being worked on", "accent"],
  answered: ["Answered", "good"],
  done: ["Done", "good"],
};
let draft = "";

function viewAsk(sub) {
  // Words handed over by the Android app (shared or selected text, or
  // dictation) arrive after the "#", which never leaves the phone.
  if (sub?.startsWith("say=")) {
    try {
      draft = decodeURIComponent(sub.slice(4));
    } catch {
      // not valid text: leave the box as it was
    }
    history.replaceState(null, "", "#ask");
  }
  const box = h("textarea", {
    rows: 3,
    placeholder: "Say what you want, in your own words. An agent picks it up, does it, and reports back here.",
    "aria-label": "Your instruction",
    oninput: (e) => {
      draft = e.target.value;
    },
  });
  box.value = draft;
  const Speech = window.SpeechRecognition || window.webkitSpeechRecognition;
  let listening = null;
  const mic = Speech
    ? h(
        "button",
        {
          class: "ghost",
          "aria-label": "Dictate",
          onclick: (e) => {
            const b = e.currentTarget;
            if (listening) return listening.stop();
            listening = new Speech();
            listening.interimResults = false;
            listening.onresult = (ev) => {
              const said = Array.from(ev.results, (r) => r[0].transcript).join(" ");
              box.value = `${box.value} ${said}`.trim();
              draft = box.value;
            };
            listening.onend = () => {
              listening = null;
              b.classList.remove("listening");
              b.textContent = "Dictate";
            };
            b.classList.add("listening");
            b.textContent = "Listening…";
            listening.start();
          },
        },
        "Dictate",
      )
    : null;
  const send = h(
    "button",
    {
      onclick: async (e) => {
        const text = box.value.trim();
        if (!text) return toast("Write what you want first", true);
        const r = await act("ask.send", { text }, (x) => (x.queued ? `Sent as #${x.issue}; first in the queue` : `Filed as #${x.issue}. ${x.note || ""}`), e.target);
        if (r) {
          draft = "";
          draw();
        }
      },
    },
    "Send",
  );
  const asks = docs.core.asks || [];
  return [
    h("div", { class: "card composer" }, box, h("div", { class: "row end" }, mic, send)),
    ...notices(docs.core).slice(0, 1),
    h("h2", {}, "What you have asked"),
    ...(asks.length
      ? asks.map((a) => {
          const [label, tone] = STATUS[a.status] || [a.status, ""];
          return card(
            h("div", { class: "row" }, chip(label, tone), ext(a.url, `#${a.issue}`), h("span", { class: "meta" }, `${ago(a.updated)}`), a.worker ? h("span", { class: "meta" }, `· ${a.worker}`) : null),
            h("div", { class: "bubble you" }, h("b", {}, "You"), a.text),
            ...a.replies.map((r) => h("div", { class: `bubble${r.you ? " you" : ""}` }, h("b", {}, `${r.you ? "You" : "Fleet"} · ${ago(r.at)}`), r.text)),
            h(
              "div",
              { class: "row end" },
              a.status !== "done"
                ? h("button", { class: "small ghost", onclick: (e) => act("ask.close", { issue: a.issue }, "Closed", e.target) }, "Mark done")
                : null,
              h(
                "button",
                {
                  class: "small",
                  onclick: async (e) => {
                    const text = await ask({ title: `Reply on #${a.issue}`, label: "Reply", okLabel: "Send", required: true });
                    if (text) act("ask.reply", { issue: a.issue, text }, "Sent; the fleet is woken for it", e.target);
                  },
                },
                "Reply",
              ),
            ),
          );
        })
      : [empty("Nothing yet. Anything you send here becomes the first thing in the queue.")]),
  ];
}

// ---- more --------------------------------------------------------------

function viewMore() {
  const ops = docs.ops || {};
  const b = docs.core.board || { counts: {} };
  const tile = (to, title, sub) => h("button", { class: "tile", onclick: () => go(to) }, title, h("span", {}, sub));
  const out = [
    h("h2", {}, "Operations"),
    h(
      "div",
      { class: "tiles" },
      tile("more/queue", "Queue", `${b.counts.blocked ?? "–"} blocked · ${b.counts.parked ?? "–"} parked`),
      tile("more/prs", "Pull requests", ops.prs ? `${ops.prs.filter((p) => !p.draft).length} ready of ${ops.prs.length}` : "open and ready to merge"),
      tile("more/issues", "Ranked issues", ops.priorities ? `${ops.priorities.issues.length} ranked` : "change what matters most"),
      tile("more/agents", "Agents", ops.agents ? `${ops.agents.length} registered` : "message one or all"),
      tile("more/services", "Services", ops.services ? `${ops.services.filter((u) => u.active === "failed").length} failed of ${ops.services.length}` : "start, stop, logs"),
      tile("more/notify", "Notifications", "choose what reaches this device"),
      tile("more/phone", "Phones", "the Android app and its home-screen widget"),
      tile("more/signin", "Sign-in", "Google first, the emailed code as the fallback"),
      tile("more/away", "Laptop away", "what Cloudflare does by itself when the laptop cannot be reached"),
      tile("more/releases", "Releases", "what is deployed, the Android app, and putting either out again"),
    ),
  ];
  for (const g of ops.more || []) {
    out.push(h("h2", {}, `Dashboard: ${g.group}`), ...g.pages.map((p) => h("a", { class: "link", href: p.route }, p.title, h("span", {}, p.blurb))));
  }
  out.push(
    h("h2", {}, "This app"),
    h("p", { class: "meta" }, `Signed in as ${me.email || me.certName || me.via || "…"}`),
    me.androidApp ? h("a", { class: "link", href: "/console/pm-console.apk" }, "Android app", h("span", {}, "Download the signed build for this phone")) : null,
    h("a", { class: "link", href: "/cdn-cgi/access/logout" }, "Sign out"),
  );
  return out;
}

const NOTICE = {
  replies: "The fleet replied to something you asked",
  needs: "Something new needs you",
  questions: "New questions for you",
  trouble: "A unit failed",
};

/** What the browser says it runs on: "Linux; Android 10; K", "Windows NT 10.0; Win64; x64". */
export function deviceLabel(agent = navigator.userAgent) {
  const m = /\(([^)]*)\)/.exec(agent);
  return (m ? m[1] : agent).trim().slice(0, 60);
}

/** A short mark for a push address: how this device tells whether the
 * sender of notifications still has it, without the address going
 * anywhere. The same as endpointMark in src/push.ts. */
export async function endpointMark(endpoint) {
  const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(endpoint)));
  return b64url(hash).slice(0, 16);
}

function b64url(bytes) {
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/** What this device's notification subscription needs, given what the
 * sender says of itself: "keep" it, hand it over "again" (the sender
 * has lost it), or "renew" it (it was made for a key the sender does not
 * use, as when the app moved from the laptop to the edge). `madeFor` is
 * the key it was made for, or "" where the browser does not say. */
export function subscriptionState(madeFor, mark, status) {
  if (!status || !status.key) return "keep";
  if (madeFor && madeFor !== status.key) return "renew";
  return (status.known || []).includes(mark) ? "keep" : "again";
}

/** Where this device stands: "on" (it is subscribed and the sender has
 * it), "unfinished" (the browser said yes but the sender does not have
 * this device, so nothing would arrive), or "off". `state` is
 * subscriptionState's answer for this device's subscription. */
export function deviceState(permission, subscribed, state) {
  if (!subscribed || permission !== "granted") return "off";
  return state === "keep" ? "on" : "unfinished";
}

const SENT_FOR = {
  replies: "a reply from the fleet",
  needs: "something that needs you",
  questions: "a question for you",
  trouble: "a unit that failed",
  test: "a test",
};
const devices = (n) => `${n} device${n === 1 ? "" : "s"}`;

/** What came of the last notification, in a sentence or two. A push
 * service taking a message is not the phone showing it, so this says
 * "took", not "delivered". */
export function deliveryNote(last, now = Date.now()) {
  if (!last) return "Nothing has been sent yet.";
  const said = [`Last sent ${ago(last.at, now) || "at an unknown time"}: ${SENT_FOR[last.class] || "a notification"}.`];
  if (last.sent) said.push(`The push service took it for ${devices(last.sent)}.`);
  if (last.failed) {
    const first = (last.refused || [])[0];
    const why = first ? `: ${first.host} ${first.status ? `answered ${first.status}` : "did not answer"}` : "";
    said.push(`Not taken for ${devices(last.failed)}${why}.`);
  }
  if (last.gone) said.push(`${devices(last.gone)} no longer subscribed ${last.gone === 1 ? "was" : "were"} forgotten.`);
  return said.join(" ");
}

/** Notifications that are on for this device stay on: nothing is asked,
 * because the owner already said yes on this device. A device where
 * they are off is left alone. */
async function keepSubscribed() {
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) return;
  if (Notification.permission !== "granted") return;
  const reg = await navigator.serviceWorker.getRegistration("/console/");
  let sub = reg ? await reg.pushManager.getSubscription() : null;
  if (!sub) return;
  const status = await api("push");
  const key = sub.options?.applicationServerKey;
  const state = subscriptionState(key ? b64url(new Uint8Array(key)) : "", await endpointMark(sub.endpoint), status);
  if (state === "keep") return;
  if (state === "renew") {
    await sub.unsubscribe();
    sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(status.key) });
  }
  await api("act", { kind: "push.on", args: { subscription: sub.toJSON(), label: deviceLabel() } });
}

function keyBytes(b64) {
  const text = atob((b64 + "=".repeat((4 - (b64.length % 4)) % 4)).replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(text, (c) => c.charCodeAt(0));
}

/** What a test send came to, for the line shown when it is done. */
export function testSaid(r) {
  if (r.sent && !r.failed) return `Test sent to ${devices(r.sent)}`;
  if (r.sent) return `Test sent to ${devices(r.sent)}; not taken for ${devices(r.failed)}`;
  return r.failed ? "The push service did not take the test" : "No device took the test";
}

async function viewNotify() {
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) {
    return [empty("This browser cannot receive notifications. On Android, use Chrome or the app.")];
  }
  const status = await api("push");
  const reg = await navigator.serviceWorker.getRegistration("/console/");
  const sub = reg ? await reg.pushManager.getSubscription() : null;
  // The browser being subscribed is half of it: the sender has to hold
  // this device too, or nothing would arrive.
  const madeFor = sub?.options?.applicationServerKey;
  const state = sub ? subscriptionState(madeFor ? b64url(new Uint8Array(madeFor)) : "", await endpointMark(sub.endpoint), status) : "keep";
  const where = deviceState(Notification.permission, Boolean(sub), state);
  const on = where === "on";
  const turnOn = async (e) => {
    e.target.disabled = true;
    try {
      if ((await Notification.requestPermission()) !== "granted") {
        throw new Error("Notifications are blocked for this site. Allow them in the browser's site settings, then try again.");
      }
      const r = await navigator.serviceWorker.register("/console/sw.js", { scope: "/console/" });
      await navigator.serviceWorker.ready;
      // A subscription made for another sender's key cannot be reused.
      if (state === "renew") await (await r.pushManager.getSubscription())?.unsubscribe();
      const s = await r.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(status.key) });
      // Refused by the sender: the reason is on screen, and the button
      // is there to press again.
      if (!(await act("push.on", { subscription: s.toJSON(), label: deviceLabel() }, "Notifications are on for this device"))) {
        e.target.disabled = false;
      }
    } catch (error) {
      toast(error.message, true);
      e.target.disabled = false;
    }
  };
  const turnOff = async (e) => {
    const endpoint = sub.endpoint;
    await sub.unsubscribe().catch(() => {});
    act("push.off", { endpoint }, "Turned off for this device", e.target);
  };
  return [
    card(
      h("h3", {}, on ? "On for this device" : where === "unfinished" ? "Not finished on this device" : "Off for this device"),
      where === "unfinished"
        ? h("p", { class: "why" }, "This device said yes, but the sender of notifications does not have it, so nothing would arrive here.")
        : null,
      h("p", { class: "meta" }, `${devices(status.devices)} subscribed in all. ${deliveryNote(status.last)}`),
      Notification.permission === "denied" ? h("p", { class: "why" }, "This browser is set to block notifications from this site.") : null,
      h(
        "div",
        { class: "row end" },
        on ? h("button", { class: "small ghost", onclick: turnOff }, "Turn off") : null,
        on ? h("button", { class: "small ghost", onclick: (e) => act("push.test", {}, testSaid, e.target) }, "Send a test") : null,
        on ? null : h("button", { onclick: turnOn }, where === "unfinished" ? "Finish turning on" : "Turn on for this device"),
      ),
    ),
    h("h2", {}, "Tell me when"),
    card(
      ...Object.entries(NOTICE).map(([cls, label]) =>
        h(
          "label",
          { class: "row check" },
          h("input", { type: "checkbox", checked: status.classes[cls], onchange: (e) => act("push.class", { class: cls, on: e.target.checked }, e.target.checked ? "On" : "Off") }),
          label,
        ),
      ),
    ),
  ];
}

// ---- releases ----------------------------------------------------------

const RUN_STATE = {
  success: "Done",
  failure: "Failed",
  cancelled: "Cancelled",
  timed_out: "Ran out of time",
  in_progress: "Running now",
  queued: "Waiting to start",
};

/** One run of a deploy or a build, in a line: how it stands, when it
 * started, of which commit, and whether someone started it by hand. */
export function runSaid(run, now = Date.now()) {
  const state = RUN_STATE[run.state] || capital(String(run.state || "unknown").replace(/_/g, " "));
  return [state, ago(run.at, now), String(run.commit || "").slice(0, 7), run.byHand ? "started by hand" : null].filter(Boolean).join(" · ");
}

/** What is being served, in a sentence, from the edge's health answer:
 * which commit Cloudflare serves, and whether the laptop runs the same. */
export function servingSaid(health) {
  if (!health || health.edge !== true || !health.version) return "This is served straight from the laptop.";
  const edge = String(health.version).slice(0, 7);
  const host = health.hostVersion ? String(health.hostVersion).slice(0, 7) : null;
  if (!host) return `Cloudflare is serving ${edge}. The laptop has not said what it runs.`;
  if (host === edge) return `Cloudflare and the laptop are both on ${edge}.`;
  return `Cloudflare is serving ${edge}; the laptop runs ${host}. After a merge the laptop follows within a minute or two.`;
}

/** Releases: what is deployed and what is released, as Cloudflare and
 * GitHub themselves say, and the buttons that put either out again. A
 * merge does both by itself; this is for looking, and for doing it over. */
async function viewReleases() {
  let health = null;
  try {
    const res = await fetch("/console/health", { cache: "no-store" });
    health = res.ok ? await res.json() : null;
  } catch {
    health = null;
  }
  const [runs, android] = await Promise.all([api("releases").catch((error) => ({ error: error.message })), me.edge ? api("android").catch(() => null) : null]);
  const list = (got) => {
    if (!Array.isArray(got)) return [h("p", { class: "why" }, `Its runs could not be read: ${got?.error || runs.error || "no answer"}`)];
    if (got.length === 0) return [h("p", { class: "meta" }, "It has not run yet.")];
    return [h("div", { class: "runs" }, ...got.slice(0, 3).map((r) => ext(r.url, runSaid(r))))];
  };
  const start = (what, scratch, title, text, said) => async (e) => {
    const yes = await ask({ title, text, okLabel: "Start it" });
    if (yes !== null) act("deploy.start", { what, scratch }, said, e.target);
  };
  const release = android?.release || health?.androidRelease || null;
  const page = `https://github.com/${me.repo || "developing-today/code"}/releases/tag/pm-console-android`;
  return [
    h("h2", {}, "This site"),
    card(
      h("h3", {}, servingSaid(health)),
      ...list(runs.site),
      h(
        "div",
        { class: "row end" },
        h(
          "button",
          {
            class: "small ghost",
            onclick: start("site", true, "Stand it up from nothing, then deploy?", "GitHub first stands a throwaway copy of the Worker up from nothing, checks it and removes it, then deploys main to the real one. It takes a few minutes more.", "Started: from nothing first, then the deploy"),
          },
          "From nothing first",
        ),
        h("button", { class: "small", onclick: start("site", false, "Deploy the site again?", "GitHub deploys main to Cloudflare again, checks it, and puts the last version back if the check fails.", "Deploy started") }, "Deploy again"),
      ),
    ),
    h("h2", {}, "Android app"),
    card(
      h("h3", {}, release ? `Released: ${release.name}` : "No release yet"),
      release ? h("p", { class: "meta" }, `Built ${ago(release.built)}. `, ext(page, "The release on GitHub")) : h("p", { class: "meta" }, "A build is released within about ten minutes of being made."),
      ...list(runs.app),
      h(
        "div",
        { class: "row end" },
        h("button", { class: "small ghost", onclick: start("app", false, "Build the app again?", "GitHub builds the app from main. It is then signed where its key is and released, within about ten minutes.", "Build started") }, "Build again"),
        me.androidApp || release ? h("a", { class: "button", href: "/console/pm-console.apk" }, "Download") : null,
      ),
    ),
  ];
}

// ---- phones ------------------------------------------------------------

/** A request from the Android app on this phone to be linked, until it is answered. */
let linkRequest = null;
/** How many phones were linked when the request was approved, or null before that. */
let linkApprovedAt = null;

/** The address that tells the Android app to collect its token. It carries nothing. */
function collectUrl() {
  const back = encodeURIComponent(`${location.origin}/console/#more/phone`);
  return `intent://finish#Intent;scheme=pmconsole;package=${me.androidPackage || "today.developing.pmconsole"};S.browser_fallback_url=${back};end`;
}

/** Sign-in: what is on, and the one part only the owner can do to put
 * Google first. Google lets nobody but a signed-in person make the
 * client its sign-in runs on; everything after that is done here. */
/** What the edge says of acting by itself, read from its public health
 * answer. `served` is false when the app is coming straight from the
 * laptop and there is no edge to ask. */
export function awayState(health) {
  if (!health || health.edge !== true) return { served: false };
  const kinds = health.selfCheck?.kinds || {};
  const names = { board: "The board", prs: "Pull requests", priorities: "Ranked issues", asks: "Your prompts" };
  return {
    served: true,
    on: health.directEnabled === true,
    // On, and holding both tokens with a laptop new enough to have said
    // where things are: it will act once the laptop has been quiet.
    ready: health.directEnabled === true && health.directReady === true,
    checkedAt: typeof health.selfCheck?.at === "number" ? health.selfCheck.at : null,
    checks: Object.keys(names)
      .filter((kind) => kinds[kind])
      .map((kind) => ({ what: names[kind], ok: kinds[kind].ok === true, error: String(kinds[kind].error || "") })),
  };
}

/** Laptop away: whether Cloudflare acts by itself when the laptop cannot
 * be reached, what that covers, and the switch. The switch is the
 * owner's because turning it on hands the Worker two of his tokens. */
async function viewAway() {
  let health = null;
  try {
    const res = await fetch("/console/health", { cache: "no-store" });
    health = res.ok ? await res.json() : null;
  } catch {
    health = null;
  }
  const s = awayState(health);
  if (!s.served) {
    return [
      card(
        h("h3", {}, "The app is coming straight from the laptop"),
        h("p", { class: "meta" }, "It is not being served from Cloudflare right now, so nothing there can act while the laptop is away."),
      ),
    ];
  }
  const flip = async (e) => {
    const done = await act(
      "edge.direct",
      { on: !s.on },
      s.on ? "Cloudflare no longer acts by itself" : "Cloudflare will act while the laptop is away",
      e.target,
    );
    if (!done) return;
    // The Worker restarts with what it was given; ask it again shortly.
    await new Promise((resolve) => setTimeout(resolve, 5000));
    draw();
  };
  const checked = s.on
    ? s.checks.length > 0
      ? h(
          "ul",
          { class: "plain" },
          s.checks.map((c) => h("li", {}, `${c.what}: ${c.ok ? "read from Cloudflare" : `could not be read (${c.error || "no reason given"})`}`)),
        )
      : h("p", { class: "meta" }, "Not tried yet. It tries one read of each kind within a minute of being turned on.")
    : null;
  return [
    card(
      h("h3", {}, s.on ? "Cloudflare acts by itself while the laptop is away" : "While the laptop is away, Cloudflare only waits"),
      h(
        "p",
        { class: "meta" },
        s.on
          ? s.ready
            ? "It starts once the laptop has been quiet for a minute, and stops when the laptop reports again."
            : "It is on, but the Worker does not have everything it needs yet. Turning it off and on again gives it the tokens afresh."
          : "The app still opens and shows what the laptop last reported. What you send is kept and done when the laptop is back.",
      ),
      checked,
      s.on && s.checkedAt ? h("p", { class: "meta" }, `Tried ${ago(new Date(s.checkedAt).toISOString())}.`) : null,
    ),
    h("h2", {}, s.on ? "Turn it off" : "Turn it on"),
    card(
      h(
        "p",
        { class: "meta" },
        "On, Cloudflare reads the board, pull requests, ranked issues and your prompts itself, and does these at once: reply to or close a prompt, comment, rank an issue, move a task. A new prompt is filed at once and queued for the fleet when the laptop is back.",
      ),
      h(
        "p",
        { class: "meta" },
        "Turning it on gives the Worker two tokens from your secret store: the repository automation token and the queue store token. They are kept as the Worker's secrets in your Cloudflare account. Turning it off takes them away again. The laptop has to be reachable for either.",
      ),
      h("div", { class: "row end" }, h("button", { class: s.on ? "ghost" : "", onclick: flip }, s.on ? "Turn off" : "Turn on")),
    ),
  ];
}

async function viewSignin() {
  let s;
  try {
    s = await api("signin");
  } catch (error) {
    return [empty(`The sign-in settings could not be read just now: ${error.message}`)];
  }
  const copy = (text) =>
    h(
      "button",
      {
        class: "small ghost",
        onclick: () =>
          navigator.clipboard.writeText(text).then(
            () => toast("Copied"),
            () => toast("Could not copy. Select the text and copy it.", true),
          ),
      },
      "Copy",
    );
  const value = (text) => h("div", { class: "row" }, h("code", { class: "wrap" }, text), copy(text));
  const id = h("input", { type: "text", placeholder: "Client ID", autocomplete: "off", autocapitalize: "none", spellcheck: false });
  const secret = h("input", { type: "password", placeholder: "Client secret", autocomplete: "off" });
  const send = async (e) => {
    if (!id.value.trim() || !secret.value.trim()) return toast("Paste both values from Google first", true);
    const done = await act("sso.google", { clientId: id.value.trim(), clientSecret: secret.value.trim() }, "Google sign-in is on", e.target);
    secret.value = "";
    if (done) draw();
    else e.target.disabled = false;
  };
  return [
    card(
      h("h3", {}, s.google ? "Google sign-in is on" : "Google sign-in is not on yet"),
      h(
        "p",
        { class: "meta" },
        s.google
          ? "The sign-in page offers Google. The code sent by email stays as the fallback."
          : s.pin
            ? "For now the sign-in page asks for a code sent by email."
            : "No sign-in method is on.",
      ),
    ),
    h("h2", {}, s.google ? "Use a different Google client" : "Turn it on"),
    card(
      h("p", { class: "meta" }, "Google lets only a signed-in person make the client its sign-in runs on, so this part is yours. A few minutes, once."),
      h(
        "ol",
        { class: "steps" },
        h("li", {}, "Open ", ext("https://console.cloud.google.com/auth/clients", "Google Cloud, Clients"), ". If it asks, set the app up first: any name, audience External."),
        h("li", {}, "Create client. Application type: Web application."),
        h("li", {}, "Authorized JavaScript origins, add:", value(s.origin)),
        h("li", {}, "Authorized redirect URIs, add:", value(s.redirect)),
        h("li", {}, "Create, then paste the two values it shows:"),
      ),
      id,
      secret,
      h("div", { class: "row end" }, h("button", { onclick: send }, s.google ? "Use this client" : "Turn on Google sign-in")),
      h("p", { class: "meta" }, "Both values go to Cloudflare, which runs the sign-in page, and are kept nowhere else."),
    ),
  ];
}

async function viewPhone(sub) {
  if (sub?.startsWith("link=")) {
    const request = sub.slice(5);
    linkRequest = /^[A-Za-z0-9_-]{43}$/.test(request) ? request : null;
    linkApprovedAt = null;
    history.replaceState(null, "", "#more/phone");
  }
  const devices = await api("devices");
  // Where a download would come from. Only the edge can say; the
  // laptop serving the app itself has no such answer.
  const android = me.edge ? await api("android").catch(() => null) : null;
  // The phone has collected its token: the request is finished.
  if (linkApprovedAt !== null && devices.length > linkApprovedAt) {
    linkRequest = null;
    linkApprovedAt = null;
  }
  const approve = async (e) => {
    const done = await act("device.approve", { challenge: linkRequest }, "Approved", e.target);
    if (!done) return;
    linkApprovedAt = devices.length;
    draw();
    // Hand back to the app, which collects its token and fills the widget.
    location.href = collectUrl();
  };
  const out = [];
  if (linkRequest && linkApprovedAt === null) {
    out.push(
      card(
        h("h3", {}, "Link this phone?"),
        h(
          "p",
          { class: "meta" },
          "The app's home-screen widget asked to be linked. It will be able to read three counts (in progress, queued, needs you) and the titles at the top of each list, and nothing else. It cannot change anything, and you can remove it here at any time.",
        ),
        h("p", { class: "why" }, "Approve only if you just tapped the PM widget on this phone. If this appeared after following a link from somewhere else, choose Not now."),
        h(
          "div",
          { class: "row end" },
          h(
            "button",
            {
              class: "small ghost",
              onclick: () => {
                linkRequest = null;
                draw();
              },
            },
            "Not now",
          ),
          h("button", { onclick: approve }, "Link this phone"),
        ),
      ),
    );
  } else if (linkRequest) {
    out.push(
      card(
        h("h3", {}, "Approved"),
        h("p", { class: "meta" }, "The widget fills in a moment. If it does not, tap below, or tap the widget."),
        h("div", { class: "row end" }, h("a", { class: "button", href: collectUrl() }, "Finish on this phone")),
      ),
    );
  }
  out.push(
    h("h2", {}, `Linked phones (${devices.length})`),
    ...(devices.length
      ? devices.map((d) =>
          card(
            h("h3", {}, d.name),
            h("p", { class: "meta" }, `Linked ${ago(d.createdAt)} · ${d.seenAt ? `last read ${ago(d.seenAt)}` : "has not read yet"}`),
            h(
              "div",
              { class: "row end" },
              h(
                "button",
                {
                  class: "small ghost",
                  onclick: async (e) => {
                    const sure = await ask({ title: "Remove this phone?", text: `${d.name} will stop showing the counts on its home screen.`, okLabel: "Remove", danger: true });
                    if (sure !== null) act("device.remove", { id: d.id }, "Removed", e.target);
                  },
                },
                "Remove",
              ),
            ),
          ),
        )
      : [empty("No phone is linked. On a phone with the app, add the PM widget to the home screen and tap it.")]),
    h("h2", {}, "The Android app"),
    card(
      h(
        "ul",
        { class: "plain" },
        h("li", {}, h("b", {}, "Home-screen widget. "), "In progress, queued next and needs you, with the task being worked on. Each number opens its screen."),
        h("li", {}, h("b", {}, "Say it. "), "Long-press the icon and choose Dictate, or add the “Ask the fleet” tile to quick settings: speak, check the words, send."),
        h("li", {}, h("b", {}, "Share or select. "), "Share text from any app to PM Console, or select text anywhere and choose “Ask the fleet”: it lands in the Ask box."),
        h("li", {}, h("b", {}, "Shortcuts. "), "Long-press the icon for Ask, Needs me and Now."),
        h("li", {}, h("b", {}, "Notifications. "), "Turn them on under Notifications; they arrive as the app's own."),
      ),
      me.androidApp || android?.release
        ? h("div", { class: "row end" }, h("a", { class: "button", href: "/console/pm-console.apk" }, "Download the app"))
        : null,
      androidNote(me, android),
    ),
  );
  return out;
}

/** What is said under the download: which build it is and who made it. */
export function androidSource(me, android) {
  // Released by GitHub Actions: that is what the download is.
  if (android?.release) return { by: "github", name: android.release.name, at: android.release.built };
  // Served from Cloudflare's copy when the laptop is away: worth saying
  // only when that copy is not the laptop's newest build.
  if (me?.androidBuild && !me.androidBuild.current) return { by: "copy", at: new Date(me.androidBuild.at).toISOString() };
  return null;
}

function androidNote(me, android) {
  const source = androidSource(me, android);
  if (!source) return null;
  if (source.by === "github") {
    return h("p", { class: "meta" }, `Build ${source.name}, built by GitHub and released ${ago(source.at)}. It installs over the one you have.`);
  }
  return h(
    "p",
    { class: "meta" },
    `With the laptop away you get the copy kept at Cloudflare, built ${ago(source.at)}. The laptop has a newer build, which will be here once it has reported again.`,
  );
}

const QUEUE_STATES = ["blocked", "queued", "claimed", "parked"];
let queueState = { status: "blocked", q: "" };
async function viewQueue() {
  const data = await api(`queue?status=${queueState.status}&q=${encodeURIComponent(queueState.q)}`);
  const pick = h(
    "select",
    {
      class: "narrow",
      "aria-label": "State",
      onchange: (e) => {
        queueState.status = e.target.value;
        draw();
      },
    },
    QUEUE_STATES.map((s) => h("option", { value: s, selected: s === queueState.status }, `${s} (${data.counts[s] ?? 0})`)),
  );
  const search = h("input", {
    type: "search",
    placeholder: "Filter by title",
    "aria-label": "Filter by title",
    value: queueState.q,
    onchange: (e) => {
      queueState.q = e.target.value;
      draw();
    },
  });
  const reason = async (t, verb, danger) => ask({ title: `${verb} this task?`, text: t.title, label: "Why", okLabel: verb, danger, required: true });
  return [
    card(h("div", { class: "row" }, pick, h("span", { class: "grow" }), search)),
    ...(data.tasks.length
      ? data.tasks.map((t) =>
          card(
            h("h3", {}, t.title),
            h("p", { class: "meta" }, [band(t.priority), t.updated ? `updated ${ago(t.updated)}` : ""].filter(Boolean).join(" · ")),
            t.why ? h("p", { class: "why" }, String(t.why)) : null,
            h(
              "div",
              { class: "row end" },
              t.status === "blocked"
                ? h(
                    "button",
                    {
                      class: "small",
                      onclick: async (e) => {
                        const why = await reason(t, "Unblock", false);
                        if (why) act("task.unblock", { task: t.id, reason: why }, "Unblocked", e.target);
                      },
                    },
                    "Unblock",
                  )
                : null,
              h(
                "button",
                {
                  class: "small ghost",
                  onclick: async (e) => {
                    const why = await reason(t, "Cancel", true);
                    if (why) act("task.cancel", { task: t.id, reason: why }, "Cancelled", e.target);
                  },
                },
                "Cancel",
              ),
            ),
          ),
        )
      : [empty("No tasks in this state.")]),
  ];
}

function viewPrs() {
  const prs = docs.ops?.prs;
  if (!prs) return [empty("Loading…")];
  const row = (p) => {
    const detail = h("div", {});
    return card(
      h("h3", {}, ext(p.url, `#${p.number}`), ` ${p.title}`),
      h("p", { class: "meta" }, `${p.draft ? "draft · " : ""}${p.author} · ${p.branch} · opened ${ago(p.created)}`),
      detail,
      h(
        "div",
        { class: "row end" },
        h(
          "button",
          {
            class: "small ghost",
            onclick: async (e) => {
              e.target.disabled = true;
              try {
                const d = await api(`prs/${p.number}`);
                detail.replaceChildren(h("p", { class: d?.mergeable === false ? "why" : "meta" }, prCheck(d)));
              } catch (error) {
                toast(error.message, true);
              }
              e.target.disabled = false;
            },
          },
          "Check",
        ),
        p.draft
          ? null
          : h(
              "button",
              {
                class: "small",
                onclick: async (e) => {
                  const yes = await ask({ title: `Merge #${p.number}?`, text: p.title, okLabel: "Merge" });
                  if (yes !== null) act("pr.merge", { pr: p.number }, `#${p.number} merged`, e.target);
                },
              },
              "Merge",
            ),
      ),
    );
  };
  const ready = prs.filter((p) => !p.draft);
  const drafts = prs.filter((p) => p.draft);
  return [
    h("h2", {}, `Ready (${ready.length})`),
    ...(ready.length ? ready.map(row) : [empty("No pull request is ready.")]),
    drafts.length ? h("h2", {}, `Drafts (${drafts.length})`) : null,
    ...drafts.map(row),
  ];
}

function viewIssues() {
  const list = docs.ops?.priorities;
  if (!list) return [empty("Loading…")];
  return [
    list.truncated ? empty("There are more ranked issues than this list reads. The most urgent may not all be here.") : null,
    ...(list.issues.length ? list.issues.map(issueCard) : [empty("No ranked issues.")]),
  ];
}

async function viewAgents() {
  const agents = docs.ops?.agents || [];
  const to = h("select", { "aria-label": "To" }, h("option", { value: "all" }, "Everyone"), agents.map((a) => h("option", { value: `agent:${a.name}` }, a.name)));
  const text = h("textarea", { rows: 3, placeholder: "A message, or a command to act on", "aria-label": "Message" });
  const command = h("input", { type: "checkbox", id: "cmd" });
  const compose = card(
    to,
    text,
    h(
      "div",
      { class: "row end" },
      h("label", { for: "cmd", class: "row check meta" }, command, "Send as a command"),
      h(
        "button",
        {
          onclick: (e) =>
            text.value.trim() ? act("message.send", { to: [to.value], text: text.value, command: command.checked }, "Sent", e.target) : toast("Write a message first", true),
        },
        "Send",
      ),
    ),
  );
  let recent = [];
  // The roster below is still useful without the log; but a log that
  // could not be read is not the same as one with nothing in it.
  let unread = "";
  try {
    const log = await api("messages?limit=15");
    if (Array.isArray(log)) recent = log;
  } catch (error) {
    unread = error.message;
  }
  return [
    compose,
    h("h2", {}, `Agents (${agents.length})`),
    ...agents.map((a) =>
      card(h("h3", {}, a.name), h("p", { class: "meta" }, `${(a.groups || []).join(", ")} · seen ${ago(a.last_seen) || "never"} · ${a.unread_commands ?? 0} commands and ${a.unread_messages ?? 0} messages unread`)),
    ),
    h("h2", {}, "Recent messages"),
    ...(recent.length
      ? recent.map((m) => card(h("p", { class: "meta" }, `${ago(m.ts)} · ${m.direction} · ${m.agent} · ${m.kind}`), h("p", {}, String(m.body ?? "").slice(0, 600))))
      : [empty(unread ? `The messages could not be read just now: ${unread}` : "No messages.")]),
  ];
}

function viewServices() {
  const units = docs.ops?.services;
  if (!units) return [empty("Loading…")];
  const row = (u) => {
    const log = h("div", {});
    const doit = (action, danger) =>
      h(
        "button",
        {
          class: `small${danger ? " ghost" : ""}`,
          onclick: async (e) => {
            const yes = await ask({ title: `${capital(action)} ${u.unit}?`, okLabel: capital(action), danger });
            if (yes !== null) act("unit.act", { unit: u.unit, action }, `${u.unit}: ${action} done`, e.target);
          },
        },
        capital(action),
      );
    return h(
      "div",
      { class: `card${u.active === "failed" ? " flag" : ""}` },
      h("div", { class: "row" }, h("h3", {}, u.unit), chip(`${u.active} · ${u.sub}`, u.active === "active" ? "good" : u.active === "failed" ? "warn" : "")),
      h("p", { class: "meta" }, u.description),
      log,
      h(
        "div",
        { class: "row end" },
        h(
          "button",
          {
            class: "small ghost",
            onclick: async (e) => {
              e.target.disabled = true;
              try {
                const d = await api(`services/${encodeURIComponent(u.unit)}/logs?lines=40`);
                log.replaceChildren(h("pre", {}, d.log || "(no log lines)"));
              } catch (error) {
                toast(error.message, true);
              }
              e.target.disabled = false;
            },
          },
          "Logs",
        ),
        u.active === "active" ? doit("restart") : doit("start"),
        u.active === "active" ? doit("stop", true) : null,
      ),
    );
  };
  const failed = units.filter((u) => u.active === "failed");
  const rest = units.filter((u) => u.active !== "failed");
  return [failed.length ? h("h2", {}, `Failed (${failed.length})`) : null, ...failed.map(row), h("h2", {}, `All units (${units.length})`), ...rest.map(row)];
}

// ---- shell -------------------------------------------------------------

const TABS = [
  ["now", "Now", "M3 12h4l3-8 4 16 3-8h4"],
  ["next", "Next", "M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01"],
  ["needs", "Needs me", "M12 8v5M12 16.5h.01M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z"],
  ["ask", "Ask", "M21 15a2 2 0 0 1-2 2H8l-5 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"],
  ["more", "More", "M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z"],
];
const VIEWS = {
  now: ["Now", viewNow],
  next: ["Next", viewNext, "ops"],
  needs: ["Needs me", viewNeeds, "questions"],
  ask: ["Ask", viewAsk],
  more: ["More", viewMore, "ops"],
  "more/queue": ["Queue", viewQueue],
  "more/prs": ["Pull requests", viewPrs, "ops"],
  "more/issues": ["Ranked issues", viewIssues, "ops"],
  "more/agents": ["Agents", viewAgents, "ops"],
  "more/services": ["Services", viewServices, "ops"],
  "more/notify": ["Notifications", viewNotify],
  "more/phone": ["Phones", viewPhone],
  "more/signin": ["Sign-in", viewSignin],
  "more/away": ["Laptop away", viewAway],
  "more/releases": ["Releases", viewReleases],
};
let route = "now";
let drawSeq = 0;
/** The screen whose content the view holds, once it has been drawn. */
let drawn = null;
/** A screen that takes longer than this to fetch says it is loading. */
const LOADING_AFTER_MS = 150;

function parse(hash) {
  const r = hash.replace(/^#/, "");
  if (VIEWS[r]) return [r, undefined];
  // The Android app asking for its phone to be linked: #more/link=<request>.
  if (r.startsWith("more/link=")) return ["more/phone", r.slice(5)];
  const [top, sub] = r.split("/");
  return VIEWS[top] ? [top, sub] : ["now", undefined];
}

function go(to) {
  if (location.hash === `#${to}`) draw();
  else location.hash = to;
}

/** A card's title is a third-level heading, under a section's
 * second-level one. On a screen with no section above it, it comes
 * straight after the screen's title: it is then said to be second-level,
 * so that someone listening does not hear a level skipped. It looks the
 * same. `headings` are the screen's h2 and h3 elements, in order. */
export function levelHeadings(headings) {
  let sectioned = false;
  for (const el of headings) {
    if (el.tagName === "H2") sectioned = true;
    else if (!sectioned) el.setAttribute("aria-level", "2");
  }
}

function drawTabs() {
  const n = needCounts();
  const fresh = (docs.core?.asks || []).filter((a) => a.status === "answered").length;
  tabsEl.replaceChildren(
    ...TABS.map(([name, label, d]) => {
      const count = name === "needs" ? n.total : name === "ask" ? fresh : 0;
      return h(
        "button",
        { "aria-current": route.split("/")[0] === name ? "page" : null, onclick: () => go(name) },
        icon(d),
        label,
        count ? h("span", { class: "badge" }, count > 99 ? "99+" : String(count)) : null,
      );
    }),
  );
}

function drawFresh() {
  const t = asOf.core;
  const age = t ? (Date.now() - t) / 1000 : null;
  freshEl.className = `fresh${signedOut ? " down" : age !== null && age > 180 ? " stale" : ""}`;
  freshEl.textContent = signedOut ? "signed out" : age === null ? "" : age < 45 ? "live" : `as of ${ago(new Date(t).toISOString())}`;
}

/** How many quick looks have been taken since the state was last fresh. */
let catchUp = 0;

/** Draw the current section from what is already loaded. Only the latest
 * call may draw, so a slow section never lands on top of a newer one. */
async function draw() {
  const seq = ++drawSeq;
  const [name, sub] = parse(location.hash);
  route = sub ? `${name}/${sub}` : name;
  const [title, render, extra] = VIEWS[name];
  $("title").textContent = title;
  $("back").hidden = !name.includes("/");
  drawTabs();
  drawFresh();
  if (!docs.core) return view.replaceChildren(empty("Loading…"));
  if (extra && !docs[extra]) loadDoc(extra).then(() => seq === drawSeq && draw()).catch(() => {});
  // Moving to another screen: what the last one showed must not sit
  // under this one's name while this one is fetched. A screen that has
  // everything it needs is drawn before this fires, so nothing flickers;
  // a screen being drawn again keeps what it shows until the new is in.
  const moved = drawn !== route;
  const waiting = moved
    ? setTimeout(() => {
        if (seq !== drawSeq) return;
        view.replaceChildren(empty(`Loading ${title}…`));
        scrollTo(0, 0);
      }, LOADING_AFTER_MS)
    : null;
  try {
    const nodes = await render(sub);
    if (seq !== drawSeq) return;
    view.replaceChildren(...nodes.filter(Boolean));
    levelHeadings(view.querySelectorAll("h2, h3"));
    if (moved) scrollTo(0, 0);
    drawn = route;
  } catch (error) {
    if (seq !== drawSeq) return;
    view.replaceChildren(
      card(h("h3", {}, "This section could not load"), h("p", { class: "why" }, error.message), h("div", { class: "row end" }, h("button", { onclick: () => (signedOut ? location.reload() : draw()) }, signedOut ? "Reload" : "Try again"))),
    );
    levelHeadings(view.querySelectorAll("h2, h3"));
    if (moved) scrollTo(0, 0);
    drawn = route;
  } finally {
    if (waiting) clearTimeout(waiting);
  }
}

/** Read the documents again; draw only if something changed, or if asked. */
async function refresh(force) {
  const [name] = parse(location.hash);
  const extra = VIEWS[name][2];
  let changed = false;
  try {
    changed = await loadDoc("core");
    if (extra) changed = (await loadDoc(extra)) || changed;
    // An old copy means the laptop has just been told someone is
    // looking: look again shortly, a few times, instead of waiting out
    // the full interval.
    const old = asOf.core && Date.now() - asOf.core > 60_000;
    if (!old) catchUp = 0;
    else if (catchUp < 4) {
      catchUp++;
      setTimeout(() => document.visibilityState === "visible" && refresh(false), 4_000);
    }
  } catch (error) {
    if (!docs.core) view.replaceChildren(card(h("h3", {}, "Could not load"), h("p", { class: "why" }, error.message), h("div", { class: "row end" }, h("button", { onclick: () => location.reload() }, "Reload"))));
  }
  // An open dialog or a half-typed answer is not redrawn from under the reader.
  const busy = $("ask").open || pending.size > 0 || (document.activeElement && ["TEXTAREA", "INPUT", "SELECT"].includes(document.activeElement.tagName));
  if (force || (changed && !busy)) draw();
  else {
    drawTabs();
    drawFresh();
  }
}

if (typeof document !== "undefined" && view) {
  $("refresh").addEventListener("click", () => refresh(true));
  $("back").addEventListener("click", () => go(route.split("/")[0]));
  addEventListener("hashchange", () => {
    shown = 12;
    draw();
  });
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") flushPending();
    else {
      refresh(false);
      // Back from the Android app after linking: show the phone it added.
      if (route === "more/phone") draw();
    }
  });
  addEventListener("pagehide", flushPending);
  setInterval(() => document.visibilityState === "visible" && refresh(false), 20_000);
  setInterval(drawFresh, 10_000);
  api("me")
    .then((m) => {
      me = m;
    })
    .catch(() => {});
  // The worker shows notifications and opens the right screen when one is tapped.
  if ("serviceWorker" in navigator) navigator.serviceWorker.register("/console/sw.js", { scope: "/console/" }).catch(() => {});
  // And a device that has notifications on keeps getting them when the
  // sender changed or lost track of it.
  keepSubscribed().catch(() => {});
  draw();
  refresh(true);
}
