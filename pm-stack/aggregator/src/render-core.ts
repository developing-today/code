// render.ts split (#13051): shared primitives: escaping/formatting, page layout and nav, comment and action forms, session helpers, coverage strip, item briefs.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import { executionStrip } from "./derive.ts";
import { latestInFamily, newClientToken, type DashAction } from "./actions.ts";
import { cardState, pendingVerb, type CardState } from "./card-state.ts";
import {
  DELEGATION_DEFAULT_OBJECT_REF,
  effectiveStandingAuto,
  proposalExpired,
  proposalIdentity,
  questionIdForOption,
} from "./tier-items.ts";
import type { CommentKindCounts } from "./comments.ts";
import { ageMsText } from "./feed.ts";
import {
  type BoardItem,
  type DashAuthor,
  type DashComment,
  type DelegationMode,
  type GhItem,
  type Provenance,
  type ProvenanceDetail,
  type RefKey,
  type SessionRecord,
  type SourceCoverage,
  type TierItem,
  type TierItemQuestion,
  type WhyUserClass,
  isValidRef,
  refUrl,
} from "./model.ts";
import { groupedPages, primaryGroupedPages, manifestRouteFor, pageByRoute } from "./pages.ts";
import { KEY_HINT, KEYBOARD_SCRIPT, paletteHtml } from "./keyboard.ts";
import { CANONICAL_STATUSES } from "./status-taxonomy.ts";
import type { ItemBrief } from "./brief.ts";
import type { AuthorIdentity } from "./registry.ts";
import type { SearchResult } from "./search-view.ts";

export function esc(s: unknown): string {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function fmtDate(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d
    .toISOString()
    .replace("T", " ")
    .replace(/\.\d+Z$/, "Z");
}

/** Central-time stamp for Drew-facing quota/reset surfaces (#9553):
 * "Sat, Nov 14, 8:00 AM CST". fmtDate above renders raw UTC ISO; the
 * /quota page (pool resets, wall lifts, freshness, burn windows) must
 * never show UTC to Drew, so renderQuota uses this formatter. */
const centralFmt = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/Chicago",
  weekday: "short",
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  timeZoneName: "short",
});

export function fmtCentral(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return centralFmt.format(d);
}

/* The stylesheet lives in src/ui.css (Tailwind v4 + DaisyUI v5 plus the
   legacy component rules below it) and is built to the committed static
   asset src/assets/app.css by `bun run build:css` — see the README's
   "Styling" section. Do not inline page CSS here. */

/** Canonical tailnet URL for this dashboard (MagicDNS — never the raw IP). */
export const TAILNET_URL = "http://amd-6.raptor-pumpkinseed.ts.net:4870";

/**
 * Shared post-submit feedback (#1443): the click must visibly land.
 * The card's controls disable, a "✓ sent · pending" line appears in
 * place, and a suppressing verb (snooze / dismiss / mark-done)
 * tombstones the card immediately — the server render (via the
 * Datastar fragment refresh this dispatches, or the next page load)
 * then moves it to its strip for real. Pages without Datastar
 * fragment containers keep the historical full reload.
 */
export const DASH_AFTER_JS = `
window.__dashAfter = window.__dashAfter || function (form, msg) {
  var card = form.closest(".card");
  form.querySelectorAll("button, input, select, textarea").forEach(function (b) { b.disabled = true; });
  if (card) {
    var line = document.createElement("div");
    line.className = "sentline";
    line.textContent = msg;
    card.appendChild(line);
    var verb = form.dataset.verb;
    if (verb === "snooze" || verb === "dismiss" || verb === "mark-done") card.classList.add("tombstone");
  }
  window.dispatchEvent(new CustomEvent("dashrefresh"));
  if (!document.querySelector("[data-frag]")) location.reload();
};
window.__dashAuthorPickers = window.__dashAuthorPickers || function () {
  var fill = function (list) {
    document.querySelectorAll("select[name=author][data-author-picker]").forEach(function (s) {
      if (s.dataset.hydrated) return;
      var cur = s.value;
      var frag = document.createDocumentFragment();
      list.forEach(function (i) {
        var o = document.createElement("option");
        o.value = i.kind + "|" + i.id + "|" + (i.sessionId || "");
        o.textContent = i.label + (i.kind === "agent" ? " (agent)" : "");
        if (i.id === "drew") o.defaultSelected = true;
        frag.appendChild(o);
      });
      s.replaceChildren(frag);
      s.value = cur;
      s.dataset.hydrated = "1";
    });
  };
  if (window.__dashIdentities) { fill(window.__dashIdentities); return; }
  if (window.__dashIdentitiesPromise) return;
  window.__dashIdentitiesPromise = fetch("/api/author-identities")
    .then(function (r) { return r.json(); })
    .then(function (d) { window.__dashIdentities = (d && d.identities) || []; fill(window.__dashIdentities); })
    .catch(function () { window.__dashIdentitiesPromise = null; });
};
if (!window.__dashAuthorPickerHook) {
  window.__dashAuthorPickerHook = true;
  document.addEventListener("pointerdown", function (ev) {
    var t = ev.target;
    if (t && t.closest && t.closest("select[name=author][data-author-picker], .actbox, .cbox")) window.__dashAuthorPickers();
  });
  document.addEventListener("focusin", function (ev) {
    var t = ev.target;
    if (t && t.matches && t.matches("select[name=author][data-author-picker]")) window.__dashAuthorPickers();
  });
}
`;

/**
 * The client side of the comment form (one delegated submit handler;
 * pages without a form never notice it). Kept dependency-free and
 * inline, like everything else in this app. The card comment box
 * (#1446) posts kind "message" for the orchestrator and — when its
 * "also post publicly" box is ticked and the card carries a gh: ref
 * in data-ghref — a second, kind "github" comment the orchestrator
 * posts to the issue (the aggregator holds no GitHub credentials).
 */
export const COMMENT_SCRIPT = `<script>${DASH_AFTER_JS}
const commentOperatorTokenKey = "pm-aggregator.operator-token";
const savedCommentOperatorToken = () => {
  try { return localStorage.getItem(commentOperatorTokenKey) || ""; } catch { return ""; }
};
document.addEventListener("submit", async (ev) => {
  const form = ev.target;
  if (!(form instanceof HTMLFormElement) || !form.classList.contains("cform")) return;
  ev.preventDefault();
  const err = form.querySelector(".err");
  const data = new FormData(form);
  let operatorToken = String(data.get("operatorToken") || "").trim();
  if (operatorToken) {
    try { localStorage.setItem(commentOperatorTokenKey, operatorToken); } catch { /* send this request even if storage is unavailable */ }
  } else {
    operatorToken = savedCommentOperatorToken();
  }
  const kind = String(data.get("kind") || "note");
  const [akind, aid, asession] = String(data.get("author") || "drew|drew|").split("|");
  const labelEl = form.querySelector('option[value="' + CSS.escape(String(data.get("author"))) + '"]');
  const author = { kind: akind, id: aid, label: labelEl ? labelEl.textContent : aid, ...(asession ? { sessionId: asession } : {}) };
  const payload = {
    objectRef: form.dataset.object,
    kind,
    author,
    body: String(data.get("body") || ""),
    ...(kind === "github" ? { githubRef: String(data.get("githubRef") || "") } : {}),
  };
  const post = async (p) => {
    const headers = { "content-type": "application/json", ...(operatorToken ? { authorization: "Bearer " + operatorToken } : {}) };
    const res = await fetch("/api/comments", { method: "POST", headers, body: JSON.stringify(p) });
    if (!res.ok) { const b = await res.json().catch(() => ({})); throw new Error(b.error || ("HTTP " + res.status)); }
  };
  try {
    await post(payload);
    const also = form.querySelector("input[name=alsoGithub]");
    if (also && also.checked && form.dataset.ghref) {
      await post({ objectRef: form.dataset.object, kind: "github", author, body: payload.body, githubRef: form.dataset.ghref });
    }
    window.__dashAfter(form, "✓ sent · pending — the orchestrator picks it up on its next pass");
  } catch (e) { if (err) err.textContent = "Not posted: " + e.message; }
});
document.addEventListener("click", (ev) => {
  const button = ev.target;
  if (!(button instanceof HTMLButtonElement) || !button.matches("[data-forget-operator-token]")) return;
  try { localStorage.removeItem(commentOperatorTokenKey); } catch { /* storage may be disabled */ }
  const input = button.closest(".cform")?.querySelector('input[name="operatorToken"]');
  if (input instanceof HTMLInputElement) input.value = "";
});
document.addEventListener("change", (ev) => {
  const t = ev.target;
  if (!(t instanceof HTMLInputElement) || t.name !== "kind") return;
  const form = t.closest(".cform");
  const wrap = form && form.querySelector(".ghtarget");
  if (wrap) wrap.style.display = t.value === "github" && t.checked ? "" : "none";
});
</script>`;

export const ROADMAP_ANNOTATION_SCRIPT = `<script>${DASH_AFTER_JS}
document.addEventListener("submit", async (ev) => {
  const form = ev.target;
  if (!(form instanceof HTMLFormElement) || !form.classList.contains("raform")) return;
  ev.preventDefault();
  const err = form.querySelector(".err");
  const data = new FormData(form);
  const raw = String(data.get("author") || "drew|drew|");
  const [kind, id, sessionId] = raw.split("|");
  const option = form.querySelector('option[value="' + CSS.escape(raw) + '"]');
  const payload = {
    objectRef: form.dataset.object,
    kind: String(data.get("kind") || "context"),
    date: String(data.get("date") || ""),
    author: { kind, id, label: option ? option.textContent : id, ...(sessionId ? { sessionId } : {}) },
    body: String(data.get("body") || "")
  };
  const button = form.querySelector("button[type=submit]");
  if (button) button.disabled = true;
  try {
    const res = await fetch("/api/roadmap/annotations", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(payload) });
    if (!res.ok) { const body = await res.json().catch(() => ({})); throw new Error(body.error || ("HTTP " + res.status)); }
    window.location.reload();
  } catch (e) {
    if (err) err.textContent = "Not saved: " + e.message;
    if (button) button.disabled = false;
  }
});
</script>`;

/**
 * The client side of the dashboard action forms (#1152/#1154/#1156):
 * one delegated submit handler, exactly the COMMENT_SCRIPT pattern —
 * POST /api/actions, reload on success, inline error on failure. The
 * form's data-* attributes carry the envelope's fixed fields; named
 * inputs carry the payload. A stop form must have its confirm box
 * ticked before the script will send it (two-step confirm).
 */
/** Shell fragment watchdog (#1503): the shell pages stream their real
 * content in as Datastar fragments; if a fragment request fails or stalls
 * there is no error path in the markup, so the page would sit on its
 * skeleton forever. Ten seconds after the shell renders, any [data-frag]
 * container still holding its .skel skeleton gets the skeleton swapped for
 * an inline notice linking the ?full=1 server render (the no-JS path).
 * A fragment that lands in time removes the skeleton itself, so the
 * watchdog no-ops; a fragment that lands late replaces the notice. */
export const FRAG_WATCHDOG_MS = 10_000;
export const FRAG_WATCHDOG_SCRIPT = `<script>
setTimeout(function () {
  document.querySelectorAll("[data-frag]").forEach(function (c) {
    var skel = c.querySelector(".skel");
    if (!skel) return;
    var p = document.createElement("p");
    p.className = "frag-failed";
    p.textContent = "the live view did not arrive — ";
    var a = document.createElement("a");
    a.setAttribute("href", "?full=1");
    a.textContent = "load the full page";
    p.appendChild(a);
    skel.replaceWith(p);
  });
}, ${FRAG_WATCHDOG_MS});
</script>`;

export const ACTION_SCRIPT = `<script>${DASH_AFTER_JS}
document.addEventListener("submit", async (ev) => {
  const form = ev.target;
  if (!(form instanceof HTMLFormElement) || !form.classList.contains("aform")) return;
  ev.preventDefault();
  const err = form.querySelector(".err");
  const data = new FormData(form);
  if (form.dataset.verb === "stop" && !data.get("confirm")) {
    if (err) err.textContent = "Not sent: tick Confirm stop first — stopping ends the session's run.";
    return;
  }
  const submitBtn = form.querySelector("button[type=submit]");
  if (submitBtn) submitBtn.disabled = true;
  const box = form.closest(".actbox");
  const authorRaw = String((box && box.querySelector("select[name=author]") || {}).value || "drew|drew|");
  const [akind, aid, asession] = authorRaw.split("|");
  const labelEl = box ? box.querySelector('option[value="' + CSS.escape(authorRaw) + '"]') : null;
  const payload = {};
  for (const name of ["text", "reason", "status", "tier", "mode", "optionId", "proposalRunId", "proposalUpdatedAt", "needsOrder"]) {
    const v = data.get(name);
    if (v) payload[name] = String(v);
  }
  const standingAuto = data.get("standingAuto");
  if (standingAuto) payload.standingAuto = standingAuto === "true";
  const answersByQuestion = {};
  for (const el of form.querySelectorAll("[name^='answers:']")) {
    if (el.disabled) continue;
    const qid = el.name.slice("answers:".length);
    if (el.type === "checkbox") { if (el.checked) (answersByQuestion[qid] = answersByQuestion[qid] || []).push(el.value); }
    else if (el.type === "radio") { if (el.checked) answersByQuestion[qid] = el.value; }
    else if (el.value) answersByQuestion[qid] = el.value;
  }
  const answerList = Object.keys(answersByQuestion).map((questionId) => ({ questionId, value: answersByQuestion[questionId] }));
  if (answerList.length) payload.answers = answerList;
  const untilCustom = data.get("untilCustom");
  const untilPreset = data.get("untilPreset");
  if (untilCustom) payload.until = new Date(String(untilCustom)).toISOString();
  else if (untilPreset) payload.until = String(untilPreset);
  const body = {
    clientToken: form.dataset.token,
    kind: form.dataset.kind,
    verb: form.dataset.verb,
    objectRef: form.dataset.object,
    author: { kind: akind, id: aid, label: labelEl ? labelEl.textContent : aid, ...(asession ? { sessionId: asession } : {}) },
    payload,
  };
  if (form.dataset.targetRef) body.targetRef = form.dataset.targetRef;
  if (form.dataset.sessionId) body.sessionId = form.dataset.sessionId;
  if (form.dataset.sessionSource) body.sessionSource = form.dataset.sessionSource;
  if (form.dataset.supersedes) body.supersedes = form.dataset.supersedes;
  try {
    const res = await fetch("/api/actions", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    if (!res.ok) {
      const b = await res.json().catch(() => ({}));
      if (typeof b.duplicateOf === "string") {
        if (submitBtn) submitBtn.disabled = false;
        // The rejected attempt is persisted under this form's token; a
        // corrected resubmit (e.g. dropping the already-answered
        // question) is a NEW attempt and needs a fresh token, or the
        // store's client-token dedupe would replay the rejected record
        // as if it had been sent (Codex P2 on PR #4024).
        form.dataset.token = "ct-" + Date.now().toString(36) + "-" + Math.random().toString(36).slice(2, 10) + Math.random().toString(36).slice(2, 10);
        if (err) err.textContent = "Duplicate recorded" + (typeof b.action?.id === "string" ? " as " + b.action.id : "") + "; points to existing answer " + b.duplicateOf + ".";
        return;
      }
      throw new Error(b.error || ("HTTP " + res.status));
    }
    window.__dashAfter(form, "✓ sent · pending — the fleet applies it on its next pass");
  } catch (e) {
    if (submitBtn) submitBtn.disabled = false;
    if (err) err.textContent = "Not recorded: " + e.message;
  }
});
</script>`;

/** Facet labels for the freshness chips (#1138) — the footer nouns. */
const FACET_LABELS: Record<keyof Provenance, string> = {
  board: "board",
  roadmap: "roadmap",
  items: "items",
  comments: "comments",
  quota: "quota",
  usage: "usage",
  attention: "attention",
  orchestration: "orchestration",
  dns: "dns",
  registry: "registry",
  aa: "aa",
};

/**
 * Per-source freshness chips (#1138): only the sources the page being
 * rendered actually uses, each with its state word, its age when the
 * source carries one, and any count WITH its noun — a chip count and
 * a page header counting the same thing read the same variable, so
 * the two can never disagree unexplained. The facet's full provenance
 * string rides in the title attribute for whoever is debugging.
 */
export function freshnessChips(detail: ProvenanceDetail, keys: (keyof Provenance)[]): string {
  const chips = keys
    .map((key) => {
      const facet = detail[key];
      const count = facet.count !== undefined ? ` · ${facet.count} ${esc(facet.countLabel ?? "")}`.trimEnd() : "";
      const age = facet.ageMs !== undefined && facet.state !== "fixtures" ? ` · ${ageMsText(facet.ageMs)}` : "";
      return `<span class="fchip ${facet.state}" title="${esc(facet.text)}">${FACET_LABELS[key]} · ${facet.state}${count}${age}</span>`;
    })
    .join("");
  return `<div class="chips fresh" aria-label="Source freshness">${chips}</div>`;
}

/**
 * The grouped nav links (#1135, #1215, #1497): one labelled section
 * per PAGE_GROUP, in bar order, marking the active page's section.
 * Since the #1497 curation the bar renders the PRIMARY-tier pages
 * only; the Pages dropdown keeps every manifest page under the same
 * sections, so demoted pages stay one click away.
 */
export function navLinks(active: string, activeGroup?: string): string {
  // A sub-route (/program/<id>, /quota/service/<id>) marks the page that
  // reaches it, so the bar always shows where you are (#1135).
  const activeRoute = manifestRouteFor(active) ?? active;
  const groupOfActive = activeGroup ?? pageByRoute(activeRoute)?.group;
  let activeMarked = false;
  return primaryGroupedPages()
    .map(({ group, pages }) => {
      const anchors = pages
        .map((p) => {
          const isActive = p.route === activeRoute && !activeMarked;
          if (isActive) activeMarked = true;
          return `<a href="${p.route}"${isActive ? ' class="active" aria-current="page"' : ""}>${esc(p.title)}</a>`;
        })
        .join("");
      const current = group.id === groupOfActive ? ' data-current="true"' : "";
      return `<span class="navgroup"${current}><span class="navlabel">${esc(group.title)}</span>${anchors}</span>`;
    })
    .join("");
}

/**
 * The Pages dropdown (#906 §1, #1215, #1497): the same grouped model
 * as the bar — one <optgroup> per manifest section — but EVERY page
 * present, primary and dropdown-tier alike: this is where demoted
 * pages live.
 */
export function pageDropdown(active: string): string {
  let selectedMarked = false;
  const groups = groupedPages()
    .map(({ group, pages }) => {
      const options = pages
        .map((p) => {
          const selected = p.route === active && !selectedMarked;
          if (selected) selectedMarked = true;
          return `<option value="${p.route}"${selected ? " selected" : ""}>${esc(p.title)} — ${esc(p.blurb)}</option>`;
        })
        .join("");
      return `<optgroup label="${esc(group.title)}">${options}</optgroup>`;
    })
    .join("");
  return `<select aria-label="Go to page" class="select select-bordered select-sm" data-on:change="if (evt.target.value) { window.location.href = evt.target.value; }"><option value="">Pages ▾</option>${groups}</select>`;
}

export function themeToggle(): string {
  // Datastar-first theme toggle (Phase C): the vendored Datastar v1 binds
  // data-on:<event> (colon form) for handlers — the dash form does not bind
  // clicks (verified live against the vendored bundle). A $pmTheme signal seeded from
  // the <html data-theme> the inline bootstrap set (stored choice or the
  // dark default); the effect is the single writer of data-theme +
  // localStorage, and the click handler only flips the signal.
  return `<span class="theme-toggle-wrap" data-signals="{ pmTheme: 'dark' }" data-init="$pmTheme = document.documentElement.getAttribute('data-theme') || 'dark'" data-effect="document.documentElement.setAttribute('data-theme', $pmTheme); try { localStorage.setItem('pm-theme', $pmTheme); } catch (e) {}"><button type="button" class="btn btn-ghost btn-sm theme-toggle" data-on:click="$pmTheme = $pmTheme === 'dark' ? 'light' : 'dark'" aria-label="Toggle between dark and light theme" title="Toggle dark / light theme">◐ <span data-text="$pmTheme === 'dark' ? 'Dark' : 'Light'"></span></button></span>`;
}

function globalSearch(): string {
  return `<form class="navsearch" action="/search" method="get" role="search"><input type="search" name="q" aria-label="Search board and roadmap work" placeholder="Search work…" required class="input input-bordered input-sm"><button type="submit" class="btn btn-sm">Search</button></form>`;
}

export function layout(
  title: string,
  body: string,
  provenance: Provenance,
  active: string,
  extra = "",
  chipsHtml = "",
): string {
  // Nav + the page dropdown both render from the manifest (#906 §1):
  // registering a page in src/pages.ts is the only step that puts it
  // in either. Both use the same grouped model (#1215): the bar groups
  // its links by section (#1135) so pages wrap onto a second row at
  // phone width instead of scrolling sideways, the dropdown mirrors
  // the sections as <optgroup>s, and both list every page.
  const links = navLinks(active);
  const dropdown = pageDropdown(active);
  return `<!doctype html>
<html lang="en" data-theme="dark"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)} · aggregator</title><link rel="manifest" href="/manifest.webmanifest"><meta name="theme-color" content="#161b22"><meta name="mobile-web-app-capable" content="yes"><meta name="apple-mobile-web-app-capable" content="yes"><meta name="apple-mobile-web-app-status-bar-style" content="black-translucent"><meta name="apple-mobile-web-app-title" content="PM Stack"><link rel="apple-touch-icon" href="/icons/apple-touch-icon.png"><script>try{var __pmTheme=localStorage.getItem('pm-theme');if(__pmTheme==='light'||__pmTheme==='dark')document.documentElement.setAttribute('data-theme',__pmTheme);}catch(e){}</script><link rel="stylesheet" href="/assets/app.css"><script type="module" src="/assets/datastar.js"></script></head>
<body><nav class="navbar bg-base-200 shadow-md flex-wrap items-center gap-x-3 px-4">${links}${dropdown}${globalSearch()}${themeToggle()}</nav><main class="w-full">${chipsHtml}${body}
<div class="foot"><details class="srcdetail"><summary>source detail</summary>board: ${esc(provenance.board)} · roadmap: ${esc(provenance.roadmap)} · items: ${esc(provenance.items)} · comments: ${esc(provenance.comments)} · quota: ${esc(provenance.quota)} · usage: ${esc(provenance.usage)} · attention: ${esc(provenance.attention)} · orchestration: ${esc(provenance.orchestration)} · dns: ${esc(provenance.dns)} · registry: ${esc(provenance.registry)} · aa: ${esc(provenance.aa)}</details><span class="kbd-hint">${KEY_HINT}</span> · tailnet: <a href="${TAILNET_URL}">${TAILNET_URL.replace("http://", "")}</a></div>
</main><script src="/assets/push-client.js?v=push-ux-2026-11-01" defer></script><script src="/assets/freshness-client.js?v=freshness-3976" defer></script>${extra}${paletteHtml()}${KEYBOARD_SCRIPT}</body></html>`;
}

/** Global search over board/roadmap subjects, linked refs, and the slice-2 sources (#1212): registry sessions, the DNS register, services, and tier items. */
export function renderSearch(
  query: string,
  results: SearchResult[],
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  detail?: ProvenanceDetail,
): string {
  const cards = results
    .map((result) => {
      const badges = result.refs
        .map((ref) => {
          const item = items.get(ref);
          // Codex P1 on PR #7331 (defense in depth): a non-GitHub ref
          // (tracked-goal id, orch: key) must never reach refUrl, which
          // throws on it — render it as a plain-text badge instead.
          return item
            ? itemBadge(item)
            : isValidRef(ref)
              ? `<a class="badge closed" href="${esc(refUrl(ref))}" title="Unresolved by the current item source">${esc(ref)} · unresolved</a>`
              : `<span class="badge closed">${esc(ref)} · unresolved</span>`;
        })
        .join(" ");
      return `<div class="card"><a href="${esc(result.href)}"><strong>${esc(result.title)}</strong></a><br><span class="muted">${result.kind} · ${esc(result.summary)}</span>${badges ? `<div class="strip">${badges}</div>` : ""}</div>`;
    })
    .join("");
  const body = `<h1>Search</h1><p class="muted">Search board items, roadmap rows, registry sessions, DNS domains, services, and tier items by subject, title, or linked issue/PR ref. Every result links to its own detail view. Linked GitHub badges show their current projected state and whether the result is fixture-only.</p><form class="search-page-form" action="/search" method="get" role="search"><label for="search-query">Search work</label><input id="search-query" type="search" name="q" value="${esc(query)}" placeholder="Title or owner/repo#123"><button type="submit" class="btn btn-sm">Search</button></form>${query ? `<h2>${results.length} result${results.length === 1 ? "" : "s"} for “${esc(query)}”</h2>${cards || '<p class="muted">No matching work found.</p>'}` : '<p class="muted">Enter a title or issue/PR reference to search.</p>'}`;
  // #1212 slice 2: the search page carries the same freshness facets
  // as the home page of each result kind present (board/roadmap/items
  // for work, registry for sessions, dns for the register, quota and
  // usage for services, orchestration/attention/registry for tiers).
  const chipKeys = new Set<keyof Provenance>(["board", "roadmap", "items"]);
  for (const result of results) {
    if (result.kind === "registry") chipKeys.add("registry");
    if (result.kind === "dns") chipKeys.add("dns");
    if (result.kind === "service") {
      chipKeys.add("quota");
      chipKeys.add("usage");
    }
    if (result.kind === "tier") {
      chipKeys.add("orchestration");
      chipKeys.add("attention");
      chipKeys.add("registry");
    }
  }
  return layout("Search", body, provenance, "/search", "", detail ? freshnessChips(detail, [...chipKeys]) : "");
}

// --- Dashboard comments (#906 §2) ------------------------------------------

/** Object refs: the keys dashboard comment threads hang on. */
export const sessionObjectRef = (id: string): string => `session:${id}`;
export const ghObjectRef = (ref: RefKey): string => `gh:${ref}`;
export function tierItemObjectRef(item: TierItem): string {
  // Watchdog items (#1148) all carry the implementation issue as their
  // display ref, but each watchdog:<input> is an independent object:
  // keying them all by the shared gh: ref would give every watchdog
  // card the same DOM id, actions, dismissal state, and comment
  // thread, so dismissing one input's card would suppress them all
  // (Codex P1 on PR #2431). Watchdog cards key by their unique id;
  // the gh: ref still renders as the card's issue link.
  if (item.source === "watchdog") return `orch:${item.id}`;
  return item.ref ? ghObjectRef(item.ref) : `orch:${item.id}`;
}

/** A 💬 link to an object's comment thread, with its comment count. */
export function commentBadge(objectRef: string, counts?: Map<string, number | CommentKindCounts>): string {
  // #1210: a detailed count distinguishes notes from actionable
  // comments (awaiting triage / to post / failed) in the badge itself;
  // a plain number count keeps the original rendering.
  const v = counts?.get(objectRef) as number | CommentKindCounts | undefined;
  const n = typeof v === "number" ? v : (v?.total ?? 0);
  const actionable = typeof v === "object" && v !== null ? v.actionable : 0;
  const title =
    typeof v === "object" && v !== null
      ? `Dashboard comments on this item: ${v.total} total — ${v.notes} notes, ${v.messagesNew} messages awaiting triage, ${v.githubPending} GitHub comments to post, ${v.githubFailed} failed to post`
      : "Dashboard comments on this item";
  const queued = actionable ? ` · ${actionable} queued` : "";
  return ` <a class="badge" href="/discuss/${encodeURIComponent(objectRef)}" title="${esc(title)}">💬${n ? ` ${n}` : ""}${queued}</a>`;
}

export function authorBadge(a: DashAuthor, linkedIdentity?: { source: string; id: string }): string {
  const cls = a.kind === "drew" ? "drew" : a.kind === "muse" ? "Muse" : "agent";
  const link = a.sessionId
    ? ` <a href="/sessions?id=${encodeURIComponent(a.sessionId)}#${encodeURIComponent(a.sessionId)}" class="muted">session</a>`
    : "";
  const linked = linkedIdentity
    ? ` <a href="/sessions?id=${encodeURIComponent(linkedIdentity.id)}#${encodeURIComponent(linkedIdentity.id)}" class="muted" title="Owner-confirmed display link">linked identity</a>`
    : "";
  return `<span class="badge ${cls}">${esc(a.label)}</span>${link}${linked}`;
}

export function dashCommentHtml(c: DashComment, showObject = false): string {
  const verificationChip =
    c.provenance === "verified"
      ? `<span class="badge">verified identity</span>`
      : `<span class="badge">asserted-legacy — author is a claim</span>`;
  const kindChip =
    c.kind === "message"
      ? `<span class="badge msg">✉ message to Muse</span> <span class="muted">${c.triageState === "triaged" ? "triaged by the orchestrator" : "awaiting triage"}</span>`
      : c.kind === "github"
        ? `<span class="badge gh">GitHub comment</span> <span class="muted">${
            c.githubState === "posted"
              ? `posted${c.githubUrl ? ` — <a href="${esc(c.githubUrl)}">view</a>` : ""}`
              : c.githubState === "failed"
                ? `posting failed ${relTime(c.githubFailedAt ?? c.createdAt)} — no automatic retry exists; a fleet poster must post it by hand${c.githubError ? ` — last error: ${esc(c.githubError)}` : ""}`
                : `pending — will post to ${esc(c.githubRef ?? "")}`
          }</span>`
        : `<span class="badge">note</span>`;
  const obj = showObject ? ` <span class="muted">on <code>${esc(c.objectRef)}</code></span>` : "";
  return `<div class="cmt ${esc(c.kind)}">${authorBadge(c.author, c.linkedIdentity)} ${verificationChip} ${kindChip}${obj}<br>${esc(c.body)}<br><span class="muted">${fmtDate(c.createdAt)}</span></div>`;
}

export interface CommentFormOpts {
  /** Refs the "post as GitHub comment" kind may target (explicit select). */
  githubRefs?: RefKey[];
  /** Author identities for the picker (standing + registry sessions). */
  identities: AuthorIdentity[];
}

/**
 * The author picker's server-rendered options (#1179): the standing
 * identities only. Session identities (one per registry session --
 * ~700 and growing) used to be inlined into every card's picker,
 * which made list pages quadratic: /program shipped 351 copies of a
 * ~123 KB option list (44 MB of HTML), /board 94 copies (12.9 MB).
 * The full list now loads on demand from /api/author-identities the
 * first time any picker on the page is touched and is applied to
 * every picker at once (see DASH_AFTER_JS). Pickers keep working
 * without that fetch -- standing identities are the common case.
 */
export function authorOptionsHtml(identities: AuthorIdentity[]): string {
  return identities
    .filter((i) => !i.sessionId)
    .map(
      (i) =>
        `<option value="${esc(`${i.kind}|${i.id}|${i.sessionId ?? ""}`)}"${i.id === "drew" ? " selected" : ""}>${esc(i.label)}${i.kind === "agent" ? " (agent)" : ""}</option>`,
    )
    .join("");
}

export function commentFormHtml(objectRef: string, opts: CommentFormOpts): string {
  const ghRefs = opts.githubRefs ?? [];
  const authorOptions = authorOptionsHtml(opts.identities);
  const ghTarget = ghRefs.length
    ? `<div class="ghtarget" style="display:none"><label>Post to GitHub issue / PR (explicit target)</label><select name="githubRef">${ghRefs.map((r) => `<option value="${esc(r)}">${esc(r)}</option>`).join("")}</select></div>`
    : "";
  const ghKind = ghRefs.length
    ? `<label><input type="radio" name="kind" value="github"> Post as GitHub comment</label>`
    : "";
  return `<form class="cform" data-object="${esc(objectRef)}">
<label>Comment as</label><select name="author" data-author-picker>${authorOptions}</select>
<label>Operator token <span class="muted">(needed for verified Drew/Muse posts after setup; saved only in this browser)</span></label><input type="password" name="operatorToken" autocomplete="off" placeholder="Paste operator token when issued">
<button type="button" data-forget-operator-token>Forget saved token</button>
<label>Kind</label><div class="kinds">
<label><input type="radio" name="kind" value="note" checked> Note — read when this is next picked up</label>
<label><input type="radio" name="kind" value="message"> ✉ Message to Muse — the orchestrator triages it on its next pass</label>
${ghKind}</div>
${ghTarget}
<label>Comment</label><textarea name="body" rows="3" required placeholder="Write a comment…"></textarea>
<button type="submit">Post comment</button><div class="err"></div></form>`;
}

/** A full thread + form for one object ref (discuss page, item page). */
export function commentThreadHtml(
  objectRef: string,
  comments: DashComment[],
  opts: CommentFormOpts,
  heading = "Dashboard comments",
): string {
  const list = comments.length
    ? comments.map((c) => dashCommentHtml(c)).join("")
    : `<p class="muted">No dashboard comments yet.</p>`;
  return `<h2>${esc(heading)}</h2>${list}${commentFormHtml(objectRef, opts)}`;
}

// --- Dashboard actions (#1152/#1154/#1156) --------------------------------
// Forms post DashActions to the actions store (src/actions.ts); the
// fleet consumers apply them and report back. Nothing here writes
// anywhere else — a control renders the REQUEST and the store's state
// (pending / applied / rejected), never a pretend-completed change.

/** "12m ago" style relative stamp for action badges. */
export function relTime(iso: string, now = Date.now()): string {
  const ms = now - Date.parse(iso);
  if (Number.isNaN(ms)) return iso;
  if (ms < 60_000) return "just now";
  const m = Math.floor(ms / 60_000);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

const VERB_BADGE_LABELS: Record<string, string> = {
  acknowledge: "acknowledge",
  snooze: "snooze",
  escalate: "escalate",
  "de-escalate": "de-escalate",
  answer: "answer",
  delegate: "delegate",
  dismiss: "dismiss",
  "mark-done": "already done",
  restore: "restore",
  "set-needs-order": "card order",
  "approve-proposal": "approve proposal",
  "reject-proposal": "reject proposal",
  "request-status": "status change",
  "set-delegation-default": "standing default",
  poke: "poke",
  stop: "stop",
};

function pendingBadgeLabel(a: DashAction): string {
  if (a.verb === "request-status") return `status → ${a.payload.status ?? "?"} requested`;
  if (a.verb === "snooze") return `snooze until ${a.payload.until ? fmtDate(a.payload.until) : "?"} requested`;
  if (a.verb === "delegate") return `delegate (${a.payload.mode ?? "?"}) requested`;
  return `${VERB_BADGE_LABELS[a.verb] ?? a.verb} requested`;
}

/**
 * The visible state of an object's actions: every still-pending action
 * as a ⏳ badge, plus the latest applied/rejected outcome per verb
 * family (snooze / status / acknowledge / tier / delegation) so the
 * user can SEE that an action landed — or that it was rejected, with
 * the consumer's note.
 */
export function actionBadgesHtml(actions: DashAction[] | undefined, now = Date.now()): string {
  if (!actions?.length) return "";
  const badges: string[] = [];
  for (const a of actions) {
    if (a.state === "new") {
      badges.push(
        `<span class="badge act-pending" title="Action ${esc(a.id)} — waiting for the consuming fleet job's next run">⏳ ${esc(pendingBadgeLabel(a))} · ${esc(relTime(a.createdAt, now))}</span>`,
      );
    }
  }
  const snooze = latestInFamily(actions, "snooze");
  if (snooze?.state === "applied" && snooze.payload.until && Date.parse(snooze.payload.until) > now) {
    badges.push(
      `<span class="badge act-snoozed" title="Snooze applied by ${esc(snooze.appliedByRun ?? "a fleet run")}">💤 snoozed until ${esc(fmtDate(snooze.payload.until))}</span>`,
    );
  }
  const status = latestInFamily(actions, "status");
  if (status?.state === "applied") {
    badges.push(
      `<span class="badge act-applied">✓ status → ${esc(status.payload.status ?? "")} applied · ${esc(relTime(status.appliedAt ?? status.createdAt, now))}</span>`,
    );
  } else if (status?.state === "rejected") {
    badges.push(
      `<span class="badge act-rejected" title="${esc(status.resultNote ?? "")}">✗ status → ${esc(status.payload.status ?? "")} rejected${status.resultNote ? ` — ${esc(status.resultNote)}` : ""}</span>`,
    );
  }
  const ack = latestInFamily(actions, "ack");
  if (ack?.state === "applied") {
    badges.push(
      `<span class="badge act-applied">✓ acknowledged · ${esc(relTime(ack.appliedAt ?? ack.createdAt, now))}</span>`,
    );
  }
  const tier = latestInFamily(actions, "tier");
  if (tier?.state === "applied") {
    badges.push(
      `<span class="badge act-applied">✓ moved to ${tier.payload.tier === "needs-drew" ? "Needs User" : "Needs Orchestrator"} · ${esc(relTime(tier.appliedAt ?? tier.createdAt, now))}</span>`,
    );
  } else if (tier?.state === "rejected") {
    badges.push(
      `<span class="badge act-rejected" title="${esc(tier.resultNote ?? "")}">✗ ${esc(VERB_BADGE_LABELS[tier.verb] ?? tier.verb)} rejected${tier.resultNote ? ` — ${esc(tier.resultNote)}` : ""}</span>`,
    );
  }
  const delegation = latestInFamily(actions, "delegation");
  if (delegation?.state === "applied") {
    badges.push(
      `<span class="badge act-applied">✓ delegated (${esc(delegation.payload.mode ?? "")}) · ${esc(relTime(delegation.appliedAt ?? delegation.createdAt, now))}</span>`,
    );
  } else if (delegation?.state === "rejected") {
    badges.push(
      `<span class="badge act-rejected" title="${esc(delegation.resultNote ?? "")}">✗ delegate rejected${delegation.resultNote ? ` — ${esc(delegation.resultNote)}` : ""}</span>`,
    );
  }
  return badges.length ? ` <span class="actbadges">${badges.join(" ")}</span>` : "";
}

export interface ActionFormOpts {
  /** Author identities for the picker (standing + registry sessions). */
  identities: AuthorIdentity[];
  /** When set, every control renders disabled with this reason shown. */
  disabledReason?: string;
}

/** Stable DOM id for one card, from its object ref (#1444 fragments
 *  patch sections; #1443's in-place feedback finds cards by ref). */
export function cardDomId(objectRef: string): string {
  return `card-${objectRef.replace(/[^a-zA-Z0-9]+/g, "-")}`;
}

/** The card's action history (#1445): every recorded action with its
 *  state — the audit trail the store keeps, readable on the card. */
export function actionHistoryHtml(actions: DashAction[] | undefined): string {
  if (!actions?.length) return `<p class="muted">No actions recorded on this card yet.</p>`;
  return [...actions]
    .reverse()
    .map((a) => {
      const bits = [
        `<strong>${esc(VERB_BADGE_LABELS[a.verb] ?? a.verb)}</strong>`,
        a.state === "new"
          ? "⏳ pending"
          : a.state === "applied"
            ? `✓ applied${a.appliedByRun ? ` by ${esc(a.appliedByRun)}` : ""}`
            : a.state === "rejected"
              ? "✗ rejected"
              : "superseded",
        esc(relTime(a.createdAt)),
        a.payload.reason ? `— ${esc(a.payload.reason)}` : "",
        a.payload.until ? `until ${esc(fmtDate(a.payload.until))}` : "",
        a.resultNote ? `· ${esc(a.resultNote)}` : "",
      ].filter(Boolean);
      return `<div class="histline">${bits.join(" ")}</div>`;
    })
    .join("");
}

/**
 * The per-card comment box (#1446): the card's existing dashboard
 * comments with their queued/posted state, plus a compact form. The
 * form posts kind "message" (the orchestrator's queue input); the
 * "also post publicly" checkbox adds a kind "github" comment when
 * the card carries a gh: ref — posted later by the orchestrator,
 * never by the aggregator (no GitHub credentials here).
 */
export function cardCommentHtml(
  objectRef: string,
  ghRef: RefKey | undefined,
  comments: DashComment[] | undefined,
  opts: CommentFormOpts,
): string {
  const list = comments?.length
    ? comments.map((c) => dashCommentHtml(c)).join("")
    : `<p class="muted">No comments on this card yet.</p>`;
  const authorOptions = authorOptionsHtml(opts.identities);
  const ghBox = ghRef
    ? `<label><input type="checkbox" name="alsoGithub" value="1"> Also post publicly to the GitHub issue (${esc(ghRef)})</label>`
    : `<span class="muted">No GitHub issue on this card — the comment stays internal.</span>`;
  return `<div class="cbox"><details><summary>💬 Comments for the orchestrator${comments?.length ? ` (${comments.length})` : ""}</summary>${list}<form class="cform" data-object="${esc(objectRef)}"${ghRef ? ` data-ghref="${esc(ghRef)}"` : ""}>
<input type="hidden" name="kind" value="message">
<label>Comment as</label><select name="author" data-author-picker>${authorOptions}</select>
<label>Comment</label><textarea name="body" rows="2" required placeholder="Note for the orchestrator…"></textarea>
<div class="kinds">${ghBox}</div>
<button type="submit">Send comment</button><div class="err"></div></form></details></div>`;
}

/** One-tap dismissal reasons (#1449): the common resolutions. */
export const DISMISS_REASONS = ["Already handled", "No longer relevant", "Duplicate", "Won't do"] as const;
/** One-tap de-escalation reasons (#1449): Drew's "send why" chips. */
export const DEESCALATE_REASONS = [
  "Orchestrator should have handled this",
  "Passing to orchestrator this time",
  "Need more info",
  "Need recommendations",
] as const;

/**
 * The quick-action bar (#1449): one-tap resolutions with the reason
 * attached, writing the existing action vocabulary. Chips disable
 * while their verb is pending, so a resolution can't be double-sent.
 */
export function quickBarHtml(objectRef: string, tier: "needs-drew" | "needs-orchestrator", cs: CardState): string {
  const chip = (
    verb: DashAction["verb"],
    label: string,
    hidden: string,
    opts: { primary?: boolean; disabled?: boolean; sentLabel?: string } = {},
  ): string =>
    aformHtml({
      kind: "triage",
      verb,
      objectRef,
      inner: `${hidden}<button type="submit" class="qchip${opts.primary ? " primary" : ""}"${opts.disabled ? " disabled" : ""}>${esc(opts.disabled && opts.sentLabel ? opts.sentLabel : label)}</button>`,
    }).replace('class="aform"', 'class="aform qform"');
  const donePending = pendingVerb(cs, "mark-done");
  const dismissPending = pendingVerb(cs, "dismiss");
  const movePending = pendingVerb(cs, "de-escalate") ?? pendingVerb(cs, "escalate");
  const parts: string[] = [
    chip("mark-done", "Already done", "", {
      primary: true,
      disabled: !!donePending,
      sentLabel: "✓ Already done · sent",
    }),
  ];
  for (const reason of DISMISS_REASONS) {
    parts.push(
      chip("dismiss", `Dismiss: ${reason}`, `<input type="hidden" name="reason" value="${esc(reason)}">`, {
        disabled: !!dismissPending,
        sentLabel: "Dismiss · sent",
      }),
    );
  }
  if (tier === "needs-drew") {
    for (const reason of DEESCALATE_REASONS) {
      parts.push(
        chip(
          "de-escalate",
          reason,
          `<input type="hidden" name="tier" value="needs-orchestrator"><input type="hidden" name="reason" value="${esc(reason)}">`,
          { disabled: !!movePending, sentLabel: "Sent to orchestrator" },
        ),
      );
    }
  }
  const note = donePending || dismissPending || movePending ? ` <span class="muted">⏳ sent · pending</span>` : "";
  return `<div class="quickbar"><span class="qlabel">Quick actions — one tap, reason attached${note}</span>${parts.join("")}</div>`;
}

export function authorSelectHtml(identities: AuthorIdentity[]): string {
  return `<div class="ahead"><label>Act as</label><select name="author" data-author-picker>${authorOptionsHtml(identities)}</select></div>`;
}

interface AFormSpec {
  kind: DashAction["kind"];
  verb: DashAction["verb"];
  objectRef: string;
  targetRef?: RefKey;
  sessionId?: string;
  sessionSource?: string;
  supersedes?: string;
  /** Inner controls (inputs/buttons), already escaped HTML. */
  inner: string;
}

export function aformHtml(spec: AFormSpec): string {
  const data = [
    `data-kind="${esc(spec.kind)}"`,
    `data-verb="${esc(spec.verb)}"`,
    `data-object="${esc(spec.objectRef)}"`,
    `data-token="${esc(newClientToken())}"`,
    spec.targetRef ? `data-target-ref="${esc(spec.targetRef)}"` : "",
    spec.sessionId ? `data-session-id="${esc(spec.sessionId)}"` : "",
    spec.sessionSource ? `data-session-source="${esc(spec.sessionSource)}"` : "",
    spec.supersedes ? `data-supersedes="${esc(spec.supersedes)}"` : "",
  ]
    .filter(Boolean)
    .join(" ");
  return `<form class="aform" ${data}>${spec.inner}<div class="err"></div></form>`;
}

/** Wrap a card's action forms: one author picker + the disabled gate. */
export function actboxHtml(opts: ActionFormOpts, forms: string): string {
  const inner = `${authorSelectHtml(opts.identities)}${forms}`;
  return opts.disabledReason
    ? `<div class="actbox"><p class="muted">⚠ Actions unavailable — ${esc(opts.disabledReason)}</p><fieldset disabled>${inner}</fieldset></div>`
    : `<div class="actbox">${inner}</div>`;
}

function snoozeFieldsHtml(): string {
  const now = new Date();
  const tonight = new Date(now);
  tonight.setHours(23, 59, 0, 0);
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  tomorrow.setHours(9, 0, 0, 0);
  const nextWeek = new Date(now);
  nextWeek.setDate(nextWeek.getDate() + 7);
  nextWeek.setHours(9, 0, 0, 0);
  return `<select name="untilPreset" aria-label="Snooze until"><option value="${tonight.toISOString()}">until tonight</option><option value="${tomorrow.toISOString()}">until tomorrow 9:00</option><option value="${nextWeek.toISOString()}">until next week</option></select><input type="datetime-local" name="untilCustom" aria-label="Snooze until (custom)">`;
}

/**
 * The #1152 triage control set for one tier item: acknowledge (a
 * non-resolving "seen" record), snooze (presets + custom), and the
 * tier move toward the OTHER tier (de-escalate on Needs User cards,
 * escalate on Needs Orchestrator cards), each with an optional reason.
 * The answer control lives with the item's own surface (session cards
 * carry the session answer; #1291 adds the tier answer row).
 */
export function triageActionsHtml(
  objectRef: string,
  tier: "needs-drew" | "needs-orchestrator",
  opts: ActionFormOpts,
  extraForms = "",
  cs: CardState = cardState(undefined),
): string {
  // #1443: a control whose verb is already pending (or, for
  // acknowledge, already in effect) renders disabled — repeat clicks
  // cannot pile up duplicate pending actions, and the card SAYS so.
  const ackPending = pendingVerb(cs, "acknowledge");
  const snoozePending = pendingVerb(cs, "snooze");
  const moveVerb = tier === "needs-drew" ? "de-escalate" : "escalate";
  const movePending = pendingVerb(cs, moveVerb);
  const move =
    tier === "needs-drew"
      ? aformHtml({
          kind: "triage",
          verb: "de-escalate",
          objectRef,
          inner: `<input type="hidden" name="tier" value="needs-orchestrator"><input type="text" name="reason" placeholder="Why? (optional free text)" aria-label="De-escalate reason"><button type="submit"${movePending ? " disabled" : ""}>${movePending ? "De-escalate · sent · pending" : "De-escalate to Needs Orchestrator"}</button>`,
        })
      : aformHtml({
          kind: "triage",
          verb: "escalate",
          objectRef,
          inner: `<input type="hidden" name="tier" value="needs-drew"><input type="text" name="reason" placeholder="Why? (optional free text)" aria-label="Escalate reason"><button type="submit"${movePending ? " disabled" : ""}>${movePending ? "Escalate · sent · pending" : "Escalate to Needs User"}</button>`,
        });
  const ackInner = ackPending
    ? `<button type="submit" class="primary" disabled>⏳ Acknowledge sent · pending</button><span class="why">Recorded — the fleet applies it on its next pass.</span>`
    : cs.acked
      ? `<button type="submit" class="primary" disabled>✓ Acknowledged</button><span class="why">You have seen this — it stays until a resolving action or a new signal.</span>`
      : `<button type="submit" class="primary">Acknowledge</button><span class="why">Records that you have seen this — it stays until a resolving action or a new signal.</span>`;
  const forms = [
    quickBarHtml(objectRef, tier, cs),
    aformHtml({
      kind: "triage",
      verb: "acknowledge",
      objectRef,
      inner: ackInner,
    }),
    aformHtml({
      kind: "triage",
      verb: "snooze",
      objectRef,
      inner: `${snoozeFieldsHtml()}<button type="submit"${snoozePending ? " disabled" : ""}>${snoozePending ? "Snooze · sent · pending" : "Snooze"}</button>`,
    }),
    move,
  ].join("");
  return actboxHtml(opts, forms + extraForms);
}

// --- Needs-user action layer (#1291) ---------------------------------------
// Everything in this block renders ONLY what the published item and
// the actions store actually say — why-class, options, survey,
// delegation state, proposal, inability. No decorative states: a
// control exists exactly when its verb can be posted and consumed.

const WHY_USER_LABELS: Record<WhyUserClass, string> = {
  decision: "Decision",
  "missing-info": "Missing Info",
  approval: "Approval",
  access: "Access",
  policy: "Policy",
};

const DELEGATION_MODE_LABELS: Record<DelegationMode, string> = {
  user: "With You",
  "first-pass": "First Pass",
  auto: "Auto",
};

export function whyUserBadge(item: TierItem): string {
  if (!item.whyUser) return "";
  return ` <span class="badge whyuser ${esc(item.whyUser)}" title="Why this needs the user: ${esc(WHY_USER_LABELS[item.whyUser])}">${esc(WHY_USER_LABELS[item.whyUser])}</span>`;
}

function questionFieldHtml(q: TierItemQuestion, answeredValue?: string | string[], answerAction?: DashAction): string {
  const name = `answers:${q.id}`;
  const answered = answeredValue !== undefined;
  const disabled = answered ? " disabled" : "";
  let field: string;
  if (q.type === "single" || q.type === "yesno") {
    const opts = q.options ?? [
      { id: "yes", label: "Yes" },
      { id: "no", label: "No" },
    ];
    field = `<span class="qopts">${opts
      .map(
        (o) =>
          `<label><input type="radio" name="${esc(name)}" value="${esc(o.id)}"${answeredValue === o.id ? " checked" : ""}${disabled}> ${esc(o.label)}</label>`,
      )
      .join("")}</span>`;
  } else if (q.type === "multi") {
    const chosen = Array.isArray(answeredValue) ? answeredValue : answeredValue ? [answeredValue] : [];
    field = `<span class="qopts">${(q.options ?? [])
      .map(
        (o) =>
          `<label><input type="checkbox" name="${esc(name)}" value="${esc(o.id)}"${chosen.includes(o.id) ? " checked" : ""}${disabled}> ${esc(o.label)}</label>`,
      )
      .join("")}</span>`;
  } else {
    field = `<input type="text" name="${esc(name)}" aria-label="${esc(q.prompt)}" value="${esc(typeof answeredValue === "string" ? answeredValue : "")}"${disabled}>`;
  }
  // #1451: answered state rides the card — a partial submit records
  // what it covered and the still-open required questions say so.
  const state = answered
    ? ` <span class="qanswered">✓ answered: ${esc(Array.isArray(answeredValue) ? answeredValue.join(", ") : (answeredValue ?? ""))}${answerAction ? ` · ${answerAction.state === "new" ? "pending" : "applied"} · action ${esc(answerAction.id)}` : ""}</span>`
    : q.required
      ? ` <span class="qremaining">still needed</span>`
      : "";
  return `<div class="qblock"><strong>${esc(q.prompt)}</strong>${q.required ? "" : ` <span class="muted">(optional)</span>`}${state}<br>${field}<br><span class="muted">Needed: ${esc(q.covers)}</span></div>`;
}

/** The latest applied reject-proposal on an item, when one exists —
 *  its reason rides the next proposal's box (#1293). */
function lastRejection(actions: DashAction[] | undefined): DashAction | undefined {
  return (actions ?? []).filter((a) => a.verb === "reject-proposal" && a.state === "applied").at(-1);
}

/**
 * The needs-user body block for one card: summary, the what-happens-
 * if-this-sits line, the delegation status line, and the proposal /
 * inability boxes (#1291/#1293). Pure content — the forms live in
 * needsUserFormsHtml, inside the card's actbox.
 */
export function needsUserBodyHtml(item: TierItem, actions: DashAction[] | undefined, now = Date.now()): string {
  const parts: string[] = [];
  if (item.summary) parts.push(`<br><span>${esc(item.summary)}</span>`);
  if (item.ifIgnored)
    parts.push(`<br><span class="muted"><strong>If nothing happens:</strong> ${esc(item.ifIgnored)}</span>`);
  const d = item.delegation;
  if (d) {
    const rejected = lastRejection(actions);
    const statusBits = [
      `Delegation: <strong>${esc(DELEGATION_MODE_LABELS[d.mode])}</strong>`,
      d.status === "idle"
        ? "not started"
        : d.status === "working"
          ? "the orchestrator is working it"
          : d.status === "proposal-ready"
            ? "proposal ready for your review"
            : d.status === "unable"
              ? "returned — the orchestrator could not resolve it"
              : "done",
      d.runId ? `run ${esc(d.runId)}` : "",
      `updated ${esc(relTime(d.updatedAt, now))}`,
    ].filter(Boolean);
    parts.push(`<br><span class="muted">${statusBits.join(" · ")}</span>`);
    if (rejected && d.mode === "first-pass" && d.status !== "proposal-ready")
      parts.push(
        `<br><span class="muted">Previous proposal rejected: ${esc(rejected.payload.reason ?? "")} — a new first pass must differ materially or say why it cannot.</span>`,
      );
    if (d.status === "proposal-ready" && d.proposal) {
      if (proposalExpired(d, now)) {
        parts.push(
          `<div class="propbox expired"><strong>Proposal expired</strong> — drafted ${esc(fmtDate(d.proposal.at))} by run ${esc(d.runId ?? "unknown")}; it is no longer actionable. Delegate a fresh first pass below.<br><strong>${esc(d.proposal.summary)}</strong></div>`,
        );
      } else {
        parts.push(
          `<div class="propbox"><strong>First-pass proposal</strong>${rejected ? ` — second first pass; previous rejected: ${esc(rejected.payload.reason ?? "")}` : ""}<br><strong>${esc(d.proposal.summary)}</strong>${
            d.proposal.detail ? `<br>${esc(d.proposal.detail)}` : ""
          }${
            d.proposal.proposedChanges?.length
              ? `<br>Proposed changes:<ul>${d.proposal.proposedChanges.map((c) => `<li>${esc(c)}</li>`).join("")}</ul>`
              : ""
          }<span class="muted">Drafted ${esc(fmtDate(d.proposal.at))} — nothing is applied unless you approve; approval takes effect on the next orchestration pass.</span></div>`,
        );
      }
    }
    if (d.status === "unable" && d.unable) {
      parts.push(
        `<div class="unablebox"><strong>The orchestrator could not resolve this</strong> — ${esc(d.unable.reason)}<br>Tried:<ul>${d.unable.whatItTried.map((t) => `<li>${esc(t)}</li>`).join("")}</ul>Needs: ${esc(d.unable.whatItNeeds)}<br><span class="muted">Declared ${esc(fmtDate(d.unable.at))} by run ${esc(d.runId ?? "unknown")}.</span></div>`,
      );
    }
  }
  // #1293: a pending approve/reject whose proposal has since changed
  // renders as no longer applicable, not as a live request.
  const pendingProposalAction = (actions ?? []).some(
    (a) => a.state === "new" && (a.verb === "approve-proposal" || a.verb === "reject-proposal"),
  );
  if (pendingProposalAction) {
    const live = proposalIdentity(item);
    const stillLive = (actions ?? []).some(
      (a) =>
        a.state === "new" &&
        (a.verb === "approve-proposal" || a.verb === "reject-proposal") &&
        live !== null &&
        a.payload.proposalRunId === live.runId &&
        a.payload.proposalUpdatedAt === live.updatedAt,
    );
    if (!stillLive)
      parts.push(
        `<br><span class="muted">⏳ A pending proposal action is no longer applicable — the proposal changed since it was recorded; the pass will reject it with that note.</span>`,
      );
  }
  return parts.join("");
}

/**
 * The needs-user forms for one card (they join the card's actbox):
 * the item's own options as answer buttons, the survey as one answer
 * form, a free-text answer when the item offers neither, delegation
 * mode controls, proposal approve/reject, and dismiss (#1291).
 */
export function needsUserFormsHtml(
  item: TierItem,
  objectRef: string,
  now = Date.now(),
  cs: CardState = cardState(undefined),
): string {
  const forms: string[] = [];
  // Older option actions predate the canonical question pointer. Resolve
  // them from the currently published item so already-answered cards
  // cannot reopen the same question after this deploy.
  const answered = new Map(cs.answered);
  const answerActionsByQuestion = new Map(cs.answerActionsByQuestion);
  for (const [optionId, action] of cs.answeredOptionActionsById) {
    const questionId = questionIdForOption(item, optionId);
    if (questionId && !answered.has(questionId)) {
      answered.set(questionId, optionId);
      answerActionsByQuestion.set(questionId, action);
    }
  }
  for (const opt of item.options ?? []) {
    const questionId = questionIdForOption(item, opt.id);
    const questionValue = questionId ? answered.get(questionId) : undefined;
    const answerAction = questionId ? answerActionsByQuestion.get(questionId) : cs.answeredOptionAction;
    const locked = questionId ? questionValue !== undefined : cs.answeredOptionActionsById.size > 0;
    const picked = cs.answeredOptionId === opt.id || (questionId !== undefined && questionValue === opt.id);
    forms.push(
      aformHtml({
        kind: "triage",
        verb: "answer",
        objectRef,
        inner: `<input type="hidden" name="optionId" value="${esc(opt.id)}"><button type="submit" class="primary"${locked ? " disabled" : ""}>${esc(opt.label)}${locked ? ` ✓ answered${answerAction?.state === "new" ? " · pending" : ""}` : ""}</button>${locked && answerAction ? `<span class="muted">Answer action ${esc(answerAction.id)}</span>` : ""}${
          opt.description ? `<span class="why">${esc(opt.description)}</span>` : ""
        }${opt.effect ? `<span class="effect">Effect: ${esc(opt.effect)}</span>` : ""}`,
      }),
    );
  }
  if (item.questions?.length) {
    const remaining = item.questions.filter((q) => q.required && !answered.has(q.id));
    const allDone = remaining.length === 0 && item.questions.some((q) => answered.has(q.id));
    const hasUnanswered = item.questions.some((q) => !answered.has(q.id));
    forms.push(
      aformHtml({
        kind: "triage",
        verb: "answer",
        objectRef,
        inner: `${item.questions.map((q) => questionFieldHtml(q, answered.get(q.id), answerActionsByQuestion.get(q.id))).join("")}<button type="submit" class="primary"${hasUnanswered ? "" : " disabled"}>Submit answers</button><span class="why">${
          allDone
            ? "All required questions are answered — the answers are recorded and pending with the orchestrator."
            : `Answer any subset and submit (#1451): answered questions are recorded, and the card keeps showing what remains${remaining.length ? ` — still needed: ${esc(remaining.map((q) => q.id).join(", "))}` : ""}.`
        }</span>`,
      }),
    );
  }
  if (!item.options?.length && !item.questions?.length) {
    forms.push(
      aformHtml({
        kind: "triage",
        verb: "answer",
        objectRef,
        inner: `<input type="text" name="text" placeholder="Your answer / decision" aria-label="Answer" required><button type="submit" class="primary">Answer</button>`,
      }),
    );
  }
  const d = item.delegation;
  if (d) {
    const autoBlocked = item.whyUser === "access" || item.whyUser === "policy";
    const expired = proposalExpired(d, now);
    const modeButton = (mode: DelegationMode, label: string) => {
      if (mode === "auto" && autoBlocked)
        return `<span class="aform"><button type="button" disabled>Delegate: Auto</button><span class="why">${esc(WHY_USER_LABELS[item.whyUser!])} items can never be auto-delegated — they stay with you.</span></span>`;
      const current = d.mode === mode && !(expired && mode === "first-pass");
      return aformHtml({
        kind: "triage",
        verb: "delegate",
        objectRef,
        inner: `<input type="hidden" name="mode" value="${esc(mode)}"><button type="submit"${current ? " disabled" : ""}>${esc(label)}${current ? " (current)" : ""}</button>`,
      });
    };
    forms.push(
      modeButton(
        "first-pass",
        expired && d.mode === "first-pass" ? "Delegate: First Pass (fresh draft)" : "Delegate: First Pass",
      ),
      modeButton("auto", "Delegate: Auto"),
      modeButton("user", "Bring Back to Me"),
    );
    if (d.status === "proposal-ready" && d.proposal && !expired && d.runId) {
      const identity =
        `<input type="hidden" name="proposalRunId" value="${esc(d.runId)}">` +
        `<input type="hidden" name="proposalUpdatedAt" value="${esc(d.updatedAt)}">`;
      forms.push(
        aformHtml({
          kind: "triage",
          verb: "approve-proposal",
          objectRef,
          inner: `${identity}<button type="submit" class="primary">I accept your proposal</button><span class="why">The pass applies it on its next run and reports the application.</span>`,
        }),
        aformHtml({
          kind: "triage",
          verb: "reject-proposal",
          objectRef,
          inner: `${identity}<input type="text" name="reason" placeholder="Why not? (required)" aria-label="Rejection reason" required><button type="submit">No — make another</button>`,
        }),
      );
    }
  }
  forms.push(
    aformHtml({
      kind: "triage",
      verb: "dismiss",
      objectRef,
      inner: `<input type="text" name="reason" placeholder="Reason (required)" aria-label="Dismiss reason" required><button type="submit">Dismiss</button><span class="why">Dismissed items stay out of the queue unless a new material fact appears — then they resurface with the fact cited.</span>`,
    }),
  );
  // A pending delegation action supersedes the stamped mode display:
  // the badges already say so; nothing extra to render here.
  return forms.join("");
}

/** The standing delegation-default toggle (#1290/#1291): the current
 *  effective value from the preference records, with the change form. */
export function delegationDefaultHtml(records: DashAction[] | undefined, opts: ActionFormOpts): string {
  const eff = effectiveStandingAuto(records ?? []);
  const state = eff.standingAuto
    ? `<strong>On</strong> — Decision, Missing Info, and Approval items delegate to Auto by default`
    : `<strong>Off</strong> — items stay with you unless you delegate them per item`;
  const pending = eff.pending
    ? ` <span class="badge act-pending">⏳ change pending — applies on the next orchestration pass</span>`
    : "";
  const form = aformHtml({
    kind: "preference",
    verb: "set-delegation-default",
    objectRef: DELEGATION_DEFAULT_OBJECT_REF,
    inner: `<label>Standing auto <select name="standingAuto"><option value="true"${eff.standingAuto ? " selected" : ""}>On</option><option value="false"${eff.standingAuto ? "" : " selected"}>Off</option></select></label><button type="submit">Save standing default</button><span class="why">Access and Policy items never auto, whatever this says. Per-item delegation always overrides the default. Initial record: the user's decision, 2026-10-27.</span>`,
  });
  return `<div class="card"><strong>Standing delegation default</strong> — ${state}.${pending}${actboxHtml(opts, form)}</div>`;
}

/** The Project Status values a status request may target (#910 field).
 * #1198: this set IS the canonical status taxonomy's set — one
 * definition (status-taxonomy.ts), adopted here, never a second copy. */
export const PROJECT_STATUS_VALUES = CANONICAL_STATUSES;

/** The #1154 control: request a Project Status change (never a direct write). */
export function statusRequestHtml(
  objectRef: string,
  targetRef: RefKey,
  currentStatus: string | null,
  opts: ActionFormOpts,
): string {
  const options = PROJECT_STATUS_VALUES.map(
    (s) => `<option value="${esc(s)}"${s === currentStatus ? " selected" : ""}>${esc(s)}</option>`,
  ).join("");
  const form = aformHtml({
    kind: "status-request",
    verb: "request-status",
    objectRef,
    targetRef,
    inner: `<label>Request status change to <select name="status">${options}</select></label><input type="text" name="reason" placeholder="Reason (optional)" aria-label="Status change reason"><button type="submit">Request status change</button><span class="why">A request, not a write — the orchestration pass (the Project's single writer) applies it on its next run.</span>`,
  });
  return actboxHtml(opts, form);
}

/** Which session verbs a source can support, and why not when it can't. */
export function sessionVerbSupport(source: string): { verb: "poke" | "answer" | "stop"; reason?: string }[] {
  if (source === "opencode-sandbox" || source === "opencode-laptop") {
    return [{ verb: "poke" }, { verb: "answer" }, { verb: "stop" }];
  }
  if (source === "claude-code") {
    const reason = "No live control channel for Muse sessions — launch records only";
    return [
      { verb: "poke", reason },
      { verb: "answer", reason },
      { verb: "stop", reason },
    ];
  }
  return [];
}

/** The #1156 control set for one live session (merged registry identity). */
export function sessionActionsHtml(sessionId: string, source: string, opts: ActionFormOpts): string {
  const support = sessionVerbSupport(source);
  if (!support.length) return "";
  const objectRef = sessionObjectRef(sessionId);
  const forms = support
    .map(({ verb, reason }) => {
      if (reason) {
        return `<div class="aform"><button type="button" disabled>${esc(verb === "poke" ? "Poke" : verb === "answer" ? "Answer" : "Stop")}</button><span class="why">${esc(reason)}.</span></div>`;
      }
      if (verb === "poke") {
        return aformHtml({
          kind: "session",
          verb,
          objectRef,
          sessionId,
          sessionSource: source,
          inner: `<button type="submit" class="primary">Poke</button><span class="why">Nudges the session via its owning server.</span>`,
        });
      }
      if (verb === "answer") {
        return aformHtml({
          kind: "session",
          verb,
          objectRef,
          sessionId,
          sessionSource: source,
          inner: `<input type="text" name="text" placeholder="Answer for the waiting session…" aria-label="Answer text" required><button type="submit">Answer</button>`,
        });
      }
      return aformHtml({
        kind: "session",
        verb,
        objectRef,
        sessionId,
        sessionSource: source,
        inner: `<label class="confirm"><input type="checkbox" name="confirm" value="yes"> Confirm stop</label><button type="submit" class="danger">Stop session</button><span class="why">Two-step: tick Confirm stop, then Stop — the owning job terminates the run.</span>`,
      });
    })
    .join("");
  return actboxHtml(opts, forms);
}

/** Terminal registry statuses — a record in one of these is history, not live. */
const TERMINAL_SESSION_STATUSES = new Set([
  "completed",
  "complete",
  "done",
  "failed",
  "error",
  "stopped",
  "terminated",
  "succeeded",
  "cancelled",
]);

/** A registry record counts as live inside the attention currency window (2 h). */
export function sessionRecordIsLive(r: SessionRecord, now = Date.now()): boolean {
  if (TERMINAL_SESSION_STATUSES.has(r.status.toLowerCase())) return false;
  if (!r.lastSeenAt) return false;
  return now - Date.parse(r.lastSeenAt) < 2 * 60 * 60 * 1000;
}

export function coverageHtml(coverage: SourceCoverage[]): string {
  const rows = coverage
    .map(
      (c) =>
        `<li>${c.wired ? "✓" : "⚠"} <strong>${esc(c.label)}</strong> — ${c.wired ? "wired" : "<strong>not wired</strong>"} · <span class="muted">${esc(c.detail)}</span></li>`,
    )
    .join("");
  return `<div class="cov"><strong>Session coverage</strong><ul>${rows}</ul></div>`;
}

export function itemBadge(item: GhItem): string {
  // #1141: fixture-resolved state is invented fallback data — render it
  // hollow (dashed, transparent) and named "unverified" so it can never
  // be mistaken for a live badge on board/roadmap/item/program pages.
  const unverified = item.resolution === "fixture";
  const cls = item.state === "merged" ? "merged" : item.state === "open" ? "open" : "closed";
  const kind = item.kind === "pr" ? (item.state === "merged" ? "PR merged" : "PR") : "issue";
  const draft = item.draft ? " · draft" : "";
  const marker = unverified ? " · unverified" : "";
  const title = unverified ? `${item.title} (unverified — fixture state, not live GitHub data)` : item.title;
  return `<a class="badge ${cls}${unverified ? " unverified" : ""}" href="${esc(item.url)}" title="${esc(title)}">${kind} ${esc(item.ref)}${draft}${marker}</a>`;
}

export function stripHtml(items: Map<RefKey, GhItem>, links: BoardItem["links"]): string {
  const strip = executionStrip(links, items);
  const buckets: [string, string, GhItem[]][] = [
    ["implementing", "implementing", strip.implementing],
    ["review", "review", strip.review],
    ["merged", "merged", strip.merged],
    ["open", "queued", strip.queued],
    ["closed", "closed", strip.closed],
  ];
  const parts: string[] = [];
  for (const [cls, label, list] of buckets) {
    if (list.length === 0) continue;
    parts.push(`<span class="badge ${cls}">${label}: ${list.length}</span>` + list.map(itemBadge).join(" "));
  }
  if (strip.unknown.length) {
    parts.push(`<span class="badge closed">unresolved: ${strip.unknown.map(esc).join(", ")}</span>`);
  }
  return `<div class="strip">${parts.join(" ")}</div>`;
}

export function briefPanelsForRefs(refs: RefKey[], briefs?: Map<RefKey, ItemBrief>): string {
  return [...new Set(refs)]
    .map((ref) => briefs?.get(ref))
    .filter((brief): brief is ItemBrief => Boolean(brief))
    .map(briefPanelHtml)
    .join("");
}

export function briefPanelHtml(brief: ItemBrief): string {
  const ev = (e: { text: string; url: string }) => `<a href="${esc(e.url)}">${esc(e.text)}</a>`;
  const activity = brief.lastActivity.evidence
    ? `${esc(brief.lastActivity.text)} Evidence: ${ev(brief.lastActivity.evidence)}.`
    : esc(brief.lastActivity.text);
  const clash = brief.disagreement
    ? `<br><span>⚠ ${esc(brief.disagreement.text)} Evidence: ${ev(brief.disagreement.evidence)}.</span>`
    : "";
  return `<div class="brief" style="border-left:3px solid #888;padding-left:0.6em;margin-top:0.4em"><strong>Derived brief</strong> <span class="muted">(derived from the projection — not authored)</span><br><span>State: ${esc(brief.state.text)} Evidence: ${ev(brief.state.evidence)}.</span><br><span>Blocker: ${esc(brief.blocker.text)} Evidence: ${ev(brief.blocker.evidence)}.</span><br><span>${activity}</span><br><span>Next: ${esc(brief.nextAction.text)} Evidence: ${ev(brief.nextAction.evidence)}.</span>${clash}</div>`;
}
